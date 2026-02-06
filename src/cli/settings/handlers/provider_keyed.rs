//! Provider-keyed setting handlers for HashMap<String, String> settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{mutate_config, validate_provider};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;

/// Handler for the `default-model` setting.
pub struct DefaultModelHandler;

impl SettingHandler for DefaultModelHandler {
    fn key(&self) -> &'static str {
        "default-model"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.len() < 2 {
            return Err(SettingError::MissingArgs {
                hint: "To set a default model, specify the provider and model:",
                example: "chabeau set default-model openai gpt-4o",
            });
        }

        let provider_input = &args[0];
        let model = args[1..].join(" ");

        let resolved_provider = validate_provider(ctx.config, provider_input)?;

        let provider_msg = resolved_provider.clone();
        let model_msg = model.clone();

        mutate_config(move |config| {
            config.set_default_model(resolved_provider, model);
            Ok(())
        })?;

        Ok(format!(
            "✅ Set default-model for provider '{}' to: {}",
            provider_msg, model_msg
        ))
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let provider = args.ok_or(SettingError::MissingArgs {
            hint: "To unset a default model, specify the provider:",
            example: "chabeau unset default-model openai",
        })?;

        let resolved_provider = validate_provider(ctx.config, provider)?;
        let provider_msg = resolved_provider.clone();

        mutate_config(move |config| {
            config.unset_default_model(&resolved_provider);
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset default-model for provider: {provider_msg}"
        ))
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
            output.pop(); // Remove trailing newline
            output
        }
    }
}
