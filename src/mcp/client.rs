use crate::cli::refresh_oauth_grant_if_needed;
use crate::core::app::session::ToolCallRequest;
use crate::core::config::data::{Config, McpServerConfig};
use crate::core::mcp_auth::McpTokenStore;
use crate::mcp::events::McpServerRequest;
use futures_util::StreamExt;
use rust_mcp_schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, NotificationFromClient, RequestFromClient,
    ResultFromClient, ServerMessage,
};
use rust_mcp_schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientSampling,
    GetPromptRequestParams, GetPromptResult, Implementation, InitializeRequestParams,
    InitializeResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult,
    RequestId, RpcError, ServerCapabilities, LATEST_PROTOCOL_VERSION,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex, Notify, RwLock};
use tracing::debug;

const MCP_METHOD_NOT_FOUND: i64 = -32601;
const MCP_MAX_TOOL_LIST: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    StreamableHttp,
    Stdio,
}

impl McpTransportKind {
    pub fn from_config(config: &McpServerConfig) -> Result<Self, String> {
        let transport = config
            .transport
            .as_deref()
            .unwrap_or("streamable-http")
            .to_ascii_lowercase();
        match transport.as_str() {
            "streamable-http" | "streamable_http" | "http" => Ok(McpTransportKind::StreamableHttp),
            "stdio" => Ok(McpTransportKind::Stdio),
            other => Err(format!("Unsupported MCP transport: {}", other)),
        }
    }
}

fn require_http_base_url(config: &McpServerConfig) -> Result<String, String> {
    config
        .base_url
        .clone()
        .ok_or_else(|| "MCP base_url is required for HTTP transports.".to_string())
}

fn require_stdio_command(config: &McpServerConfig) -> Result<String, String> {
    config
        .command
        .clone()
        .ok_or_else(|| "MCP command is required for stdio transport.".to_string())
}

fn stdio_args(config: &McpServerConfig) -> Vec<String> {
    config.args.clone().unwrap_or_default()
}

fn stdio_env(config: &McpServerConfig) -> Option<HashMap<String, String>> {
    config.env.clone()
}

const STDIO_REQUEST_TIMEOUT_SECONDS: u64 = 60;
const STDIO_SAMPLING_TIMEOUT_MULTIPLIER: u64 = 5;

struct StdioClient {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>,
    next_request_id: AtomicI64,
    server_details: RwLock<Option<rust_mcp_schema::InitializeResult>>,
    server_id: String,
    request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
    activity_notify: Arc<Notify>,
    inflight_server_requests: Arc<AtomicI64>,
}

#[derive(Default)]
struct SseLineBuffer {
    buffer: Vec<u8>,
}

impl SseLineBuffer {
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        self.drain_lines(false)
    }

    fn finish(&mut self) -> Vec<String> {
        self.drain_lines(true)
    }

    fn drain_lines(&mut self, flush: bool) -> Vec<String> {
        let mut lines = Vec::new();
        let mut search_index = 0;

        while let Some(relative_pos) = self.buffer[search_index..].iter().position(|b| *b == b'\n')
        {
            let newline_index = search_index + relative_pos;
            let mut line_end = newline_index;
            if line_end > search_index && self.buffer[line_end - 1] == b'\r' {
                line_end -= 1;
            }

            let line_bytes = &self.buffer[search_index..line_end];
            if let Ok(text) = std::str::from_utf8(line_bytes) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }

            search_index = newline_index + 1;
        }

        if flush {
            if let Ok(text) = std::str::from_utf8(&self.buffer[search_index..]) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
            self.buffer.clear();
        } else if search_index > 0 {
            self.buffer.drain(..search_index);
        }

        lines
    }
}

impl StdioClient {
    async fn connect(
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
                debug!(
                    server_id = %server_id,
                    response_id = ?response.id,
                    "Received MCP stdio response"
                );
                if let Some(tx) = pending.lock().await.remove(&response.id) {
                    let _ = tx.send(message);
                }
            }
            ServerMessage::Error(error) => {
                debug!(
                    server_id = %server_id,
                    error_id = ?error.id,
                    error_code = error.error.code,
                    "Received MCP stdio error"
                );
                if let Some(id) = error.id.as_ref() {
                    if let Some(tx) = pending.lock().await.remove(id) {
                        let _ = tx.send(message);
                    }
                }
            }
            ServerMessage::Request(request) => {
                let inflight = inflight_server_requests.fetch_add(1, Ordering::SeqCst) + 1;
                debug!(
                    server_id = %server_id,
                    method = %request.method(),
                    request_id = ?request.request_id(),
                    inflight_server_requests = inflight,
                    "Received MCP stdio request"
                );
                activity_notify.notify_waiters();
                if let Some(tx) = request_tx {
                    let _ = tx.send(McpServerRequest {
                        server_id: server_id.to_string(),
                        request: request.clone(),
                    });
                }
            }
            ServerMessage::Notification(_) => {
                debug!(server_id = %server_id, "Received MCP stdio notification");
                activity_notify.notify_waiters();
            }
        }
    }
}

impl StdioClient {
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
}

impl StdioClient {
    fn next_request_id(&self) -> RequestId {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        RequestId::Integer(id)
    }

    async fn send_request(&self, request: RequestFromClient) -> Result<ServerMessage, String> {
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

        let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
        let lock_timeout = tokio::time::Duration::from_secs(10);
        let write_timeout = tokio::time::Duration::from_secs(10);
        let lock_start = std::time::Instant::now();
        debug!(request_id = ?request_id, "Awaiting MCP stdio stdin lock for request");
        let mut stdin = match tokio::time::timeout(lock_timeout, self.stdin.lock()).await {
            Ok(stdin) => stdin,
            Err(_) => {
                return Err("Timed out waiting for MCP stdio stdin lock.".to_string());
            }
        };
        debug!(
            request_id = ?request_id,
            elapsed_ms = lock_start.elapsed().as_millis(),
            bytes = payload.len(),
            "Writing MCP stdio request"
        );
        tokio::time::timeout(write_timeout, stdin.write_all(payload.as_bytes()))
            .await
            .map_err(|_| "Timed out writing MCP stdio request.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.write_all(b"\n"))
            .await
            .map_err(|_| "Timed out writing MCP stdio request newline.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.flush())
            .await
            .map_err(|_| "Timed out flushing MCP stdio request.".to_string())?
            .map_err(|err| err.to_string())?;
        debug!(request_id = ?request_id, "MCP stdio request sent");
        drop(stdin);

        let mut timeout = self.timeout_for_wait();
        let mut deadline = tokio::time::Instant::now() + timeout;
        let mut rx = rx;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                debug!(
                    request_id = ?request_id,
                    timeout_secs = timeout.as_secs(),
                    "MCP stdio request timed out (deadline reached)"
                );
                return Err("MCP stdio request timed out.".to_string());
            }
            let remaining = deadline - now;
            debug!(
                request_id = ?request_id,
                remaining_ms = remaining.as_millis(),
                inflight_server_requests = self.inflight_server_requests.load(Ordering::SeqCst),
                timeout_secs = timeout.as_secs(),
                "Awaiting MCP stdio response"
            );
            tokio::select! {
                result = &mut rx => {
                    return match result {
                        Ok(message) => {
                            debug!(request_id = ?request_id, "MCP stdio response received");
                            Ok(message)
                        }
                        Err(_) => {
                            debug!(request_id = ?request_id, "MCP stdio response channel closed");
                            Err("MCP stdio response channel closed.".to_string())
                        }
                    };
                }
                _ = tokio::time::sleep(remaining) => {
                    debug!(
                        request_id = ?request_id,
                        timeout_secs = timeout.as_secs(),
                        "MCP stdio request timed out (sleep elapsed)"
                    );
                    return Err("MCP stdio request timed out.".to_string());
                }
                _ = self.activity_notify.notified() => {
                    timeout = self.timeout_for_wait();
                    debug!(
                        request_id = ?request_id,
                        timeout_secs = timeout.as_secs(),
                        inflight_server_requests = self.inflight_server_requests.load(Ordering::SeqCst),
                        "MCP stdio timeout reset after server activity"
                    );
                    deadline = tokio::time::Instant::now() + timeout;
                }
            }
        }
    }

