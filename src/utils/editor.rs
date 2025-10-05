//! External editor integration
//!
//! This module handles integration with external text editors for composing longer messages.

use std::error::Error;
use std::fs;
use std::io;
use std::process::Command;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorOutcome {
    pub message: Option<String>,
    pub status: Option<String>,
    pub clear_input: bool,
}

impl ExternalEditorOutcome {
    fn with_status<S: Into<String>>(status: S) -> Self {
        Self {
            message: None,
            status: Some(status.into()),
            clear_input: false,
        }
    }

    fn with_message(message: String) -> Self {
        Self {
            message: Some(message),
            status: None,
            clear_input: true,
        }
    }
}

pub async fn launch_external_editor(
    initial_text: &str,
) -> Result<ExternalEditorOutcome, Box<dyn Error>> {
    // Check if EDITOR environment variable is set
    let editor = match std::env::var("EDITOR") {
        Ok(editor) if !editor.trim().is_empty() => editor,
        _ => {
            return Ok(ExternalEditorOutcome::with_status(
                "EDITOR not set. Configure $EDITOR (e.g., nano)",
            ));
        }
    };

    // Create a temporary file
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    // Write current input to the temp file if there's any
    if !initial_text.is_empty() {
        fs::write(&temp_path, initial_text)?;
    }

    // We need to temporarily exit raw mode to allow the editor to run
    ratatui::crossterm::terminal::disable_raw_mode()?;
    ratatui::crossterm::execute!(
        io::stdout(),
        ratatui::crossterm::terminal::LeaveAlternateScreen
    )?;

    // Run the editor
    let mut command = Command::new(&editor);
    command.arg(&temp_path);

    let status = command.status()?;

    // Restore terminal mode
    ratatui::crossterm::terminal::enable_raw_mode()?;
    ratatui::crossterm::execute!(
        io::stdout(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;

    if !status.success() {
        return Ok(ExternalEditorOutcome::with_status(format!(
            "Editor exited with status: {}",
            status
        )));
    }

    // Read the file content
    let content = fs::read_to_string(&temp_path)?;

    // Check if file has content (not zero bytes and not just whitespace)
    if content.trim().is_empty() {
        Ok(ExternalEditorOutcome::with_status(
            "Editor file empty — no message",
        ))
    } else {
        // Clear the input and return the content to be sent immediately
        let message = content.trim_end().to_string(); // Remove trailing newlines but preserve internal formatting
        Ok(ExternalEditorOutcome::with_message(message))
    }

    // Temp file will be automatically cleaned up when it goes out of scope
}
