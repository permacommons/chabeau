use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub struct LoggingState {
    file_path: Option<String>,
    is_active: bool,
}

impl LoggingState {
    pub fn new(log_file: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut logging = LoggingState {
            file_path: log_file,
            is_active: false,
        };

        // If a log file was provided via command line, enable logging immediately
        if logging.file_path.is_some() {
            logging.is_active = true;
        }

        Ok(logging)
    }

    pub fn set_log_file(&mut self, path: String) -> Result<String, Box<dyn std::error::Error>> {
        // Test if we can create/write to the file
        self.test_file_access(&path)?;

        self.file_path = Some(path.clone());
        self.is_active = true;

        Ok(format!("Logging enabled to: {}", path))
    }

    pub fn toggle_logging(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        match &self.file_path {
            Some(path) => {
                self.is_active = !self.is_active;
                if self.is_active {
                    Ok(format!("Logging resumed to: {}", path))
                } else {
                    Ok(format!("Logging paused (file: {})", path))
                }
            }
            None => Err("No log file specified. Use /log <filename> to enable logging first.".into()),
        }
    }

    pub fn log_message(&self, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_active || self.file_path.is_none() {
            return Ok(());
        }

        let file_path = self.file_path.as_ref().unwrap();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;

        // Write each line of content, preserving the exact formatting
        for line in content.lines() {
            writeln!(file, "{}", line)?;
        }

        // Add an empty line after each message for spacing (matching screen display)
        writeln!(file)?;

        file.flush()?;
        Ok(())
    }

    pub fn get_status_string(&self) -> String {
        match (&self.file_path, self.is_active) {
            (None, _) => "disabled".to_string(),
            (Some(path), true) => format!("active ({})", Path::new(path).file_name().unwrap_or_default().to_string_lossy()),
            (Some(path), false) => format!("paused ({})", Path::new(path).file_name().unwrap_or_default().to_string_lossy()),
        }
    }

    fn test_file_access(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Try to create/open the file to ensure we have write permissions
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        // Test write access
        file.flush()?;
        Ok(())
    }
}
