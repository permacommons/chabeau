//! Helper functions for settings operations.

use std::collections::HashMap;

use crate::core::builtin_providers::find_builtin_provider;
use crate::core::config::data::Config;
use crate::ui::builtin_themes::find_builtin_theme;

use super::error::SettingError;
use super::SetContext;

/// Wrapper around `Config::mutate` that maps errors to `SettingError::ConfigError`.
pub fn mutate_config<F>(f: F) -> Result<(), SettingError>
where
    F: FnOnce(&mut Config) -> Result<(), Box<dyn std::error::Error>>,
{
    Config::mutate(f).map_err(|e| SettingError::ConfigError(e.to_string()))
}

/// Execute a config mutation and return a standardized success message.
pub fn mutate_config_with_message<F>(f: F, message: String) -> Result<String, SettingError>
where
    F: FnOnce(&mut Config) -> Result<(), Box<dyn std::error::Error>>,
{
    mutate_config(f)?;
    Ok(message)
}

/// Parse and validate a provider/model/value tuple from `set` args.
pub fn parse_provider_model_value(
    args: &[String],
    ctx: &SetContext<'_>,
    hint: &'static str,
    example: &'static str,
) -> Result<(String, String, String), SettingError> {
    if args.len() < 3 {
        return Err(SettingError::MissingArgs { hint, example });
    }

    let provider = validate_provider(ctx.config, &args[0])?;
    let model = args[1].clone();
    let value = args[2..].join(" ");

    Ok((provider, model, value))
}

/// Parse and validate a provider/model tuple from `unset` args.
pub fn parse_provider_model(
    args: Option<&str>,
    ctx: &SetContext<'_>,
    hint: &'static str,
    example: &'static str,
) -> Result<(String, String), SettingError> {
    let value = args.ok_or(SettingError::MissingArgs { hint, example })?;
    let parts: Vec<&str> = value.splitn(2, ' ').collect();

    if parts.len() < 2 {
        return Err(SettingError::MissingArgs { hint, example });
    }

    let provider = validate_provider(ctx.config, parts[0])?;
    let model = parts[1].to_string();
    Ok((provider, model))
}

/// Parse and validate a provider/value tuple from `set` args.
pub fn parse_provider_value(
    args: &[String],
    ctx: &SetContext<'_>,
    hint: &'static str,
    example: &'static str,
) -> Result<(String, String), SettingError> {
    if args.len() < 2 {
        return Err(SettingError::MissingArgs { hint, example });
    }

    let provider = validate_provider(ctx.config, &args[0])?;
    let value = args[1..].join(" ");
    Ok((provider, value))
}

/// Build a standardized success message for set operations.
pub fn success_set(setting: &str, value: &str) -> String {
    format!("✅ Set {setting} to: {value}")
}

/// Build a standardized success message for unset operations.
pub fn success_unset(setting: &str) -> String {
    format!("✅ Unset {setting}")
}

/// Build a standardized success message for provider-keyed set operations.
pub fn success_set_provider_value(setting: &str, provider: &str, value: &str) -> String {
    format!("✅ Set {setting} for provider '{provider}' to: {value}")
}

/// Build a standardized success message for provider-keyed unset operations.
pub fn success_unset_provider_value(setting: &str, provider: &str) -> String {
    format!("✅ Unset {setting} for provider: {provider}")
}

/// Build a standardized success message for provider/model-keyed set operations.
pub fn success_set_provider_model_value(
    setting: &str,
    provider: &str,
    model: &str,
    value: &str,
) -> String {
    format!("✅ Set {setting} for '{provider}:{model}' to: {value}")
}

/// Build a standardized success message for provider/model-keyed unset operations.
pub fn success_unset_provider_model_value(setting: &str, provider: &str, model: &str) -> String {
    format!("✅ Unset {setting} for '{provider}:{model}'")
}

/// Format a provider/model nested map for display.
pub fn format_provider_model_map(
    name: &str,
    map: &HashMap<String, HashMap<String, String>>,
) -> String {
    if map.is_empty() {
        format!("  {name}: (none set)")
    } else {
        let mut output = format!("  {name}:\n");
        let mut providers: Vec<_> = map.iter().collect();
        providers.sort_by_key(|(k, _)| *k);
        for (provider, models) in providers {
            let mut model_entries: Vec<_> = models.iter().collect();
            model_entries.sort_by_key(|(k, _)| *k);
            for (model, value) in model_entries {
                output.push_str(&format!("    {provider}:{model}: {value}\n"));
            }
        }
        output.pop();
        output
    }
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
