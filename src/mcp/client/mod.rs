use crate::core::config::data::{Config, McpServerConfig};
use crate::core::mcp_auth::McpTokenStore;
use crate::core::oauth::refresh_oauth_grant_if_needed;
use crate::mcp::events::McpServerRequest;
pub use crate::mcp::transport::McpTransportKind;
use crate::mcp::transport::{self, ListFetch};
use futures_util::{stream, StreamExt};
pub use operations::{
    execute_prompt, execute_resource_list, execute_resource_read, execute_resource_template_list,
    execute_tool_call, send_client_error, send_client_result,
};
use rust_mcp_schema::schema_utils::{RequestFromClient, ServerMessage};
use rust_mcp_schema::{
    ClientCapabilities, ClientSampling, Implementation, InitializeRequestParams, InitializeResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, RpcError, ServerCapabilities,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

mod operations;
mod protocol;
mod transport_http;
mod transport_stdio;

use transport_http::StreamableHttpContext;
use transport_stdio::StdioClient;

const MCP_MAX_TOOL_LIST: usize = 100;
const MCP_STARTUP_CONCURRENCY_LIMIT: usize = 3;
const MCP_JSON_CONTENT_TYPE: &str = "application/json";
const MCP_JSON_AND_SSE_ACCEPT: &str = "application/json, text/event-stream";
const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const MCP_HTTP_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const MCP_HTTP_REQUEST_TIMEOUT_SECONDS: u64 = 60;
const MCP_HTTP_POOL_IDLE_TIMEOUT_SECONDS: u64 = 90;
const MCP_HTTP_POOL_MAX_IDLE_PER_HOST: usize = 8;

fn build_mcp_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(MCP_HTTP_CONNECT_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(MCP_HTTP_REQUEST_TIMEOUT_SECONDS))
        .pool_idle_timeout(Duration::from_secs(MCP_HTTP_POOL_IDLE_TIMEOUT_SECONDS))
        .pool_max_idle_per_host(MCP_HTTP_POOL_MAX_IDLE_PER_HOST)
        .build()
        .map_err(|err| err.to_string())
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

fn apply_streamable_http_client_post_headers(
    request: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    request
        .header("Content-Type", MCP_JSON_CONTENT_TYPE)
        .header("Accept", MCP_JSON_AND_SSE_ACCEPT)
}

fn apply_streamable_http_protocol_version_header(
    request: reqwest::RequestBuilder,
    protocol_version: Option<&str>,
) -> reqwest::RequestBuilder {
    match protocol_version {
        Some(protocol_version) if !protocol_version.trim().is_empty() => {
            request.header(MCP_PROTOCOL_VERSION_HEADER, protocol_version)
        }
        _ => request,
    }
}

const STDIO_REQUEST_TIMEOUT_SECONDS: u64 = 60;
const STDIO_SAMPLING_TIMEOUT_MULTIPLIER: u64 = 5;

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
    pub negotiated_protocol_version: Option<String>,
    pub streamable_http_request_id: u64,
    pub event_listener_started: bool,
    http_client: Option<reqwest::Client>,
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
            negotiated_protocol_version: None,
            streamable_http_request_id: 0,
            event_listener_started: false,
            http_client: None,
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
        self.negotiated_protocol_version = None;
        self.streamable_http_request_id = 0;
        self.event_listener_started = false;
        self.http_client = None;
        self.client = None;
    }
}

#[derive(Default, Clone)]
pub struct McpClientManager {
    servers: HashMap<String, McpServerState>,
    server_request_tx: Option<mpsc::UnboundedSender<McpServerRequest>>,
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

