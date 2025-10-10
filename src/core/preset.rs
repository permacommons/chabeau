use crate::api::ChatMessage;
use crate::core::config::{Config, Preset};
use crate::core::persona::PersonaManager;
use std::collections::HashMap;

/// Manages preset state and operations
pub struct PresetManager {
    presets: Vec<Preset>,
    active_preset: Option<Preset>,
    defaults: HashMap<(String, String), String>,
}

impl PresetManager {
    /// Create a new PresetManager and load presets from configuration
    pub fn load_presets(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let mut defaults = HashMap::new();
        for (provider, models) in &config.default_presets {
            for (model, preset_id) in models {
                defaults.insert((provider.to_lowercase(), model.clone()), preset_id.clone());
            }
        }

        Ok(Self {
            presets: config.presets.clone(),
            active_preset: None,
            defaults,
        })
    }

    /// Get the list of available presets
    pub fn list_presets(&self) -> &Vec<Preset> {
        &self.presets
    }

    /// Find a preset by its ID
    pub fn find_preset_by_id(&self, id: &str) -> Option<&Preset> {
        self.presets.iter().find(|preset| preset.id == id)
    }

    /// Set the active preset by ID
    pub fn set_active_preset(&mut self, preset_id: &str) -> Result<(), String> {
        match self.find_preset_by_id(preset_id) {
            Some(preset) => {
                self.active_preset = Some(preset.clone());
                Ok(())
            }
            None => {
                let available_ids: Vec<&str> = self
                    .presets
                    .iter()
                    .map(|preset| preset.id.as_str())
                    .collect();
                Err(format!(
                    "Preset '{}' not found. Available presets: {}",
                    preset_id,
                    available_ids.join(", ")
                ))
            }
        }
    }

    /// Clear the active preset (deactivate)
    pub fn clear_active_preset(&mut self) {
        self.active_preset = None;
    }

    /// Get the currently active preset
    pub fn get_active_preset(&self) -> Option<&Preset> {
        self.active_preset.as_ref()
    }

    /// Apply preset instructions to the provided messages
    /// Adds or augments system messages at the beginning/end after persona substitutions
    pub fn apply_to_messages(
        &self,
        messages: &mut Vec<ChatMessage>,
        persona_manager: &PersonaManager,
        char_name: Option<&str>,
    ) {
        let Some(active_preset) = &self.active_preset else {
            return;
        };

        let pre_text = active_preset.pre.trim();
        let post_text = active_preset.post.trim();

        let substituted_pre = if pre_text.is_empty() {
            None
        } else {
            let substituted = persona_manager.apply_substitutions(pre_text, char_name);
            let trimmed = substituted.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        let substituted_post = if post_text.is_empty() {
            None
        } else {
            let substituted = persona_manager.apply_substitutions(post_text, char_name);
            let trimmed = substituted.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        if substituted_pre.is_none() && substituted_post.is_none() {
            return;
        }

        if substituted_pre.is_some()
            && messages
                .first()
                .map(|msg| msg.role != "system")
                .unwrap_or(true)
        {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".to_string(),
                    content: String::new(),
                },
            );
        }

        if substituted_post.is_some()
            && messages
                .last()
                .map(|msg| msg.role != "system")
                .unwrap_or(true)
        {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: String::new(),
            });
        }

        if let Some(pre) = substituted_pre {
            if let Some(first) = messages.first_mut() {
                if first.content.trim().is_empty() {
                    first.content = pre;
                } else {
                    first.content = format!("{pre}\n\n{}", first.content);
                }
            }
        }

        if let Some(post) = substituted_post {
            if let Some(last) = messages.last_mut() {
                if last.content.trim().is_empty() {
                    last.content = post;
                } else {
                    last.content = format!("{}\n\n{post}", last.content);
                }
            }
        }
    }

    /// Get the default preset for a provider/model combination
    pub fn get_default_for_provider_model(&self, provider: &str, model: &str) -> Option<&str> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.get(&key).map(|s| s.as_str())
    }

    /// Set the default preset for a provider/model combination and persist to config
    pub fn set_default_for_provider_model_persistent(
        &mut self,
        provider: &str,
        model: &str,
        preset_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.insert(key, preset_id.to_string());

        let provider = provider.to_string();
        let model = model.to_string();
        let preset_id = preset_id.to_string();

        Config::mutate(move |config| {
            config.set_default_preset(provider, model, preset_id);
            Ok(())
        })?;

        Ok(())
    }

    /// Unset the default preset for a provider/model combination and persist to config
    pub fn unset_default_for_provider_model_persistent(
        &mut self,
        provider: &str,
        model: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.remove(&key);

        let provider = provider.to_string();
        let model = model.to_string();

        Config::mutate(move |config| {
            config.unset_default_preset(&provider, &model);
            Ok(())
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Persona, Preset};

    fn create_test_config() -> Config {
        Config {
            personas: vec![Persona {
                id: "tester".to_string(),
                display_name: "Tester".to_string(),
                bio: Some("You are speaking with {{user}}.".to_string()),
            }],
            presets: vec![Preset {
                id: "focus".to_string(),
                pre: "Focus on {{user}}'s requirements.".to_string(),
                post: "Confirm actions with {{char}}.".to_string(),
            }],
            ..Default::default()
        }
    }

    fn create_messages() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }]
    }

    #[test]
    fn test_set_and_clear_active_preset() {
        let config = create_test_config();
        let mut manager = PresetManager::load_presets(&config).expect("load presets");

        assert!(manager.get_active_preset().is_none());
        manager.set_active_preset("focus").expect("set preset");
        assert_eq!(
            manager.get_active_preset().map(|p| p.id.as_str()),
            Some("focus")
        );

        manager.clear_active_preset();
        assert!(manager.get_active_preset().is_none());
    }

    #[test]
    fn test_apply_to_messages_inserts_system_messages() {
        let config = create_test_config();
        let mut manager = PresetManager::load_presets(&config).expect("load presets");
        manager.set_active_preset("focus").expect("activate preset");

        let mut persona_manager = PersonaManager::load_personas(&config).expect("load personas");
        persona_manager
            .set_active_persona("tester")
            .expect("set persona");

        let mut messages = create_messages();
        manager.apply_to_messages(&mut messages, &persona_manager, Some("HelperBot"));

        assert!(messages.first().unwrap().role == "system");
        assert!(messages.last().unwrap().role == "system");
        assert!(messages[0]
            .content
            .contains("Focus on Tester\'s requirements."));
        assert!(messages
            .last()
            .unwrap()
            .content
            .contains("Confirm actions with HelperBot."));
    }

    #[test]
    fn test_apply_to_messages_skips_when_empty() {
        let mut config = create_test_config();
        config.presets[0].pre.clear();
        config.presets[0].post.clear();

        let mut manager = PresetManager::load_presets(&config).expect("load presets");
        manager.set_active_preset("focus").expect("activate preset");

        let persona_manager = PersonaManager::load_personas(&config).expect("load personas");

        let mut messages = create_messages();
        manager.apply_to_messages(&mut messages, &persona_manager, None);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn test_get_default_for_provider_model() {
        let mut config = create_test_config();
        config
            .default_presets
            .entry("openai".to_string())
            .or_default()
            .insert("gpt-4".to_string(), "focus".to_string());

        let manager = PresetManager::load_presets(&config).expect("load presets");
        assert_eq!(
            manager.get_default_for_provider_model("OpenAI", "gpt-4"),
            Some("focus")
        );
    }
}
