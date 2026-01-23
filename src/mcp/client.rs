use crate::core::app::session::ToolCallRequest;
use crate::core::config::data::{Config, McpServerConfig};
use crate::core::mcp_auth::McpTokenStore;
use rust_mcp_sdk::mcp_client::{client_runtime, ClientHandler, McpClientOptions};
use rust_mcp_sdk::schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, RequestFromClient, ServerMessage,
};
use rust_mcp_sdk::schema::RequestId;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams,
    GetPromptResult, Implementation, InitializeRequestParams, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ReadResourceRequestParams,
    ReadResourceResult, LATEST_PROTOCOL_VERSION,
};
use rust_mcp_sdk::{ClientSseTransport, ClientSseTransportOptions, McpClient, ToMcpClientHandler};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    StreamableHttp,
    Sse,
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
            "sse" => Ok(McpTransportKind::Sse),
            other => Err(format!("Unsupported MCP transport: {}", other)),
        }
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
    client: Option<Arc<rust_mcp_sdk::mcp_client::ClientRuntime>>,
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

impl McpClientManager {
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
        let (config, transport_kind, headers, client_details, auth_header) = {
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

            let token = match token_store.get_token(&server.config.id) {
                Ok(token) => token,
                Err(err) => {
                    server.last_error = Some(format!("Token lookup failed: {}", err));
                    server.connected = false;
                    server.client = None;
                    return;
                }
            };

            let auth_header = token.map(|token| format!("Bearer {}", token));

            let headers = auth_header.as_ref().map(|token| {
                let mut map = HashMap::new();
                map.insert("Authorization".to_string(), token.clone());
                map.insert(
                    "Accept".to_string(),
                    "application/json, text/event-stream".to_string(),
                );
                map.insert("Content-Type".to_string(), "application/json".to_string());
                map
            });

            let protocol_version = server
                .config
                .protocol_version
                .clone()
                .unwrap_or_else(|| LATEST_PROTOCOL_VERSION.to_string());

            let client_details = InitializeRequestParams {
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
            };

            let transport_kind = match McpTransportKind::from_config(&server.config) {
                Ok(kind) => kind,
                Err(err) => {
                    server.last_error = Some(err);
                    server.connected = false;
                    server.client = None;
                    return;
                }
            };

            server.auth_header = auth_header.clone();
            (
                server.config.clone(),
                transport_kind,
                headers,
                client_details,
                auth_header,
            )
        };