    fn ensure_http_client(&mut self, id: &str) -> Result<(), String> {
        let Some(server) = self.server_mut(id) else {
            return Err("Unknown MCP server".to_string());
        };
        if server.http_client.is_none() {
            let client = build_mcp_http_client()
                .map_err(|err| format!("Failed to build HTTP client: {err}"))?;
            server.http_client = Some(client);
        }
        Ok(())
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
        let request_tx = self.server_request_tx.clone();
        let enabled_server_configs: Vec<McpServerConfig> = self
            .servers
            .values()
            .filter(|server| server.config.is_enabled())
            .map(|server| server.config.clone())
            .collect();

        let connected_states: Vec<McpServerState> = stream::iter(enabled_server_configs)
            .map(|server_config| {
                let request_tx = request_tx.clone();
                async move {
                    let server_id = server_config.id.clone();
                    let mut manager = McpClientManager::from_config(&Config {
                        mcp_servers: vec![server_config],
                        ..Config::default()
                    });
                    if let Some(tx) = request_tx {
                        manager.set_request_sender(tx);
                    }
                    manager.connect_server(&server_id, token_store).await;
                    manager.server(&server_id).cloned()
                }
            })
            .buffer_unordered(MCP_STARTUP_CONCURRENCY_LIMIT)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flatten()
            .collect();

        for server_state in connected_states {
            self.servers
                .insert(server_state.config.id.to_ascii_lowercase(), server_state);
        }
    }

