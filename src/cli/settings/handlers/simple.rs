//! Simple setting handlers for single-value settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{
    mutate_config_with_message, success_set, success_unset, validate_provider, validate_theme,
};
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

        let provider = validate_provider(ctx.config, &args.join(" "))?;
        let message = success_set("default-provider", &provider);

        mutate_config_with_message(
            move |config| {
                config.default_provider = Some(provider);
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
                config.default_provider = None;
                Ok(())
            },
            success_unset("default-provider"),
        )
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

        let theme = validate_theme(ctx.config, &args.join(" "))?;
        let message = success_set("theme", &theme);

        mutate_config_with_message(
            move |config| {
                config.theme = Some(theme);
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
                config.theme = None;
                Ok(())
            },
            success_unset("theme"),
        )
    }

    fn format(&self, config: &Config) -> String {
        match &config.theme {
            Some(theme) => format!("  theme: {theme}"),
            None => "  theme: (unset)".to_string(),
        }
    }
}
