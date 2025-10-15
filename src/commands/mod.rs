mod registry;

pub use registry::{all_commands, matching_commands, CommandInvocation};

use crate::core::app::App;
use crate::core::message::{self, AppMessageKind};
use chrono::Utc;
use registry::DispatchOutcome;
use std::fs::File;
use std::io::{BufWriter, Write};

pub enum CommandResult {
    Continue,
    ProcessAsMessage(String),
    OpenModelPicker,
    OpenProviderPicker,
    OpenThemePicker,
    OpenCharacterPicker,
    OpenPersonaPicker,
    OpenPresetPicker,
}

pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    match registry::registry().dispatch(input) {
        DispatchOutcome::NotACommand | DispatchOutcome::UnknownCommand => {
            CommandResult::ProcessAsMessage(input.to_string())
        }
        DispatchOutcome::Invocation(invocation) => {
            let handler = invocation.command.handler;
            handler(app, invocation)
        }
    }
}

pub(super) fn handle_help(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    let mut help_md = crate::ui::help::builtin_help_md().to_string();
    help_md.push_str("\n\n## Commands\n");
    for command in all_commands() {
        for usage in command.usages {
            help_md.push_str(&format!("- `{}` — {}\n", usage.syntax, usage.description));
        }
        for line in command.extra_help {
            help_md.push_str(line);
            help_md.push('\n');
        }
    }
    app.conversation()
        .add_app_message(AppMessageKind::Info, help_md);
    CommandResult::Continue
}

