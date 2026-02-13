use super::{App, AppActionContext, AppCommand};

pub(super) fn handle_mcp_init_completed(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_mcp_init_completed(app, ctx)
}

pub(super) fn handle_mcp_send_without_tools(
    app: &mut App,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_mcp_send_without_tools(app, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{AppCommand, StreamingAction};
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
        let command = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::McpInitCompleted,
            default_ctx(),
        );
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));
    }
}
