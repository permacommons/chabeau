use super::{App, AppActionContext, AppCommand};

pub(super) fn handle_mcp_init_completed(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    app.session.mcp_init_complete = true;
    app.session.mcp_init_in_progress = false;
    app.end_mcp_operation_if_active();
    if let Some(message) = app.session.pending_mcp_message.take() {
        super::stream_lifecycle::spawn_stream_for_message(app, message, ctx)
    } else {
        None
    }
}

pub(super) fn handle_mcp_send_without_tools(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if let Some(message) = app.session.pending_mcp_message.take() {
        app.end_mcp_operation_if_active();
        Some(AppCommand::SpawnStream(
            super::prepare_stream_params_for_message(app, message, ctx),
        ))
    } else {
        None
    }
}

pub(super) fn should_defer_for_mcp(app: &App) -> bool {
    if app.session.mcp_tools_unsupported
        || app.session.mcp_init_complete
        || !app.session.mcp_init_in_progress
    {
        return false;
    }

    app.mcp.servers().any(|server| server.config.is_enabled())
}

pub(super) fn set_status_for_mcp_wait(app: &mut App, ctx: AppActionContext) {
    let input_area_height = app.input_area_height(ctx.term_width);
    app.begin_mcp_operation();
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::AppCommand;
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn mcp_init_completed_spawns_pending_message() {
        let mut app = create_test_app();
        app.session.pending_mcp_message = Some("hi".into());
        let command = handle_mcp_init_completed(&mut app, default_ctx());
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
    }
}
