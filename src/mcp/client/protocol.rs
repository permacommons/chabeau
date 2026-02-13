use super::{format_rpc_error, format_unexpected_server_message};
use crate::core::config::data::McpServerConfig;
use rust_mcp_schema::schema_utils::ServerMessage;
use rust_mcp_schema::{
    CallToolResult, GetPromptResult, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ReadResourceResult,
    LATEST_PROTOCOL_VERSION,
};
use serde_json::Value;

pub(crate) fn requested_protocol_version(config: &McpServerConfig) -> String {
    config
        .protocol_version
        .clone()
        .unwrap_or_else(|| LATEST_PROTOCOL_VERSION.to_string())
}

pub(crate) fn effective_protocol_version(
    config: &McpServerConfig,
    negotiated_version: Option<&str>,
) -> String {
    match negotiated_version {
        Some(version) if !version.trim().is_empty() => version.to_string(),
        _ => requested_protocol_version(config),
    }
}

pub(crate) fn parse_initialize_result(message: ServerMessage) -> Result<InitializeResult, String> {
    let value = parse_response_value(message)?;
    let result =
        serde_json::from_value::<InitializeResult>(value).map_err(|err| err.to_string())?;
    if result.protocol_version.trim().is_empty() {
        return Err("Unexpected initialize response.".to_string());
    }
    Ok(result)
}

pub(crate) fn parse_list_tools(message: ServerMessage) -> Result<ListToolsResult, String> {
    parse_response(message)
}

pub(crate) fn parse_list_resources(message: ServerMessage) -> Result<ListResourcesResult, String> {
    parse_response(message)
}

pub(crate) fn parse_list_resource_templates(
    message: ServerMessage,
) -> Result<ListResourceTemplatesResult, String> {
    parse_response(message)
}

pub(crate) fn parse_list_prompts(message: ServerMessage) -> Result<ListPromptsResult, String> {
    parse_response(message)
}

pub(crate) fn parse_get_prompt(message: ServerMessage) -> Result<GetPromptResult, String> {
    parse_response(message)
}

pub(crate) fn parse_read_resource(message: ServerMessage) -> Result<ReadResourceResult, String> {
    parse_response(message)
}

pub(crate) fn parse_call_tool(message: ServerMessage) -> Result<CallToolResult, String> {
    parse_response(message)
}

fn parse_response<T: serde::de::DeserializeOwned>(message: ServerMessage) -> Result<T, String> {
    let value = parse_response_value(message)?;
    serde_json::from_value::<T>(value).map_err(|err| err.to_string())
}

pub(crate) fn parse_response_value(message: ServerMessage) -> Result<Value, String> {
    match message {
        ServerMessage::Response(response) => {
            serde_json::to_value(&response.result).map_err(|err| err.to_string())
        }
        ServerMessage::Error(error) => Err(format_rpc_error(&error.error)),
        other => Err(format_unexpected_server_message(&other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::data::McpServerConfig;

    #[test]
    fn parse_initialize_rejects_blank_protocol_version() {
        let message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
            "capabilities": {},
            "protocolVersion": " ",
            "serverInfo": {"name": "x", "version": "1.0.0"}
            }
        }))
        .expect("message should parse");

        assert!(parse_initialize_result(message).is_err());
    }

    #[test]
    fn effective_protocol_prefers_negotiated() {
        let config = McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: None,
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: None,
            allowed_tools: None,
            protocol_version: Some("2025-01-01".to_string()),
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        };

        assert_eq!(
            effective_protocol_version(&config, Some("2025-11-25")),
            "2025-11-25"
        );
        assert_eq!(effective_protocol_version(&config, None), "2025-01-01");
    }
}
