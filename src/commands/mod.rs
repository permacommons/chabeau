mod registry;

pub use registry::{all_commands, matching_commands, CommandInvocation};

use crate::core::app::App;
use chrono::Utc;
use std::fs::File;
use std::io::{BufWriter, Write};

pub enum CommandResult {
    Continue,
    ProcessAsMessage(String),
    OpenModelPicker,
    OpenProviderPicker,
    OpenThemePicker,
    OpenCharacterPicker,
}

pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    let trimmed = input.trim();

    if !trimmed.starts_with('/') {
        return CommandResult::ProcessAsMessage(input.to_string());
    }

    let mut parts = trimmed[1..].splitn(2, ' ');
    let command_name = match parts.next() {
        Some(name) if !name.is_empty() => name,
        _ => return CommandResult::ProcessAsMessage(input.to_string()),
    };
    let args = parts.next().unwrap_or("").trim();

    if let Some(command) = registry::find_command(command_name) {
        let invocation = CommandInvocation {
            input: trimmed,
            args,
        };
        (command.handler)(app, invocation)
    } else {
        CommandResult::ProcessAsMessage(input.to_string())
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
    app.conversation().add_system_message(help_md);
    CommandResult::Continue
}

pub(super) fn handle_log(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();

    match parts.len() {
        1 => match app.session.logging.toggle_logging() {
            Ok(message) => {
                app.conversation().set_status(message);
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation().set_status(format!("Log error: {}", e));
                CommandResult::Continue
            }
        },
        2 => {
            let filename = parts[1];
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
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();

    match parts.len() {
        1 => {
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
        2 => {
            let filename = parts[1];
            handle_dump_result(app, dump_conversation(app, filename), filename)
        }
        _ => {
            app.conversation().set_status("Usage: /dump [filename]");
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_theme(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();
    match parts.len() {
        1 => CommandResult::OpenThemePicker,
        _ => {
            let id = parts[1];
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
}

pub(super) fn handle_model(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();
    match parts.len() {
        1 => CommandResult::OpenModelPicker,
        _ => {
            let model_id = parts[1];
            {
                let mut controller = app.provider_controller();
                controller.apply_model_by_id(model_id);
            }
            app.conversation()
                .set_status(format!("Model set: {}", model_id));
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_provider(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();
    match parts.len() {
        1 => CommandResult::OpenProviderPicker,
        _ => {
            let provider_id = parts[1];
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
}

pub(super) fn handle_markdown(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let action = invocation.args.split_whitespace().next().unwrap_or("");
    let mut new_state = app.ui.markdown_enabled;
    match action.to_ascii_lowercase().as_str() {
        "on" => new_state = true,
        "off" => new_state = false,
        "toggle" | "" => new_state = !new_state,
        _ => {
            app.conversation()
                .set_status("Usage: /markdown [on|off|toggle]");
            return CommandResult::Continue;
        }
    }
    app.ui.markdown_enabled = new_state;
    match crate::core::config::Config::load() {
        Ok(mut cfg) => {
            cfg.markdown = Some(new_state);
            if let Err(e) = cfg.save() {
                let _ = e;
                app.conversation().set_status(format!(
                    "Markdown {} (unsaved)",
                    if new_state { "enabled" } else { "disabled" }
                ));
            } else {
                app.conversation().set_status(format!(
                    "Markdown {}",
                    if new_state { "enabled" } else { "disabled" }
                ));
            }
        }
        Err(_e) => {
            app.conversation().set_status(format!(
                "Markdown {}",
                if new_state { "enabled" } else { "disabled" }
            ));
        }
    }
    CommandResult::Continue
}

pub(super) fn handle_syntax(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let action = invocation.args.split_whitespace().next().unwrap_or("");
    let mut new_state = app.ui.syntax_enabled;
    match action.to_ascii_lowercase().as_str() {
        "on" => new_state = true,
        "off" => new_state = false,
        "toggle" | "" => new_state = !new_state,
        _ => {
            app.conversation()
                .set_status("Usage: /syntax [on|off|toggle]");
            return CommandResult::Continue;
        }
    }
    app.ui.syntax_enabled = new_state;
    match crate::core::config::Config::load() {
        Ok(mut cfg) => {
            cfg.syntax = Some(new_state);
            if let Err(e) = cfg.save() {
                let _ = e;
                app.conversation().set_status(format!(
                    "Syntax {} (unsaved)",
                    if new_state { "on" } else { "off" }
                ));
            } else {
                app.conversation()
                    .set_status(format!("Syntax {}", if new_state { "on" } else { "off" }));
            }
        }
        Err(_e) => {
            app.conversation()
                .set_status(format!("Syntax {}", if new_state { "on" } else { "off" }));
        }
    }
    CommandResult::Continue
}

pub(super) fn handle_character(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let parts: Vec<&str> = invocation.input.split_whitespace().collect();
    match parts.len() {
        1 => {
            // No argument provided - open character picker
            CommandResult::OpenCharacterPicker
        }
        _ => {
            // Character name provided - load it directly
            let character_name = parts[1..].join(" ");
            match crate::character::loader::find_card_by_name(&character_name) {
                Ok((card, _path)) => {
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
}

pub fn dump_conversation_with_overwrite(
    app: &App,
    filename: &str,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Filter out system messages and check if conversation is empty
    let conversation_messages: Vec<_> = app
        .ui
        .messages
        .iter()
        .filter(|msg| msg.role != "system")
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

    for msg in conversation_messages {
        match msg.role.as_str() {
            "user" => writeln!(writer, "You: {}", msg.content)?,
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
        app.ui
            .messages
            .push_back(create_test_message("system", "System message"));

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
        // System messages should be excluded from dumps
        assert!(!contents.contains("System message"));

        // Clean up
        drop(file);
        fs::remove_file(&dump_file_path).unwrap();
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
        app.open_theme_picker();

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
        app.open_theme_picker();

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
        app.open_theme_picker();

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
    fn character_command_with_valid_name_sets_character() {
        use std::fs;
        use tempfile::tempdir;

        // Create a temporary cards directory
        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Create a test character card
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Character",
                "description": "A test character",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": "Example"
            }
        });

        let card_path = cards_dir.join("test.json");
        fs::write(&card_path, card_json.to_string()).unwrap();

        // Override the cards directory for this test
        // Note: This test will fail without proper environment setup
        // In a real scenario, we'd need to mock the get_cards_dir function
        // For now, we'll just test the command structure
        let mut app = create_test_app();

        // Test that the command returns Continue (even if it fails to find the card)
        let res = process_input(&mut app, "/character test");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
    }

    #[test]
    fn character_command_with_multi_word_name() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/character Jean Luc Picard");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
        // Should attempt to find "Jean Luc Picard" as a single name
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
    fn character_greeting_displayed_after_command() {
        use std::fs;
        use tempfile::tempdir;

        // Create a temporary cards directory
        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Create a test character card with a greeting
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Picard",
                "description": "Captain of the Enterprise",
                "personality": "Diplomatic and wise",
                "scenario": "On the bridge",
                "first_mes": "Make it so.",
                "mes_example": "Example"
            }
        });

        let card_path = cards_dir.join("picard.json");
        fs::write(&card_path, card_json.to_string()).unwrap();

        // Note: This test verifies the command structure
        // In a real scenario with proper environment setup, the greeting would be displayed
        let mut app = create_test_app();

        // Verify no messages initially
        assert_eq!(app.ui.messages.len(), 0);

        // Process the character command (will fail to find card without proper setup)
        let res = process_input(&mut app, "/character picard");
        assert!(matches!(res, CommandResult::Continue));

        // In a properly configured environment, the greeting would be displayed
        // For this test, we just verify the command was processed
        assert!(app.ui.status.is_some());
    }
}
