use super::{App, AppActionContext, AppCommand, ComposeAction};

pub(super) fn handle_compose_action(
    app: &mut App,
    action: ComposeAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        ComposeAction::ToggleComposeMode => {
            app.toggle_compose_mode();
            None
        }
        ComposeAction::CancelFilePrompt => {
            app.cancel_file_prompt();
            None
        }
        ComposeAction::CancelMcpPromptInput => {
            app.ui.cancel_mcp_prompt_input();
            None
        }
        ComposeAction::CancelInPlaceEdit => {
            if app.has_in_place_edit() {
                app.cancel_in_place_edit();
                app.clear_input();
            }
            None
        }
        ComposeAction::ClearInput => {
            app.clear_input();
            if ctx.term_width > 0 {
                app.recompute_input_layout_after_edit(ctx.term_width);
            }
            None
        }
        ComposeAction::InsertIntoInput { text } => {
            if !text.is_empty() {
                app.insert_into_input(&text, ctx.term_width);
            }
            None
        }
    }
}
