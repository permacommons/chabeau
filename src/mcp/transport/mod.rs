//! Shared MCP transport abstractions.
//!
//! Implementations normalize protocol differences across stdio and streamable
//! HTTP so higher-level code can preserve common state invariants.

use crate::core::config::data::McpServerConfig;
use async_trait::async_trait;
use rust_mcp_schema::schema_utils::{RequestFromClient, ServerMessage};
use rust_mcp_schema::{
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult,
};

pub mod stdio;
pub mod streamable_http;

/// JSON-RPC code used by servers to indicate unsupported list methods.
pub const MCP_METHOD_NOT_FOUND: i64 = -32601;

/// Supported MCP transport backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
///
/// - [`McpTransportKind::Stdio`] for locally spawned processes.
/// - [`McpTransportKind::StreamableHttp`] for remote servers over HTTP/SSE.
pub enum McpTransportKind {
    StreamableHttp,
    Stdio,
}

/// Normalized outcome for metadata list calls across transports.
pub enum ListFetch<T> {
    Ok(T, Option<String>),
    MethodNotFound(Option<String>),
    Err(String),
}

#[async_trait]
/// Transport contract required by MCP metadata refresh and operation flows.
pub trait McpTransport {
    async fn initialize(
        &mut self,
        request: InitializeRequestParams,
    ) -> Result<InitializeResult, String>;

    async fn send_request(&mut self, request: RequestFromClient) -> Result<ServerMessage, String>;

    async fn list_tools(&mut self) -> ListFetch<ListToolsResult>;

    async fn list_resources(&mut self) -> ListFetch<ListResourcesResult>;

    async fn list_resource_templates(&mut self) -> ListFetch<ListResourceTemplatesResult>;

    async fn list_prompts(&mut self) -> ListFetch<ListPromptsResult>;
}

/// Converts a transport response into a list-fetch status while preserving
/// "method not found" as a soft capability signal.
pub fn list_fetch_from_response<T>(
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

/// Returns true when a server reports the JSON-RPC method-not-found code.
pub fn is_method_not_found(message: &ServerMessage) -> bool {
    matches!(
        message,
        ServerMessage::Error(error) if error.error.code == MCP_METHOD_NOT_FOUND
    )
}

impl McpTransportKind {
    /// Resolves transport type from config, defaulting to streamable HTTP.
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