    async fn send_notification(&self, notification: NotificationFromClient) -> Result<(), String> {
        let message = ClientMessage::from_message(
            MessageFromClient::NotificationFromClient(notification),
            None,
        )
        .map_err(|err| err.to_string())?;
        let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
        let lock_timeout = tokio::time::Duration::from_secs(10);
        let write_timeout = tokio::time::Duration::from_secs(10);
        let lock_start = std::time::Instant::now();
        debug!(server_id = %self.server_id, "Awaiting MCP stdio stdin lock for notification");
        let mut stdin = match tokio::time::timeout(lock_timeout, self.stdin.lock()).await {
            Ok(stdin) => stdin,
            Err(_) => {
                return Err("Timed out waiting for MCP stdio stdin lock.".to_string());
            }
        };
        debug!(
            server_id = %self.server_id,
            elapsed_ms = lock_start.elapsed().as_millis(),
            bytes = payload.len(),
            "Writing MCP stdio notification"
        );
        tokio::time::timeout(write_timeout, stdin.write_all(payload.as_bytes()))
            .await
            .map_err(|_| "Timed out writing MCP stdio notification.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.write_all(b"\n"))
            .await
            .map_err(|_| "Timed out writing MCP stdio notification newline.".to_string())?
            .map_err(|err| err.to_string())?;
        tokio::time::timeout(write_timeout, stdin.flush())
            .await
            .map_err(|_| "Timed out flushing MCP stdio notification.".to_string())?
            .map_err(|err| err.to_string())?;
        debug!(server_id = %self.server_id, "MCP stdio notification sent");
        Ok(())
    }

    async fn initialize(
        &self,
        details: InitializeRequestParams,
    ) -> Result<InitializeResult, String> {
        let response = self
            .send_request(RequestFromClient::InitializeRequest(details))
            .await?;
        let result = parse_initialize_result(response)?;
        *self.server_details.write().await = Some(result.clone());
        self.send_notification(NotificationFromClient::InitializedNotification(None))
            .await?;
        Ok(result)
    }

    async fn send_result(
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

    async fn send_error(&self, request_id: RequestId, error: RpcError) -> Result<(), String> {
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

    async fn send_client_message(&self, message: &ClientMessage) -> Result<(), String> {
        let lock_timeout = tokio::time::Duration::from_secs(10);
        debug!(server_id = %self.server_id, "Awaiting MCP stdio stdin lock");
        let payload = serde_json::to_string(message).map_err(|err| err.to_string())?;
        let mut stdin = match tokio::time::timeout(lock_timeout, self.stdin.lock()).await {
            Ok(stdin) => stdin,
            Err(_) => {
                return Err("Timed out waiting for MCP stdio stdin lock.".to_string());
            }
        };
        debug!(server_id = %self.server_id, bytes = payload.len(), "Writing MCP stdio client message");
        let write_timeout = tokio::time::Duration::from_secs(10);
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
        debug!(server_id = %self.server_id, "MCP stdio client message sent");
        Ok(())
    }
}

#[derive(Clone)]
pub struct McpServerState {
    pub config: McpServerConfig,
    pub connected: bool,
    pub last_error: Option<String>,
    pub cached_tools: Option<ListToolsResult>,
    pub cached_resources: Option<ListResourcesResult>,
    pub cached_resource_templates: Option<ListResourceTemplatesResult>,
    pub cached_prompts: Option<ListPromptsResult>,
    pub session_id: Option<String>,
    pub auth_header: Option<String>,
    pub server_details: Option<InitializeResult>,
    pub streamable_http_request_id: u64,
    pub event_listener_started: bool,
    client: Option<Arc<StdioClient>>,
}

impl McpServerState {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            connected: false,
            last_error: None,
            cached_tools: None,
            cached_resources: None,
            cached_resource_templates: None,
            cached_prompts: None,
            session_id: None,
            auth_header: None,
            server_details: None,
            streamable_http_request_id: 0,
            event_listener_started: false,
            client: None,
        }
    }

    pub fn allowed_tools(&self) -> Option<&[String]> {
        self.config.allowed_tools.as_deref()
    }

    fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_details
            .as_ref()
            .map(|details| &details.capabilities)
    }

    fn supports_tools(&self) -> bool {
        self.server_capabilities()
            .map(|caps| caps.tools.is_some())
            .unwrap_or(true)
    }

    fn supports_resources(&self) -> bool {
        self.server_capabilities()
            .map(|caps| caps.resources.is_some())
            .unwrap_or(true)
    }

    fn supports_prompts(&self) -> bool {
        self.server_capabilities()
            .map(|caps| caps.prompts.is_some())
            .unwrap_or(true)
    }

    pub fn clear_runtime_state(&mut self) {
        self.connected = false;
        self.last_error = None;
        self.cached_tools = None;
        self.cached_resources = None;
        self.cached_resource_templates = None;
        self.cached_prompts = None;
        self.session_id = None;
        self.auth_header = None;
        self.server_details = None;
        self.streamable_http_request_id = 0;
        self.event_listener_started = false;
        self.client = None;
    }
}

#[derive(Default, Clone)]
pub struct McpClientManager {
    servers: HashMap<String, McpServerState>,
    server_request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
}

enum ListFetch<T> {
    Ok(T, Option<String>),
    MethodNotFound(Option<String>),
    Err(String),
}

macro_rules! paginate_tools_list_with {
    ($fetch_fn:path, ($($arg:expr),*)) => {{
        match $fetch_fn($($arg),*, None).await {
            Ok(Some(mut list)) => {
                let meta = list.meta.take();
                let mut tools = std::mem::take(&mut list.tools);
                let mut next_cursor = list.next_cursor.take();
                let mut error: Option<String> = None;

                if tools.len() >= MCP_MAX_TOOL_LIST {
                    tools.truncate(MCP_MAX_TOOL_LIST);
                } else {
                    while let Some(cursor) = next_cursor.clone() {
                        match $fetch_fn($($arg),*, Some(cursor)).await {
                            Ok(Some(next_list)) => {
                                tools.extend(next_list.tools);
                                next_cursor = next_list.next_cursor;
                                if tools.len() >= MCP_MAX_TOOL_LIST {
                                    tools.truncate(MCP_MAX_TOOL_LIST);
                                    break;
                                }
                            }
                            Ok(None) => {
                                next_cursor = None;
                                break;
                            }
                            Err(message) => {
                                error = Some(message);
                                break;
                            }
                        }
                    }
                }

                match error {
                    Some(message) => Err(message),
                    None => Ok(Some(ListToolsResult {
                        meta,
                        next_cursor,
                        tools,
                    })),
                }
            }
            Ok(None) => Ok(None),
            Err(message) => Err(message),
        }
    }};
}

