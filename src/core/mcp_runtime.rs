use crate::core::app::session::McpSamplingRequest;
use crate::core::app::ui_state::ToolPromptRequest;
use crate::core::app::{App, AppActionContext, AppCommand};
use crate::core::mcp_sampling::{
    build_sampling_messages, serialize_sampling_params, summarize_sampling_request,
};
use crate::mcp::events::McpServerRequest;
use crate::mcp::permissions::ToolPermissionDecision;
use rust_mcp_schema::schema_utils::ServerJsonrpcRequest;
use rust_mcp_schema::{ClientCapabilities, ClientSampling, RpcError};
use tracing::debug;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

pub fn serialize_result<T: Serialize>(result: &T) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|_| "Unable to serialize MCP result.".to_string())
}

pub async fn run_cancellable<F, T>(
    cancel_token: Option<&CancellationToken>,
    operation: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Some(token) = cancel_token {
        tokio::select! {
            _ = token.cancelled() => Err("MCP operation interrupted by user.".to_string()),
            result = operation => result,
        }
    } else {
        operation.await
    }
}

pub enum McpRuntimeInput {
    ServerRequest(McpServerRequest),
    SamplingPermission {
        request: McpSamplingRequest,
        decision: ToolPermissionDecision,
    },
    SamplingFinished,
    AdvanceQueue,
}

pub fn handle_runtime_input(
    app: &mut App,
    input: McpRuntimeInput,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match input {
        McpRuntimeInput::ServerRequest(request) => handle_server_request(app, request, ctx),
        McpRuntimeInput::SamplingPermission { request, decision } => {
            handle_sampling_permission_decision(app, request, decision, ctx)
        }
        McpRuntimeInput::SamplingFinished => handle_sampling_finished(app, ctx),
        McpRuntimeInput::AdvanceQueue => advance_sampling_queue(app, ctx),
    }
}

fn handle_server_request(
    app: &mut App,
    request: McpServerRequest,
    ctx: AppActionContext,
) -> Option<AppCommand> {
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

    advance_sampling_queue(app, ctx)
}

fn handle_sampling_permission_decision(
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
        let message = if decision == ToolPermissionDecision::Block {
            "Sampling blocked by user."
        } else {
            "Sampling denied by user."
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
    set_status_for_sampling_run(app, &request, ctx);
    Some(AppCommand::RunMcpSampling(Box::new(request)))
}

fn handle_sampling_finished(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    app.session.active_sampling_request = None;
    if app.session.active_tool_request.is_none() {
        app.end_mcp_operation_if_active();
    }
    advance_sampling_queue(app, ctx)
}

fn advance_sampling_queue(app: &mut App, _ctx: AppActionContext) -> Option<AppCommand> {
    if app.session.active_sampling_request.is_some() || app.ui.tool_prompt().is_some() {
        return None;
    }

    let request = app.session.pending_sampling_queue.pop_front()?;
    if let Some(decision) = app
        .mcp_permissions
        .decision_for(&request.server_id, crate::mcp::MCP_SAMPLING_TOOL)
    {
        if decision == ToolPermissionDecision::Block {
            return Some(AppCommand::SendMcpServerError {
                server_id: request.server_id,
                request_id: request.request.id.clone(),
                error: RpcError {
                    code: -1,
                    message: "Sampling blocked by user.".to_string(),
                    data: None,
                },
            });
        }

        if decision == ToolPermissionDecision::AllowSession {
            app.session.active_sampling_request = Some(request.clone());
            return Some(AppCommand::RunMcpSampling(Box::new(request)));
        }
    }

    let server_name = app
        .mcp
        .server(&request.server_id)
        .map(|s| s.config.display_name.clone())
        .unwrap_or_else(|| request.server_id.clone());
    debug!(server_id = %request.server_id, "Prompting user for sampling permission");
    app.session.active_sampling_request = Some(request.clone());
    app.ui.start_tool_prompt(ToolPromptRequest {
        server_id: request.server_id,
        server_name,
        tool_name: crate::mcp::MCP_SAMPLING_TOOL.to_string(),
        display_name: Some(format!("Allow sampling with {}?", app.session.model)),
        args_summary: summarize_sampling_request(&request.request),
        raw_arguments: serialize_sampling_params(&request.request),
        batch_index: 0,
    });
    None
}

fn set_status_for_sampling_run(
    app: &mut App,
    _request: &McpSamplingRequest,
    _ctx: AppActionContext,
) {
    app.begin_mcp_operation();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::App;
    use crate::ui::theme::Theme;

    #[test]
    fn advance_queue_without_items_is_noop() {
        let mut app = App::new_test_app(Theme::dark_default(), true, true);
        let cmd = handle_runtime_input(
            &mut app,
            McpRuntimeInput::AdvanceQueue,
            AppActionContext::default(),
        );
        assert!(cmd.is_none());
    }
}
