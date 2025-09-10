//! External editor integration
//!
//! This module handles integration with external text editors for composing longer messages.

use crate::core::app::App;
use std::error::Error;
use std::fs;
use std::io;
use std::process::Command;
use tempfile::NamedTempFile;

pub async fn handle_external_editor(app: &mut App) -> Result<Option<String>, Box<dyn Error>> {
    // Check if EDITOR environment variable is set
    let editor = match std::env::var("EDITOR") {
        Ok(editor) if !editor.trim().is_empty() => editor,
        _ => {
            app.set_status("EDITOR not set. Configure $EDITOR (e.g., nano)");
            return Ok(None);
        }
    };

    // Create a temporary file
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    // Write current input to the temp file if there's any
    let current_text = app.get_input_text();
    if !current_text.is_empty() {
        fs::write(&temp_path, current_text)?;
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
        app.set_status(format!("Editor exited with status: {}", status));
        return Ok(None);
    }

    // Read the file content
    let content = fs::read_to_string(&temp_path)?;

    // Check if file has content (not zero bytes and not just whitespace)
    if content.trim().is_empty() {
        app.set_status("Editor file empty — no message");
        Ok(None)
    } else {
        // Clear the input and return the content to be sent immediately
        app.clear_input();
        let message = content.trim_end().to_string(); // Remove trailing newlines but preserve internal formatting
        Ok(Some(message))
    }

    // Temp file will be automatically cleaned up when it goes out of scope
}
