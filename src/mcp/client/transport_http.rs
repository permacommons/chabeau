use super::{
    apply_streamable_http_client_post_headers, apply_streamable_http_protocol_version_header,
    client_details_for, is_event_stream_content_type, read_sse_response_messages,
    require_http_base_url, McpServerRequestContext, StreamableHttpContext,
};
use rust_mcp_schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, NotificationFromClient, RequestFromClient,
    ServerMessage,
};
use rust_mcp_schema::RequestId;
use tracing::debug;

pub(crate) async fn ensure_session_context<C: StreamableHttpContext>(
    context: &mut C,
) -> Result<(), String> {
    if context.session_id().is_some() {
        return Ok(());
    }

    let client_details = client_details_for(context.config());
    let response = send_request_with_context(
        context,
        RequestFromClient::InitializeRequest(client_details),
    )
    .await?;
    let initialize = super::protocol::parse_initialize_result(response)?;
    context.set_negotiated_protocol_version(Some(initialize.protocol_version));

    if context.session_id().is_none() {
        return Err("Missing session id on initialize response.".to_string());
    }

    send_notification(
        context,
        NotificationFromClient::InitializedNotification(None),
    )
    .await
}

pub(crate) async fn send_request_with_context<C: StreamableHttpContext>(
    context: &mut C,
    request: RequestFromClient,
) -> Result<ServerMessage, String> {
    let request_id = context.next_request_id();
    let message = ClientMessage::from_message(
        MessageFromClient::RequestFromClient(request),
        Some(RequestId::Integer(request_id)),
    )
    .map_err(|err| err.to_string())?;
    send_message(context, message).await
}

pub(crate) async fn send_server_result_message(
    context: &mut McpServerRequestContext,
    message: ClientMessage,
) -> Result<(), String> {
    send_client_message_with_context(context, message).await
}

async fn send_notification<C: StreamableHttpContext>(
    context: &mut C,
    notification: NotificationFromClient,
) -> Result<(), String> {
    let message = ClientMessage::from_message(
        MessageFromClient::NotificationFromClient(notification),
        None,
    )
    .map_err(|err| err.to_string())?;
    send_client_message_with_context(context, message).await
}

async fn send_client_message_with_context<C: StreamableHttpContext>(
    context: &mut C,
    message: ClientMessage,
) -> Result<(), String> {
    let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
    let client = context
        .http_client()
        .ok_or_else(|| "MCP HTTP client not connected.".to_string())?;
    let base_url = require_http_base_url(context.config())?;
    let protocol_version = context.effective_protocol_version();
    let mut request = apply_streamable_http_protocol_version_header(
        apply_streamable_http_client_post_headers(client.post(base_url)),
        Some(protocol_version.as_str()),
    )
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
    if let Some(session_id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
    {
        context.set_session_id(Some(session_id));
    }

    Ok(())
}

async fn send_message<C: StreamableHttpContext>(
    context: &mut C,
    message: ClientMessage,
) -> Result<ServerMessage, String> {
    let payload = serde_json::to_string(&message).map_err(|err| err.to_string())?;
    let client = context
        .http_client()
        .ok_or_else(|| "MCP HTTP client not connected.".to_string())?;
    let base_url = require_http_base_url(context.config())?;
    debug!(server_id = %context.config().id, url = %base_url, "Sending MCP HTTP request");
    let protocol_version = context.effective_protocol_version();
    let mut request = apply_streamable_http_protocol_version_header(
        apply_streamable_http_client_post_headers(client.post(base_url)),
        Some(protocol_version.as_str()),
    )
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

    let server_message = if is_event_stream_content_type(&content_type) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::data::McpServerConfig;

    #[test]
    fn ensure_session_requires_http_client() {
        struct Dummy {
            config: McpServerConfig,
            session: Option<String>,
        }
        impl StreamableHttpContext for Dummy {
            fn config(&self) -> &McpServerConfig {
                &self.config
            }
            fn auth_header(&self) -> Option<&String> {
                None
            }
            fn session_id(&self) -> Option<&String> {
                self.session.as_ref()
            }
            fn http_client(&self) -> Option<&reqwest::Client> {
                None
            }
            fn set_session_id(&mut self, session_id: Option<String>) {
                self.session = session_id;
            }
            fn next_request_id(&mut self) -> i64 {
                0
            }
            fn negotiated_protocol_version(&self) -> Option<&str> {
                None
            }
            fn set_negotiated_protocol_version(&mut self, _protocol_version: Option<String>) {}
        }

        let mut ctx = Dummy {
            config: McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                base_url: Some("https://example.com".to_string()),
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
            },
            session: None,
        };
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let err = rt
            .block_on(ensure_session_context(&mut ctx))
            .expect_err("expected error");
        assert_eq!(err, "MCP HTTP client not connected.");
    }
}