impl McpClientManager {
    pub fn set_request_sender(&mut self, sender: mpsc::UnboundedSender<McpServerRequest>) {
        self.server_request_tx = Some(sender);
    }

    fn apply_list_fetch<T>(
        server: &mut McpServerState,
        fetch: ListFetch<T>,
        label: &str,
        empty: impl FnOnce() -> T,
        set: impl FnOnce(&mut McpServerState, T),
    ) {
        match fetch {
            ListFetch::Ok(list, session_id) => {
                set(server, list);
                server.last_error = None;
                if let Some(session_id) = session_id {
                    server.session_id = Some(session_id);
                }
            }
            ListFetch::MethodNotFound(session_id) => {
                set(server, empty());
                server.last_error = None;
                if let Some(session_id) = session_id {
                    server.session_id = Some(session_id);
                }
            }
            ListFetch::Err(message) => {
                server.last_error = Some(format!("{label} listing failed: {message}"));
            }
        }
    }

    fn apply_tools_list_result(&mut self, id: &str, result: Result<ListToolsResult, String>) {
        match result {
            Ok(list) => {
                if let Some(server) = self.server_mut(id) {
                    server.cached_tools = Some(list);
                    server.last_error = None;
                }
            }
            Err(message) => {
                if let Some(server) = self.server_mut(id) {
                    server.last_error = Some(format!("Tools listing failed: {message}"));
                }
            }
        }
    }

    pub fn from_config(config: &Config) -> Self {
        let servers = config
            .mcp_servers
            .iter()
            .cloned()
            .map(|server| (server.id.to_ascii_lowercase(), McpServerState::new(server)))
            .collect();
        Self {
            servers,
            server_request_tx: None,
        }
    }

    pub fn servers(&self) -> impl Iterator<Item = &McpServerState> {
        self.servers.values()
    }

    pub fn server(&self, id: &str) -> Option<&McpServerState> {
        self.servers.get(&id.to_ascii_lowercase())
    }

    pub fn server_mut(&mut self, id: &str) -> Option<&mut McpServerState> {
        self.servers.get_mut(&id.to_ascii_lowercase())
    }

    pub async fn connect_all(&mut self, token_store: &McpTokenStore) {
        let ids: Vec<String> = self.servers.keys().cloned().collect();
        for id in ids {
            if let Some(server) = self.servers.get(&id) {
                if !server.config.is_enabled() {
                    continue;
                }
            }
            self.connect_server(&id, token_store).await;
        }
    }

    pub async fn connect_server(&mut self, id: &str, token_store: &McpTokenStore) {
        let oauth_refresh_warning = {
            let Some(server) = self.server(id) else {
                return;
            };
            if !server.config.is_enabled() {
                None
            } else if matches!(
                McpTransportKind::from_config(&server.config),
                Ok(McpTransportKind::StreamableHttp)
            ) {
                match refresh_oauth_grant_if_needed(&server.config.id, token_store).await {
                    Ok(_) => None,
                    Err(err) => Some(format!(
                        "OAuth refresh failed for {}: {}. Re-authenticate with `chabeau mcp oauth add {}`.",
                        server.config.display_name, err, server.config.id
                    )),
                }
            } else {
                None
            }
        };

        let (config, transport_kind, auth_header) = {
            let Some(server) = self.server_mut(id) else {
                return;
            };

            if !server.config.is_enabled() {
                server.connected = false;
                server.client = None;
                return;
            }

            if server.connected && server.client.is_some() {
                return;
            }

            let transport_kind = match McpTransportKind::from_config(&server.config) {
                Ok(kind) => kind,
                Err(err) => {
                    server.last_error = Some(err);
                    server.connected = false;
                    server.client = None;
                    return;
                }
            };

            let auth_header = match transport_kind {
                McpTransportKind::StreamableHttp => {
                    if let Err(err) = require_http_base_url(&server.config) {
                        server.last_error = Some(err);
                        server.connected = false;
                        server.client = None;
                        return;
                    }

                    let token = match token_store.get_token(&server.config.id) {
                        Ok(token) => token,
                        Err(err) => {
                            server.last_error = Some(format!("Token lookup failed: {}", err));
                            server.connected = false;
                            server.client = None;
                            return;
                        }
                    };

                    token.map(|token| format!("Bearer {}", token))
                }
                McpTransportKind::Stdio => {
                    if let Err(err) = require_stdio_command(&server.config) {
                        server.last_error = Some(err);
                        server.connected = false;
                        server.client = None;
                        return;
                    }
                    None
                }
            };

            server.auth_header = auth_header.clone();
            (server.config.clone(), transport_kind, auth_header)
        };

        let request_tx = self.server_request_tx.clone();

        match transport_kind {
            McpTransportKind::StreamableHttp => {
                match self.ensure_streamable_http_session(id).await {
                    Ok(()) => {
                        if let Some(server) = self.server_mut(id) {
                            server.connected = true;
                            server.last_error = oauth_refresh_warning.clone();
                            server.auth_header = auth_header;
                        }
                        if let Some(tx) = request_tx.clone() {
                            if let Some(server) = self.server_mut(id) {
                                if !server.event_listener_started {
                                    server.event_listener_started = true;
                                    spawn_streamable_http_listener(
                                        server.config.id.clone(),
                                        server.config.base_url.clone(),
                                        server.auth_header.clone(),
                                        server.session_id.clone(),
                                        tx,
                                    );
                                }
                            }
                        }
                    }
                    Err(err) => {
                        if let Some(server) = self.server_mut(id) {
                            server.connected = false;
                            server.last_error = Some(match oauth_refresh_warning.clone() {
                                Some(refresh_warning) => {
                                    format!("{refresh_warning} Connection failed: {err}")
                                }
                                None => err,
                            });
                            server.auth_header = auth_header;
                        }
                    }
                }
            }
            McpTransportKind::Stdio => {
                let client = match StdioClient::connect(
                    config.id.clone(),
                    &config,
                    request_tx.clone(),
                )
                .await
                {
                    Ok(client) => client,
                    Err(err) => {
                        if let Some(server) = self.server_mut(id) {
                            server.last_error = Some(err);
                            server.connected = false;
                            server.client = None;
                        }
                        return;
                    }
                };

                let client_details = client_details_for(&config);
                match client.initialize(client_details).await {
                    Ok(details) => {
                        if let Some(server) = self.server_mut(id) {
                            server.server_details = Some(details);
                        }
                    }
                    Err(err) => {
                        if let Some(server) = self.server_mut(id) {
                            server.last_error = Some(err);
                            server.connected = false;
                            server.client = None;
                        }
                        return;
                    }
                }

                if let Some(server) = self.server_mut(id) {
                    server.connected = true;
                    server.client = Some(client.clone());
                    server.last_error = None;
                    server.auth_header = auth_header;
                }
            }
        }
    }

