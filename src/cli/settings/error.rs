//! Error types for settings operations.

use std::fmt;

/// Errors that can occur when modifying configuration settings.
#[derive(Debug)]
pub enum SettingError {
    /// The provided setting key is not recognized.
    UnknownKey(String),
    /// The provided provider identifier was not found.
    UnknownProvider { input: String },
    /// The provided theme identifier was not found.
    UnknownTheme { input: String },
    /// The provided item (character, persona, preset) was not found.
    UnknownItem {
        kind: &'static str,
        input: String,
        hint: Option<String>,
    },
    /// The provided value could not be parsed as a boolean.
    InvalidBoolean(String),
    /// Required arguments are missing.
    MissingArgs {
        hint: &'static str,
        example: &'static str,
    },
    /// An error occurred while persisting the configuration.
    ConfigError(String),
}

impl SettingError {
    /// Print the error message to stderr with appropriate formatting.
    pub fn print(&self) {
        match self {
            SettingError::UnknownKey(key) => {
                eprintln!("❌ Unknown config key: {key}");
            }
            SettingError::UnknownProvider { input } => {
                eprintln!(
                    "❌ Unknown provider: {input}. Run 'chabeau -p' to list available providers."
                );
            }
            SettingError::UnknownTheme { input } => {
                eprintln!(
                    "❌ Unknown theme: {input}. Run 'chabeau themes' to list available themes."
                );
            }
            SettingError::UnknownItem { kind, input, hint } => {
                eprintln!("❌ {kind} '{input}' not found.");
                if let Some(hint) = hint {
                    eprintln!("   {hint}");
                }
            }
            SettingError::InvalidBoolean(input) => {
                eprintln!("❌ Invalid boolean value: {input}");
                eprintln!("   Use 'on' or 'off' (also accepts true/false, yes/no)");
            }
            SettingError::MissingArgs { hint, example } => {
                eprintln!("⚠️  {hint}");
                eprintln!("Example: {example}");
            }
            SettingError::ConfigError(msg) => {
                eprintln!("❌ Failed to save configuration: {msg}");
            }
        }
    }

    /// Returns the exit code for this error.
    pub fn exit_code(&self) -> i32 {
        1
    }
}

impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingError::UnknownKey(key) => write!(f, "Unknown config key: {key}"),
            SettingError::UnknownProvider { input } => write!(f, "Unknown provider: {input}"),
            SettingError::UnknownTheme { input } => write!(f, "Unknown theme: {input}"),
            SettingError::UnknownItem { kind, input, .. } => {
                write!(f, "{kind} '{input}' not found")
            }
            SettingError::InvalidBoolean(input) => write!(f, "Invalid boolean value: {input}"),
            SettingError::MissingArgs { hint, .. } => write!(f, "{hint}"),
            SettingError::ConfigError(msg) => write!(f, "Config error: {msg}"),
        }
    }
}

impl std::error::Error for SettingError {}
