use crate::core::config::{Config, Persona};
use std::collections::HashMap;

/// Manages persona state and operations
pub struct PersonaManager {
    /// List of available personas loaded from configuration
    personas: Vec<Persona>,
    /// Currently active persona
    active_persona: Option<Persona>,
    /// Provider-specific default persona storage
    /// Key format: "provider_model" -> persona_id
    defaults: HashMap<String, String>,
}

impl PersonaManager {
    /// Create a new PersonaManager and load personas from configuration
    pub fn load_personas(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(PersonaManager {
            personas: config.personas.clone(),
            active_persona: None,
            defaults: HashMap::new(),
        })
    }

    /// Get the list of available personas
    pub fn list_personas(&self) -> &Vec<Persona> {
        &self.personas
    }

    /// Find a persona by its ID
    pub fn find_persona_by_id(&self, id: &str) -> Option<&Persona> {
        self.personas.iter().find(|p| p.id == id)
    }

    /// Set the active persona by ID
    pub fn set_active_persona(&mut self, persona_id: &str) -> Result<(), String> {
        match self.find_persona_by_id(persona_id) {
            Some(persona) => {
                self.active_persona = Some(persona.clone());
                Ok(())
            }
            None => {
                let available_ids: Vec<&str> =
                    self.personas.iter().map(|p| p.id.as_str()).collect();
                Err(format!(
                    "Persona '{}' not found. Available personas: {}",
                    persona_id,
                    available_ids.join(", ")
                ))
            }
        }
    }

    /// Clear the active persona (deactivate)
    pub fn clear_active_persona(&mut self) {
        self.active_persona = None;
    }

    /// Get the currently active persona
    pub fn get_active_persona(&self) -> Option<&Persona> {
        self.active_persona.as_ref()
    }

    /// Apply character and user substitutions to text
    /// {{char}} is replaced with the character name (or "Assistant" if None)
    /// {{user}} is replaced with the active persona name (or "Anon" if no persona)
    pub fn apply_substitutions(&self, text: &str, char_name: Option<&str>) -> String {
        let char_replacement = char_name.unwrap_or("Assistant");
        let user_replacement = match &self.active_persona {
            Some(persona) => &persona.name,
            None => "Anon",
        };

        text.replace("{{char}}", char_replacement)
            .replace("{{user}}", user_replacement)
    }

    /// Get the display name for the user in conversations
    /// Returns the active persona's name or "You" if no persona is active
    pub fn get_display_name(&self) -> String {
        match &self.active_persona {
            Some(persona) => persona.name.clone(),
            None => "You".to_string(),
        }
    }

    /// Get the modified system prompt with persona bio prepended
    /// If a persona is active, prepends the persona's bio (with substitutions applied) to the base prompt
    pub fn get_modified_system_prompt(&self, base_prompt: &str) -> String {
        match &self.active_persona {
            Some(persona) => {
                if let Some(bio) = &persona.bio {
                    // Apply substitutions to the persona bio, using the persona's own name for {{user}}
                    let substituted_bio = bio
                        .replace("{{char}}", "Assistant") // Default char name in bio
                        .replace("{{user}}", &persona.name); // Use persona's own name for {{user}} in bio

                    format!("{}\n\n{}", substituted_bio, base_prompt)
                } else {
                    base_prompt.to_string()
                }
            }
            None => base_prompt.to_string(),
        }
    }

    /// Set the default persona for a provider/model combination
    pub fn set_default_for_provider_model(&mut self, provider_model: &str, persona_id: &str) {
        self.defaults
            .insert(provider_model.to_string(), persona_id.to_string());
    }

