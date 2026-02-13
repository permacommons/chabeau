use super::{App, AppActionContext, AppCommand};

pub(super) fn handle_mcp_server_request(
    app: &mut App,
    request: crate::mcp::events::McpServerRequest,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_mcp_server_request(app, request, ctx)
}

pub(super) fn handle_mcp_sampling_finished(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_mcp_sampling_finished(app, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::StreamingAction;
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
        let _ = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::McpSamplingFinished,
            default_ctx(),
        );
        assert!(app.session.active_sampling_request.is_none());
    }
}
