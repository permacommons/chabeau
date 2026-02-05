//! Simple setting handlers for single-value settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{mutate_config, validate_provider, validate_theme};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;

/// Handler for the `default-provider` setting.
pub struct DefaultProviderHandler;

impl SettingHandler for DefaultProviderHandler {
    fn key(&self) -> &'static str {
        "default-provider"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: "To set a default provider, specify the provider:",
                example: "chabeau set default-provider openai",
            });
        }

        let provider_input = args.join(" ");
        let resolved = validate_provider(ctx.config, &provider_input)?;
        let msg = resolved.clone();

        mutate_config(move |config| {
            config.default_provider = Some(resolved);
            Ok(())
        })?;

        Ok(format!("✅ Set default-provider to: {msg}"))
    }

    fn unset(&self, _args: Option<&str>, _ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        mutate_config(|config| {
            config.default_provider = None;
            Ok(())
        })?;

        Ok("✅ Unset default-provider".to_string())
    }

    fn format(&self, config: &Config) -> String {
        match &config.default_provider {
            Some(provider) => format!("  default-provider: {provider}"),
            None => "  default-provider: (unset)".to_string(),
        }
    }
}

/// Handler for the `theme` setting.
pub struct ThemeHandler;

impl SettingHandler for ThemeHandler {
    fn key(&self) -> &'static str {
        "theme"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: "To set a theme, specify the theme name:",
                example: "chabeau set theme dark",
            });
        }

        let theme_input = args.join(" ");
        let resolved = validate_theme(ctx.config, &theme_input)?;
        let msg = resolved.clone();

        mutate_config(move |config| {
            config.theme = Some(resolved);
            Ok(())
        })?;

        Ok(format!("✅ Set theme to: {msg}"))
    }

    fn unset(&self, _args: Option<&str>, _ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        mutate_config(|config| {
            config.theme = None;
            Ok(())
        })?;

        Ok("✅ Unset theme".to_string())
    }

    fn format(&self, config: &Config) -> String {
        match &config.theme {
            Some(theme) => format!("  theme: {theme}"),
            None => "  theme: (unset)".to_string(),
        }
    }
}
