//! Character, persona and preset pickers, each with a "turn off" entry that
//! stays pinned above the sorted items.

use super::inspect::character_inspect;
use super::{
    sanitize_picker_metadata, sanitize_picker_metadata_for_inspect, ActivePicker,
    CharacterPickerState, PersonaPickerState, PickerController, PickerData, PickerMode,
    PresetPickerState, TURN_OFF_CHARACTER_ID, TURN_OFF_PERSONA_ID, TURN_OFF_PRESET_ID,
};
use crate::character::CharacterCard;
use crate::core::app::session::SessionContext;
use crate::core::config::data::Config;
use crate::ui::picker::{PickerItem, PickerState};

impl PickerController {
    pub fn open_character_picker(
        &mut self,
        cards: Vec<CharacterCard>,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        if cards.is_empty() {
            return Err(
                "No character cards found. Use 'chabeau import <file>' to import cards."
                    .to_string(),
            );
        }

        // Get the default character for the current provider/model
        let cfg = Config::load_test_safe().map_err(|err| err.to_string())?;
        let default_character =
            cfg.get_default_character(&session_context.provider_name, &session_context.model);

        let active_character_id = session_context
            .get_character()
            .map(|character| character.data.name.clone());

        let mut items: Vec<PickerItem> = cards
            .into_iter()
            .map(|card| {
                let name = card.data.name.clone();
                let sanitized_description = sanitize_picker_metadata(&card.data.description);
                let metadata_text = if sanitized_description.is_empty() {
                    "No description".to_string()
                } else {
                    sanitized_description
                };
                let inspect_definition = character_inspect(&card);
                let is_default = default_character.map(|def| def == &name).unwrap_or(false);
                let label = if is_default {
                    format!("{}*", name)
                } else {
                    name.clone()
                };
                PickerItem {
                    id: name.clone(),
                    label,
                    metadata: Some(metadata_text),
                    inspect_metadata: Some(inspect_definition),
                    sort_key: Some(name),
                }
            })
            .collect();

        // Add "turn off character mode" entry at the beginning if a character is active
        if session_context.active_character.is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_CHARACTER_ID.to_string(),
                    label: "[Turn off character mode]".to_string(),
                    metadata: Some("Disable character and return to normal mode".to_string()),
                    inspect_metadata: Some(
                        "Disable character and return to normal mode".to_string(),
                    ),
                    sort_key: None,
                },
            );
        }

        let selected = active_character_id
            .as_deref()
            .and_then(|active_id| items.iter().position(|item| item.id == active_id))
            .unwrap_or(0);
        let picker_state = PickerState::new("Pick Character", items.clone(), selected);
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        };

        self.start_active_picker(session, active_character_id);

        Ok(())
    }

    pub fn filter_characters(&mut self) {
        self.filter_session_items(PickerMode::Character, &[TURN_OFF_CHARACTER_ID]);
    }

    pub fn open_persona_picker(
        &mut self,
        persona_manager: &crate::core::persona::PersonaManager,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        let personas = persona_manager.list_personas();
        let active_persona_id = persona_manager
            .get_active_persona()
            .map(|persona| persona.id.clone());

        if personas.is_empty() {
            return Err("No personas found. Add personas to your config.toml file.".to_string());
        }

        // Get the default persona for the current provider/model
        let default_persona = persona_manager
            .get_default_for_provider_model(&session_context.provider_name, &session_context.model);

        let active_character_name = session_context
            .get_character()
            .map(|character| character.data.name.as_str());

        let mut items: Vec<PickerItem> = personas
            .iter()
            .map(|persona| {
                let is_default = default_persona
                    .map(|def| def == persona.id)
                    .unwrap_or(false);
                let display_label = if is_default {
                    format!("{} ({})*", persona.display_name, persona.id)
                } else {
                    format!("{} ({})", persona.display_name, persona.id)
                };
                let (metadata, inspect_metadata) = persona
                    .bio
                    .as_ref()
                    .and_then(|bio| {
                        let char_replacement = active_character_name.unwrap_or("Assistant");
                        let user_replacement = persona.display_name.as_str();
                        let substituted = bio
                            .replace("{{char}}", char_replacement)
                            .replace("{{user}}", user_replacement);
                        let sanitized = sanitize_picker_metadata(&substituted);
                        if sanitized.is_empty() {
                            None
                        } else {
                            let inspect = sanitize_picker_metadata_for_inspect(&substituted);
                            Some((sanitized, inspect))
                        }
                    })
                    .unwrap_or_else(|| {
                        let fallback = "No bio".to_string();
                        (fallback.clone(), fallback)
                    });
                PickerItem {
                    id: persona.id.clone(),
                    label: display_label,
                    metadata: Some(metadata),
                    inspect_metadata: Some(inspect_metadata),
                    sort_key: Some(persona.display_name.clone()),
                }
            })
            .collect();

        // Add "turn off persona" entry at the beginning if a persona is active
        if persona_manager.get_active_persona().is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_PERSONA_ID.to_string(),
                    label: "[Turn off persona]".to_string(),
                    metadata: Some(
                        "Deactivate current persona and return to normal mode".to_string(),
                    ),
                    inspect_metadata: Some(
                        "Deactivate current persona and return to normal mode".to_string(),
                    ),
                    sort_key: None,
                },
            );
        }

        let selected = active_persona_id
            .as_deref()
            .and_then(|active_id| items.iter().position(|item| item.id == active_id))
            .unwrap_or(0);
        let picker_state = PickerState::new("Pick Persona", items.clone(), selected);
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Persona(PersonaPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        };

        self.start_active_picker(session, active_persona_id);

        Ok(())
    }

    pub fn filter_personas(&mut self) {
        self.filter_session_items(PickerMode::Persona, &[TURN_OFF_PERSONA_ID]);
    }

    pub fn open_preset_picker(
        &mut self,
        preset_manager: &crate::core::preset::PresetManager,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        let presets = preset_manager.list_presets();
        let active_preset_id = preset_manager
            .get_active_preset()
            .map(|preset| preset.id.clone());

        if presets.is_empty() {
            return Err("No presets found. Add presets to your config.toml file.".to_string());
        }

        let default_preset = preset_manager
            .get_default_for_provider_model(&session_context.provider_name, &session_context.model);

        let mut items: Vec<PickerItem> = presets
            .iter()
            .map(|preset| {
                let is_default = default_preset.map(|def| def == preset.id).unwrap_or(false);
                let label = if is_default {
                    format!("{}*", preset.id)
                } else {
                    preset.id.clone()
                };

                let mut parts = Vec::new();
                let mut inspect_parts = Vec::new();
                let pre_trim = preset.pre.trim();
                if !pre_trim.is_empty() {
                    let sanitized = sanitize_picker_metadata(pre_trim);
                    if !sanitized.is_empty() {
                        parts.push(format!("Pre: {}", sanitized));
                        let inspect = sanitize_picker_metadata_for_inspect(pre_trim);
                        inspect_parts.push(format!("Pre:\n{}", inspect));
                    }
                }
                let post_trim = preset.post.trim();
                if !post_trim.is_empty() {
                    let sanitized = sanitize_picker_metadata(post_trim);
                    if !sanitized.is_empty() {
                        parts.push(format!("Post: {}", sanitized));
                        let inspect = sanitize_picker_metadata_for_inspect(post_trim);
                        inspect_parts.push(format!("Post:\n{}", inspect));
                    }
                }

                let metadata = if parts.is_empty() {
                    Some("No instructions".to_string())
                } else {
                    Some(parts.join(" • "))
                };

                let inspect_metadata = if inspect_parts.is_empty() {
                    Some("No instructions".to_string())
                } else {
                    Some(inspect_parts.join("\n\n"))
                };

                PickerItem {
                    id: preset.id.clone(),
                    label,
                    metadata,
                    inspect_metadata,
                    sort_key: Some(preset.id.clone()),
                }
            })
            .collect();

        if preset_manager.get_active_preset().is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_PRESET_ID.to_string(),
                    label: "[Turn off preset]".to_string(),
                    metadata: Some("Deactivate current preset".to_string()),
                    inspect_metadata: Some("Deactivate current preset".to_string()),
                    sort_key: None,
                },
            );
        }

        let selected = active_preset_id
            .as_deref()
            .and_then(|active_id| items.iter().position(|item| item.id == active_id))
            .unwrap_or(0);
        let picker_state = PickerState::new("Pick Preset", items.clone(), selected);
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Preset(PresetPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        };

        self.start_active_picker(session, active_preset_id);

        Ok(())
    }

    pub fn filter_presets(&mut self) {
        self.filter_session_items(PickerMode::Preset, &[TURN_OFF_PRESET_ID]);
    }
}
