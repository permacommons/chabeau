use crate::core::app::App;

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
    } else {
        // Not a command, process as regular message
        CommandResult::ProcessAsMessage(input.to_string())
    }
}
