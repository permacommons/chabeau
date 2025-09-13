use crate::core::app::App;
use chrono::Utc;
use std::fs::File;
use std::io::{BufWriter, Write};

pub enum CommandResult {
    Continue,
    ProcessAsMessage(String),
    OpenModelPicker,
    OpenProviderPicker,
}

pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    let trimmed = input.trim();

    if trimmed.starts_with("/help") {
        let help_md = crate::ui::help::builtin_help_md();
        app.add_system_message(help_md.to_string());
        CommandResult::Continue
    } else if trimmed.starts_with("/log") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();

        match parts.len() {
            1 => {
                // Just "/log" - toggle logging if file is set
                match app.logging.toggle_logging() {
                    Ok(message) => {
                        app.set_status(message);
                        CommandResult::Continue
                    }
                    Err(e) => {
                        app.set_status(format!("Log error: {}", e));
                        CommandResult::Continue
                    }
                }
            }
            2 => {
                // "/log <filename>" - set log file and enable logging
                let filename = parts[1];
                match app.logging.set_log_file(filename.to_string()) {
                    Ok(message) => {
                        app.set_status(message);
                        CommandResult::Continue
                    }
                    Err(e) => {
                        app.set_status(format!("Logfile error: {}", e));
                        CommandResult::Continue
                    }
                }
            }
            _ => {
                app.set_status("Usage: /log [filename]");
                CommandResult::Continue
            }
        }
    } else if trimmed.starts_with("/dump") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();

        match parts.len() {
            1 => {
                // Just "/dump" - dump to default filename with ISO date
                let timestamp = Utc::now().format("%Y-%m-%d").to_string();
                let filename = format!("chabeau-log-{}.txt", timestamp);
                match dump_conversation(app, &filename) {
                    Ok(()) => handle_dump_result(app, Ok(()), &filename),
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("already exists") {
                            app.set_status("Log file already exists.");
                            app.start_file_prompt_dump(filename);
                            CommandResult::Continue
                        } else {
                            handle_dump_result(app, Err(e), &filename)
                        }
                    }
                }
            }
            2 => {
                // "/dump <filename>" - dump to specified filename
                let filename = parts[1];
                handle_dump_result(app, dump_conversation(app, filename), filename)
            }
            _ => {
                app.set_status("Usage: /dump [filename]");
                CommandResult::Continue
            }
        }
    } else if trimmed.starts_with("/theme") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.len() {
            1 => {
                // Open picker
                app.open_theme_picker();
                CommandResult::Continue
            }
            _ => {
                // Try to set theme directly by id/name
                let id = parts[1];
                match app.apply_theme_by_id(id) {
                    Ok(_) => {
                        app.set_status(format!("Theme set: {}", id));
                        CommandResult::Continue
                    }
                    Err(_e) => {
                        app.set_status("Theme error");
                        CommandResult::Continue
                    }
                }
            }
        }
    } else if trimmed.starts_with("/model") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.len() {
            1 => {
                // Open model picker - this is async, so we return a special command result
                CommandResult::OpenModelPicker
            }
            _ => {
                // Try to set model directly by id/name
                let model_id = parts[1];
                app.apply_model_by_id(model_id);
                app.set_status(format!("Model set: {}", model_id));
                CommandResult::Continue
            }
        }
    } else if trimmed.starts_with("/provider") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.len() {
            1 => {
                // Open provider picker
                CommandResult::OpenProviderPicker
            }
            _ => {
                // Try to set provider directly by id/name
                let provider_id = parts[1];
                let (result, should_open_model_picker) = app.apply_provider_by_id(provider_id);
                match result {
                    Ok(_) => {
                        app.set_status(format!("Provider set: {}", provider_id));
                        if should_open_model_picker {
                            // Return special command to trigger model picker
                            CommandResult::OpenModelPicker
                        } else {
                            CommandResult::Continue
                        }
                    }
                    Err(e) => {
                        app.set_status(format!("Provider error: {}", e));
                        CommandResult::Continue
                    }
                }
            }
        }
    } else if trimmed.starts_with("/markdown") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let action = parts.get(1).copied().unwrap_or("");
        let mut new_state = app.markdown_enabled;
        match action.to_ascii_lowercase().as_str() {
            "on" => new_state = true,
            "off" => new_state = false,
            "toggle" | "" => new_state = !new_state,
            _ => {
                app.set_status("Usage: /markdown [on|off|toggle]");
                return CommandResult::Continue;
            }
        }
        app.markdown_enabled = new_state;
        // Persist
        match crate::core::config::Config::load() {
            Ok(mut cfg) => {
                cfg.markdown = Some(new_state);
                if let Err(e) = cfg.save() {
                    let _ = e; // keep detail out of status
                    app.set_status(format!(
                        "Markdown {} (unsaved)",
                        if new_state { "enabled" } else { "disabled" }
                    ));
                } else {
                    app.set_status(format!(
                        "Markdown {}",
                        if new_state { "enabled" } else { "disabled" }
                    ));
                }
            }
            Err(_e) => {
                app.set_status(format!(
                    "Markdown {}",
                    if new_state { "enabled" } else { "disabled" }
                ));
            }
        }
        CommandResult::Continue
    } else if trimmed.starts_with("/syntax") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let action = parts.get(1).copied().unwrap_or("");
        let mut new_state = app.syntax_enabled;
        match action.to_ascii_lowercase().as_str() {
            "on" => new_state = true,
            "off" => new_state = false,
            "toggle" | "" => new_state = !new_state,
            _ => {
                app.set_status("Usage: /syntax [on|off|toggle]");
                return CommandResult::Continue;
            }
        }
        app.syntax_enabled = new_state;
        // Persist
        match crate::core::config::Config::load() {
            Ok(mut cfg) => {
                cfg.syntax = Some(new_state);
                if let Err(e) = cfg.save() {
                    let _ = e;
                    app.set_status(format!(
                        "Syntax {} (unsaved)",
                        if new_state { "on" } else { "off" }
                    ));
                } else {
                    app.set_status(format!("Syntax {}", if new_state { "on" } else { "off" }));
                }
            }
            Err(_e) => {
                app.set_status(format!("Syntax {}", if new_state { "on" } else { "off" }));
            }
        }
        CommandResult::Continue
    } else {
        // Not a command, process as regular message
        CommandResult::ProcessAsMessage(input.to_string())
    }
}