pub(super) fn handle_log(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => match app.session.logging.toggle_logging() {
            Ok(message) => {
                app.conversation().set_status(message);
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation().set_status(format!("Log error: {}", e));
                CommandResult::Continue
            }
        },
        1 => {
            let filename = invocation.arg(0).unwrap();
            match app.session.logging.set_log_file(filename.to_string()) {
                Ok(message) => {
                    app.conversation().set_status(message);
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation()
                        .set_status(format!("Logfile error: {}", e));
                    CommandResult::Continue
                }
            }
        }
        _ => {
            app.conversation().set_status("Usage: /log [filename]");
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_dump(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => {
            let timestamp = Utc::now().format("%Y-%m-%d").to_string();
            let filename = format!("chabeau-log-{}.txt", timestamp);
            match dump_conversation(app, &filename) {
                Ok(()) => handle_dump_result(app, Ok(()), &filename),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("already exists") {
                        app.conversation().set_status("Log file already exists.");
                        app.ui.start_file_prompt_dump(filename);
                        CommandResult::Continue
                    } else {
                        handle_dump_result(app, Err(e), &filename)
                    }
                }
            }
        }
        1 => {
            let filename = invocation.arg(0).unwrap();
            handle_dump_result(app, dump_conversation(app, filename), filename)
        }
        _ => {
            app.conversation().set_status("Usage: /dump [filename]");
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_theme(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenThemePicker
    } else {
        let id = invocation.arg(0).unwrap();
        let res = {
            let mut controller = app.theme_controller();
            controller.apply_theme_by_id(id)
        };
        match res {
            Ok(_) => {
                app.conversation().set_status(format!("Theme set: {}", id));
                CommandResult::Continue
            }
            Err(_e) => {
                app.conversation().set_status("Theme error");
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_model(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenModelPicker
    } else {
        let model_id = invocation.arg(0).unwrap();
        {
            let mut controller = app.provider_controller();
            controller.apply_model_by_id(model_id);
        }
        app.conversation()
            .set_status(format!("Model set: {}", model_id));
        CommandResult::Continue
    }
}

pub(super) fn handle_provider(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenProviderPicker
    } else {
        let provider_id = invocation.arg(0).unwrap();
        let (result, should_open_model_picker) = {
            let mut controller = app.provider_controller();
            controller.apply_provider_by_id(provider_id)
        };
        match result {
            Ok(_) => {
                app.conversation()
                    .set_status(format!("Provider set: {}", provider_id));
                if should_open_model_picker {
                    CommandResult::OpenModelPicker
                } else {
                    CommandResult::Continue
                }
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Provider error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_markdown(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.markdown_enabled,
        ToggleText {
            usage: "Usage: /markdown [on|off|toggle]",
            feature: "Markdown",
            on_word: "enabled",
            off_word: "disabled",
        },
        |app, new_state| app.ui.markdown_enabled = new_state,
        |cfg, new_state| cfg.markdown = Some(new_state),
    )
}

pub(super) fn handle_syntax(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.syntax_enabled,
        ToggleText {
            usage: "Usage: /syntax [on|off|toggle]",
            feature: "Syntax",
            on_word: "on",
            off_word: "off",
        },
        |app, new_state| app.ui.syntax_enabled = new_state,
        |cfg, new_state| cfg.syntax = Some(new_state),
    )
}

pub(super) fn handle_character(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_text().is_empty() {
        CommandResult::OpenCharacterPicker
    } else {
        let character_name = invocation.args_text();
        match app.character_service.resolve(character_name) {
            Ok(card) => {
                let name = card.data.name.clone();
                app.session.set_character(card);
                app.conversation()
                    .set_status(format!("Character set: {}", name));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Character error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_persona(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenPersonaPicker
    } else {
        let persona_id = invocation.arg(0).unwrap();
        match app.persona_manager.set_active_persona(persona_id) {
            Ok(()) => {
                let active_persona_name = app
                    .persona_manager
                    .get_active_persona()
                    .map(|p| p.display_name.clone());

                let persona_name = active_persona_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());

                if active_persona_name.is_some() {
                    let display_name = app.persona_manager.get_display_name();
                    app.ui.update_user_display_name(display_name);
                } else {
                    app.ui.update_user_display_name("You".to_string());
                }
                app.conversation()
                    .set_status(format!("Persona activated: {}", persona_name));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Persona error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_preset(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenPresetPicker
    } else {
        let preset_id = invocation.arg(0).unwrap();
        if preset_id.eq_ignore_ascii_case("off") || preset_id == "[turn_off_preset]" {
            app.preset_manager.clear_active_preset();
            app.conversation()
                .set_status("Preset deactivated".to_string());
            CommandResult::Continue
        } else {
            match app.preset_manager.set_active_preset(preset_id) {
                Ok(()) => {
                    app.conversation()
                        .set_status(format!("Preset activated: {}", preset_id));
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation()
                        .set_status(format!("Preset error: {}", e));
                    CommandResult::Continue
                }
            }
        }
    }
}

struct ToggleText {
    usage: &'static str,
    feature: &'static str,
    on_word: &'static str,
    off_word: &'static str,
}

fn handle_toggle_command<F, G>(
    app: &mut App,
    invocation: CommandInvocation<'_>,
    current_state: bool,
    text: ToggleText,
    mut apply_ui: F,
    mut persist_config: G,
) -> CommandResult
where
    F: FnMut(&mut App, bool),
    G: FnMut(&mut crate::core::config::Config, bool),
{
    let action = match invocation.toggle_action() {
        Ok(action) => action,
        Err(_) => {
            app.conversation().set_status(text.usage);
            return CommandResult::Continue;
        }
    };

    let new_state = action.apply(current_state);
    apply_ui(app, new_state);

    let state_word = if new_state {
        text.on_word
    } else {
        text.off_word
    };

    match crate::core::config::Config::load() {
        Ok(mut cfg) => {
            persist_config(&mut cfg, new_state);
            let status = if cfg.save().is_ok() {
                format!("{} {}", text.feature, state_word)
            } else {
                format!("{} {} (unsaved)", text.feature, state_word)
            };
            app.conversation().set_status(status);
        }
        Err(_) => {
            app.conversation()
                .set_status(format!("{} {}", text.feature, state_word));
        }
    }

    CommandResult::Continue
}

pub fn dump_conversation_with_overwrite(
    app: &App,
    filename: &str,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Filter out app messages and check if conversation is empty
    let conversation_messages: Vec<_> = app
        .ui
        .messages
        .iter()
        .filter(|msg| !message::is_app_message_role(&msg.role))
        .collect();

    if conversation_messages.is_empty() {
        return Err("No conversation to dump - the chat history is empty.".into());
    }

    // Check if file already exists
    if !overwrite && std::path::Path::new(filename).exists() {
        return Err(format!(
            "File '{}' already exists. Please specify a different filename with /dump <filename>.",
            filename
        )
        .into());
    }

    let file = File::create(filename)?;
    let mut writer = BufWriter::new(file);

    let user_display_name = app.persona_manager.get_display_name();

    for msg in conversation_messages {
        match msg.role.as_str() {
            "user" => writeln!(writer, "{}: {}", user_display_name, msg.content)?,
            _ => writeln!(writer, "{}", msg.content)?, // For assistant messages
        }
        writeln!(writer)?; // Empty line for spacing
    }

    writer.flush()?;
    Ok(())
}

fn dump_conversation(app: &App, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    dump_conversation_with_overwrite(app, filename, false)
}

fn handle_dump_result(
    app: &mut App,
    result: Result<(), Box<dyn std::error::Error>>,
    filename: &str,
) -> CommandResult {
    match result {
        Ok(_) => {
            app.conversation()
                .set_status(format!("Dumped: {}", filename));
            CommandResult::Continue
        }
        Err(e) => {
            app.conversation().set_status(format!("Dump error: {}", e));
            CommandResult::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::{create_test_app, create_test_message, with_test_config_env};
    use std::fs;
    use std::io::Read;
    use std::path::Path;
    use tempfile::tempdir;
    use toml::Value;

    fn read_config(path: &Path) -> Value {
        let contents = std::fs::read_to_string(path).unwrap();
        toml::from_str(&contents).unwrap()
    }

    #[test]
    fn registry_lists_commands() {
        let commands = super::all_commands();
        assert!(commands.iter().any(|cmd| cmd.name == "help"));
        assert!(commands.iter().any(|cmd| cmd.name == "markdown"));
        assert!(super::registry::find_command("help").is_some());
    }

    #[test]
    fn help_command_includes_registry_metadata() {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/help");
        assert!(matches!(result, CommandResult::Continue));
        let last_message = app.ui.messages.back().expect("help message");
        assert!(last_message
            .content
            .contains("- `/help` — Show available commands"));
    }

    #[test]
    fn commands_dispatch_case_insensitively() {
        with_test_config_env(|_| {
            let mut app = create_test_app();
            app.ui.markdown_enabled = false;
            let result = process_input(&mut app, "/MarkDown On");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.markdown_enabled);
        });
    }

    #[test]
    fn dispatch_provides_multi_word_arguments() {
        use super::registry::DispatchOutcome;

        let registry = super::registry::registry();
        match registry.dispatch("/character Jean Luc Picard") {
            DispatchOutcome::Invocation(invocation) => {
                assert_eq!(invocation.command.name, "character");
                assert_eq!(invocation.args_text(), "Jean Luc Picard");
                let args: Vec<_> = invocation.args_iter().collect();
                assert_eq!(args, vec!["Jean", "Luc", "Picard"]);
                assert_eq!(invocation.arg(1), Some("Luc"));
            }
            other => panic!("unexpected dispatch outcome: {:?}", other),
        }
    }

    #[test]
    fn dispatch_reports_unknown_commands() {
        use super::registry::DispatchOutcome;

        let registry = super::registry::registry();
        assert!(matches!(
            registry.dispatch("/does-not-exist"),
            DispatchOutcome::UnknownCommand
        ));
    }

    #[test]
    fn markdown_command_rejects_invalid_argument() {
        with_test_config_env(|_| {
            let mut app = create_test_app();
            let result = process_input(&mut app, "/markdown banana");
            assert!(matches!(result, CommandResult::Continue));
            assert_eq!(
                app.ui.status.as_deref(),
                Some("Usage: /markdown [on|off|toggle]")
            );
        });
    }

    #[test]
    fn test_dump_conversation() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));
        app.ui.messages.push_back(create_test_message(
            crate::core::message::ROLE_APP_INFO,
            "App message",
        ));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("test_dump.txt");

        // Test the dump_conversation function
        assert!(dump_conversation(&app, dump_file_path.to_str().unwrap()).is_ok());

        // Read the dumped file and verify its contents
        let mut file = File::open(&dump_file_path).unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();

        // Check that the contents match what we expect
        assert!(contents.contains("You: Hello"));
        assert!(contents.contains("Hi there!"));
        // App messages should be excluded from dumps
        assert!(!contents.contains("App message"));

        // Clean up
        drop(file);
        fs::remove_file(&dump_file_path).unwrap();
    }

    #[test]
    fn dump_conversation_uses_persona_display_name() {
        let mut app = create_test_app();

        let config = Config {
            personas: vec![Persona {
                id: "captain".to_string(),
                display_name: "Captain".to_string(),
                bio: None,
            }],
            ..Default::default()
        };

        app.persona_manager = PersonaManager::load_personas(&config).unwrap();
        app.persona_manager
            .set_active_persona("captain")
            .expect("Failed to activate persona");

        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("persona_dump.txt");
        dump_conversation_with_overwrite(&app, dump_file_path.to_str().unwrap(), true)
            .expect("failed to dump conversation");

        let contents = fs::read_to_string(&dump_file_path).expect("failed to read dump file");
        assert!(
            contents.contains("Captain: Hello"),
            "Dump should include persona display name, contents: {contents}"
        );
    }

    #[test]
    fn markdown_command_updates_state_and_persists() {
        with_test_config_env(|config_root| {
            let config_path = config_root.join("chabeau").join("config.toml");
            let mut app = create_test_app();
            app.ui.markdown_enabled = true;

            let result = process_input(&mut app, "/markdown off");
            assert!(matches!(result, CommandResult::Continue));
            assert!(!app.ui.markdown_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Markdown disabled"));

            assert!(config_path.exists());
            let config = read_config(&config_path);
            assert_eq!(config["markdown"].as_bool(), Some(false));

            let result = process_input(&mut app, "/markdown toggle");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.markdown_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Markdown enabled"));

            let config = read_config(&config_path);
            assert_eq!(config["markdown"].as_bool(), Some(true));
        });
    }

    #[test]
    fn syntax_command_updates_state_and_persists() {
        with_test_config_env(|config_root| {
            let config_path = config_root.join("chabeau").join("config.toml");
            let mut app = create_test_app();
            app.ui.syntax_enabled = true;

            let result = process_input(&mut app, "/syntax off");
            assert!(matches!(result, CommandResult::Continue));
            assert!(!app.ui.syntax_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Syntax off"));

            assert!(config_path.exists());
            let config = read_config(&config_path);
            assert_eq!(config["syntax"].as_bool(), Some(false));

            let result = process_input(&mut app, "/syntax toggle");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.syntax_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Syntax on"));

            let config = read_config(&config_path);
            assert_eq!(config["syntax"].as_bool(), Some(true));
        });
    }

    #[test]
    fn test_dump_conversation_file_exists() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("test_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Create a file that already exists
        fs::write(&dump_file_path, "existing content").unwrap();

        // Test the dump_conversation function with existing file
        // This should fail because the file already exists
        let result = dump_conversation(&app, dump_filename);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));

        // Check that the existing file content is still there
        let contents = fs::read_to_string(&dump_file_path).unwrap();
        assert_eq!(contents, "existing content");

        // Clean up
        fs::remove_file(&dump_file_path).unwrap();
    }

    #[test]
    fn test_process_input_dump_with_filename() {
        let mut app = create_test_app();

        // Add a message to test dumping
        app.ui
            .messages
            .push_back(create_test_message("user", "Test message"));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("custom_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should set a status about the dump
        assert!(app.ui.status.is_some());
        assert!(app.ui.status.as_ref().unwrap().starts_with("Dumped: "));

        // Clean up
        fs::remove_file(dump_filename).ok();
    }

    #[test]
    fn test_process_input_dump_empty_conversation() {
        let mut app = create_test_app();

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("empty_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command with an empty conversation
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should set a status with an error
        assert!(app.ui.status.is_some());
        assert!(app.ui.status.as_ref().unwrap().starts_with("Dump error:"));
    }

    #[test]
    fn theme_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/theme");
        assert!(matches!(res, CommandResult::OpenThemePicker));
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn model_command_returns_open_picker_result() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/model");
        assert!(matches!(res, CommandResult::OpenModelPicker));
    }

    #[test]
    fn model_command_with_id_sets_model() {
        let mut app = create_test_app();
        let original_model = app.session.model.clone();
        let res = process_input(&mut app, "/model gpt-4");
        assert!(matches!(res, CommandResult::Continue));
        assert_eq!(app.session.model, "gpt-4");
        assert_ne!(app.session.model, original_model);
    }

    #[test]
    fn provider_command_with_same_id_reuses_session() {
        let mut app = create_test_app();
        app.picker.provider_model_transition_state = Some((
            "prev-provider".into(),
            "Prev".into(),
            "prev-model".into(),
            "prev-key".into(),
            "https://prev.example".into(),
        ));
        app.picker.in_provider_model_transition = false;

        let result = process_input(&mut app, "/provider TEST");

        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(app.session.provider_name, "test");
        assert_eq!(app.session.api_key, "test-key");
        assert_eq!(app.ui.status.as_deref(), Some("Provider set: TEST"));
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
    }

    #[test]
    fn theme_picker_supports_filtering() {
        let mut app = create_test_app();
        app.open_theme_picker().expect("theme picker opens");

        // Should store all themes for filtering
        assert!(app
            .theme_picker_state()
            .map(|state| !state.all_items.is_empty())
            .unwrap_or(false));

        // Should start with empty filter
        assert!(app
            .theme_picker_state()
            .map(|state| state.search_filter.is_empty())
            .unwrap_or(true));

        // Add a filter and verify filtering works
        if let Some(state) = app.theme_picker_state_mut() {
            state.search_filter.push_str("dark");
        }
        app.filter_themes();

        if let Some(picker) = app.picker_state() {
            // Should have filtered results
            let total = app
                .theme_picker_state()
                .map(|state| state.all_items.len())
                .unwrap_or(0);
            assert!(picker.items.len() <= total);
            // Title should show filter status
            assert!(picker.title.contains("filter: 'dark'"));
        }
    }

    #[test]
    fn picker_supports_home_end_navigation_and_metadata() {
        let mut app = create_test_app();
        app.open_theme_picker().expect("theme picker opens");

        if let Some(picker) = app.picker_state_mut() {
            // Test Home key (move to start)
            picker.selected = picker.items.len() - 1; // Move to last
            picker.move_to_start();
            assert_eq!(picker.selected, 0);

            // Test End key (move to end)
            picker.move_to_end();
            assert_eq!(picker.selected, picker.items.len() - 1);

            // Test metadata is available
            let metadata = picker.get_selected_metadata();
            assert!(metadata.is_some());

            // Test sort mode cycling
            let original_sort = picker.sort_mode.clone();
            picker.cycle_sort_mode();
            assert_ne!(picker.sort_mode, original_sort);

            // Test items have metadata
            assert!(picker.items.iter().any(|item| item.metadata.is_some()));
        }
    }

    #[test]
    fn theme_picker_shows_a_z_sort_indicators() {
        let mut app = create_test_app();

        // Open theme picker - should default to A-Z (Name mode)
        app.open_theme_picker().expect("theme picker opens");

        if let Some(picker) = app.picker_state() {
            // Should default to Name mode (A-Z)
            assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Name);
            // Title should show "Sort by: A-Z"
            assert!(
                picker.title.contains("Sort by: A-Z"),
                "Theme picker should show 'Sort by: A-Z', got: {}",
                picker.title
            );
        }

        // Cycle to Z-A mode
        if let Some(picker) = app.picker_state_mut() {
            picker.cycle_sort_mode();
        }
        app.sort_picker_items();
        app.update_picker_title();

        if let Some(picker) = app.picker_state() {
            // Should now be in Date mode (Z-A for themes)
            assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Date);
            // Title should show "Sort by: Z-A"
            assert!(
                picker.title.contains("Sort by: Z-A"),
                "Theme picker should show 'Sort by: Z-A', got: {}",
                picker.title
            );
        }
    }

    #[test]
    fn character_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/character");
        assert!(matches!(res, CommandResult::OpenCharacterPicker));
    }

    #[test]
    fn character_command_with_invalid_name_shows_error() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/character nonexistent_character");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
        let status = app.ui.status.as_ref().unwrap();
        assert!(
            status.contains("Character error") || status.contains("not found"),
            "Expected error message, got: {}",
            status
        );
    }

    #[test]
    fn character_command_registered_in_help() {
        let commands = super::all_commands();
        assert!(commands.iter().any(|cmd| cmd.name == "character"));

        let character_cmd = commands.iter().find(|cmd| cmd.name == "character").unwrap();
        assert_eq!(character_cmd.usages.len(), 2);
        assert!(character_cmd.usages[0].syntax.contains("/character"));
        assert!(character_cmd.usages[1].syntax.contains("<name>"));
    }

    #[test]
    fn persona_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/persona");
        assert!(matches!(res, CommandResult::OpenPersonaPicker));
    }

    #[test]
    fn persona_command_with_invalid_id_shows_error() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/persona nonexistent_persona");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
        let status = app.ui.status.as_ref().unwrap();
        assert!(
            status.contains("Persona error") || status.contains("not found"),
            "Expected error message, got: {}",
            status
        );
    }

    #[test]
    fn persona_command_with_valid_id_updates_user_display_name() {
        let mut app = create_test_app();
        let mut config = crate::core::config::Config::default();
        config.personas.push(crate::core::config::Persona {
            id: "alice-dev".to_string(),
            display_name: "Alice".to_string(),
            bio: Some("A senior software developer".to_string()),
        });
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config).unwrap();
        assert_eq!(app.ui.user_display_name, "You");

        let res = process_input(&mut app, "/persona alice-dev");

        assert!(matches!(res, CommandResult::Continue));
        assert_eq!(app.ui.user_display_name, "Alice");
    }
}
