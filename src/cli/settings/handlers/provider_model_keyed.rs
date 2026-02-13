//! Provider+model-keyed setting handlers for HashMap<String, HashMap<String, String>> settings.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{
    format_provider_model_map, mutate_config_with_message, parse_provider_model,
    parse_provider_model_value, success_set_provider_model_value,
    success_unset_provider_model_value,
};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;
use crate::core::persona::PersonaManager;
use crate::core::preset::PresetManager;

fn set_provider_model_value<V, M>(
    args: &[String],
    ctx: &mut SetContext<'_>,
    setting: &'static str,
    missing_hint: &'static str,
    missing_example: &'static str,
    validate_value: V,
    mutate: M,
) -> Result<String, SettingError>
where
    V: FnOnce(&mut SetContext<'_>, String) -> Result<String, SettingError>,
    M: FnOnce(&mut Config, String, String, String),
{
    let (provider, model, value) =
        parse_provider_model_value(args, ctx, missing_hint, missing_example)?;
    let value = validate_value(ctx, value)?;

    let message = success_set_provider_model_value(setting, &provider, &model, &value);

    mutate_config_with_message(
        move |config| {
            mutate(config, provider, model, value);
            Ok(())
        },
        message,
    )
}

fn unset_provider_model_value<M>(
    args: Option<&str>,
    ctx: &mut SetContext<'_>,
    setting: &'static str,
    missing_hint: &'static str,
    missing_example: &'static str,
    mutate: M,
) -> Result<String, SettingError>
where
    M: FnOnce(&mut Config, &str, &str),
{
    let (provider, model) = parse_provider_model(args, ctx, missing_hint, missing_example)?;
    let message = success_unset_provider_model_value(setting, &provider, &model);

    mutate_config_with_message(
        move |config| {
            mutate(config, &provider, &model);
            Ok(())
        },
        message,
    )
}

/// Handler for the `default-character` setting.
pub struct DefaultCharacterHandler;

impl SettingHandler for DefaultCharacterHandler {
    fn key(&self) -> &'static str {
        "default-character"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        set_provider_model_value(
            args,
            ctx,
            "default character",
            "To set a default character, specify provider, model, and character:",
            "chabeau set default-character openai gpt-4 alice",
            |ctx, character| {
                ctx.character_service
                    .resolve_by_name(&character)
                    .map_err(|_| SettingError::UnknownItem {
                        kind: "Character",
                        input: character.clone(),
                        hint: Some(
                            "Run 'chabeau import <file>' to import a character card first".into(),
                        ),
                    })?;
                Ok(character)
            },
            |config, provider, model, character| {
                config.set_default_character(provider, model, character);
            },
        )
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        unset_provider_model_value(
            args,
            ctx,
            "default character",
            "To unset a default character, specify provider and model:",
            "chabeau unset default-character \"openai gpt-4\"",
            |config, provider, model| {
                config.unset_default_character(provider, model);
            },
        )
    }

    fn format(&self, config: &Config) -> String {
        format_provider_model_map("default-characters", &config.default_characters)
    }
}

/// Handler for the `default-persona` setting.
pub struct DefaultPersonaHandler;

impl SettingHandler for DefaultPersonaHandler {
    fn key(&self) -> &'static str {
        "default-persona"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        set_provider_model_value(
            args,
            ctx,
            "default persona",
            "To set a default persona, specify provider, model, and persona:",
            "chabeau set default-persona anthropic claude-3-5-sonnet friendly",
            |ctx, persona_id| {
                let persona_manager = PersonaManager::load_personas(ctx.config)
                    .map_err(|e| SettingError::ConfigError(e.to_string()))?;

                if persona_manager.find_persona_by_id(&persona_id).is_none() {
                    let available: Vec<_> = persona_manager
                        .list_personas()
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect();

                    let hint = if available.is_empty() {
                        Some(
                            "Add personas to your config.toml file in the [[personas]] section."
                                .into(),
                        )
                    } else {
                        Some(format!("Available personas: {}", available.join(", ")))
                    };

                    return Err(SettingError::UnknownItem {
                        kind: "Persona",
                        input: persona_id,
                        hint,
                    });
                }

                Ok(persona_id)
            },
            |config, provider, model, persona_id| {
                config.set_default_persona(provider, model, persona_id);
            },
        )
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        unset_provider_model_value(
            args,
            ctx,
            "default persona",
            "To unset a default persona, specify provider and model:",
            "chabeau unset default-persona \"anthropic claude-3-5-sonnet\"",
            |config, provider, model| {
                config.unset_default_persona(provider, model);
            },
        )
    }

    fn format(&self, config: &Config) -> String {
        format_provider_model_map("default-personas", &config.default_personas)
    }
}

/// Handler for the `default-preset` setting.
pub struct DefaultPresetHandler;

impl SettingHandler for DefaultPresetHandler {
    fn key(&self) -> &'static str {
        "default-preset"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        set_provider_model_value(
            args,
            ctx,
            "default preset",
            "To set a default preset, specify provider, model, and preset:",
            "chabeau set default-preset openai gpt-4o short",
            |ctx, preset_id| {
                let preset_manager = PresetManager::load_presets(ctx.config)
                    .map_err(|e| SettingError::ConfigError(e.to_string()))?;

                if preset_manager.find_preset_by_id(&preset_id).is_none() {
                    let available: Vec<_> = preset_manager
                        .list_presets()
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect();

                    let hint = if available.is_empty() {
                        Some(
                            "Add presets to your config.toml file in the [[presets]] section."
                                .into(),
                        )
                    } else {
                        Some(format!("Available presets: {}", available.join(", ")))
                    };

                    return Err(SettingError::UnknownItem {
                        kind: "Preset",
                        input: preset_id,
                        hint,
                    });
                }

                Ok(preset_id)
            },
            |config, provider, model, preset_id| {
                config.set_default_preset(provider, model, preset_id);
            },
        )
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        unset_provider_model_value(
            args,
            ctx,
            "default preset",
            "To unset a default preset, specify provider and model:",
            "chabeau unset default-preset \"openai gpt-4o\"",
            |config, provider, model| {
                config.unset_default_preset(provider, model);
            },
        )
    }

    fn format(&self, config: &Config) -> String {
        format_provider_model_map("default-presets", &config.default_presets)
    }
}