pub fn dump_conversation_with_overwrite(
    app: &App,
    filename: &str,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Filter out system messages and check if conversation is empty
    let conversation_messages: Vec<_> = app
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
            app.set_status(format!("Dumped: {}", filename));
            CommandResult::Continue
        }
        Err(e) => {
            app.set_status(format!("Dump error: {}", e));
            CommandResult::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use std::fs;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_dump_conversation() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.messages.push_back(create_test_message("user", "Hello"));
        app.messages
            .push_back(create_test_message("assistant", "Hi there!"));
        app.messages
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
    fn test_dump_conversation_file_exists() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.messages.push_back(create_test_message("user", "Hello"));
        app.messages
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
        app.messages
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
        assert!(app.status.is_some());
        assert!(app.status.as_ref().unwrap().starts_with("Dumped: "));

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
        assert!(app.status.is_some());
        assert!(app.status.as_ref().unwrap().starts_with("Dump error:"));
    }

    #[test]
    fn theme_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/theme");
        matches!(res, CommandResult::Continue);
        assert!(app.picker.is_some());
        // Picker should have at least the built-ins
        let picker = app.picker.as_ref().unwrap();
        assert!(picker.items.len() >= 3);
    }

    #[test]
    fn model_command_returns_open_picker_result() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/model");
        matches!(res, CommandResult::OpenModelPicker);
    }

    #[test]
    fn model_command_with_id_sets_model() {
        let mut app = create_test_app();
        let original_model = app.model.clone();
        let res = process_input(&mut app, "/model gpt-4");
        matches!(res, CommandResult::Continue);
        assert_eq!(app.model, "gpt-4");
        assert_ne!(app.model, original_model);
    }

    #[test]
    fn theme_picker_supports_filtering() {
        let mut app = create_test_app();
        app.open_theme_picker();

        // Should store all themes for filtering
        assert!(!app.all_available_themes.is_empty());

        // Should start with empty filter
        assert!(app.theme_search_filter.is_empty());

        // Add a filter and verify filtering works
        app.theme_search_filter.push_str("dark");
        app.filter_themes();

        if let Some(picker) = &app.picker {
            // Should have filtered results
            assert!(picker.items.len() <= app.all_available_themes.len());
            // Title should show filter status
            assert!(picker.title.contains("filter: 'dark'"));
        }
    }

    #[test]
    fn picker_supports_home_end_navigation_and_metadata() {
        let mut app = create_test_app();
        app.open_theme_picker();

        if let Some(picker) = &mut app.picker {
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

        if let Some(picker) = &app.picker {
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
        if let Some(picker) = &mut app.picker {
            picker.cycle_sort_mode();
        }
        app.sort_picker_items();
        app.update_picker_title();

        if let Some(picker) = &app.picker {
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
}
