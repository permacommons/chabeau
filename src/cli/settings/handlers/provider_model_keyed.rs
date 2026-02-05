//! Provider+model-keyed setting handlers for HashMap<String, HashMap<String, String>> settings.

use std::collections::HashMap;

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{mutate_config, validate_provider};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;
use crate::core::persona::PersonaManager;
use crate::core::preset::PresetManager;

/// Parse "provider model" from an unset command's optional value string and validate the provider.
fn parse_provider_model_args(
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

    let resolved_provider = validate_provider(ctx.config, parts[0])?;
    Ok((resolved_provider, parts[1].to_string()))
}

/// Handler for the `default-character` setting.
pub struct DefaultCharacterHandler;

impl SettingHandler for DefaultCharacterHandler {
    fn key(&self) -> &'static str {
        "default-character"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.len() < 3 {
            return Err(SettingError::MissingArgs {
                hint: "To set a default character, specify provider, model, and character:",
                example: "chabeau set default-character openai gpt-4 alice",
            });
        }

        let provider_input = &args[0];
        let model = &args[1];
        let character = args[2..].join(" ");

        let resolved_provider = validate_provider(ctx.config, provider_input)?;

        // Validate character exists
        ctx.character_service
            .resolve_by_name(&character)
            .map_err(|_| SettingError::UnknownItem {
                kind: "Character",
                input: character.clone(),
                hint: Some("Run 'chabeau import <file>' to import a character card first".into()),
            })?;

        let provider_msg = resolved_provider.clone();
        let model_msg = model.clone();
        let model_for_closure = model.clone();
        let character_msg = character.clone();

        mutate_config(move |config| {
            config.set_default_character(resolved_provider, model_for_closure, character);
            Ok(())
        })?;

        Ok(format!(
            "✅ Set default character for '{}:{}' to: {}",
            provider_msg, model_msg, character_msg
        ))
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let (provider, model) = parse_provider_model_args(
            args,
            ctx,
            "To unset a default character, specify provider and model:",
            "chabeau unset default-character \"openai gpt-4\"",
        )?;

        let provider_msg = provider.clone();
        let model_msg = model.clone();

        mutate_config(move |config| {
            config.unset_default_character(&provider, &model);
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset default character for '{provider_msg}:{model_msg}'"
        ))
    }

    fn format(&self, config: &Config) -> String {
        format_nested_map("default-characters", &config.default_characters)
    }
}

/// Handler for the `default-persona` setting.
pub struct DefaultPersonaHandler;

impl SettingHandler for DefaultPersonaHandler {
    fn key(&self) -> &'static str {
        "default-persona"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.len() < 3 {
            return Err(SettingError::MissingArgs {
                hint: "To set a default persona, specify provider, model, and persona:",
                example: "chabeau set default-persona anthropic claude-3-5-sonnet friendly",
            });
        }

        let provider_input = &args[0];
        let model = &args[1];
        let persona_id = args[2..].join(" ");

        let resolved_provider = validate_provider(ctx.config, provider_input)?;

        // Validate persona exists
        let persona_manager = PersonaManager::load_personas(ctx.config)
            .map_err(|e| SettingError::ConfigError(e.to_string()))?;

        if persona_manager.find_persona_by_id(&persona_id).is_none() {
            let available: Vec<_> = persona_manager
                .list_personas()
                .iter()
                .map(|p| p.id.as_str())
                .collect();

            let hint = if available.is_empty() {
                Some("Add personas to your config.toml file in the [[personas]] section.".into())
            } else {
                Some(format!("Available personas: {}", available.join(", ")))
            };

            return Err(SettingError::UnknownItem {
                kind: "Persona",
                input: persona_id,
                hint,
            });
        }

        let provider_msg = resolved_provider.clone();
        let model_msg = model.clone();
        let model_for_closure = model.clone();
        let persona_msg = persona_id.clone();

        mutate_config(move |config| {
            config.set_default_persona(resolved_provider, model_for_closure, persona_id);
            Ok(())
        })?;

        Ok(format!(
            "✅ Set default persona for '{}:{}' to: {}",
            provider_msg, model_msg, persona_msg
        ))
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let (provider, model) = parse_provider_model_args(
            args,
            ctx,
            "To unset a default persona, specify provider and model:",
            "chabeau unset default-persona \"anthropic claude-3-5-sonnet\"",
        )?;

        let provider_msg = provider.clone();
        let model_msg = model.clone();

        mutate_config(move |config| {
            config.unset_default_persona(&provider, &model);
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset default persona for '{provider_msg}:{model_msg}'"
        ))
    }

    fn format(&self, config: &Config) -> String {
        format_nested_map("default-personas", &config.default_personas)
    }
}

/// Handler for the `default-preset` setting.
pub struct DefaultPresetHandler;

impl SettingHandler for DefaultPresetHandler {
    fn key(&self) -> &'static str {
        "default-preset"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.len() < 3 {
            return Err(SettingError::MissingArgs {
                hint: "To set a default preset, specify provider, model, and preset:",
                example: "chabeau set default-preset openai gpt-4o short",
            });
        }

        let provider_input = &args[0];
        let model = &args[1];
        let preset_id = args[2..].join(" ");

        let resolved_provider = validate_provider(ctx.config, provider_input)?;

        // Validate preset exists
        let preset_manager = PresetManager::load_presets(ctx.config)
            .map_err(|e| SettingError::ConfigError(e.to_string()))?;

        if preset_manager.find_preset_by_id(&preset_id).is_none() {
            let available: Vec<_> = preset_manager
                .list_presets()
                .iter()
                .map(|p| p.id.as_str())
                .collect();

            let hint = if available.is_empty() {
                Some("Add presets to your config.toml file in the [[presets]] section.".into())
            } else {
                Some(format!("Available presets: {}", available.join(", ")))
            };

            return Err(SettingError::UnknownItem {
                kind: "Preset",
                input: preset_id,
                hint,
            });
        }

        let provider_msg = resolved_provider.clone();
        let model_msg = model.clone();
        let model_for_closure = model.clone();
        let preset_msg = preset_id.clone();

        mutate_config(move |config| {
            config.set_default_preset(resolved_provider, model_for_closure, preset_id);
            Ok(())
        })?;

        Ok(format!(
            "✅ Set default preset for '{}:{}' to: {}",
            provider_msg, model_msg, preset_msg
        ))
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let (provider, model) = parse_provider_model_args(
            args,
            ctx,
            "To unset a default preset, specify provider and model:",
            "chabeau unset default-preset \"openai gpt-4o\"",
        )?;

        let provider_msg = provider.clone();
        let model_msg = model.clone();

        mutate_config(move |config| {
            config.unset_default_preset(&provider, &model);
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset default preset for '{provider_msg}:{model_msg}'"
        ))
    }

    fn format(&self, config: &Config) -> String {
        format_nested_map("default-presets", &config.default_presets)
    }
}

/// Format a nested HashMap<String, HashMap<String, String>> for display.
fn format_nested_map(name: &str, map: &HashMap<String, HashMap<String, String>>) -> String {
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
        output.pop(); // Remove trailing newline
        output
    }
}
