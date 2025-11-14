use crate::core::message::{Message, ROLE_APP_LOG, ROLE_ASSISTANT, ROLE_USER};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use tempfile::NamedTempFile;

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

        Ok(format!("Logging enabled to: {path}"))
    }

    pub fn toggle_logging(
        &mut self,
        pause_message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        match &self.file_path {
            Some(path) => {
                if self.is_active {
                    // Write pause message to log BEFORE pausing
                    self.log_message(&format!("## {}", pause_message))?;
                    self.is_active = false;
                    Ok(format!("Logging paused (file: {path})"))
                } else {
                    self.is_active = true;
                    Ok(format!("Logging resumed to: {path}"))
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

        // Open file in append mode
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;

        // Use BufWriter with 64KB buffer for better I/O performance
        // This reduces syscalls and handles partial writes more efficiently
        let mut writer = BufWriter::with_capacity(64 * 1024, file);

        // Write each line of content, preserving the exact formatting
        for line in content.lines() {
            writeln!(writer, "{line}")?;
        }

        // Add an empty line after each message for spacing (matching screen display)
        writeln!(writer)?;

        // Ensure all buffered data is written to disk
        writer.flush()?;
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.is_active
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
        self.rewrite_log_skip_index(messages, user_display_name, None)
    }

    pub fn rewrite_log_skip_index(
        &self,
        messages: &std::collections::VecDeque<Message>,
        user_display_name: &str,
        skip_index: Option<usize>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_active || self.file_path.is_none() {
            return Ok(());
        }

        let file_path = self.file_path.as_ref().unwrap();
        let target_path = Path::new(file_path);
        let parent = target_path.parent().unwrap_or_else(|| Path::new("."));

        // Create temp file in same directory (ensures atomic rename)
        let mut temp_file = NamedTempFile::new_in(parent)?;

        // Write all messages in the same format as log_message
        for (i, msg) in messages.iter().enumerate() {
            // Skip message at specified index (for retry/refine)
            if Some(i) == skip_index {
                continue;
            }
            if msg.role == ROLE_USER {
                // Write user messages with the current user display name prefix
                for line in format!("{}: {}", user_display_name, msg.content).lines() {
                    writeln!(temp_file, "{line}")?;
                }
                writeln!(temp_file)?; // Empty line for spacing
            } else if msg.role == ROLE_ASSISTANT && !msg.content.is_empty() {
                // Write assistant messages as-is (no prefix)
                for line in msg.content.lines() {
                    writeln!(temp_file, "{line}")?;
                }
                writeln!(temp_file)?; // Empty line for spacing
            } else if msg.role == ROLE_APP_LOG {
                // Write log-type app messages with ## prefix
                for line in format!("## {}", msg.content).lines() {
                    writeln!(temp_file, "{line}")?;
                }
                writeln!(temp_file)?; // Empty line for spacing
            }
            // Other app messages (info, warning, error) are intentionally skipped to maintain consistency with dumps
        }

        // Ensure data is written to disk
        temp_file.flush()?;
        temp_file.as_file().sync_all()?;

        // Atomic rename - original file only replaced after complete write
        temp_file.persist(file_path)?;

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
