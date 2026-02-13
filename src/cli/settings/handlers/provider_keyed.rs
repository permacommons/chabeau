//! Provider-keyed setting handlers for HashMap<String, String> settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{
    mutate_config_with_message, parse_provider_value, success_set_provider_value,
    success_unset_provider_value, validate_provider,
};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;

/// Handler for the `default-model` setting.
pub struct DefaultModelHandler;

impl SettingHandler for DefaultModelHandler {
    fn key(&self) -> &'static str {
        "default-model"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let (provider, model) = parse_provider_value(
            args,
            ctx,
            "To set a default model, specify the provider and model:",
            "chabeau set default-model openai gpt-4o",
        )?;

        let message = success_set_provider_value("default-model", &provider, &model);

        mutate_config_with_message(
            move |config| {
                config.set_default_model(provider, model);
                Ok(())
            },
            message,
        )
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let provider = args.ok_or(SettingError::MissingArgs {
            hint: "To unset a default model, specify the provider:",
            example: "chabeau unset default-model openai",
        })?;

        let provider = validate_provider(ctx.config, provider)?;
        let message = success_unset_provider_value("default-model", &provider);

        mutate_config_with_message(
            move |config| {
                config.unset_default_model(&provider);
                Ok(())
            },
            message,
        )
    }

    fn format(&self, config: &Config) -> String {
        if config.default_models.is_empty() {
            "  default-models: (none set)".to_string()
        } else {
            let mut output = String::from("  default-models:\n");
            let mut entries: Vec<_> = config.default_models.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            for (provider, model) in entries {
                output.push_str(&format!("    {provider}: {model}\n"));
            }
            output.pop();
            output
        }
    }
}
