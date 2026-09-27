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
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, Mutex, Notify, RwLock};
use tracing::debug;

const STDIN_LOCK_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);
const STDIN_WRITE_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);

type PendingRequests = Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>;

struct PendingRequestGuard {
    pending: PendingRequests,
    request_id: Option<RequestId>,
}

impl PendingRequestGuard {
    fn new(pending: PendingRequests, request_id: RequestId) -> Self {
        Self {
            pending,
            request_id: Some(request_id),
        }
    }

    fn disarm(&mut self) {
        self.request_id = None;
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        if let Ok(mut pending) = self.pending.try_lock() {
            pending.remove(&request_id);
        } else {
            let pending = self.pending.clone();
            tokio::spawn(async move {
                pending.lock().await.remove(&request_id);
            });
        }
    }
}

/// Stateful stdio transport client with pending-request correlation.
///
/// This client tracks inflight server-initiated work so request timeouts can be
/// extended while the application is processing callbacks such as sampling.
pub(crate) struct StdioClient {
    stdin: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: PendingRequests,
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
            stdin: Mutex::new(Box::new(stdin)),
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
        let mut pending_guard = PendingRequestGuard::new(self.pending.clone(), request_id.clone());

        self.send_client_message(&message).await?;

        let wait_timeout = self.timeout_for_wait();
        match tokio::time::timeout(wait_timeout, rx).await {
            Ok(Ok(message)) => {
                pending_guard.disarm();
                Ok(message)
            }
            Ok(Err(_)) => Err("MCP stdio response channel closed.".to_string()),
            Err(_) => Err("Timed out waiting for MCP stdio response.".to_string()),
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
        let payload = serde_json::to_string(message).map_err(|err| err.to_string())?;
        let mut stdin = match tokio::time::timeout(STDIN_LOCK_TIMEOUT, self.stdin.lock()).await {
            Ok(stdin) => stdin,
            Err(_) => return Err("Timed out waiting for MCP stdio stdin lock.".to_string()),
        };

        tokio::time::timeout(STDIN_WRITE_TIMEOUT, stdin.write_all(payload.as_bytes()))
            .await
            .map_err(|_| "Timed out writing MCP stdio client message.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(STDIN_WRITE_TIMEOUT, stdin.write_all(b"\n"))
            .await
            .map_err(|_| "Timed out writing MCP stdio newline.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(STDIN_WRITE_TIMEOUT, stdin.flush())
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
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    enum Failure {
        Write,
        Flush,
    }

    struct FailingWriter(Failure);

    struct StalledWriter;

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            match self.0 {
                Failure::Write => Poll::Ready(Err(io::Error::other("controlled write failure"))),
                Failure::Flush => Poll::Ready(Ok(buffer.len())),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            match self.0 {
                Failure::Write => Poll::Ready(Ok(())),
                Failure::Flush => Poll::Ready(Err(io::Error::other("controlled flush failure"))),
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for StalledWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn test_client(writer: impl AsyncWrite + Send + Unpin + 'static) -> StdioClient {
        StdioClient {
            stdin: Mutex::new(Box::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: AtomicI64::new(0),
            server_details: RwLock::new(None),
            server_id: "test-server".to_string(),
            request_tx: None,
            activity_notify: Arc::new(Notify::new()),
            inflight_server_requests: Arc::new(AtomicI64::new(0)),
        }
    }

    async fn assert_send_failure_cleans_pending(client: &StdioClient, expected: &str) {
        let error = client
            .send_request(RequestFromClient::PingRequest(None))
            .await
            .expect_err("request should fail");

        assert_eq!(error, expected);
        assert!(client.pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stdio_requires_connected_client() {
        let err = send_request(None, RequestFromClient::PingRequest(None))
            .await
            .expect_err("expected missing client error");
        assert_eq!(err, "MCP client not connected.");
    }

    #[tokio::test]
    async fn write_failure_removes_pending_request() {
        let client = test_client(FailingWriter(Failure::Write));

        assert_send_failure_cleans_pending(&client, "controlled write failure").await;
    }

    #[tokio::test]
    async fn flush_failure_removes_pending_request() {
        let client = test_client(FailingWriter(Failure::Flush));

        assert_send_failure_cleans_pending(&client, "controlled flush failure").await;
    }

    #[tokio::test(start_paused = true)]
    async fn stdin_lock_timeout_removes_pending_request() {
        let client = test_client(tokio::io::sink());
        let _stdin = client.stdin.lock().await;

        assert_send_failure_cleans_pending(&client, "Timed out waiting for MCP stdio stdin lock.")
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_write_timeout_removes_pending_request() {
        let client = test_client(StalledWriter);

        assert_send_failure_cleans_pending(&client, "Timed out writing MCP stdio client message.")
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_request_removes_pending_request() {
        let client = test_client(tokio::io::sink());

        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(1),
            client.send_request(RequestFromClient::PingRequest(None)),
        )
        .await;

        assert!(result.is_err());
        tokio::task::yield_now().await;
        assert!(client.pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn successful_response_disarms_pending_guard() {
        let client = test_client(tokio::io::sink());
        let unrelated_id = RequestId::Integer(99);
        let (unrelated_tx, _unrelated_rx) = oneshot::channel();
        client
            .pending
            .lock()
            .await
            .insert(unrelated_id.clone(), unrelated_tx);

        let pending = client.pending.clone();
        tokio::spawn(async move {
            loop {
                if let Some(tx) = pending.lock().await.remove(&RequestId::Integer(0)) {
                    let response = serde_json::from_value(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 0,
                        "result": {}
                    }))
                    .expect("response should deserialize");
                    tx.send(response).expect("request should receive response");
                    return;
                }
                tokio::task::yield_now().await;
            }
        });

        let response = client
            .send_request(RequestFromClient::PingRequest(None))
            .await
            .expect("request should receive response");

        assert!(matches!(response, ServerMessage::Response(_)));
        let pending = client.pending.lock().await;
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key(&unrelated_id));
    }
}
