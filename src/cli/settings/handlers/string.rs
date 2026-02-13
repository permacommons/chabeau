//! String setting handlers for text-based settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{mutate_config_with_message, success_set};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::{Config, DEFAULT_REFINE_INSTRUCTIONS, DEFAULT_REFINE_PREFIX};

/// Handler for the `refine-instructions` setting.
pub struct RefineInstructionsHandler;

impl SettingHandler for RefineInstructionsHandler {
    fn key(&self) -> &'static str {
        "refine-instructions"
    }

    fn set(&self, args: &[String], _ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: "To set refine instructions, provide the instruction text:",
                example: "chabeau set refine-instructions \"Custom refine instructions here\"",
            });
        }

        let value = args.join(" ");
        let display = truncate_with_ellipsis(&value, 50);

        mutate_config_with_message(
            move |config| {
                config.refine_instructions = Some(value);
                Ok(())
            },
            success_set("refine-instructions", &display),
        )
    }

    fn unset(
        &self,
        _args: Option<&str>,
        _ctx: &mut SetContext<'_>,
    ) -> Result<String, SettingError> {
        mutate_config_with_message(
            |config| {
                config.refine_instructions = None;
                Ok(())
            },
            "✅ Unset refine-instructions (will use default)".to_string(),
        )
    }

    fn format(&self, config: &Config) -> String {
        match &config.refine_instructions {
            Some(instructions) => {
                let flat = instructions.replace('\n', " ");
                let display = truncate_with_ellipsis(&flat, 50);
                format!("  refine-instructions: {display}")
            }
            None => {
                let default_preview = truncate_with_ellipsis(
                    &DEFAULT_REFINE_INSTRUCTIONS.trim().replace('\n', " "),
                    40,
                );
                format!("  refine-instructions: (unset, default: {default_preview})")
            }
        }
    }
}

/// Truncate a string to `max_chars` characters, appending "..." if truncated.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

/// Handler for the `refine-prefix` setting.
pub struct RefinePrefixHandler;

impl SettingHandler for RefinePrefixHandler {
    fn key(&self) -> &'static str {
        "refine-prefix"
    }

    fn set(&self, args: &[String], _ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: "To set the refine prefix, provide the prefix text:",
                example: "chabeau set refine-prefix \"REVISE:\"",
            });
        }

        let value = args.join(" ");
        let message = success_set("refine-prefix", &value);

        mutate_config_with_message(
            move |config| {
                config.refine_prefix = Some(value);
                Ok(())
            },
            message,
        )
    }

    fn unset(
        &self,
        _args: Option<&str>,
        _ctx: &mut SetContext<'_>,
    ) -> Result<String, SettingError> {
        mutate_config_with_message(
            |config| {
                config.refine_prefix = None;
                Ok(())
            },
            format!(
                "✅ Unset refine-prefix (will use default: {})",
                DEFAULT_REFINE_PREFIX
            ),
        )
    }

    fn format(&self, config: &Config) -> String {
        match &config.refine_prefix {
            Some(prefix) => format!("  refine-prefix: {prefix}"),
            None => format!("  refine-prefix: (unset, default: {DEFAULT_REFINE_PREFIX})"),
        }
    }
}
