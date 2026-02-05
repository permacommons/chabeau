//! Helper functions for settings operations.

use crate::core::builtin_providers::find_builtin_provider;
use crate::core::config::data::Config;
use crate::ui::builtin_themes::find_builtin_theme;

use super::error::SettingError;

/// Wrapper around `Config::mutate` that maps errors to `SettingError::ConfigError`.
pub fn mutate_config<F>(f: F) -> Result<(), SettingError>
where
    F: FnOnce(&mut Config) -> Result<(), Box<dyn std::error::Error>>,
{
    Config::mutate(f).map_err(|e| SettingError::ConfigError(e.to_string()))
}

/// Parse a boolean value from user input.
///
/// Accepts: on/off, true/false, yes/no (case-insensitive).
pub fn parse_bool(input: &str) -> Option<bool> {
    match input.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// Format a boolean value for display.
pub fn format_bool(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// Validate and resolve a provider identifier.
///
/// Checks both built-in and custom providers, returning the canonical provider ID.
pub fn validate_provider(config: &Config, input: &str) -> Result<String, SettingError> {
    if let Some(provider) = find_builtin_provider(input) {
        return Ok(provider.id);
    }

    if let Some(provider) = config.get_custom_provider(input) {
        return Ok(provider.id.clone());
    }

    Err(SettingError::UnknownProvider {
        input: input.to_string(),
    })
}

/// Validate and resolve a theme identifier.
///
/// Checks both built-in and custom themes, returning the canonical theme ID.
pub fn validate_theme(config: &Config, input: &str) -> Result<String, SettingError> {
    if let Some(theme) = find_builtin_theme(input) {
        return Ok(theme.id);
    }

    if let Some(theme) = config.get_custom_theme(input) {
        return Ok(theme.id.clone());
    }

    Err(SettingError::UnknownTheme {
        input: input.to_string(),
    })
}