    pub async fn refresh_server_metadata_concurrently(&mut self, id: &str) {
        let Some(server) = self.server(id).cloned() else {
            return;
        };

        #[derive(Clone, Copy)]
        enum RefreshOperation {
            Tools,
            Prompts,
            Resources,
            ResourceTemplates,
        }

        let server_id = server.config.id.clone();
        let request_tx = self.server_request_tx.clone();

        let operation_results: Vec<McpServerState> = stream::iter([
            RefreshOperation::Tools,
            RefreshOperation::Prompts,
            RefreshOperation::Resources,
            RefreshOperation::ResourceTemplates,
        ])
        .map(|operation| {
            let mut manager = Self {
                servers: HashMap::from([(server_id.to_ascii_lowercase(), server.clone())]),
                server_request_tx: request_tx.clone(),
            };
            let server_id = server_id.clone();
            async move {
                match operation {
                    RefreshOperation::Tools => manager.refresh_tools(&server_id).await,
                    RefreshOperation::Prompts => manager.refresh_prompts(&server_id).await,
                    RefreshOperation::Resources => manager.refresh_resources(&server_id).await,
                    RefreshOperation::ResourceTemplates => {
                        manager.refresh_resource_templates(&server_id).await
                    }
                }
                manager.server(&server_id).cloned()
            }
        })
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect();

        if let Some(target) = self.server_mut(id) {
            for result in operation_results {
                if let Some(cached_tools) = result.cached_tools {
                    target.cached_tools = Some(cached_tools);
                }
                if let Some(cached_prompts) = result.cached_prompts {
                    target.cached_prompts = Some(cached_prompts);
                }
                if let Some(cached_resources) = result.cached_resources {
                    target.cached_resources = Some(cached_resources);
                }
                if let Some(cached_resource_templates) = result.cached_resource_templates {
                    target.cached_resource_templates = Some(cached_resource_templates);
                }
                if result.last_error.is_some() {
                    target.last_error = result.last_error;
                }
                if result.session_id.is_some() {
                    target.session_id = result.session_id;
                }
            }
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
                server.http_client = None;
                server.client = None;
                return;
            }

            let transport_kind = match McpTransportKind::from_config(&server.config) {
                Ok(kind) => kind,
                Err(err) => {
                    server.last_error = Some(err);
                    server.connected = false;
                    server.http_client = None;
                    server.client = None;
                    return;
                }
            };

            if server.connected {
                match transport_kind {
                    McpTransportKind::Stdio if server.client.is_some() => return,
                    McpTransportKind::StreamableHttp if server.http_client.is_some() => return,
                    _ => {}
                }
            }

            let auth_header = match transport_kind {
                McpTransportKind::StreamableHttp => {
                    if let Err(err) = require_http_base_url(&server.config) {
                        server.last_error = Some(err);
                        server.connected = false;
                        server.http_client = None;
                        server.client = None;
                        return;
                    }

                    let http_client = match server.http_client.clone() {
                        Some(client) => client,
                        None => match build_mcp_http_client() {
                            Ok(client) => client,
                            Err(err) => {
                                server.last_error =
                                    Some(format!("Failed to build HTTP client: {err}"));
                                server.connected = false;
                                server.http_client = None;
                                server.client = None;
                                return;
                            }
                        },
                    };

                    let token = match token_store.get_token(&server.config.id) {
                        Ok(token) => token,
                        Err(err) => {
                            server.last_error = Some(format!("Token lookup failed: {}", err));
                            server.connected = false;
                            server.http_client = None;
                            server.client = None;
                            return;
                        }
                    };

                    server.http_client = Some(http_client);
                    token.map(|token| format!("Bearer {}", token))
                }
                McpTransportKind::Stdio => {
                    if let Err(err) = require_stdio_command(&server.config) {
                        server.last_error = Some(err);
                        server.connected = false;
                        server.http_client = None;
                        server.client = None;
                        return;
                    }
                    server.http_client = None;
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
                                    transport_http::spawn_streamable_http_listener(
                                        server
                                            .http_client
                                            .clone()
                                            .expect("HTTP client should be available"),
                                        server.config.id.clone(),
                                        server.config.base_url.clone(),
                                        server.auth_header.clone(),
                                        server.session_id.clone(),
                                        tx,
                                        Some(protocol::effective_protocol_version(
                                            &server.config,
                                            server.negotiated_protocol_version.as_deref().or_else(
                                                || {
                                                    server.server_details.as_ref().map(|details| {
                                                        details.protocol_version.as_str()
                                                    })
                                                },
                                            ),
                                        )),
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
                            server.negotiated_protocol_version =
                                Some(details.protocol_version.clone());
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
        let fetch = self.fetch_tools_listing(id).await;
        self.refresh_listing(
            id,
            |server| server.supports_tools(),
            "Tools",
            empty_list_tools,
            |server, list| server.cached_tools = Some(list),
            fetch,
        )
        .await;
    }

    pub async fn refresh_resources(&mut self, id: &str) {
        let fetch = self.fetch_resources_listing(id).await;
        self.refresh_listing(
            id,
            |server| server.supports_resources(),
            "Resources",
            empty_list_resources,
            |server, list| server.cached_resources = Some(list),
            fetch,
        )
        .await;
    }

    pub async fn refresh_resource_templates(&mut self, id: &str) {
        let fetch = self.fetch_resource_templates_listing(id).await;
        self.refresh_listing(
            id,
            |server| server.supports_resources(),
            "Resource templates",
            empty_list_resource_templates,
            |server, list| server.cached_resource_templates = Some(list),
            fetch,
        )
        .await;
    }

    pub async fn refresh_prompts(&mut self, id: &str) {
        let fetch = self.fetch_prompts_listing(id).await;
        self.refresh_listing(
            id,
            |server| server.supports_prompts(),
            "Prompts",
            empty_list_prompts,
            |server, list| server.cached_prompts = Some(list),
            fetch,
        )
        .await;
    }

    async fn refresh_listing<T, Supports, Empty, Setter>(
        &mut self,
        id: &str,
        supports: Supports,
        label: &str,
        empty: Empty,
        set: Setter,
        fetch: ListFetch<T>,
    ) where
        Supports: Fn(&McpServerState) -> bool,
        Empty: FnOnce() -> T,
        Setter: FnOnce(&mut McpServerState, T),
    {
        if let Some(server) = self.server(id) {
            if !supports(server) {
                if let Some(server) = self.server_mut(id) {
                    set(server, empty());
                    server.last_error = None;
                }
                return;
            }
        }

        if let Some(server) = self.server_mut(id) {
            Self::apply_list_fetch(server, fetch, label, empty, set);
        }
    }

    async fn fetch_tools_listing(&mut self, id: &str) -> ListFetch<ListToolsResult> {
        if self.uses_http(id) {
            if let Err(err) = self.ensure_streamable_http_session(id).await {
                return ListFetch::Err(err);
            }
            let list = paginate_tools_list_with!(fetch_tools_page_http, (self, id));
            return match list {
                Ok(Some(list)) => ListFetch::Ok(list, None),
                Ok(None) => ListFetch::MethodNotFound(None),
                Err(message) => ListFetch::Err(message),
            };
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return ListFetch::Err("MCP client not connected.".to_string());
        };

        let list = paginate_tools_list_with!(fetch_tools_page_stdio, (&client));
        match list {
            Ok(Some(list)) => ListFetch::Ok(list, None),
            Ok(None) => ListFetch::MethodNotFound(None),
            Err(message) => ListFetch::Err(message),
        }
    }

    async fn fetch_resources_listing(&mut self, id: &str) -> ListFetch<ListResourcesResult> {
        if self.uses_http(id) {
            if let Err(err) = self.ensure_streamable_http_session(id).await {
                return ListFetch::Err(err);
            }

            let response = self
                .send_streamable_http_request(id, RequestFromClient::ListResourcesRequest(None))
                .await;
            return transport::streamable_http::fetch_list(
                response,
                protocol::parse_list_resources,
            );
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return ListFetch::Err("MCP client not connected.".to_string());
        };

        transport::stdio::fetch_list(
            client.send_request(RequestFromClient::ListResourcesRequest(None)),
            protocol::parse_list_resources,
        )
        .await
    }

    async fn fetch_resource_templates_listing(
        &mut self,
        id: &str,
    ) -> ListFetch<ListResourceTemplatesResult> {
        if self.uses_http(id) {
            if let Err(err) = self.ensure_streamable_http_session(id).await {
                return ListFetch::Err(err);
            }

            let response = self
                .send_streamable_http_request(
                    id,
                    RequestFromClient::ListResourceTemplatesRequest(None),
                )
                .await;
            return transport::streamable_http::fetch_list(
                response,
                protocol::parse_list_resource_templates,
            );
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return ListFetch::Err("MCP client not connected.".to_string());
        };

        transport::stdio::fetch_list(
            client.send_request(RequestFromClient::ListResourceTemplatesRequest(None)),
            protocol::parse_list_resource_templates,
        )
        .await
    }

    async fn fetch_prompts_listing(&mut self, id: &str) -> ListFetch<ListPromptsResult> {
        if self.uses_http(id) {
            if let Err(err) = self.ensure_streamable_http_session(id).await {
                return ListFetch::Err(err);
            }

            let response = self
                .send_streamable_http_request(id, RequestFromClient::ListPromptsRequest(None))
                .await;
            return transport::streamable_http::fetch_list(response, protocol::parse_list_prompts);
        }

        let Some(client) = self
            .servers
            .get(&id.to_ascii_lowercase())
            .and_then(|server| server.client.clone())
        else {
            return ListFetch::Err("MCP client not connected.".to_string());
        };

        transport::stdio::fetch_list(
            client.send_request(RequestFromClient::ListPromptsRequest(None)),
            protocol::parse_list_prompts,
        )
        .await
    }

    fn uses_http(&self, id: &str) -> bool {
        self.server(id).is_some_and(|server| {
            matches!(
                McpTransportKind::from_config(&server.config),
                Ok(McpTransportKind::StreamableHttp)
            )
        })
    }

    async fn ensure_streamable_http_session(&mut self, id: &str) -> Result<(), String> {
        self.ensure_http_client(id)?;

        let Some(mut context) = self.tool_call_context(id) else {
            return Err("Unknown MCP server".to_string());
        };

        transport_http::ensure_session_context(&mut context).await?;
        self.update_tool_call_session(id, context.session_id.clone(), None);
        if let Some(server) = self.server_mut(id) {
            server.negotiated_protocol_version = context.negotiated_protocol_version.clone();
        }
        Ok(())
    }

    async fn send_streamable_http_request(
        &mut self,
        id: &str,
        request: RequestFromClient,
    ) -> Result<ServerMessage, String> {
        self.ensure_http_client(id)?;

        let Some(mut context) = self.tool_call_context(id) else {
            return Err("Unknown MCP server".to_string());
        };

        let response = transport_http::send_request_with_context(
            &mut context,
            request,
            self.server_request_tx.clone(),
        )
        .await?;
        self.update_tool_call_session(id, context.session_id.clone(), None);
        if let Some(server) = self.server_mut(id) {
            server.negotiated_protocol_version = context.negotiated_protocol_version.clone();
        }
        Ok(response)
    }
}

#[derive(Clone)]
pub struct McpToolCallContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    http_client: Option<reqwest::Client>,
    client: Option<Arc<StdioClient>>,
    streamable_http_request_id: u64,
    negotiated_protocol_version: Option<String>,
}

pub struct McpPromptContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    http_client: Option<reqwest::Client>,
    client: Option<Arc<StdioClient>>,
    streamable_http_request_id: u64,
    negotiated_protocol_version: Option<String>,
}

#[derive(Clone)]
pub struct McpServerRequestContext {
    pub(crate) server_id: String,
    config: McpServerConfig,
    transport_kind: McpTransportKind,
    auth_header: Option<String>,
    pub(crate) session_id: Option<String>,
    http_client: Option<reqwest::Client>,
    client: Option<Arc<StdioClient>>,
    negotiated_protocol_version: Option<String>,
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