        let client = match transport_kind {
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
                return;
            }
            McpTransportKind::Sse => {
                let transport = match ClientSseTransport::new(
                    &config.base_url,
                    ClientSseTransportOptions {
                        custom_headers: headers,
                        ..ClientSseTransportOptions::default()
                    },
                ) {
                    Ok(transport) => transport,
                    Err(err) => {
                        if let Some(server) = self.server_mut(id) {
                            server.last_error = Some(err.to_string());
                            server.connected = false;
                            server.client = None;
                        }
                        return;
                    }
                };
                client_runtime::create_client(McpClientOptions {
                    client_details,
                    transport,
                    handler: ChabeauMcpClientHandler.to_mcp_client_handler(),
                    task_store: None,
                    server_task_store: None,
                })
            }
        };

        if let Err(err) = client.clone().start().await {
            if let Some(server) = self.server_mut(id) {
                server.last_error = Some(err.to_string());
                server.connected = false;
                server.client = None;
            }
            return;
        }

        if let Some(server) = self.server_mut(id) {
            server.connected = client.is_initialized();
            server.client = Some(client.clone());
            server.last_error = None;
            server.session_id = client.session_id().await;
            server.auth_header = auth_header;
        }
    }

    pub async fn refresh_tools(&mut self, id: &str) {
        if self.uses_streamable_http(id) {
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

        if !client.server_has_tools().unwrap_or(true) {
            return;
        }

        if let Some(server) = self.server_mut(id) {
            match client.request_tool_list(None).await {
                Ok(list) => {
                    server.cached_tools = Some(list);
                    server.last_error = None;
                    server.session_id = client.session_id().await;
                }
                Err(err) => {
                    server.last_error = Some(format!("Tools listing failed: {}", err));
                }
            }
        }
    }

    pub async fn refresh_resources(&mut self, id: &str) {
        if self.uses_streamable_http(id) {
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

        if !client.server_has_resources().unwrap_or(true) {
            return;
        }

        if let Some(server) = self.server_mut(id) {
            match client.request_resource_list(None).await {
                Ok(list) => {
                    server.cached_resources = Some(list);
                    server.last_error = None;
                    server.session_id = client.session_id().await;
                }
                Err(err) => {
                    server.last_error = Some(format!("Resources listing failed: {}", err));
                }
            }
        }
    }

    pub async fn refresh_resource_templates(&mut self, id: &str) {
        if self.uses_streamable_http(id) {
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

        if let Some(server) = self.server_mut(id) {
            match client.request_resource_template_list(None).await {
                Ok(list) => {
                    server.cached_resource_templates = Some(list);
                    server.last_error = None;
                    server.session_id = client.session_id().await;
                }
                Err(err) => {
                    server.last_error = Some(format!("Resource templates listing failed: {}", err));
                }
            }
        }
    }

    pub async fn refresh_prompts(&mut self, id: &str) {
        if self.uses_streamable_http(id) {
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

        if !client.server_has_prompts().unwrap_or(true) {
            return;
        }

        if let Some(server) = self.server_mut(id) {
            match client.request_prompt_list(None).await {
                Ok(list) => {
                    server.cached_prompts = Some(list);
                    server.last_error = None;
                    server.session_id = client.session_id().await;
                }
                Err(err) => {
                    server.last_error = Some(format!("Prompts listing failed: {}", err));
                }
            }
        }
    }

    fn uses_streamable_http(&self, id: &str) -> bool {
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

        match response.and_then(parse_list_tools) {
            Ok(list) => {
                if let Some(server) = self.server_mut(id) {
                    server.cached_tools = Some(list);
                    server.last_error = None;
                }
            }
            Err(err) => {
                if let Some(server) = self.server_mut(id) {
                    server.last_error = Some(format!("Tools listing failed: {}", err));
                }
            }
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

        match response.and_then(parse_list_resources) {
            Ok(list) => {
                if let Some(server) = self.server_mut(id) {
                    server.cached_resources = Some(list);
                    server.last_error = None;
                }
            }
            Err(err) => {
                if let Some(server) = self.server_mut(id) {
                    server.last_error = Some(format!("Resources listing failed: {}", err));
                }
            }
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

        match response.and_then(parse_list_resource_templates) {
            Ok(list) => {
                if let Some(server) = self.server_mut(id) {
                    server.cached_resource_templates = Some(list);
                    server.last_error = None;
                }
            }
            Err(err) => {
                if let Some(server) = self.server_mut(id) {
                    server.last_error = Some(format!("Resource templates listing failed: {}", err));
                }
            }
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

        match response.and_then(parse_list_prompts) {
            Ok(list) => {
                if let Some(server) = self.server_mut(id) {
                    server.cached_prompts = Some(list);
                    server.last_error = None;
                }
            }
            Err(err) => {
                if let Some(server) = self.server_mut(id) {
                    server.last_error = Some(format!("Prompts listing failed: {}", err));
                }
            }
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
            let request_id = server.streamable_http_request_id;
            server.streamable_http_request_id = server.streamable_http_request_id.saturating_add(1);
            (
                server.config.base_url.clone(),
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
    pub server_id: String,
    pub config: McpServerConfig,
    pub transport_kind: McpTransportKind,
    pub auth_header: Option<String>,
    pub session_id: Option<String>,
    pub client: Option<Arc<rust_mcp_sdk::mcp_client::ClientRuntime>>,
    pub streamable_http_request_id: u64,
}

pub struct McpPromptContext {
    pub server_id: String,
    pub config: McpServerConfig,
    pub transport_kind: McpTransportKind,
    pub auth_header: Option<String>,
    pub session_id: Option<String>,
    pub client: Option<Arc<rust_mcp_sdk::mcp_client::ClientRuntime>>,
    pub streamable_http_request_id: u64,
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
        McpTransportKind::Sse => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let result = client
                .request_tool_call(params)
                .await
                .map_err(|err| err.to_string())?;
            context.session_id = client.session_id().await;
            Ok(result)
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
        McpTransportKind::Sse => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let result = client
                .request_resource_read(params)
                .await
                .map_err(|err| err.to_string())?;
            context.session_id = client.session_id().await;
            Ok(result)
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
        McpTransportKind::Sse => {
            let Some(client) = context.client.clone() else {
                return Err("MCP client not connected.".to_string());
            };
            let result = client
                .request_prompt(params)
                .await
                .map_err(|err| err.to_string())?;
            context.session_id = client.session_id().await;
            Ok(result)
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
    let mut request = client
        .post(context.config().base_url.clone())
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

struct ChabeauMcpClientHandler;

impl ClientHandler for ChabeauMcpClientHandler {}

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
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    if value.get("protocolVersion").is_none() && value.get("protocol_version").is_none() {
        return Err("Unexpected initialize response.".to_string());
    }
    Ok(())
}

fn parse_list_tools(message: ServerMessage) -> Result<ListToolsResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<ListToolsResult>(value).map_err(|err| err.to_string())
}

fn parse_list_resources(message: ServerMessage) -> Result<ListResourcesResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<ListResourcesResult>(value).map_err(|err| err.to_string())
}

fn parse_list_resource_templates(
    message: ServerMessage,
) -> Result<ListResourceTemplatesResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<ListResourceTemplatesResult>(value).map_err(|err| err.to_string())
}

fn parse_list_prompts(message: ServerMessage) -> Result<ListPromptsResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<ListPromptsResult>(value).map_err(|err| err.to_string())
}

fn parse_get_prompt(message: ServerMessage) -> Result<GetPromptResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<GetPromptResult>(value).map_err(|err| err.to_string())
}

fn parse_read_resource(message: ServerMessage) -> Result<ReadResourceResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<ReadResourceResult>(value).map_err(|err| err.to_string())
}

fn parse_call_tool(message: ServerMessage) -> Result<CallToolResult, String> {
    let response = message.as_response().map_err(|err| err.to_string())?;
    let value = serde_json::to_value(&response.result).map_err(|err| err.to_string())?;
    serde_json::from_value::<CallToolResult>(value).map_err(|err| err.to_string())
}