    pub async fn refresh_tools(&mut self, id: &str) {
        if let Some(server) = self.server(id) {
            if !server.supports_tools() {
                if let Some(server) = self.server_mut(id) {
                    server.cached_tools = Some(empty_list_tools());
                    server.last_error = None;
                }
                return;
            }
        }

        if self.uses_http(id) {
            self.refresh_tools_streamable_http(id).await;
            return;
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return;
        };

        let list = paginate_tools_list_with!(fetch_tools_page_stdio, (&client));
        match list {
            Ok(Some(list)) => self.apply_tools_list_result(id, Ok(list)),
            Ok(None) => self.apply_tools_list_result(id, Ok(empty_list_tools())),
            Err(message) => self.apply_tools_list_result(id, Err(message)),
        }
    }

    pub async fn refresh_resources(&mut self, id: &str) {
        if let Some(server) = self.server(id) {
            if !server.supports_resources() {
                if let Some(server) = self.server_mut(id) {
                    server.cached_resources = Some(empty_list_resources());
                    server.last_error = None;
                }
                return;
            }
        }

        if self.uses_http(id) {
            self.refresh_resources_streamable_http(id).await;
            return;
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return;
        };

        let response = client
            .send_request(RequestFromClient::ListResourcesRequest(None))
            .await;
        let fetch = match response {
            Ok(message) if is_method_not_found(&message) => ListFetch::MethodNotFound(None),
            Ok(message) => match parse_list_resources(message) {
                Ok(list) => ListFetch::Ok(list, None),
                Err(err) => ListFetch::Err(err),
            },
            Err(err) => ListFetch::Err(err),
        };
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Resources",
                empty_list_resources,
                |server, list| server.cached_resources = Some(list),
            );
        }
    }

    pub async fn refresh_resource_templates(&mut self, id: &str) {
        if let Some(server) = self.server(id) {
            if !server.supports_resources() {
                if let Some(server) = self.server_mut(id) {
                    server.cached_resource_templates = Some(empty_list_resource_templates());
                    server.last_error = None;
                }
                return;
            }
        }

        if self.uses_http(id) {
            self.refresh_resource_templates_streamable_http(id).await;
            return;
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return;
        };

        let response = client
            .send_request(RequestFromClient::ListResourceTemplatesRequest(None))
            .await;
        let fetch = match response {
            Ok(message) if is_method_not_found(&message) => ListFetch::MethodNotFound(None),
            Ok(message) => match parse_list_resource_templates(message) {
                Ok(list) => ListFetch::Ok(list, None),
                Err(err) => ListFetch::Err(err),
            },
            Err(err) => ListFetch::Err(err),
        };
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Resource templates",
                empty_list_resource_templates,
                |server, list| server.cached_resource_templates = Some(list),
            );
        }
    }

    pub async fn refresh_prompts(&mut self, id: &str) {
        if let Some(server) = self.server(id) {
            if !server.supports_prompts() {
                if let Some(server) = self.server_mut(id) {
                    server.cached_prompts = Some(empty_list_prompts());
                    server.last_error = None;
                }
                return;
            }
        }

        if self.uses_http(id) {
            self.refresh_prompts_streamable_http(id).await;
            return;
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return;
        };

        let response = client
            .send_request(RequestFromClient::ListPromptsRequest(None))
            .await;
        let fetch = match response {
            Ok(message) if is_method_not_found(&message) => ListFetch::MethodNotFound(None),
            Ok(message) => match parse_list_prompts(message) {
                Ok(list) => ListFetch::Ok(list, None),
                Err(err) => ListFetch::Err(err),
            },
            Err(err) => ListFetch::Err(err),
        };
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Prompts",
                empty_list_prompts,
                |server, list| server.cached_prompts = Some(list),
            );
        }
    }

    fn uses_http(&self, id: &str) -> bool {
        self.server(id).is_some_and(|server| {
            matches!(
                McpTransportKind::from_config(&server.config),
                Ok(McpTransportKind::StreamableHttp)
            )
        })
    }

    async fn refresh_tools_streamable_http(&mut self, id: &str) {
        if let Err(err) = self.ensure_streamable_http_session(id).await {
            if let Some(server) = self.server_mut(id) {
                server.last_error = Some(err);
            }
            return;
        }

        let list = paginate_tools_list_with!(fetch_tools_page_http, (self, id));
        match list {
            Ok(Some(list)) => self.apply_tools_list_result(id, Ok(list)),
            Ok(None) => self.apply_tools_list_result(id, Ok(empty_list_tools())),
            Err(message) => self.apply_tools_list_result(id, Err(message)),
        }
    }

    async fn refresh_resources_streamable_http(&mut self, id: &str) {
        if let Err(err) = self.ensure_streamable_http_session(id).await {
            if let Some(server) = self.server_mut(id) {
                server.last_error = Some(err);
            }
            return;
        }

        let response = self
            .send_streamable_http_request(id, RequestFromClient::ListResourcesRequest(None))
            .await;

        let fetch = streamable_http_list_fetch(response, parse_list_resources);
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Resources",
                empty_list_resources,
                |server, list| server.cached_resources = Some(list),
            );
        }
    }

    async fn refresh_resource_templates_streamable_http(&mut self, id: &str) {
        if let Err(err) = self.ensure_streamable_http_session(id).await {
            if let Some(server) = self.server_mut(id) {
                server.last_error = Some(err);
            }
            return;
        }

        let response = self
            .send_streamable_http_request(id, RequestFromClient::ListResourceTemplatesRequest(None))
            .await;

        let fetch = streamable_http_list_fetch(response, parse_list_resource_templates);
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Resource templates",
                empty_list_resource_templates,
                |server, list| server.cached_resource_templates = Some(list),
            );
        }
    }

    async fn refresh_prompts_streamable_http(&mut self, id: &str) {
        if let Err(err) = self.ensure_streamable_http_session(id).await {
            if let Some(server) = self.server_mut(id) {
                server.last_error = Some(err);
            }
            return;
        }

        let response = self
            .send_streamable_http_request(id, RequestFromClient::ListPromptsRequest(None))
            .await;

        let fetch = streamable_http_list_fetch(response, parse_list_prompts);
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(
                server,
                fetch,
                "Prompts",
                empty_list_prompts,
                |server, list| server.cached_prompts = Some(list),
            );
        }
    }

    async fn ensure_streamable_http_session(&mut self, id: &str) -> Result<(), String> {
        let needs_init = self
            .server(id)
            .map(|server| server.session_id.is_none())
            .unwrap_or(false);
        if !needs_init {
            return Ok(());
        }

        let client_details = self
            .server(id)
            .map(|server| client_details_for(&server.config))
            .ok_or_else(|| "Unknown MCP server".to_string())?;

        let response = self
            .send_streamable_http_request(id, RequestFromClient::InitializeRequest(client_details))
            .await?;
        let initialize = parse_initialize_result(response)?;

        if let Some(server) = self.server_mut(id) {
            server.server_details = Some(initialize);
        }

        if self
            .server(id)
            .and_then(|server| server.session_id.as_deref())
            .is_none()
        {
            return Err("Missing session id on initialize response.".to_string());
        }

        Ok(())
    }

    async fn send_streamable_http_request(
        &mut self,
        id: &str,
        request: RequestFromClient,
    ) -> Result<ServerMessage, String> {
        let (base_url, auth_header, session_id, request_id) = {
            let Some(server) = self.server_mut(id) else {
                return Err("Unknown MCP server".to_string());
            };
            let base_url = match require_http_base_url(&server.config) {
                Ok(base_url) => base_url,
                Err(err) => return Err(err),
            };
            let request_id = server.streamable_http_request_id;
            server.streamable_http_request_id = server.streamable_http_request_id.saturating_add(1);
            (
                base_url,
                server.auth_header.clone(),
                server.session_id.clone(),
                request_id,
            )
        };

        let message = ClientMessage::from_message(
            MessageFromClient::RequestFromClient(request),
            Some(RequestId::Integer(request_id as i64)),
        )
        .map_err(|err| err.to_string())?;

        let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;

        let client = reqwest::Client::new();
        debug!(
            server_id = %id,
            request_id,
            url = %base_url,
            "Sending MCP HTTP request"
        );
        let mut request = client
            .post(base_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(payload);

        if let Some(auth) = auth_header {
            request = request.header("Authorization", auth);
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }

        let response = request.send().await.map_err(|err| err.to_string())?;
        debug!(
            server_id = %id,
            status = %response.status(),
            "Received MCP HTTP response"
        );
        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        let request_tx = self.server_request_tx.clone();
        let server_message = if content_type.starts_with("text/event-stream") {
            read_sse_response_messages(response, id, request_tx).await?
        } else {
            let body = response.bytes().await.map_err(|err| err.to_string())?;
            serde_json::from_slice::<ServerMessage>(&body).map_err(|err| err.to_string())?
        };

        if let Some(server) = self.server_mut(id) {
            if session_id.is_some() {
                server.session_id = session_id;
            }
        }

        Ok(server_message)
    }
}