    fn http_client(&self) -> Option<&reqwest::Client> {
        self.http_client.as_ref()
    }

    fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.streamable_http_request_id as i64;
        self.streamable_http_request_id = self.streamable_http_request_id.saturating_add(1);
        request_id
    }

    fn negotiated_protocol_version(&self) -> Option<&str> {
        self.negotiated_protocol_version.as_deref()
    }

    fn set_negotiated_protocol_version(&mut self, protocol_version: Option<String>) {
        self.negotiated_protocol_version = protocol_version;
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

    fn http_client(&self) -> Option<&reqwest::Client> {
        self.http_client.as_ref()
    }

    fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.streamable_http_request_id as i64;
        self.streamable_http_request_id = self.streamable_http_request_id.saturating_add(1);
        request_id
    }

    fn negotiated_protocol_version(&self) -> Option<&str> {
        self.negotiated_protocol_version.as_deref()
    }

    fn set_negotiated_protocol_version(&mut self, protocol_version: Option<String>) {
        self.negotiated_protocol_version = protocol_version;
    }
}

impl StreamableHttpContext for McpServerRequestContext {
    fn config(&self) -> &McpServerConfig {
        &self.config
    }

    fn auth_header(&self) -> Option<&String> {
        self.auth_header.as_ref()
    }

