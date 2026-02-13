use super::{App, AppActionContext, AppCommand};
use crate::core::app::session::ToolFailureKind;
use crate::mcp::permissions::ToolPermissionDecision;

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
    super::handle_tool_permission_decision(app, decision, ctx)
}

pub(super) fn handle_tool_call_completed(
    app: &mut App,
    tool_name: String,
    tool_call_id: Option<String>,
    result: Result<String, String>,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_tool_call_completed(app, tool_name, tool_call_id, result, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::StreamingAction;
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
        app.session.active_tool_request = Some(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "lookup".to_string(),
            arguments: None,
            raw_arguments: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
        });

        let payload = serde_json::json!({"content": [], "isError": true}).to_string();
        let result = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::ToolCallCompleted {
                tool_name: "lookup".to_string(),
                tool_call_id: Some("call-1".to_string()),
                result: Ok(payload),
            },
            ctx,
        );
        assert!(result.is_none());

        let record = app.session.tool_result_history.last().expect("record");
        assert_eq!(record.status, ToolResultStatus::Error);
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolError));
    }
}