fn sse_data_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

async fn read_sse_response_messages(
    response: reqwest::Response,
    server_id: &str,
    request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
) -> Result<ServerMessage, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = SseLineBuffer::default();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| err.to_string())?;
        for line in buffer.push(&chunk) {
            let Some(payload) = sse_data_payload(&line) else {
                continue;
            };
            if payload.is_empty() {
                continue;
            }

            let message =
                serde_json::from_str::<ServerMessage>(payload).map_err(|err| err.to_string())?;
            match message {
                ServerMessage::Response(_) | ServerMessage::Error(_) => return Ok(message),
                ServerMessage::Request(request) => {
                    if let Some(tx) = request_tx.as_ref() {
                        let _ = tx.send(McpServerRequest {
                            server_id: server_id.to_string(),
                            request,
                        });
                    }
                }
                ServerMessage::Notification(_) => {}
            }
        }
    }

    for line in buffer.finish() {
        let Some(payload) = sse_data_payload(&line) else {
            continue;
        };
        if payload.is_empty() {
            continue;
        }
        let message =
            serde_json::from_str::<ServerMessage>(payload).map_err(|err| err.to_string())?;
        match message {
            ServerMessage::Response(_) | ServerMessage::Error(_) => return Ok(message),
            ServerMessage::Request(request) => {
                if let Some(tx) = request_tx.as_ref() {
                    let _ = tx.send(McpServerRequest {
                        server_id: server_id.to_string(),
                        request,
                    });
                }
            }
            ServerMessage::Notification(_) => {}
        }
    }

    Err("Empty event-stream response.".to_string())
}

fn spawn_streamable_http_listener(
    server_id: String,
    base_url: Option<String>,
    auth_header: Option<String>,
    session_id: Option<String>,
    request_tx: mpsc::UnboundedSender<McpServerRequest>,
) {
    let Some(base_url) = base_url else {
        return;
    };

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut request = client.get(&base_url).header("Accept", "text/event-stream");

        if let Some(auth) = auth_header {
            request = request.header("Authorization", auth);
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => return,
        };

        if !response.status().is_success() {
            return;
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !content_type.starts_with("text/event-stream") {
            return;
        }

        let mut stream = response.bytes_stream();
        let mut buffer = SseLineBuffer::default();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(_) => return,
            };
            for line in buffer.push(&chunk) {
                let Some(payload) = sse_data_payload(&line) else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                if let Ok(ServerMessage::Request(request)) =
                    serde_json::from_str::<ServerMessage>(payload)
                {
                    let _ = request_tx.send(McpServerRequest {
                        server_id: server_id.clone(),
                        request,
                    });
                }
            }
        }

        for line in buffer.finish() {
            let Some(payload) = sse_data_payload(&line) else {
                continue;
            };
            if payload.is_empty() {
                continue;
            }
            if let Ok(ServerMessage::Request(request)) =
                serde_json::from_str::<ServerMessage>(payload)
            {
                let _ = request_tx.send(McpServerRequest {
                    server_id: server_id.clone(),
                    request,
                });
            }
        }
    });
}

#[derive(Clone)]
pub struct McpToolCallContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    client: Option<Arc<StdioClient>>,
    streamable_http_request_id: u64,
}

pub struct McpPromptContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    client: Option<Arc<StdioClient>>,
    streamable_http_request_id: u64,
}

#[derive(Clone)]
pub struct McpServerRequestContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    client: Option<Arc<StdioClient>>,
}

trait StreamableHttpContext {
    fn config(&self) -> &McpServerConfig;
    fn auth_header(&self) -> Option<&String>;
    fn session_id(&self) -> Option<&String>;
    fn set_session_id(&mut self, session_id: Option<String>);
    fn next_request_id(&mut self) -> i64;
}

impl StreamableHttpContext for McpToolCallContext {
    fn config(&self) -> &McpServerConfig {
        &self.config
    }

    fn auth_header(&self) -> Option<&String> {
        self.auth_header.as_ref()
    }

    fn session_id(&self) -> Option<&String> {
        self.session_id.as_ref()
    }

    fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.streamable_http_request_id as i64;
        self.streamable_http_request_id = self.streamable_http_request_id.saturating_add(1);
        request_id
    }
}

impl StreamableHttpContext for McpPromptContext {
    fn config(&self) -> &McpServerConfig {
        &self.config
    }

    fn auth_header(&self) -> Option<&String> {
        self.auth_header.as_ref()
    }

    fn session_id(&self) -> Option<&String> {
        self.session_id.as_ref()
    }

    fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.streamable_http_request_id as i64;
        self.streamable_http_request_id = self.streamable_http_request_id.saturating_add(1);
        request_id
    }
}

