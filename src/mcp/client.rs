use crate::core::app::session::ToolCallRequest;
use crate::core::config::data::{Config, McpServerConfig};
use crate::core::mcp_auth::McpTokenStore;
use rust_mcp_schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, NotificationFromClient, RequestFromClient,
    ServerMessage,
};
use rust_mcp_schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams,
    GetPromptResult, Implementation, InitializeRequestParams, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ReadResourceRequestParams,
    ReadResourceResult, RequestId, RpcError, LATEST_PROTOCOL_VERSION,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::debug;

const MCP_METHOD_NOT_FOUND: i64 = -32601;

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

struct StdioClient {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerMessage>>>>,
    next_request_id: AtomicI64,
    server_details: RwLock<Option<rust_mcp_schema::InitializeResult>>,
}

impl StdioClient {
    async fn connect(config: &McpServerConfig) -> Result<Arc<Self>, String> {
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
        });

        Self::spawn_stdout_reader(pending.clone(), stdout);
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
                            Self::dispatch_message(&pending, message).await;
                        }
                    }
                } else if let Ok(message) = serde_json::from_value::<ServerMessage>(value) {
                    Self::dispatch_message(&pending, message).await;
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
            _ => {}
        }
    }

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
            pending.insert(request_id, tx);
        }

        let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|err| err.to_string())?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|err| err.to_string())?;
        stdin.flush().await.map_err(|err| err.to_string())?;

        let timeout = tokio::time::Duration::from_secs(STDIO_REQUEST_TIMEOUT_SECONDS);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(_)) => Err("MCP stdio response channel closed.".to_string()),
            Err(_) => Err("MCP stdio request timed out.".to_string()),
        }
    }

    async fn send_notification(&self, notification: NotificationFromClient) -> Result<(), String> {
        let message = ClientMessage::from_message(
            MessageFromClient::NotificationFromClient(notification),
            None,
        )
        .map_err(|err| err.to_string())?;
        let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|err| err.to_string())?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|err| err.to_string())?;
        stdin.flush().await.map_err(|err| err.to_string())?;
        Ok(())
    }

    async fn initialize(&self, details: InitializeRequestParams) -> Result<(), String> {
        let response = self
            .send_request(RequestFromClient::InitializeRequest(details))
            .await?;
        parse_initialize(response.clone())?;
        let initialize = parse_response_value(response)?;
        let result = serde_json::from_value::<rust_mcp_schema::InitializeResult>(initialize)
            .map_err(|err| err.to_string())?;
        *self.server_details.write().await = Some(result);
        self.send_notification(NotificationFromClient::InitializedNotification(None))
            .await?;
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
    pub streamable_http_request_id: u64,
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
            streamable_http_request_id: 0,
            client: None,
        }
    }

    pub fn allowed_tools(&self) -> Option<&[String]> {
        self.config.allowed_tools.as_deref()
    }
}

#[derive(Default, Clone)]
pub struct McpClientManager {
    servers: HashMap<String, McpServerState>,
}

enum ListFetch<T> {
    Ok(T, Option<String>),
    MethodNotFound(Option<String>),
    Err(String),
}

impl McpClientManager {
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

    pub fn from_config(config: &Config) -> Self {
        let servers = config
            .mcp_servers
            .iter()
            .cloned()
            .map(|server| (server.id.to_ascii_lowercase(), McpServerState::new(server)))
            .collect();
        Self { servers }
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

        match transport_kind {
            McpTransportKind::StreamableHttp => {
                match self.ensure_streamable_http_session(id).await {
                    Ok(()) => {
                        if let Some(server) = self.server_mut(id) {
                            server.connected = true;
                            server.last_error = None;
                            server.auth_header = auth_header;
                        }
                    }
                    Err(err) => {
                        if let Some(server) = self.server_mut(id) {
                            server.connected = false;
                            server.last_error = Some(err);
                            server.auth_header = auth_header;
                        }
                    }
                }
            }
            McpTransportKind::Stdio => {
                let client = match StdioClient::connect(&config).await {
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
                if let Err(err) = client.initialize(client_details).await {
                    if let Some(server) = self.server_mut(id) {
                        server.last_error = Some(err);
                        server.connected = false;
                        server.client = None;
                    }
                    return;
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

        let response = client
            .send_request(RequestFromClient::ListToolsRequest(None))
            .await;
        let fetch = match response {
            Ok(message) if is_method_not_found(&message) => ListFetch::MethodNotFound(None),
            Ok(message) => match parse_list_tools(message) {
                Ok(list) => ListFetch::Ok(list, None),
                Err(err) => ListFetch::Err(err),
            },
            Err(err) => ListFetch::Err(err),
        };
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(server, fetch, "Tools", empty_list_tools, |server, list| {
                server.cached_tools = Some(list)
            });
        }
    }

    pub async fn refresh_resources(&mut self, id: &str) {
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

        let response = self
            .send_streamable_http_request(id, RequestFromClient::ListToolsRequest(None))
            .await;

        let fetch = streamable_http_list_fetch(response, parse_list_tools);
        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(server, fetch, "Tools", empty_list_tools, |server, list| {
                server.cached_tools = Some(list)
            });
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
        parse_initialize(response)?;

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

        let body = response.bytes().await.map_err(|err| err.to_string())?;
        let server_message = if content_type.starts_with("text/event-stream") {
            let text = String::from_utf8_lossy(&body);
            let data = text
                .lines()
                .find_map(|line| line.strip_prefix("data:"))
                .map(|value| value.trim())
                .ok_or_else(|| "Empty event-stream response.".to_string())?;
            serde_json::from_str::<ServerMessage>(data).map_err(|err| err.to_string())?
        } else {
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
    parse_initialize(response)?;

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

    let body = response.bytes().await.map_err(|err| err.to_string())?;
    let server_message = if content_type.starts_with("text/event-stream") {
        let text = String::from_utf8_lossy(&body);
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .map(|value| value.trim())
            .ok_or_else(|| "Empty event-stream response.".to_string())?;
        serde_json::from_str::<ServerMessage>(data).map_err(|err| err.to_string())?
    } else {
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
    InitializeRequestParams {
        capabilities: ClientCapabilities::default(),
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

fn parse_initialize(message: ServerMessage) -> Result<(), String> {
    let value = parse_response_value(message)?;
    if value.get("protocolVersion").is_none() && value.get("protocol_version").is_none() {
        return Err("Unexpected initialize response.".to_string());
    }
    Ok(())
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
