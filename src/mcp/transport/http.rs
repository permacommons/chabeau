use crate::core::config::data::McpServerConfig;

pub const MCP_JSON_CONTENT_TYPE: &str = "application/json";
pub const MCP_JSON_AND_SSE_ACCEPT: &str = "application/json, text/event-stream";
pub const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";

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

pub fn require_http_base_url(config: &McpServerConfig) -> Result<String, String> {
    config
        .base_url
        .clone()
        .ok_or_else(|| "MCP base_url is required for HTTP transports.".to_string())
}

pub fn apply_streamable_http_client_post_headers(
    request: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    request
        .header("Content-Type", MCP_JSON_CONTENT_TYPE)
        .header("Accept", MCP_JSON_AND_SSE_ACCEPT)
}

pub fn apply_streamable_http_protocol_version_header(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_header_ignored_when_blank() {
        let client = reqwest::Client::new();
        let req = apply_streamable_http_protocol_version_header(
            client.post("https://example.com"),
            Some("  "),
        )
        .build()
        .unwrap();
        assert!(req.headers().get(MCP_PROTOCOL_VERSION_HEADER).is_none());
    }
}
