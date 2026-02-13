use super::{App, AppActionContext, AppCommand};

pub(super) fn spawn_stream_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::spawn_stream_for_message(app, message, ctx)
}

pub(super) fn retry_last_message(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    super::retry_last_message(app, ctx)
}

pub(super) fn refine_last_message(
    app: &mut App,
    prompt: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::refine_last_message(app, prompt, ctx)
}

pub(super) fn finalize_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    super::finalize_stream(app, ctx)
}

pub(super) fn append_response_chunk(app: &mut App, chunk: &str, ctx: AppActionContext) {
    super::append_response_chunk(app, chunk, ctx)
}

pub(super) fn append_stream_app_message(
    app: &mut App,
    kind: crate::core::message::AppMessageKind,
    message: String,
    ctx: AppActionContext,
) {
    super::append_stream_app_message(app, kind, message, ctx)
}

pub(super) fn append_tool_call_delta(
    app: &mut App,
    delta: crate::core::chat_stream::ToolCallDelta,
) {
    super::append_tool_call_delta(app, delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{AppCommand, StreamingAction};
    use crate::core::message::{
        AppMessageKind, ROLE_APP_WARNING, ROLE_TOOL_CALL, ROLE_TOOL_RESULT,
    };
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn stream_app_message_adds_trimmed_content_and_keeps_stream_alive() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let command = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::SubmitMessage {
                message: "Hello there".into(),
            },
            ctx,
        );
        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            _ => panic!("expected stream"),
        };
        let result = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::StreamAppMessage {
                kind: AppMessageKind::Warning,
                message: "  invalid utf8  ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());
        let last = app.ui.messages.back().expect("message");
        assert_eq!(last.role, ROLE_APP_WARNING);
        assert_eq!(last.content, "invalid utf8");
        assert!(app.ui.is_streaming);
    }

    #[test]
    fn stream_tool_call_delta_flushes_on_complete() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let command = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::SubmitMessage {
                message: "Run a tool".into(),
            },
            ctx,
        );
        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            _ => panic!("expected stream"),
        };
        super::super::handle_streaming_action(
            &mut app,
            StreamingAction::StreamToolCallDelta {
                stream_id,
                delta: crate::core::chat_stream::ToolCallDelta {
                    index: 0,
                    id: Some("call-1".into()),
                    name: Some("lookup".into()),
                    arguments: Some("{\"q\":".into()),
                },
            },
            ctx,
        );
        super::super::handle_streaming_action(
            &mut app,
            StreamingAction::StreamToolCallDelta {
                stream_id,
                delta: crate::core::chat_stream::ToolCallDelta {
                    index: 0,
                    id: None,
                    name: None,
                    arguments: Some("\"mcp\"}".into()),
                },
            },
            ctx,
        );
        let command = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::StreamCompleted { stream_id },
            ctx,
        );
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
        assert!(app.ui.messages.iter().any(|msg| msg.role == ROLE_TOOL_CALL));
        assert!(app
            .ui
            .messages
            .iter()
            .any(|msg| msg.role == ROLE_TOOL_RESULT));
    }
}
