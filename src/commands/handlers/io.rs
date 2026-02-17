use super::{required_arg, usage_status};
use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::App;
use crate::core::message;
use chrono::Utc;
use std::fs::File;
use std::io::{BufWriter, Write};

const USAGE_LOG: &str = "Usage: /log [filename]";
const USAGE_DUMP: &str = "Usage: /dump [filename]";

pub(crate) fn handle_log(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => {
            let timestamp = chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string();
            let was_active = app.session.logging.is_active();
            let log_message = if was_active {
                format!("Logging paused at {}", timestamp)
            } else {
                format!("Logging resumed at {}", timestamp)
            };

            match app.session.logging.toggle_logging(&log_message) {
                Ok(message) => {
                    app.conversation()
                        .add_app_message(crate::core::message::AppMessageKind::Log, log_message);
                    app.conversation().set_status(message);
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation().set_status(format!("Log error: {}", e));
                    CommandResult::Continue
                }
            }
        }
        1 => {
            let Some(filename) = required_arg(app, &invocation, 0, USAGE_LOG) else {
                return CommandResult::Continue;
            };
            match app.session.logging.set_log_file(filename.to_string()) {
                Ok(message) => {
                    let timestamp = chrono::Local::now()
                        .format("%Y-%m-%d %H:%M:%S %Z")
                        .to_string();
                    let log_message = format!("Logging started at {}", timestamp);
                    app.conversation()
                        .add_app_message(crate::core::message::AppMessageKind::Log, log_message);
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
        _ => usage_status(app, USAGE_LOG),
    }
}

pub(crate) fn handle_dump(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
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
            let Some(filename) = required_arg(app, &invocation, 0, USAGE_DUMP) else {
                return CommandResult::Continue;
            };
            handle_dump_result(app, dump_conversation(app, filename), filename)
        }
        _ => usage_status(app, USAGE_DUMP),
    }
}

pub fn dump_conversation_with_overwrite(
    app: &App,
    filename: &str,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let conversation_messages: Vec<_> = app
        .ui
        .messages
        .iter()
        .filter(|msg| {
            !message::is_app_message_role(msg.role) || msg.role == message::TranscriptRole::AppLog
        })
        .collect();

    if conversation_messages.is_empty() {
        return Err("No conversation to dump - the chat history is empty.".into());
    }

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
        match msg.role {
            message::TranscriptRole::User => {
                writeln!(writer, "{}: {}", user_display_name, msg.content)?
            }
            message::TranscriptRole::AppLog => writeln!(writer, "## {}", msg.content)?,
            message::TranscriptRole::ToolCall => writeln!(writer, "Tool call: {}", msg.content)?,
            message::TranscriptRole::ToolResult => {
                writeln!(writer, "Tool result: {}", msg.content)?
            }
            _ => writeln!(writer, "{}", msg.content)?,
        }
        writeln!(writer)?;
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
