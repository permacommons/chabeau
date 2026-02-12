use super::set_status_message;
use super::{update_scroll_after_command, App, AppActionContext, AppCommand, CommandAction};
use crate::commands::{process_input, CommandResult};
use crate::core::app::actions::streaming;
use crate::core::app::StreamingAction;

pub(super) fn handle_command_action(
    app: &mut App,
    action: CommandAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        CommandAction::ProcessCommand { input } => handle_process_command(app, input, ctx),
        CommandAction::CompleteInPlaceEdit { index, new_text } => {
            app.complete_in_place_edit(index, new_text);
            None
        }
        CommandAction::CompleteAssistantEdit { content } => {
            app.complete_assistant_edit(content);
            update_scroll_after_command(app, ctx);
            None
        }
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
            let action = StreamingAction::RefineLastMessage { prompt };
            streaming::handle_streaming_action(app, action, ctx)
        }
        CommandResult::RunMcpPrompt(request) => Some(AppCommand::RunMcpPrompt(request)),
        CommandResult::RefreshMcp { server_id } => {
            app.ui.focus_transcript();
            update_scroll_after_command(app, ctx);
            Some(AppCommand::RefreshMcp { server_id })
        }
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
        let cmd = handle_command_action(
            &mut app,
            CommandAction::ProcessCommand {
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

        let _ = handle_command_action(
            &mut app,
            CommandAction::ProcessCommand {
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

        let cmd = handle_command_action(
            &mut app,
            CommandAction::ProcessCommand {
                input: "/help".into(),
            },
            ctx,
        );

        assert!(cmd.is_none());
        assert!(app.ui.is_transcript_focused());
    }

    #[test]
    fn mcp_command_focuses_transcript() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.ui.focus_input();
        app.config
            .mcp_servers
            .push(crate::core::config::data::McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha MCP".to_string(),
                transport: None,
                base_url: Some("https://mcp.example.com".to_string()),
                command: None,
                args: None,
                env: None,
                enabled: Some(true),
                allowed_tools: None,
                protocol_version: None,
                tool_payloads: None,
                tool_payload_window: None,
                yolo: None,
            });
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let cmd = handle_command_action(
            &mut app,
            CommandAction::ProcessCommand {
                input: "/mcp alpha".into(),
            },
            ctx,
        );

        assert!(matches!(cmd, Some(AppCommand::RefreshMcp { .. })));
        assert!(app.ui.is_transcript_focused());
    }
}
