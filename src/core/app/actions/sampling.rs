use super::{App, AppActionContext, AppCommand};
use crate::core::app::session::McpSamplingRequest;
use crate::core::mcp_sampling::build_sampling_messages;
use crate::mcp::permissions::ToolPermissionDecision;
use rust_mcp_schema::schema_utils::ServerJsonrpcRequest;
use rust_mcp_schema::{ClientCapabilities, ClientSampling, RpcError};
use tracing::debug;

pub(super) fn handle_mcp_server_request(
    app: &mut App,
    request: crate::mcp::events::McpServerRequest,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.request_id(),
        method = %request.request.method(),
        "Received MCP server request"
    );
    if app.session.mcp_disabled {
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.request_id().clone(),
            error: RpcError::internal_error().with_message("MCP is disabled."),
        });
    }

    let capabilities = ClientCapabilities {
        sampling: Some(ClientSampling::default()),
        ..ClientCapabilities::default()
    };
    if let Err(err) = capabilities.can_handle_request(&request.request) {
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.request_id().clone(),
            error: err,
        });
    }

    let request_id = request.request.request_id().clone();
    let ServerJsonrpcRequest::CreateMessageRequest(create_request) = request.request else {
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id,
            error: RpcError::method_not_found()
                .with_message("Unsupported MCP request from server."),
        });
    };

    let messages = match build_sampling_messages(&create_request) {
        Ok(messages) => messages,
        Err(err) => {
            return Some(AppCommand::SendMcpServerError {
                server_id: request.server_id,
                request_id: create_request.id.clone(),
                error: RpcError::invalid_params().with_message(&err),
            });
        }
    };

    app.session
        .pending_sampling_queue
        .push_back(McpSamplingRequest {
            server_id: request.server_id,
            request: create_request,
            messages,
        });

    super::advance_sampling_queue(app, ctx)
}

pub(super) fn handle_sampling_permission_decision(
    app: &mut App,
    request: McpSamplingRequest,
    decision: ToolPermissionDecision,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    app.ui.cancel_tool_prompt();
    app.clear_status();

    match decision {
        ToolPermissionDecision::AllowOnce => {}
        ToolPermissionDecision::AllowSession | ToolPermissionDecision::Block => app
            .mcp_permissions
            .record(&request.server_id, crate::mcp::MCP_SAMPLING_TOOL, decision),
        ToolPermissionDecision::DenyOnce => {}
    }

    if matches!(
        decision,
        ToolPermissionDecision::DenyOnce | ToolPermissionDecision::Block
    ) {
        let message = match decision {
            ToolPermissionDecision::Block => "Sampling blocked by user.",
            _ => "Sampling denied by user.",
        };
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.id.clone(),
            error: RpcError {
                code: -1,
                message: message.to_string(),
                data: None,
            },
        });
    }

    app.session.active_sampling_request = Some(request.clone());
    super::set_status_for_sampling_run(app, &request, ctx);
    Some(AppCommand::RunMcpSampling(Box::new(request)))
}

pub(super) fn handle_mcp_sampling_finished(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    app.session.active_sampling_request = None;
    if let Some(request) = app.session.active_tool_request.clone() {
        super::set_status_for_tool_run(app, &request, ctx);
    } else {
        app.end_mcp_operation_if_active();
    }
    super::advance_sampling_queue(app, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn sampling_finished_clears_active_request() {
        let mut app = create_test_app();
        let params = rust_mcp_schema::CreateMessageRequestParams {
            include_context: None,
            max_tokens: 16,
            messages: vec![],
            meta: None,
            metadata: None,
            model_preferences: None,
            stop_sequences: vec![],
            system_prompt: None,
            task: None,
            temperature: None,
            tool_choice: None,
            tools: vec![],
        };
        app.session.active_sampling_request = Some(crate::core::app::session::McpSamplingRequest {
            server_id: "s".into(),
            request: rust_mcp_schema::CreateMessageRequest::new(
                rust_mcp_schema::RequestId::Integer(1),
                params,
            ),
            messages: vec![],
        });
        let _ = handle_mcp_sampling_finished(&mut app, default_ctx());
        assert!(app.session.active_sampling_request.is_none());
    }
}