impl McpClientManager {
    pub fn tool_call_context(&self, id: &str) -> Option<McpToolCallContext> {
        let server = self.server(id)?;
        if !server.config.is_enabled() {
            return None;
        }
        let transport_kind = McpTransportKind::from_config(&server.config).ok()?;
        Some(McpToolCallContext {
            server_id: server.config.id.clone(),
            config: server.config.clone(),
            transport_kind,
            auth_header: server.auth_header.clone(),
            session_id: server.session_id.clone(),
            client: server.client.clone(),
            streamable_http_request_id: 0,
        })
    }

    pub fn prompt_call_context(&self, id: &str) -> Option<McpPromptContext> {
        let server = self.server(id)?;
        if !server.config.is_enabled() {
            return None;
        }
        let transport_kind = McpTransportKind::from_config(&server.config).ok()?;
        Some(McpPromptContext {
            server_id: server.config.id.clone(),
            config: server.config.clone(),
            transport_kind,
            auth_header: server.auth_header.clone(),
            session_id: server.session_id.clone(),
            client: server.client.clone(),
            streamable_http_request_id: 0,
        })
    }

    pub fn server_request_context(&self, id: &str) -> Option<McpServerRequestContext> {
        let server = self.server(id)?;
        if !server.config.is_enabled() {
            return None;
        }
        let transport_kind = McpTransportKind::from_config(&server.config).ok()?;
        Some(McpServerRequestContext {
            server_id: server.config.id.clone(),
            config: server.config.clone(),
            transport_kind,
            auth_header: server.auth_header.clone(),
            session_id: server.session_id.clone(),
            client: server.client.clone(),
        })
    }

    pub fn update_tool_call_session(
        &mut self,
        id: &str,
        session_id: Option<String>,
        last_error: Option<String>,
    ) {
        if let Some(server) = self.server_mut(id) {
            if let Some(session_id) = session_id {
                server.session_id = Some(session_id);
            }
            server.last_error = last_error;
        }
    }

    pub fn update_prompt_session(
        &mut self,
        id: &str,
        session_id: Option<String>,
        last_error: Option<String>,
    ) {
        if let Some(server) = self.server_mut(id) {
            if let Some(session_id) = session_id {
                server.session_id = Some(session_id);
            }
            server.last_error = last_error;
        }
    }

    pub fn update_server_request_session(
        &mut self,
        id: &str,
        session_id: Option<String>,
        last_error: Option<String>,
    ) {
        if let Some(server) = self.server_mut(id) {
            if let Some(session_id) = session_id {
                server.session_id = Some(session_id);
            }
            server.last_error = last_error;
        }
    }
}

pub async fn execute_tool_call(
    context: &mut McpToolCallContext,
    request: &ToolCallRequest,
) -> Result<CallToolResult, String> {
    let mut params = CallToolRequestParams::new(&request.tool_name);
    if let Some(arguments) = request.arguments.clone() {
        params = params.with_arguments(arguments);
    }

    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let response = client
                .send_request(RequestFromClient::CallToolRequest(params))
                .await?;
            parse_call_tool(response)
        }
        McpTransportKind::StreamableHttp => {
            ensure_streamable_http_session_context(context).await?;
            let response = send_streamable_http_request_with_context(
                context,
                RequestFromClient::CallToolRequest(params),
            )
            .await?;
            parse_call_tool(response)
        }
    }
}

pub async fn execute_resource_read(
    context: &mut McpToolCallContext,
    uri: &str,
) -> Result<ReadResourceResult, String> {
    let params = ReadResourceRequestParams {
        meta: None,
        uri: uri.to_string(),
    };

    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let response = client
                .send_request(RequestFromClient::ReadResourceRequest(params))
                .await?;
            parse_read_resource(response)
        }
        McpTransportKind::StreamableHttp => {
            ensure_streamable_http_session_context(context).await?;
            let response = send_streamable_http_request_with_context(
                context,
                RequestFromClient::ReadResourceRequest(params),
            )
            .await?;
            parse_read_resource(response)
        }
    }
}

pub async fn execute_resource_list(
    context: &mut McpToolCallContext,
    cursor: Option<String>,
) -> Result<ListResourcesResult, String> {
    let params = cursor.map(|cursor| PaginatedRequestParams {
        cursor: Some(cursor),
        meta: None,
    });

    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let response = client
                .send_request(RequestFromClient::ListResourcesRequest(params))
                .await?;
            parse_list_resources(response)
        }
        McpTransportKind::StreamableHttp => {
            ensure_streamable_http_session_context(context).await?;
            let response = send_streamable_http_request_with_context(
                context,
                RequestFromClient::ListResourcesRequest(params),
            )
            .await?;
            parse_list_resources(response)
        }
    }
}

pub async fn execute_resource_template_list(
    context: &mut McpToolCallContext,
    cursor: Option<String>,
) -> Result<ListResourceTemplatesResult, String> {
    let params = cursor.map(|cursor| PaginatedRequestParams {
        cursor: Some(cursor),
        meta: None,
    });

    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let response = client
                .send_request(RequestFromClient::ListResourceTemplatesRequest(params))
                .await?;
            parse_list_resource_templates(response)
        }
        McpTransportKind::StreamableHttp => {
            ensure_streamable_http_session_context(context).await?;
            let response = send_streamable_http_request_with_context(
                context,
                RequestFromClient::ListResourceTemplatesRequest(params),
            )
            .await?;
            parse_list_resource_templates(response)
        }
    }
}

pub async fn execute_prompt(
    context: &mut McpPromptContext,
    request: &crate::core::app::session::McpPromptRequest,
) -> Result<GetPromptResult, String> {
    let params = if request.arguments.is_empty() {
        GetPromptRequestParams {
            name: request.prompt_name.clone(),
            arguments: None,
            meta: None,
        }
    } else {
        GetPromptRequestParams {
            name: request.prompt_name.clone(),
            arguments: Some(request.arguments.clone()),
            meta: None,
        }
    };

    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let response = client
                .send_request(RequestFromClient::GetPromptRequest(params))
                .await?;
            parse_get_prompt(response)
        }
        McpTransportKind::StreamableHttp => {
            ensure_prompt_streamable_http_session_context(context).await?;
            let response = send_streamable_http_request_with_context_inner(
                context,
                RequestFromClient::GetPromptRequest(params),
            )
            .await?;
            parse_get_prompt(response)
        }
    }
}

pub async fn send_client_result(
    context: &mut McpServerRequestContext,
    request_id: RequestId,
    result: ResultFromClient,
) -> Result<(), String> {
    debug!(
        server_id = %context.server_id,
        request_id = ?request_id,
        transport = ?context.transport_kind,
        "Sending MCP client result"
    );
    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            client.send_result(request_id, result).await
        }
        McpTransportKind::StreamableHttp => {
            let message = ClientMessage::from_message(
                MessageFromClient::ResultFromClient(result),
                Some(request_id),
            )
            .map_err(|err| err.to_string())?;
            send_streamable_http_client_message(context, message).await
        }
    }
}

pub async fn send_client_error(
    context: &mut McpServerRequestContext,
    request_id: RequestId,
    error: RpcError,
) -> Result<(), String> {
    debug!(
        server_id = %context.server_id,
        request_id = ?request_id,
        transport = ?context.transport_kind,
        "Sending MCP client error"
    );
    match context.transport_kind {
        McpTransportKind::Stdio => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            client.send_error(request_id, error).await
        }
        McpTransportKind::StreamableHttp => {
            let message =
                ClientMessage::from_message(MessageFromClient::Error(error), Some(request_id))
                    .map_err(|err| err.to_string())?;
            send_streamable_http_client_message(context, message).await
        }
    }
}

