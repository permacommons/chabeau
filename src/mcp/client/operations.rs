use super::protocol::{
    parse_call_tool, parse_get_prompt, parse_list_resource_templates, parse_list_resources,
    parse_read_resource,
};
use super::transport_http;
use super::transport_stdio;
use super::{McpPromptContext, McpServerRequestContext, McpToolCallContext};
use crate::core::app::session::{McpPromptRequest, ToolCallRequest};
use crate::mcp::transport::McpTransportKind;
use rust_mcp_schema::schema_utils::{
    ClientMessage, FromMessage, MessageFromClient, RequestFromClient, ResultFromClient,
    ServerMessage,
};
use rust_mcp_schema::{
    CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult,
    ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, RequestId, RpcError,
};
use tracing::debug;

async fn execute_transport_request<T, C>(
    context: &mut C,
    request: RequestFromClient,
    parse: fn(ServerMessage) -> Result<T, String>,
) -> Result<T, String>
where
    C: OperationContext,
{
    let response = match context.transport_kind() {
        McpTransportKind::Stdio => transport_stdio::send_request(context.client(), request).await?,
        McpTransportKind::StreamableHttp => {
            transport_http::ensure_session_context(context).await?;
            transport_http::send_request_with_context(context, request).await?
        }
    };
    parse(response)
}

trait OperationContext: super::StreamableHttpContext {
    fn transport_kind(&self) -> McpTransportKind;
    fn client(&self) -> Option<std::sync::Arc<super::StdioClient>>;
}

impl OperationContext for McpToolCallContext {
    fn transport_kind(&self) -> McpTransportKind {
        self.transport_kind
    }

    fn client(&self) -> Option<std::sync::Arc<super::StdioClient>> {
        self.client.clone()
    }
}

impl OperationContext for McpPromptContext {
    fn transport_kind(&self) -> McpTransportKind {
        self.transport_kind
    }

    fn client(&self) -> Option<std::sync::Arc<super::StdioClient>> {
        self.client.clone()
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
    execute_transport_request(
        context,
        RequestFromClient::CallToolRequest(params),
        parse_call_tool,
    )
    .await
}

pub async fn execute_resource_read(
    context: &mut McpToolCallContext,
    uri: &str,
) -> Result<ReadResourceResult, String> {
    let params = ReadResourceRequestParams {
        meta: None,
        uri: uri.to_string(),
    };
    execute_transport_request(
        context,
        RequestFromClient::ReadResourceRequest(params),
        parse_read_resource,
    )
    .await
}

pub async fn execute_resource_list(
    context: &mut McpToolCallContext,
    cursor: Option<String>,
) -> Result<ListResourcesResult, String> {
    let params = cursor.map(|cursor| PaginatedRequestParams {
        cursor: Some(cursor),
        meta: None,
    });
    execute_transport_request(
        context,
        RequestFromClient::ListResourcesRequest(params),
        parse_list_resources,
    )
    .await
}

pub async fn execute_resource_template_list(
    context: &mut McpToolCallContext,
    cursor: Option<String>,
) -> Result<ListResourceTemplatesResult, String> {
    let params = cursor.map(|cursor| PaginatedRequestParams {
        cursor: Some(cursor),
        meta: None,
    });
    execute_transport_request(
        context,
        RequestFromClient::ListResourceTemplatesRequest(params),
        parse_list_resource_templates,
    )
    .await
}

pub async fn execute_prompt(
    context: &mut McpPromptContext,
    request: &McpPromptRequest,
) -> Result<GetPromptResult, String> {
    let params = GetPromptRequestParams {
        name: request.prompt_name.clone(),
        arguments: (!request.arguments.is_empty()).then_some(request.arguments.clone()),
        meta: None,
    };
    execute_transport_request(
        context,
        RequestFromClient::GetPromptRequest(params),
        parse_get_prompt,
    )
    .await
}

pub async fn send_client_result(
    context: &mut McpServerRequestContext,
    request_id: RequestId,
    result: ResultFromClient,
) -> Result<(), String> {
    debug!(server_id = %context.server_id, request_id = ?request_id, transport = ?context.transport_kind, "Sending MCP client result");
    match context.transport_kind {
        McpTransportKind::Stdio => {
            transport_stdio::send_result(context.client.clone(), request_id, result).await
        }
        McpTransportKind::StreamableHttp => {
            let message = ClientMessage::from_message(
                MessageFromClient::ResultFromClient(result),
                Some(request_id),
            )
            .map_err(|err| err.to_string())?;
            transport_http::send_server_result_message(context, message).await
        }
    }
}

pub async fn send_client_error(
    context: &mut McpServerRequestContext,
    request_id: RequestId,
    error: RpcError,
) -> Result<(), String> {
    debug!(server_id = %context.server_id, request_id = ?request_id, transport = ?context.transport_kind, "Sending MCP client error");
    match context.transport_kind {
        McpTransportKind::Stdio => {
            transport_stdio::send_error(context.client.clone(), request_id, error).await
        }
        McpTransportKind::StreamableHttp => {
            let message =
                ClientMessage::from_message(MessageFromClient::Error(error), Some(request_id))
                    .map_err(|err| err.to_string())?;
            transport_http::send_server_result_message(context, message).await
        }
    }
}