    fn session_id(&self) -> Option<&String> {
        self.session_id.as_ref()
    }

    fn http_client(&self) -> Option<&reqwest::Client> {
        self.http_client.as_ref()
    }

    fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    fn next_request_id(&mut self) -> i64 {
        0
    }

    fn negotiated_protocol_version(&self) -> Option<&str> {
        self.negotiated_protocol_version.as_deref()
    }

    fn set_negotiated_protocol_version(&mut self, protocol_version: Option<String>) {
        self.negotiated_protocol_version = protocol_version;
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
            http_client: server.http_client.clone(),
            client: server.client.clone(),
            streamable_http_request_id: 0,
            negotiated_protocol_version: server.negotiated_protocol_version.clone(),
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
            http_client: server.http_client.clone(),
            client: server.client.clone(),
            streamable_http_request_id: 0,
            negotiated_protocol_version: server.negotiated_protocol_version.clone(),
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
            http_client: server.http_client.clone(),
            client: server.client.clone(),
            negotiated_protocol_version: server.negotiated_protocol_version.clone(),
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

fn client_details_for(config: &McpServerConfig) -> InitializeRequestParams {
    let protocol_version = protocol::requested_protocol_version(config);
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
        Ok(message) if transport::is_method_not_found(&message) => Ok(None),
        Ok(message) => protocol::parse_list_tools(message)
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
    let fetch = transport::streamable_http::fetch_list(response, protocol::parse_list_tools);
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

#[cfg(test)]
mod tests;