async fn ensure_streamable_http_session_context(
    context: &mut McpToolCallContext,
) -> Result<(), String> {
    ensure_streamable_http_session_context_inner(context).await
}

async fn send_streamable_http_request_with_context(
    context: &mut McpToolCallContext,
    request: RequestFromClient,
) -> Result<ServerMessage, String> {
    send_streamable_http_request_with_context_inner(context, request).await
}

async fn ensure_prompt_streamable_http_session_context(
    context: &mut McpPromptContext,
) -> Result<(), String> {
    ensure_streamable_http_session_context_inner(context).await
}

async fn send_streamable_http_client_message(
    context: &mut McpServerRequestContext,
    message: ClientMessage,
) -> Result<(), String> {
    let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
    let client = reqwest::Client::new();
    let base_url = require_http_base_url(&context.config)?;
    let mut request = client
        .post(base_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(payload);

    if let Some(auth) = context.auth_header.as_ref() {
        request = request.header("Authorization", auth);
    }
    if let Some(session_id) = context.session_id.as_ref() {
        request = request.header("mcp-session-id", session_id);
    }

    let response = request.send().await.map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    if let Some(session_id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
    {
        context.session_id = Some(session_id);
    }

    Ok(())
}

async fn ensure_streamable_http_session_context_inner<C: StreamableHttpContext>(
    context: &mut C,
) -> Result<(), String> {
    if context.session_id().is_some() {
        return Ok(());
    }

    let client_details = client_details_for(context.config());
    let response = send_streamable_http_request_with_context_inner(
        context,
        RequestFromClient::InitializeRequest(client_details),
    )
    .await?;
    parse_initialize_result(response)?;

    if context.session_id().is_none() {
        return Err("Missing session id on initialize response.".to_string());
    }

    Ok(())
}

async fn send_streamable_http_request_with_context_inner<C: StreamableHttpContext>(
    context: &mut C,
    request: RequestFromClient,
) -> Result<ServerMessage, String> {
    let request_id = context.next_request_id();

    let message = ClientMessage::from_message(
        MessageFromClient::RequestFromClient(request),
        Some(RequestId::Integer(request_id)),
    )
    .map_err(|err| err.to_string())?;

    let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;

    let client = reqwest::Client::new();
    let base_url = require_http_base_url(context.config())?;
    debug!(
        server_id = %context.config().id,
        request_id,
        url = %base_url,
        "Sending MCP HTTP request"
    );
    let mut request = client
        .post(base_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(payload);

    if let Some(auth) = context.auth_header() {
        request = request.header("Authorization", auth);
    }
    if let Some(session_id) = context.session_id() {
        request = request.header("mcp-session-id", session_id);
    }

    let response = request.send().await.map_err(|err| err.to_string())?;
    debug!(
        server_id = %context.config().id,
        status = %response.status(),
        "Received MCP HTTP response"
    );
    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();

    let server_message = if content_type.starts_with("text/event-stream") {
        read_sse_response_messages(response, &context.config().id, None).await?
    } else {
        let body = response.bytes().await.map_err(|err| err.to_string())?;
        serde_json::from_slice::<ServerMessage>(&body).map_err(|err| err.to_string())?
    };

    if let Some(session_id) = session_id {
        context.set_session_id(Some(session_id));
    }

    Ok(server_message)
}

fn client_details_for(config: &McpServerConfig) -> InitializeRequestParams {
    let protocol_version = config
        .protocol_version
        .clone()
        .unwrap_or_else(|| LATEST_PROTOCOL_VERSION.to_string());
    let capabilities = ClientCapabilities {
        sampling: Some(ClientSampling::default()),
        ..ClientCapabilities::default()
    };
    InitializeRequestParams {
        capabilities,
        client_info: Implementation {
            name: "chabeau".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Chabeau MCP Client".to_string()),
            description: Some("Chabeau MCP client runtime".to_string()),
            icons: Vec::new(),
            website_url: Some("https://github.com/permacommons/chabeau".to_string()),
        },
        meta: None,
        protocol_version,
    }
}

fn parse_initialize_result(message: ServerMessage) -> Result<InitializeResult, String> {
    let value = parse_response_value(message)?;
    let result =
        serde_json::from_value::<InitializeResult>(value).map_err(|err| err.to_string())?;
    if result.protocol_version.trim().is_empty() {
        return Err("Unexpected initialize response.".to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn sample_config() -> McpServerConfig {
        McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha MCP".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        }
    }

    fn init_with_caps(caps: ServerCapabilities) -> InitializeResult {
        InitializeResult {
            capabilities: caps,
            instructions: None,
            meta: None,
            protocol_version: "2025-11-25".to_string(),
            server_info: Implementation {
                name: "server".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                description: None,
                icons: Vec::new(),
                website_url: None,
            },
        }
    }

    #[test]
    fn server_capability_defaults_to_supported() {
        let mut state = McpServerState::new(sample_config());
        state.server_details = None;
        assert!(state.supports_tools());
        assert!(state.supports_resources());
        assert!(state.supports_prompts());
    }

    #[test]
    fn server_capability_flags_disable_missing_lists() {
        let mut state = McpServerState::new(sample_config());
        let caps = ServerCapabilities::default();
        state.server_details = Some(init_with_caps(caps));
        assert!(!state.supports_tools());
        assert!(!state.supports_resources());
        assert!(!state.supports_prompts());
    }

    #[test]
    fn server_capability_flags_enable_present_lists() {
        let mut state = McpServerState::new(sample_config());
        let mut caps = ServerCapabilities::default();
        caps.tools = Some(rust_mcp_schema::ServerCapabilitiesTools::default());
        caps.resources = Some(rust_mcp_schema::ServerCapabilitiesResources::default());
        caps.prompts = Some(rust_mcp_schema::ServerCapabilitiesPrompts::default());
        state.server_details = Some(init_with_caps(caps));
        assert!(state.supports_tools());
        assert!(state.supports_resources());
        assert!(state.supports_prompts());
    }

    fn sample_tool(name: String) -> rust_mcp_schema::Tool {
        rust_mcp_schema::Tool {
            annotations: None,
            description: None,
            execution: None,
            icons: Vec::new(),
            input_schema: rust_mcp_schema::ToolInputSchema::new(Vec::new(), None, None),
            meta: None,
            name,
            output_schema: None,
            title: None,
        }
    }

    struct ToolPageState {
        calls: Vec<Option<String>>,
    }

    async fn fetch_tools_page_test(
        state: &Arc<Mutex<ToolPageState>>,
        cursor: Option<String>,
    ) -> Result<Option<ListToolsResult>, String> {
        let mut state = state.lock().await;
        state.calls.push(cursor.clone());
        let result = match cursor.as_deref() {
            None => ListToolsResult {
                meta: None,
                next_cursor: Some("c1".to_string()),
                tools: (0..60)
                    .map(|idx| sample_tool(format!("tool-{idx}")))
                    .collect(),
            },
            Some("c1") => ListToolsResult {
                meta: None,
                next_cursor: Some("c2".to_string()),
                tools: (60..120)
                    .map(|idx| sample_tool(format!("tool-{idx}")))
                    .collect(),
            },
            Some("c2") => ListToolsResult {
                meta: None,
                next_cursor: None,
                tools: vec![sample_tool("tool-120".to_string())],
            },
            Some(other) => {
                return Err(format!("Unexpected cursor: {other}"));
            }
        };

        Ok(Some(result))
    }

    #[tokio::test]
    async fn paginate_tools_list_caps_and_preserves_cursor() {
        let state = Arc::new(Mutex::new(ToolPageState { calls: Vec::new() }));
        let result = paginate_tools_list_with!(fetch_tools_page_test, (&state))
            .expect("pagination should succeed")
            .expect("expected list tools result");

        assert_eq!(result.tools.len(), MCP_MAX_TOOL_LIST);
        assert_eq!(result.next_cursor.as_deref(), Some("c2"));
        let calls = state.lock().await.calls.clone();
        assert_eq!(calls, vec![None, Some("c1".to_string())]);
    }

    async fn fetch_tools_page_first_page_full(
        state: &Arc<Mutex<ToolPageState>>,
        cursor: Option<String>,
    ) -> Result<Option<ListToolsResult>, String> {
        let mut state = state.lock().await;
        state.calls.push(cursor.clone());
        let result = ListToolsResult {
            meta: None,
            next_cursor: Some("c1".to_string()),
            tools: (0..MCP_MAX_TOOL_LIST + 5)
                .map(|idx| sample_tool(format!("tool-{idx}")))
                .collect(),
        };
        Ok(Some(result))
    }

    #[tokio::test]
    async fn paginate_tools_list_stops_when_first_page_is_full() {
        let state = Arc::new(Mutex::new(ToolPageState { calls: Vec::new() }));
        let result = paginate_tools_list_with!(fetch_tools_page_first_page_full, (&state))
            .expect("pagination should succeed")
            .expect("expected list tools result");

        assert_eq!(result.tools.len(), MCP_MAX_TOOL_LIST);
        assert_eq!(result.next_cursor.as_deref(), Some("c1"));
        let calls = state.lock().await.calls.clone();
        assert_eq!(calls, vec![None]);
    }
}

fn parse_list_tools(message: ServerMessage) -> Result<ListToolsResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<ListToolsResult>(value).map_err(|err| err.to_string())
}

fn parse_list_resources(message: ServerMessage) -> Result<ListResourcesResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<ListResourcesResult>(value).map_err(|err| err.to_string())
}

fn parse_list_resource_templates(
    message: ServerMessage,
) -> Result<ListResourceTemplatesResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<ListResourceTemplatesResult>(value).map_err(|err| err.to_string())
}

fn parse_list_prompts(message: ServerMessage) -> Result<ListPromptsResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<ListPromptsResult>(value).map_err(|err| err.to_string())
}

fn parse_get_prompt(message: ServerMessage) -> Result<GetPromptResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<GetPromptResult>(value).map_err(|err| err.to_string())
}

