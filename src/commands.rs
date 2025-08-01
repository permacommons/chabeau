use crate::app::App;

pub enum CommandResult {
    Continue,
    ProcessAsMessage(String),
}

pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    let trimmed = input.trim();

    if trimmed.starts_with("/log") {
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
                        app.add_system_message(format!("Error: {}", e));
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
                        app.add_system_message(format!("Error setting log file: {}", e));
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
