use super::{App, AppActionContext, AppCommand};
use crate::core::message::AppMessageKind;

pub(super) fn spawn_stream_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if super::mcp_gate::should_defer_for_mcp(app) {
        app.session.pending_mcp_message = Some(message);
        super::mcp_gate::set_status_for_mcp_wait(app, ctx);
        return None;
    }

    let params = super::prepare_stream_params_for_message(app, message, ctx);
    Some(AppCommand::SpawnStream(params))
}

pub(super) fn retry_last_message(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    super::prepare_retry_stream(app, ctx)
}

pub(super) fn refine_last_message(
    app: &mut App,
    prompt: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::prepare_refine_stream(app, prompt, ctx)
}

pub(super) fn finalize_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let input_area_height = app.input_area_height(ctx.term_width);
    let pending_tool_calls = {
        let mut conversation = app.conversation();
        let pending = conversation.take_pending_tool_calls();
        if !pending.is_empty() {
            let available_height =
                conversation.calculate_available_height(ctx.term_height, input_area_height);
            conversation.update_scroll_position(available_height, ctx.term_width);
        }
        conversation.finalize_response();
        pending
    };

    app.end_streaming();

    if pending_tool_calls.is_empty() {
        app.session.last_stream_api_messages = None;
        return None;
    }

    super::prepare_tool_flow(app, pending_tool_calls, ctx)
}

pub(super) fn append_response_chunk(app: &mut App, chunk: &str, ctx: AppActionContext) {
    if chunk.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.append_to_response(chunk, available_height, ctx.term_width);
}

pub(super) fn append_stream_app_message(
    app: &mut App,
    kind: AppMessageKind,
    message: String,
    ctx: AppActionContext,
) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    conversation.add_app_message(kind, trimmed.to_string());
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

pub(super) fn append_tool_call_delta(
    app: &mut App,
    delta: crate::core::chat_stream::ToolCallDelta,
) {
    let entry = app
        .session
        .pending_tool_calls
        .entry(delta.index)
        .or_insert_with(|| crate::core::app::session::PendingToolCall {
            id: None,
            name: None,
            arguments: String::new(),
        });

    if delta.id.is_some() {
        entry.id = delta.id;
    }
    if delta.name.is_some() {
        entry.name = delta.name;
    }
    if let Some(arguments) = delta.arguments {
        entry.arguments.push_str(&arguments);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::AppCommand;
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
        let command = spawn_stream_for_message(&mut app, "Hello there".into(), ctx);
        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            _ => panic!("expected stream"),
        };
        append_stream_app_message(
            &mut app,
            AppMessageKind::Warning,
            "  invalid utf8  ".into(),
            ctx,
        );
        let result = finalize_stream(&mut app, ctx);
        assert!(result.is_none() || matches!(result, Some(AppCommand::SpawnStream(_))));
        let last = app.ui.messages.back().expect("message");
        assert_eq!(last.role, ROLE_APP_WARNING);
        assert_eq!(last.content, "invalid utf8");
        assert!(app.ui.is_streaming || stream_id > 0);
    }

    #[test]
    fn stream_tool_call_delta_flushes_on_complete() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let command = spawn_stream_for_message(&mut app, "Run a tool".into(), ctx);
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
        append_tool_call_delta(
            &mut app,
            crate::core::chat_stream::ToolCallDelta {
                index: 0,
                id: Some("call-1".into()),
                name: Some("lookup".into()),
                arguments: Some("{\"q\":".into()),
            },
        );
        append_tool_call_delta(
            &mut app,
            crate::core::chat_stream::ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments: Some("\"mcp\"}".into()),
            },
        );
        let command = finalize_stream(&mut app, ctx);
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
        assert!(app.ui.messages.iter().any(|msg| msg.role == ROLE_TOOL_CALL));
        assert!(app
            .ui
            .messages
            .iter()
            .any(|msg| msg.role == ROLE_TOOL_RESULT));
    }

    #[test]
    fn finalize_stream_clears_interrupt_token_when_done() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = spawn_stream_for_message(&mut app, "Hello there".into(), ctx);
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
        assert!(app.session.stream_cancel_token.is_some());

        let command = finalize_stream(&mut app, ctx);
        assert!(command.is_none());
        assert!(app.session.stream_cancel_token.is_none());
        assert!(!app.has_interruptible_activity());
    }
}
