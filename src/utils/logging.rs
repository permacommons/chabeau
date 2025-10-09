use crate::core::message::Message;
use chrono::Utc;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub struct LoggingState {
    file_path: Option<String>,
    is_active: bool,
}

impl LoggingState {
    pub fn new(log_file: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let logging = LoggingState {
            file_path: log_file,
            is_active: false,
        };

        Ok(logging)
    }

    pub fn set_log_file(&mut self, path: String) -> Result<String, Box<dyn std::error::Error>> {
        // Test if we can create/write to the file
        self.test_file_access(&path)?;

        self.file_path = Some(path.clone());
        self.is_active = true;

        // Log timestamp when starting
        let timestamp = Utc::now().to_rfc3339();
        self.log_message(&format!("## Logging started at {}", timestamp))?;

        Ok(format!("Logging enabled to: {path}"))
    }

    pub fn toggle_logging(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        match &self.file_path {
            Some(path) => {
                self.is_active = !self.is_active;
                if self.is_active {
                    // Log timestamp when resuming
                    let timestamp = Utc::now().to_rfc3339();
                    self.write_to_log(&format!("## Logging resumed at {}", timestamp))?;
                    Ok(format!("Logging resumed to: {path}"))
                } else {
                    // Log timestamp when pausing
                    let timestamp = Utc::now().to_rfc3339();
                    self.write_to_log(&format!("## Logging paused at {}", timestamp))?;
                    Ok(format!("Logging paused (file: {path})"))
                }
            }
            None => {
                Err("No log file specified. Use /log <filename> to enable logging first.".into())
            }
        }
    }

    pub fn log_message(&self, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_active || self.file_path.is_none() {
            return Ok(());
        }

        self.write_to_log(content)
    }

    fn write_to_log(&self, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file_path = self.file_path.as_ref().unwrap();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;

        // Write each line of content, preserving the exact formatting
        for line in content.lines() {
            writeln!(file, "{line}")?;
        }

        // Add an empty line after each message for spacing (matching screen display)
        writeln!(file)?;

        file.flush()?;
        Ok(())
    }

    pub fn get_status_string(&self) -> String {
        match (&self.file_path, self.is_active) {
            (None, _) => "disabled".to_string(),
            (Some(path), true) => format!(
                "active ({})",
                Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
            (Some(path), false) => format!(
                "paused ({})",
                Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
        }
    }

    pub fn rewrite_log_without_last_response(
        &self,
        messages: &std::collections::VecDeque<Message>,
        user_display_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_active || self.file_path.is_none() {
            return Ok(());
        }

        let file_path = self.file_path.as_ref().unwrap();

        // Recreate the log file with only the current messages
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(file_path)?;

        // Write all messages in the same format as log_message
        for msg in messages {
            if msg.role == "user" {
                // Write user messages with the current user display name prefix
                for line in format!("{}: {}", user_display_name, msg.content).lines() {
                    writeln!(file, "{line}")?;
                }
                writeln!(file)?; // Empty line for spacing
            } else if msg.role == "system" {
                // Write system messages as-is
                for line in msg.content.lines() {
                    writeln!(file, "{line}")?;
                }
                writeln!(file)?; // Empty line for spacing
            } else if msg.role == "assistant" && !msg.content.is_empty() {
                // Write assistant messages as-is (no prefix)
                for line in msg.content.lines() {
                    writeln!(file, "{line}")?;
                }
                writeln!(file)?; // Empty line for spacing
            }
        }

        file.flush()?;
        Ok(())
    }

    fn test_file_access(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Try to create/open the file to ensure we have write permissions
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;

        // Test write access
        file.flush()?;
        Ok(())
    }
}
