use crate::core::app::App;
use chrono::Utc;
use std::fs::File;
use std::io::{BufWriter, Write};

pub enum CommandResult {
    Continue,
    ProcessAsMessage(String),
}

pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    let trimmed = input.trim();

    if trimmed.starts_with("/help") {
        // Display extended help information
        let help_text = vec![
            "Chabeau - Terminal Chat Interface Help",
            "",
            "Keyboard Shortcuts:",
            "  Enter             Send the message",
            "  Alt+Enter         Insert newline in input",
            "  Ctrl+A            Move cursor to beginning of input",
            "  Ctrl+E            Move cursor to end of input",
            "  Left/Right        Move cursor left/right in input",
            "  Shift+Left/Right  Move cursor left/right in input (alias)",
            "  Shift+Up/Down     Move cursor up/down lines in multi-line input",
            "  Ctrl+C            Quit the application",
            "  Ctrl+T            Open external editor (requires EDITOR env var)",
            "  Ctrl+R            Retry the last bot response",
            "  Esc               Interrupt streaming response",
            "  Up/Down           Scroll through chat history",
            "  Mouse Wheel       Scroll through chat history",
            "  Backspace         Delete characters in input field",
            "",
            "Chat Commands:",
            "  /help             Show this help message",
            "  /log <filename>   Enable logging to specified file",
            "  /log              Toggle logging pause/resume",
            "  /dump <filename>  Dump conversation to specified file",
            "  /dump             Dump conversation to chabeau-log-<isodate>.txt",
            "",
            "External Editor Setup:",
            "  export EDITOR=nano          # Use nano",
            "  export EDITOR=vim           # Use vim",
            "  export EDITOR=code          # Use VS Code",
            "  export EDITOR=\"code --wait\" # Use VS Code (wait for close)",
            "",
            "Tips:",
            "  • Use Ctrl+T to compose longer messages in your editor",
            "  • Scroll manually to disable auto-scroll to bottom",
            "  • Use /log to save conversations to files",
            "  • Use /dump to save a snapshot of the current conversation",
            "  • Press Esc to stop a streaming response early",
        ];

        // Create a single system message with proper newlines
        let help_message = help_text.join("\n");
        app.add_system_message(help_message);

        CommandResult::Continue
    } else if trimmed.starts_with("/log") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();

        match parts.len() {
            1 => {
                // Just "/log" - toggle logging if file is set
                match app.logging.toggle_logging() {
                    Ok(message) => {
                        app.add_system_message(message);
                        CommandResult::Continue
                    }
                    Err(e) => {
                        app.add_system_message(format!("Error: {e}"));
                        CommandResult::Continue
                    }
                }
            }
            2 => {
                // "/log <filename>" - set log file and enable logging
                let filename = parts[1];
                match app.logging.set_log_file(filename.to_string()) {
                    Ok(message) => {
                        app.add_system_message(message);
                        CommandResult::Continue
                    }
                    Err(e) => {
                        app.add_system_message(format!("Error setting log file: {e}"));
                        CommandResult::Continue
                    }
                }
            }
            _ => {
                app.add_system_message("Usage: /log [filename] - Enable logging to file, or /log to toggle pause/resume".to_string());
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
                handle_dump_result(app, dump_conversation(app, &filename), &filename)
            }
            2 => {
                // "/dump <filename>" - dump to specified filename
                let filename = parts[1];
                handle_dump_result(app, dump_conversation(app, filename), filename)
            }
            _ => {
                app.add_system_message("Usage: /dump [filename] - Dump conversation to file, or /dump for default filename".to_string());
                CommandResult::Continue
            }
        }
    } else {
        // Not a command, process as regular message
        CommandResult::ProcessAsMessage(input.to_string())
    }
}

fn dump_conversation(app: &App, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
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
    if std::path::Path::new(filename).exists() {
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

fn handle_dump_result(
    app: &mut App,
    result: Result<(), Box<dyn std::error::Error>>,
    filename: &str,
) -> CommandResult {
    match result {
        Ok(_) => {
            app.add_system_message(format!("Conversation dumped to: {}", filename));
            CommandResult::Continue
        }
        Err(e) => {
            app.add_system_message(format!("Error dumping conversation: {}", e));
            CommandResult::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::App;
    use crate::utils::logging::LoggingState;
    use std::collections::VecDeque;
    use std::fs;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_dump_conversation() {
        // Create a mock app with some messages
        let mut app = App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            current_response: String::new(),
            client: reqwest::Client::new(),
            model: "test-model".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            provider_name: "test".to_string(),
            provider_display_name: "Test".to_string(),
            scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
            stream_interrupted: false,
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
        };

        // Add messages
        app.messages.push_back(crate::core::message::Message {
            role: "user".to_string(),
            content: "Hello".to_string(),
        });
        app.messages.push_back(crate::core::message::Message {
            role: "assistant".to_string(),
            content: "Hi there!".to_string(),
        });
        app.messages.push_back(crate::core::message::Message {
            role: "system".to_string(),
            content: "System message".to_string(),
        });

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
        let mut app = App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            current_response: String::new(),
            client: reqwest::Client::new(),
            model: "test-model".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            provider_name: "test".to_string(),
            provider_display_name: "Test".to_string(),
            scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
            stream_interrupted: false,
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
        };

        // Add messages
        app.messages.push_back(crate::core::message::Message {
            role: "user".to_string(),
            content: "Hello".to_string(),
        });
        app.messages.push_back(crate::core::message::Message {
            role: "assistant".to_string(),
            content: "Hi there!".to_string(),
        });

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
        let mut app = App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            current_response: String::new(),
            client: reqwest::Client::new(),
            model: "test-model".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            provider_name: "test".to_string(),
            provider_display_name: "Test".to_string(),
            scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
            stream_interrupted: false,
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
        };

        // Add a message to test dumping
        app.messages.push_back(crate::core::message::Message {
            role: "user".to_string(),
            content: "Test message".to_string(),
        });

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("custom_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should have added a system message about the dump
        assert!(!app.messages.is_empty());
        let last_message = app.messages.back().unwrap();
        assert_eq!(last_message.role, "system");
        assert!(last_message.content.contains("Conversation dumped to:"));

        // Clean up
        fs::remove_file(dump_filename).ok();
    }

    #[test]
    fn test_process_input_dump_empty_conversation() {
        let mut app = App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            current_response: String::new(),
            client: reqwest::Client::new(),
            model: "test-model".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            provider_name: "test".to_string(),
            provider_display_name: "Test".to_string(),
            scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
            stream_interrupted: false,
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
        };

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("empty_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command with an empty conversation
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should have added a system message with an error
        assert!(!app.messages.is_empty());
        let last_message = app.messages.back().unwrap();
        assert_eq!(last_message.role, "system");
        assert!(last_message.content.contains("Error dumping conversation:"));
        assert!(last_message.content.contains("No conversation to dump"));
    }
}
