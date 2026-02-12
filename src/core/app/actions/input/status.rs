use super::{App, AppActionContext, AppCommand, StatusAction};

pub(super) fn handle_status_action(
    app: &mut App,
    action: StatusAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        StatusAction::SetStatus { message } => {
            set_status_message(app, message, ctx);
            None
        }
        StatusAction::ClearStatus => {
            app.clear_status();
            None
        }
    }
}

pub(crate) fn set_status_message(app: &mut App, message: String, ctx: AppActionContext) {
    app.conversation().set_status(message);
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
}