    /// Get the default persona for a provider/model combination
    pub fn get_default_for_provider_model(&self, provider_model: &str) -> Option<&str> {
        self.defaults.get(provider_model).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> Config {
        Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    name: "Alice".to_string(),
                    bio: Some("You are talking to {{user}}, a senior developer.".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    name: "Bob".to_string(),
                    bio: Some(
                        "{{user}} is a computer science student learning about AI.".to_string(),
                    ),
                },
                Persona {
                    id: "charlie-no-bio".to_string(),
                    name: "Charlie".to_string(),
                    bio: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_persona_loading_from_configuration() {
        let config = create_test_config();
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        assert_eq!(manager.list_personas().len(), 3);
        assert!(manager.find_persona_by_id("alice-dev").is_some());
        assert!(manager.find_persona_by_id("bob-student").is_some());
        assert!(manager.find_persona_by_id("charlie-no-bio").is_some());
        assert!(manager.find_persona_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_persona_activation_and_deactivation() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Initially no persona is active
        assert!(manager.get_active_persona().is_none());

        // Activate a persona
        assert!(manager.set_active_persona("alice-dev").is_ok());
        let active = manager.get_active_persona().expect("No active persona");
        assert_eq!(active.id, "alice-dev");
        assert_eq!(active.name, "Alice");

        // Switch to another persona
        assert!(manager.set_active_persona("bob-student").is_ok());
        let active = manager.get_active_persona().expect("No active persona");
        assert_eq!(active.id, "bob-student");
        assert_eq!(active.name, "Bob");

        // Deactivate persona
        manager.clear_active_persona();
        assert!(manager.get_active_persona().is_none());
    }

    #[test]
    fn test_invalid_persona_id_error_handling() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        let result = manager.set_active_persona("nonexistent");
        assert!(result.is_err());

        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("Persona 'nonexistent' not found"));
        assert!(error_msg.contains("alice-dev"));
        assert!(error_msg.contains("bob-student"));
        assert!(error_msg.contains("charlie-no-bio"));
    }

    #[test]
    fn test_substitution_logic_with_no_persona() {
        let config = create_test_config();
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        let text = "Hello {{user}}, meet {{char}}!";
        let result = manager.apply_substitutions(text, Some("Assistant"));
        assert_eq!(result, "Hello Anon, meet Assistant!");

        let result_no_char = manager.apply_substitutions(text, None);
        assert_eq!(result_no_char, "Hello Anon, meet Assistant!");
    }

    #[test]
    fn test_substitution_logic_with_active_persona() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        let text = "Hello {{user}}, meet {{char}}!";
        let result = manager.apply_substitutions(text, Some("ChatBot"));
        assert_eq!(result, "Hello Alice, meet ChatBot!");

        let result_no_char = manager.apply_substitutions(text, None);
        assert_eq!(result_no_char, "Hello Alice, meet Assistant!");
    }

    #[test]
    fn test_display_name_with_no_persona() {
        let config = create_test_config();
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        assert_eq!(manager.get_display_name(), "You");
    }

    #[test]
    fn test_display_name_with_active_persona() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");
        assert_eq!(manager.get_display_name(), "Alice");

        manager
            .set_active_persona("bob-student")
            .expect("Failed to activate persona");
        assert_eq!(manager.get_display_name(), "Bob");
    }

    #[test]
    fn test_system_prompt_modification_no_persona() {
        let config = create_test_config();
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        let base_prompt = "You are a helpful assistant.";
        let result = manager.get_modified_system_prompt(base_prompt);
        assert_eq!(result, base_prompt);
    }

    #[test]
    fn test_system_prompt_modification_with_persona_bio() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        let base_prompt = "You are a helpful assistant.";
        let result = manager.get_modified_system_prompt(base_prompt);
        let expected =
            "You are talking to Alice, a senior developer.\n\nYou are a helpful assistant.";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_system_prompt_modification_with_persona_no_bio() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        manager
            .set_active_persona("charlie-no-bio")
            .expect("Failed to activate persona");

        let base_prompt = "You are a helpful assistant.";
        let result = manager.get_modified_system_prompt(base_prompt);
        assert_eq!(result, base_prompt);
    }

    #[test]
    fn test_persona_bio_substitution() {
        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");

        manager
            .set_active_persona("bob-student")
            .expect("Failed to activate persona");

        let base_prompt = "You are a helpful assistant.";
        let result = manager.get_modified_system_prompt(base_prompt);
        let expected =
            "Bob is a computer science student learning about AI.\n\nYou are a helpful assistant.";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_ui_display_name_integration() {
        use crate::core::app::ui_state::UiState;
        use crate::ui::theme::Theme;

        let config = create_test_config();
        let mut manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);

        // Initially should show "You"
        assert_eq!(ui.user_display_name, "You");

        // Activate a persona and update UI
        manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");
        let display_name = manager.get_display_name();
        ui.update_user_display_name(display_name);

        // Should now show persona name
        assert_eq!(ui.user_display_name, "Alice");

        // Deactivate persona and update UI
        manager.clear_active_persona();
        let display_name = manager.get_display_name();
        ui.update_user_display_name(display_name);

        // Should be back to "You"
        assert_eq!(ui.user_display_name, "You");
    }
}
#[test]
fn test_message_rendering_with_persona_display_name() {
    use crate::core::message::Message;
    use crate::ui::markdown::{render_message_with_config, MessageRenderConfig};
    use crate::ui::theme::Theme;

    let theme = Theme::dark_default();
    let message = Message {
        role: "user".to_string(),
        content: "Hello world".to_string(),
    };

    // Test with default "You:"
    let config_default = MessageRenderConfig::plain();
    let rendered_default = render_message_with_config(&message, &theme, config_default);
    let first_line_default = rendered_default.lines.first().unwrap().to_string();
    assert!(first_line_default.starts_with("You: "));

    // Test with persona display name
    let config_persona =
        MessageRenderConfig::plain().with_user_display_name(Some("Alice".to_string()));
    let rendered_persona = render_message_with_config(&message, &theme, config_persona);
    let first_line_persona = rendered_persona.lines.first().unwrap().to_string();
    assert!(first_line_persona.starts_with("Alice: "));
}