fn parse_read_resource(message: ServerMessage) -> Result<ReadResourceResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<ReadResourceResult>(value).map_err(|err| err.to_string())
}

fn parse_call_tool(message: ServerMessage) -> Result<CallToolResult, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<CallToolResult>(value).map_err(|err| err.to_string())
}

fn parse_response_value(message: ServerMessage) -> Result<Value, String> {
    match message {
        ServerMessage::Response(response) => {
            serde_json::to_value(&response.result).map_err(|err| err.to_string())
        }
        ServerMessage::Error(error) => Err(format_rpc_error(&error.error)),
        other => Err(format_unexpected_server_message(&other)),
    }
}

fn streamable_http_list_fetch<T>(
    response: Result<ServerMessage, String>,
    parse: impl FnOnce(ServerMessage) -> Result<T, String>,
) -> ListFetch<T> {
    match response {
        Ok(message) if is_method_not_found(&message) => ListFetch::MethodNotFound(None),
        Ok(message) => match parse(message) {
            Ok(list) => ListFetch::Ok(list, None),
            Err(err) => ListFetch::Err(err),
        },
        Err(err) => ListFetch::Err(err),
    }
}

fn is_method_not_found(message: &ServerMessage) -> bool {
    matches!(
        message,
        ServerMessage::Error(error) if error.error.code == MCP_METHOD_NOT_FOUND
    )
}

fn paginated_params(cursor: Option<String>) -> Option<PaginatedRequestParams> {
    cursor.map(|cursor| PaginatedRequestParams {
        cursor: Some(cursor),
        meta: None,
    })
}

async fn fetch_tools_page_stdio(
    client: &Arc<StdioClient>,
    cursor: Option<String>,
) -> Result<Option<ListToolsResult>, String> {
    let params = paginated_params(cursor);
    let response = client
        .send_request(RequestFromClient::ListToolsRequest(params))
        .await;
    match response {
        Ok(message) if is_method_not_found(&message) => Ok(None),
        Ok(message) => parse_list_tools(message)
            .map(Some)
            .map_err(|err| err.to_string()),
        Err(err) => Err(err),
    }
}

async fn fetch_tools_page_http(
    manager: &mut McpClientManager,
    id: &str,
    cursor: Option<String>,
) -> Result<Option<ListToolsResult>, String> {
    let params = paginated_params(cursor);
    let response = manager
        .send_streamable_http_request(id, RequestFromClient::ListToolsRequest(params))
        .await;
    let fetch = streamable_http_list_fetch(response, parse_list_tools);
    match fetch {
        ListFetch::Ok(list, _) => Ok(Some(list)),
        ListFetch::MethodNotFound(_) => Ok(None),
        ListFetch::Err(message) => Err(message),
    }
}

fn empty_list_tools() -> ListToolsResult {
    ListToolsResult {
        meta: None,
        next_cursor: None,
        tools: Vec::new(),
    }
}

fn empty_list_resources() -> ListResourcesResult {
    ListResourcesResult {
        meta: None,
        next_cursor: None,
        resources: Vec::new(),
    }
}

fn empty_list_resource_templates() -> ListResourceTemplatesResult {
    ListResourceTemplatesResult {
        meta: None,
        next_cursor: None,
        resource_templates: Vec::new(),
    }
}

fn empty_list_prompts() -> ListPromptsResult {
    ListPromptsResult {
        meta: None,
        next_cursor: None,
        prompts: Vec::new(),
    }
}

fn format_unexpected_server_message(message: &ServerMessage) -> String {
    format!("Unexpected MCP server message: {message:?}")
}

fn format_rpc_error(error: &RpcError) -> String {
    let mut output = format!("MCP error {}: {}", error.code, error.message);
    if let Some(data) = &error.data {
        let details = data
            .get("details")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| data.as_str().map(|value| value.to_string()))
            .or_else(|| serde_json::to_string_pretty(data).ok());

        if let Some(details) = details {
            if !details.is_empty() {
                output.push('\n');
                output.push_str(&details);
            }
        }
    }
    output
}
