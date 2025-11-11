use super::{streaming, App, AppAction, AppActionContext, AppCommand};
use crate::commands::{process_input, CommandResult};

pub(super) fn handle_input_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::ClearStatus => {
            app.clear_status();
            None
        }
        AppAction::ToggleComposeMode => {
            app.toggle_compose_mode();
            None
        }
        AppAction::CancelFilePrompt => {
            app.cancel_file_prompt();
            None
        }
        AppAction::CancelInPlaceEdit => {
            if app.has_in_place_edit() {
                app.cancel_in_place_edit();
                app.clear_input();
            }
            None
        }
        AppAction::SetStatus { message } => {
            set_status_message(app, message, ctx);
            None
        }
        AppAction::ClearInput => {
            app.clear_input();
            if ctx.term_width > 0 {
                app.recompute_input_layout_after_edit(ctx.term_width);
            }
            None
        }
        AppAction::InsertIntoInput { text } => {
            if !text.is_empty() {
                app.insert_into_input(&text, ctx.term_width);
            }
            None
        }
        AppAction::ProcessCommand { input } => handle_process_command(app, input, ctx),
        AppAction::CompleteInPlaceEdit { index, new_text } => {
            app.complete_in_place_edit(index, new_text);
            None
        }
        AppAction::CompleteAssistantEdit { content } => {
            app.complete_assistant_edit(content);
            update_scroll_after_command(app, ctx);
            None
        }
        _ => unreachable!("non-input action routed to input handler"),
    }
}

fn handle_process_command(
    app: &mut App,
    input: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if input.trim().is_empty() {
        return None;
    }

    match process_input(app, &input) {
        CommandResult::Continue => {
            app.conversation().show_character_greeting_if_needed();
            update_scroll_after_command(app, ctx);
            None
        }
        CommandResult::ContinueWithTranscriptFocus => {
            app.conversation().show_character_greeting_if_needed();
            app.ui.focus_transcript();
            update_scroll_after_command(app, ctx);
            None
        }
        CommandResult::ProcessAsMessage(message) => {
            Some(streaming::spawn_stream_for_message(app, message, ctx))
        }
        CommandResult::OpenModelPicker => match app.prepare_model_picker_request() {
            Ok(request) => Some(AppCommand::LoadModelPicker(request)),
            Err(err) => {
                set_status_message(app, format!("Model picker error: {}", err), ctx);
                None
            }
        },
        CommandResult::OpenProviderPicker => {
            app.open_provider_picker();
            None
        }
        CommandResult::OpenThemePicker => {
            if let Err(err) = app.open_theme_picker() {
                set_status_message(app, format!("Theme picker error: {}", err), ctx);
            }
            None
        }
        CommandResult::OpenCharacterPicker => {
            app.open_character_picker();
            None
        }
        CommandResult::OpenPersonaPicker => {
            app.open_persona_picker();
            None
        }
        CommandResult::OpenPresetPicker => {
            app.open_preset_picker();
            None
        }
        CommandResult::Refine(prompt) => {
            let action = AppAction::RefineLastMessage { prompt };
            streaming::handle_streaming_action(app, action, ctx)
        }
    }
}

pub(super) fn set_status_message(app: &mut App, message: String, ctx: AppActionContext) {
    app.conversation().set_status(message);
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
}

fn update_scroll_after_command(app: &mut App, ctx: AppActionContext) {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn process_command_submits_message() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let cmd = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "hello there".into(),
            },
            ctx,
        );

        assert!(matches!(cmd, Some(AppCommand::SpawnStream(_))));
    }

    #[test]
    fn process_command_opens_theme_picker() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let _ = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/theme".into(),
            },
            ctx,
        );

        assert!(app.picker_session().is_some());
    }

    #[test]
    fn help_command_focuses_transcript() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.ui.focus_input();

        let cmd = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/help".into(),
            },
            ctx,
        );

        assert!(cmd.is_none());
        assert!(app.ui.is_transcript_focused());
    }
}
