use super::{App, AppActionContext, AppCommand};
use crate::core::app::session::ToolFailureKind;
use crate::mcp::permissions::ToolPermissionDecision;
use serde_json::Value;
use tracing::debug;

#[derive(Debug, Clone)]
pub(super) struct ToolResultMeta {
    pub(super) server_label: Option<String>,
    pub(super) server_id: Option<String>,
    pub(super) tool_call_id: Option<String>,
    pub(super) raw_arguments: Option<String>,
    pub(super) failure_kind: Option<ToolFailureKind>,
}

impl ToolResultMeta {
    pub(super) fn new(
        server_label: Option<String>,
        server_id: Option<String>,
        tool_call_id: Option<String>,
        raw_arguments: Option<String>,
    ) -> Self {
        Self {
            server_label,
            server_id,
            tool_call_id,
            raw_arguments,
            failure_kind: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingToolError {
    pub(super) tool_name: String,
    pub(super) server_id: Option<String>,
    pub(super) tool_call_id: Option<String>,
    pub(super) raw_arguments: Option<String>,
    pub(super) error: String,
}

pub(super) fn handle_tool_permission_decision(
    app: &mut App,
    decision: ToolPermissionDecision,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let prompt_tool = app.ui.tool_prompt().map(|prompt| prompt.tool_name.clone());
    debug!(
        decision = ?decision,
        prompt_tool = prompt_tool.as_deref().unwrap_or("<none>"),
        "Tool permission decision received"
    );

    if prompt_tool.as_deref() == Some(crate::mcp::MCP_SAMPLING_TOOL) {
        let request = app.session.tool_pipeline.active_sampling_request.take()?;
        return super::sampling::handle_sampling_permission_decision(app, request, decision, ctx);
    }

    if let Some(request) = app.session.tool_pipeline.active_tool_request.take() {
        app.ui.cancel_tool_prompt();
        app.clear_status();

        match decision {
            ToolPermissionDecision::AllowOnce => {}
            ToolPermissionDecision::AllowSession | ToolPermissionDecision::Block => app
                .mcp_permissions
                .record(&request.server_id, &request.tool_name, decision),
            ToolPermissionDecision::DenyOnce => {}
        }

        if matches!(
            decision,
            ToolPermissionDecision::DenyOnce | ToolPermissionDecision::Block
        ) {
            let message = match decision {
                ToolPermissionDecision::Block => "Tool blocked by user.",
                _ => "Tool denied by user.",
            };
            let server_label = super::resolve_server_label(app, &request.server_id);
            let status = match decision {
                ToolPermissionDecision::Block => {
                    crate::core::app::session::ToolResultStatus::Blocked
                }
                _ => crate::core::app::session::ToolResultStatus::Denied,
            };
            let meta = ToolResultMeta::new(
                Some(server_label),
                Some(request.server_id.clone()),
                request.tool_call_id.clone(),
                Some(request.raw_arguments.clone()),
            );
            super::record_tool_result(
                app,
                &request.tool_name,
                meta,
                message.to_string(),
                status,
                ctx,
            );
            return super::advance_tool_queue(app, ctx);
        }

        app.session.tool_pipeline.active_tool_request = Some(request.clone());
        if super::is_instant_recall_tool(&request.tool_name) {
            return super::handle_instant_recall_tool_request(app, request, ctx);
        }
        super::set_status_for_tool_run(app, &request, ctx);
        return Some(AppCommand::RunMcpTool(request));
    }

    let request = app.session.tool_pipeline.active_sampling_request.take()?;
    super::sampling::handle_sampling_permission_decision(app, request, decision, ctx)
}

pub(super) fn handle_tool_call_completed(
    app: &mut App,
    tool_name: String,
    tool_call_id: Option<String>,
    result: Result<String, String>,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let Some(active_request) = app.session.tool_pipeline.active_tool_request.as_ref() else {
        app.end_mcp_operation_if_active();
        return None;
    };
    if let (Some(active_id), Some(completed_id)) = (
        active_request.tool_call_id.as_deref(),
        tool_call_id.as_deref(),
    ) {
        if active_id != completed_id {
            return None;
        }
    }
    let request = app
        .session
        .tool_pipeline
        .active_tool_request
        .take()
        .expect("active tool request should still be present");
    let server_label = Some(super::resolve_server_label(app, &request.server_id));
    app.end_mcp_operation_if_active();

    match result {
        Ok(payload) => {
            let is_tool_error = is_tool_error_payload(&payload);
            let mut meta = ToolResultMeta::new(
                server_label,
                Some(request.server_id.clone()),
                tool_call_id.clone(),
                Some(request.raw_arguments.clone()),
            );
            if is_tool_error {
                meta.failure_kind = Some(ToolFailureKind::ToolError);
            }
            super::record_tool_result(
                app,
                &tool_name,
                meta,
                payload,
                if is_tool_error {
                    crate::core::app::session::ToolResultStatus::Error
                } else {
                    crate::core::app::session::ToolResultStatus::Success
                },
                ctx,
            );
        }
        Err(err) => {
            let mut meta = ToolResultMeta::new(
                server_label,
                Some(request.server_id.clone()),
                tool_call_id,
                Some(request.raw_arguments.clone()),
            );
            meta.failure_kind = Some(ToolFailureKind::ToolCallFailure);
            super::record_tool_result(
                app,
                &tool_name,
                meta,
                format!("Tool call failure: {err}"),
                crate::core::app::session::ToolResultStatus::Error,
                ctx,
            );
        }
    }

    super::advance_tool_queue(app, ctx)
}

fn is_tool_error_payload(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|value| value.get("isError").and_then(Value::as_bool))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::session::{ToolCallRequest, ToolResultStatus};
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn tool_call_completed_flags_tool_error_payloads() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.tool_pipeline.active_tool_request = Some(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "lookup".to_string(),
            arguments: None,
            raw_arguments: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
        });

        let payload = serde_json::json!({"content": [], "isError": true}).to_string();
        let result = handle_tool_call_completed(
            &mut app,
            "lookup".to_string(),
            Some("call-1".to_string()),
            Ok(payload),
            ctx,
        );
        assert!(result.is_none());

        let record = app
            .session
            .tool_pipeline
            .tool_result_history
            .last()
            .expect("record");
        assert_eq!(record.status, ToolResultStatus::Error);
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolError));
    }

    #[test]
    fn tool_call_completed_ignores_stale_completion_without_active_request() {
        let mut app = create_test_app();
        app.session.active_assistant_message_index = Some(2);
        app.begin_mcp_operation();
        let ctx = default_ctx();

        let result = handle_tool_call_completed(
            &mut app,
            "lookup".to_string(),
            Some("call-old".to_string()),
            Ok("{\"ok\":true}".to_string()),
            ctx,
        );

        assert!(result.is_none());
        assert!(app.session.tool_pipeline.tool_result_history.is_empty());
        assert!(!app.ui.messages.iter().any(|message| matches!(
            message.role,
            crate::core::message::TranscriptRole::ToolResult
        )));
    }

    #[test]
    fn tool_call_completed_ignores_stale_completion_with_mismatched_call_id() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.tool_pipeline.active_tool_request = Some(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "lookup".to_string(),
            arguments: None,
            raw_arguments: "{}".to_string(),
            tool_call_id: Some("call-current".to_string()),
        });

        let result = handle_tool_call_completed(
            &mut app,
            "lookup".to_string(),
            Some("call-stale".to_string()),
            Ok("{\"ok\":true}".to_string()),
            ctx,
        );

        assert!(result.is_none());
        assert!(app.session.tool_pipeline.tool_result_history.is_empty());
        assert_eq!(
            app.session
                .tool_pipeline
                .active_tool_request
                .as_ref()
                .and_then(|request| request.tool_call_id.as_deref()),
            Some("call-current")
        );
    }
}
