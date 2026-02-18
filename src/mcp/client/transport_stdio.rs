//! Stdio transport client for MCP servers started as local child processes.
//!
//! Transport expectations:
//! - The configured command must exist and support newline-delimited JSON-RPC
//!   messages on stdin/stdout.
//! - Optional env overrides are applied only to the child process.
//! - Server-initiated requests are forwarded through `McpServerRequest` so the
//!   app can answer sampling/tool callbacks while regular requests are pending.
//!
//! Failure semantics:
//! - Spawn/setup failures return immediate `Err(String)` values.
//! - Request send/wait paths enforce lock, write, and response timeouts.
//! - Response channel closure or malformed stdout messages are treated as
//!   per-request failures without panicking the runtime.

use super::{
    protocol, require_stdio_command, stdio_args, stdio_env, STDIO_REQUEST_TIMEOUT_SECONDS,
    STDIO_SAMPLING_TIMEOUT_MULTIPLIER,
};
use crate::core::config::data::McpServerConfig;
use crate::mcp::events::McpServerRequest;
use rust_mcp_schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, NotificationFromClient, RequestFromClient,
    ResultFromClient, ServerMessage,
};
use rust_mcp_schema::{InitializeRequestParams, InitializeResult, RequestId, RpcError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex, Notify, RwLock};
use tracing::debug;

/// Stateful stdio transport client with pending-request correlation.
///
/// This client tracks inflight server-initiated work so request timeouts can be
/// extended while the application is processing callbacks such as sampling.
pub(crate) struct StdioClient {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>,
    next_request_id: AtomicI64,
    server_details: RwLock<Option<rust_mcp_schema::InitializeResult>>,
    server_id: String,
    request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
    activity_notify: Arc<Notify>,
    inflight_server_requests: Arc<AtomicI64>,
}

impl StdioClient {
    /// Starts the configured MCP server process and wires async readers.
    pub(crate) async fn connect(
        server_id: String,
        config: &McpServerConfig,
        request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
    ) -> Result<Arc<Self>, String> {
        let command = require_stdio_command(config)?;
        let args = stdio_args(config);
        debug!(command = %command, args = ?args, "Starting MCP stdio server");
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(env) = stdio_env(config) {
            cmd.envs(env);
        }

        let mut child = cmd.spawn().map_err(|err| err.to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Unable to retrieve stdin.".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Unable to retrieve stdout.".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Unable to retrieve stderr.".to_string())?;

        let pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let client = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending: pending.clone(),
            next_request_id: AtomicI64::new(0),
            server_details: RwLock::new(None),
            server_id,
            request_tx,
            activity_notify: Arc::new(Notify::new()),
            inflight_server_requests: Arc::new(AtomicI64::new(0)),
        });

        Self::spawn_stdout_reader(
            pending.clone(),
            stdout,
            client.server_id.clone(),
            client.request_tx.clone(),
            client.activity_notify.clone(),
            client.inflight_server_requests.clone(),
        );
        Self::spawn_stderr_drain(stderr);

        tokio::spawn(async move {
            let _ = child.wait().await;
            let mut pending = pending.lock().await;
            pending.clear();
        });

