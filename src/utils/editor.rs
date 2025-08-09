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
            app.add_system_message("No EDITOR environment variable set. Please set EDITOR to your preferred text editor (e.g., export EDITOR=nano).".to_string());
            return Ok(None);
        }
    };

    // Create a temporary file
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    // Write current input to the temp file if there's any
    if !app.input.is_empty() {
        fs::write(&temp_path, &app.input)?;
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
        app.add_system_message(format!("Editor exited with non-zero status: {status}"));
        return Ok(None);
    }

    // Read the file content
    let content = fs::read_to_string(&temp_path)?;

    // Check if file has content (not zero bytes and not just whitespace)
    if content.trim().is_empty() {
        app.add_system_message(
            "Editor file was empty or contained only whitespace - no message sent.".to_string(),
        );
        Ok(None)
    } else {
        // Clear the input and return the content to be sent immediately
        app.input.clear();
        let message = content.trim_end().to_string(); // Remove trailing newlines but preserve internal formatting
        Ok(Some(message))
    }

    // Temp file will be automatically cleaned up when it goes out of scope
}
