//! Boolean setting handlers for on/off settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{format_bool, mutate_config, parse_bool};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;

/// Data-driven handler for boolean (on/off) settings.
pub struct BooleanHandler {
    key: &'static str,
    hint: &'static str,
    example: &'static str,
    default_display: &'static str,
    get: fn(&Config) -> Option<bool>,
    set_field: fn(&mut Config, Option<bool>),
}

impl SettingHandler for BooleanHandler {
    fn key(&self) -> &'static str {
        self.key
    }

    fn set(&self, args: &[String], _ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: self.hint,
                example: self.example,
            });
        }

        let input = args.join(" ");
        let value = parse_bool(&input).ok_or(SettingError::InvalidBoolean(input))?;
        let display = format_bool(value);
        let set_field = self.set_field;
        let key = self.key;

        mutate_config(move |config| {
            set_field(config, Some(value));
            Ok(())
        })?;

        Ok(format!("✅ Set {key} to: {display}"))
    }

    fn unset(
        &self,
        _args: Option<&str>,
        _ctx: &mut SetContext<'_>,
    ) -> Result<String, SettingError> {
        let set_field = self.set_field;

        mutate_config(move |config| {
            set_field(config, None);
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset {} (will use default: {})",
            self.key, self.default_display
        ))
    }

    fn format(&self, config: &Config) -> String {
        match (self.get)(config) {
            Some(value) => format!("  {}: {}", self.key, format_bool(value)),
            None => format!("  {}: (unset, default: {})", self.key, self.default_display),
        }
    }
}

/// Create a handler for the `markdown` setting.
pub fn markdown_handler() -> BooleanHandler {
    BooleanHandler {
        key: "markdown",
        hint: "To set markdown rendering, specify on or off:",
        example: "chabeau set markdown off",
        default_display: "on",
        get: |c| c.markdown,
        set_field: |c, v| c.markdown = v,
    }
}

/// Create a handler for the `syntax` setting.
pub fn syntax_handler() -> BooleanHandler {
    BooleanHandler {
        key: "syntax",
        hint: "To set syntax highlighting, specify on or off:",
        example: "chabeau set syntax off",
        default_display: "on",
        get: |c| c.syntax,
        set_field: |c, v| c.syntax = v,
    }
}

/// Create a handler for the `builtin-presets` setting.
pub fn builtin_presets_handler() -> BooleanHandler {
    BooleanHandler {
        key: "builtin-presets",
        hint: "To enable/disable built-in presets, specify on or off:",
        example: "chabeau set builtin-presets off",
        default_display: "on",
        get: |c| c.builtin_presets,
        set_field: |c, v| c.builtin_presets = v,
    }
}