        Ok(client)
    }

    /// Runs initialize/initialized handshake and caches server details.
    pub(crate) async fn initialize(
        &self,
        details: InitializeRequestParams,
    ) -> Result<InitializeResult, String> {
        let response = self
            .send_request(RequestFromClient::InitializeRequest(details))
            .await?;
        let result = protocol::parse_initialize_result(response)?;
        *self.server_details.write().await = Some(result.clone());
        self.send_notification(NotificationFromClient::InitializedNotification(None))
            .await?;
        Ok(result)
    }

    pub(crate) async fn send_request(
        &self,
        request: RequestFromClient,
    ) -> Result<ServerMessage, String> {
        let request_id = self.next_request_id();
        debug!(request_id = ?request_id, "Sending MCP stdio request");
        let message = ClientMessage::from_message(
            MessageFromClient::RequestFromClient(request),
            Some(request_id.clone()),
        )
        .map_err(|err| err.to_string())?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        self.send_client_message(&message).await?;

        let wait_timeout = self.timeout_for_wait();
        match tokio::time::timeout(wait_timeout, rx).await {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&request_id);
                Err("MCP stdio response channel closed.".to_string())
            }
            Err(_) => {
                self.pending.lock().await.remove(&request_id);
                Err("Timed out waiting for MCP stdio response.".to_string())
            }
        }
    }

    pub(crate) async fn send_result(
        &self,
        request_id: RequestId,
        result: ResultFromClient,
    ) -> Result<(), String> {
        debug!(
            server_id = %self.server_id,
            request_id = ?request_id,
            "Preparing MCP stdio result"
        );
        let message = ClientMessage::from_message(
            MessageFromClient::ResultFromClient(result),
            Some(request_id.clone()),
        )
        .map_err(|err| err.to_string())?;
        let result = self.send_client_message(&message).await;
        if result.is_ok() {
            let inflight = self.decrement_inflight();
            debug!(
                request_id = ?request_id,
                inflight_server_requests = inflight,
                "Sent MCP stdio result"
            );
            self.activity_notify.notify_waiters();
        }
        result
    }

    pub(crate) async fn send_error(
        &self,
        request_id: RequestId,
        error: RpcError,
    ) -> Result<(), String> {
        debug!(
            server_id = %self.server_id,
            request_id = ?request_id,
            "Preparing MCP stdio error response"
        );
        let message =
            ClientMessage::from_message(MessageFromClient::Error(error), Some(request_id.clone()))
                .map_err(|err| err.to_string())?;
        let result = self.send_client_message(&message).await;
        if result.is_ok() {
            let inflight = self.decrement_inflight();
            debug!(
                request_id = ?request_id,
                inflight_server_requests = inflight,
                "Sent MCP stdio error response"
            );
            self.activity_notify.notify_waiters();
        }
        result
    }

    async fn send_notification(&self, notification: NotificationFromClient) -> Result<(), String> {
        let message = ClientMessage::from_message(
            MessageFromClient::NotificationFromClient(notification),
            None,
        )
        .map_err(|err| err.to_string())?;
        self.send_client_message(&message).await
    }

    async fn send_client_message(&self, message: &ClientMessage) -> Result<(), String> {
        let lock_timeout = tokio::time::Duration::from_secs(10);
        let write_timeout = tokio::time::Duration::from_secs(10);
        let payload = serde_json::to_string(message).map_err(|err| err.to_string())?;
        let mut stdin = match tokio::time::timeout(lock_timeout, self.stdin.lock()).await {
            Ok(stdin) => stdin,
            Err(_) => return Err("Timed out waiting for MCP stdio stdin lock.".to_string()),
        };

        tokio::time::timeout(write_timeout, stdin.write_all(payload.as_bytes()))
            .await
            .map_err(|_| "Timed out writing MCP stdio client message.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.write_all(b"\n"))
            .await
            .map_err(|_| "Timed out writing MCP stdio newline.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.flush())
            .await
            .map_err(|_| "Timed out flushing MCP stdio client message.".to_string())?
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    fn timeout_for_wait(&self) -> tokio::time::Duration {
        let inflight = self.inflight_server_requests.load(Ordering::SeqCst);
        let multiplier = if inflight > 0 {
            STDIO_SAMPLING_TIMEOUT_MULTIPLIER
        } else {
            1
        };
        tokio::time::Duration::from_secs(STDIO_REQUEST_TIMEOUT_SECONDS * multiplier)
    }

    fn decrement_inflight(&self) -> i64 {
        let mut current = self.inflight_server_requests.load(Ordering::SeqCst);
        while current > 0 {
            match self.inflight_server_requests.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return current - 1,
                Err(next) => current = next,
            }
        }
        current
    }

    fn next_request_id(&self) -> RequestId {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        RequestId::Integer(id)
    }

    fn spawn_stdout_reader(
        pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>,
        stdout: tokio::process::ChildStdout,
        server_id: String,
        request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
        activity_notify: Arc<Notify>,
        inflight_server_requests: Arc<AtomicI64>,
    ) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let value = match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if let Some(items) = value.as_array() {
                    for item in items {
                        if let Ok(message) = serde_json::from_value::<ServerMessage>(item.clone()) {
                            Self::dispatch_message(
                                &pending,
                                message,
                                &server_id,
                                request_tx.as_ref(),
                                &activity_notify,
                                &inflight_server_requests,
                            )
                            .await;
                        }
                    }
                } else if let Ok(message) = serde_json::from_value::<ServerMessage>(value) {
                    Self::dispatch_message(
                        &pending,
                        message,
                        &server_id,
                        request_tx.as_ref(),
                        &activity_notify,
                        &inflight_server_requests,
                    )
                    .await;
                }
            }
        });
    }

    fn spawn_stderr_drain(stderr: tokio::process::ChildStderr) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = reader.next_line().await {}
        });
    }

    async fn dispatch_message(
        pending: &Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>,
        message: ServerMessage,
        server_id: &str,
        request_tx: Option<&mpsc::UnboundedSender<McpServerRequest>>,
        activity_notify: &Notify,
        inflight_server_requests: &AtomicI64,
    ) {
        match &message {
            ServerMessage::Response(response) => {
                if let Some(tx) = pending.lock().await.remove(&response.id) {
                    let _ = tx.send(message);
                }
            }
            ServerMessage::Error(error) => {
                if let Some(id) = error.id.as_ref() {
                    if let Some(tx) = pending.lock().await.remove(id) {
                        let _ = tx.send(message);
                    }
                }
            }
            ServerMessage::Request(request) => {
                let _ = inflight_server_requests.fetch_add(1, Ordering::SeqCst);
                activity_notify.notify_waiters();
                if let Some(tx) = request_tx {
                    let _ = tx.send(McpServerRequest {
                        server_id: server_id.to_string(),
                        request: request.clone(),
                    });
                }
            }
            ServerMessage::Notification(_) => {
                activity_notify.notify_waiters();
            }
        }
    }
}

pub(crate) async fn send_request(
    client: Option<Arc<StdioClient>>,
    request: RequestFromClient,
) -> Result<ServerMessage, String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_request(request).await
}

pub(crate) async fn send_result(
    client: Option<Arc<StdioClient>>,
    request_id: RequestId,
    result: ResultFromClient,
) -> Result<(), String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_result(request_id, result).await
}

pub(crate) async fn send_error(
    client: Option<Arc<StdioClient>>,
    request_id: RequestId,
    error: RpcError,
) -> Result<(), String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_error(request_id, error).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stdio_requires_connected_client() {
        let err = send_request(None, RequestFromClient::PingRequest(None))
            .await
            .expect_err("expected missing client error");
        assert_eq!(err, "MCP client not connected.");
    }
}
