use super::{streaming, App, AppAction, AppActionContext, AppCommand};
use crate::commands::{process_input, CommandResult};
use crate::core::app::picker::build_inspect_text;
use crate::core::app::session::ToolResultRecord;
use crate::core::app::InspectMode;

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
        AppAction::CancelMcpPromptInput => {
            app.ui.cancel_mcp_prompt_input();
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
        AppAction::InspectToolResults => {
            open_latest_tool_result_inspect(app, ctx);
            None
        }
        AppAction::InspectToolResultsStep { delta } => {
            step_tool_result_inspect(app, delta, ctx);
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
            streaming::spawn_stream_for_message(app, message, ctx)
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
        CommandResult::RunMcpPrompt(request) => Some(AppCommand::RunMcpPrompt(request)),
        CommandResult::RefreshMcp { server_id } => {
            update_scroll_after_command(app, ctx);
            Some(AppCommand::RefreshMcp { server_id })
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

fn open_latest_tool_result_inspect(app: &mut App, ctx: AppActionContext) {
    if app.session.tool_result_history.is_empty() {
        set_status_message(app, "No tool results to inspect yet.".to_string(), ctx);
        return;
    }
    let index = app.session.tool_result_history.len().saturating_sub(1);
    open_tool_result_inspect_at(app, index, ctx);
}

fn step_tool_result_inspect(app: &mut App, delta: i32, ctx: AppActionContext) {
    let Some(state) = app.inspect_state() else {
        return;
    };
    let InspectMode::ToolResults { index } = state.mode else {
        return;
    };
    let total = app.session.tool_result_history.len();
    if total < 2 {
        return;
    }
    let step = if delta >= 0 {
        1usize
    } else {
        total.saturating_sub(1)
    };
    let next = (index + step) % total;
    open_tool_result_inspect_at(app, next, ctx);
}

fn open_tool_result_inspect_at(app: &mut App, index: usize, ctx: AppActionContext) {
    let Some(record) = app.session.tool_result_history.get(index).cloned() else {
        set_status_message(app, "Tool result unavailable.".to_string(), ctx);
        return;
    };

    let title = build_tool_result_title(&record, index, app.session.tool_result_history.len());
    let content = build_tool_result_content(&record);
    app.open_tool_result_inspect(title, content, index);
    set_status_message(app, "Inspecting tool result (Esc=Close)".to_string(), ctx);
}

fn build_tool_result_title(record: &ToolResultRecord, index: usize, total: usize) -> String {
    let position = format!("{}/{}", index + 1, total.max(1));
    let status = record.status.display();
    format!("Tool result ({position}) – {} ({status})", record.tool_name)
}

fn build_tool_result_content(record: &ToolResultRecord) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Tool: {}", record.tool_name));
    lines.push(format!("Status: {}", record.status.display()));
    if let Some(server) = record.server_name.as_ref() {
        if !server.trim().is_empty() {
            lines.push(format!("Server: {}", server));
        }
    }
    if let Some(tool_call_id) = record.tool_call_id.as_ref() {
        if !tool_call_id.trim().is_empty() {
            lines.push(format!("Tool call id: {}", tool_call_id));
        }
    }
    lines.push(String::new());
    lines.push("Result:".to_string());

    let payload = format_tool_payload_for_inspect(&record.content);
    if payload.trim().is_empty() {
        lines.push("  (empty)".to_string());
    } else {
        for line in payload.lines() {
            lines.push(format!("  {}", line));
        }
    }

    build_inspect_text(lines)
}

fn format_tool_payload_for_inspect(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string()),
        Err(_) => trimmed.to_string(),
    }
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
