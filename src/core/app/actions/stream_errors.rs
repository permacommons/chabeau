use super::{App, AppActionContext, AppCommand};
use crate::core::message::AppMessageKind;

pub(super) fn handle_stream_error(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let error_message = message.trim().to_string();
    if app.session.mcp_tools_enabled
        && !app.session.mcp_tools_unsupported
        && is_tool_unsupported_error(&error_message)
    {
        return handle_mcp_unsupported_error(app, ctx);
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        conversation.clear_pending_tool_calls();
        conversation.add_app_message(AppMessageKind::Error, error_message);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
    app.end_streaming();
    None
}

fn handle_mcp_unsupported_error(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    app.session.mcp_tools_unsupported = true;
    app.session.mcp_tools_enabled = false;

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        conversation.clear_pending_tool_calls();
        conversation.add_app_message(
            AppMessageKind::Warning,
            "MCP tool-calling is enabled, but the currently selected model does not support it. It will be disabled until you switch models."
                .to_string(),
        );
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
    app.end_streaming();

    let base_messages = app.session.last_stream_api_messages_base.clone()?;

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    app.ui.focus_transcript();
    app.enable_auto_scroll();

    let input_area_height = app.input_area_height(ctx.term_width);
    let (cancel_token, stream_id) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        conversation.add_assistant_placeholder();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
        (cancel_token, stream_id)
    };

    Some(AppCommand::SpawnStream(app.build_stream_params(
        base_messages,
        cancel_token,
        stream_id,
    )))
}

fn is_tool_unsupported_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let mentions_tools = lower.contains("tools")
        || lower.contains("tool_calls")
        || lower.contains("tool call")
        || lower.contains("function_call")
        || lower.contains("function calling");
    if !mentions_tools {
        return false;
    }

    let unsupported_signals = [
        "not supported",
        "unsupported",
        "unknown field",
        "unknown parameter",
        "unrecognized",
        "unexpected field",
        "invalid parameter",
        "extra fields",
        "does not support",
    ];

    unsupported_signals
        .iter()
        .any(|signal| lower.contains(signal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::AppCommand;
    use crate::core::message::TranscriptRole;
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn stream_error_trims_message_and_keeps_user_history() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = super::super::stream_lifecycle::spawn_stream_for_message(
            &mut app,
            "Hello there".into(),
            ctx,
        );

        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));

        let result = handle_stream_error(&mut app, " network failure ".into(), ctx);
        assert!(result.is_none());
        assert!(app
            .ui
            .messages
            .iter()
            .all(|m| m.role != TranscriptRole::Assistant || !m.content.trim().is_empty()));
        let last = app.ui.messages.back().expect("last");
        assert_eq!(last.role, TranscriptRole::AppError);
        assert_eq!(last.content, "network failure");
        assert_eq!(
            app.ui.messages.front().expect("first").role,
            TranscriptRole::User
        );
    }
}
