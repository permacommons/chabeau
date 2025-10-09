use serde::{Deserialize, Serialize};

/// Character card following the v2 specification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterCard {
    pub spec: String,
    pub spec_version: String,
    pub data: CharacterData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterData {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub mes_example: String,

    // Optional fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_history_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate_greetings: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_version: Option<String>,
}

impl CharacterCard {
    /// Build the system prompt from character data (preserves {{user}} and {{char}} placeholders)
    pub fn build_system_prompt(&self) -> String {
        let mut prompt = String::new();

        if let Some(system_prompt) = &self.data.system_prompt {
            prompt.push_str(system_prompt);
            prompt.push_str("\n\n");
        }

        prompt.push_str(&format!("Character: {}\n", self.data.name));
        prompt.push_str(&format!("Description: {}\n", self.data.description));
        prompt.push_str(&format!("Personality: {}\n", self.data.personality));
        prompt.push_str(&format!("Scenario: {}\n", self.data.scenario));

        if !self.data.mes_example.is_empty() {
            prompt.push_str(&format!("\nExample dialogue:\n{}\n", self.data.mes_example));
        }

        prompt
    }

    /// Build the system prompt from character data with persona/character substitutions
    pub fn build_system_prompt_with_substitutions(
        &self,
        user_name: Option<&str>,
        char_name: Option<&str>,
    ) -> String {
        let mut prompt = String::new();

        if let Some(system_prompt) = &self.data.system_prompt {
            let substituted = self.apply_substitutions(system_prompt, user_name, char_name);
            prompt.push_str(&substituted);
            prompt.push_str("\n\n");
        }

        let char_display_name = char_name.unwrap_or(&self.data.name);
        prompt.push_str(&format!("Character: {}\n", char_display_name));
        prompt.push_str(&format!("Description: {}\n", self.data.description));
        prompt.push_str(&format!("Personality: {}\n", self.data.personality));
        prompt.push_str(&format!("Scenario: {}\n", self.data.scenario));

        if !self.data.mes_example.is_empty() {
            let substituted_example =
                self.apply_substitutions(&self.data.mes_example, user_name, char_name);
            prompt.push_str(&format!("\nExample dialogue:\n{}\n", substituted_example));
        }

        prompt
    }

    /// Get the first greeting message with substitutions applied
    pub fn get_greeting(&self) -> &str {
        &self.data.first_mes
    }

    /// Get the first greeting message with persona/character substitutions
    pub fn get_greeting_with_substitutions(
        &self,
        user_name: Option<&str>,
        char_name: Option<&str>,
    ) -> String {
        self.apply_substitutions(&self.data.first_mes, user_name, char_name)
    }

    /// Get post-history instructions if present
    pub fn get_post_history_instructions(&self) -> Option<&str> {
        self.data.post_history_instructions.as_deref()
    }

    /// Get post-history instructions with persona/character substitutions
    pub fn get_post_history_instructions_with_substitutions(
        &self,
        user_name: Option<&str>,
        char_name: Option<&str>,
    ) -> Option<String> {
        self.data
            .post_history_instructions
            .as_ref()
            .map(|instructions| self.apply_substitutions(instructions, user_name, char_name))
    }

    /// Apply {{user}} and {{char}} substitutions to text
    fn apply_substitutions(
        &self,
        text: &str,
        user_name: Option<&str>,
        char_name: Option<&str>,
    ) -> String {
        let char_replacement = char_name.unwrap_or(&self.data.name);
        let user_replacement = user_name.unwrap_or("Anon");

        text.replace("{{char}}", char_replacement)
            .replace("{{user}}", user_replacement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_card() -> CharacterCard {
        CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alice".to_string(),
                description: "A helpful AI assistant".to_string(),
                personality: "Friendly and knowledgeable".to_string(),
                scenario: "Helping users with their questions".to_string(),
                first_mes: "Hello! How can I help you today?".to_string(),
                mes_example: "{{user}}: Hi\n{{char}}: Hello there!".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        }
    }

    #[test]
    fn test_character_card_structure() {
        let card = create_test_card();
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.spec_version, "2.0");
        assert_eq!(card.data.name, "Alice");
    }

    #[test]
    fn test_build_system_prompt_basic() {
        let card = create_test_card();
        let prompt = card.build_system_prompt();

        assert!(prompt.contains("Character: Alice"));
        assert!(prompt.contains("Description: A helpful AI assistant"));
        assert!(prompt.contains("Personality: Friendly and knowledgeable"));
        assert!(prompt.contains("Scenario: Helping users with their questions"));
        assert!(prompt.contains("Example dialogue:"));
        assert!(prompt.contains("{{user}}: Hi"));
    }

    #[test]
    fn test_build_system_prompt_with_custom_system_prompt() {
        let mut card = create_test_card();
        card.data.system_prompt = Some("You are a helpful assistant.".to_string());

        let prompt = card.build_system_prompt();
        assert!(prompt.starts_with("You are a helpful assistant.\n\n"));
        assert!(prompt.contains("Character: Alice"));
    }

    #[test]
    fn test_build_system_prompt_empty_example() {
        let mut card = create_test_card();
        card.data.mes_example = String::new();

        let prompt = card.build_system_prompt();
        assert!(!prompt.contains("Example dialogue:"));
    }

    #[test]
    fn test_get_greeting() {
        let card = create_test_card();
        assert_eq!(card.get_greeting(), "Hello! How can I help you today?");
    }

    #[test]
    fn test_get_post_history_instructions_none() {
        let card = create_test_card();
        assert_eq!(card.get_post_history_instructions(), None);
    }

    #[test]
    fn test_get_post_history_instructions_some() {
        let mut card = create_test_card();
        card.data.post_history_instructions = Some("Always be polite.".to_string());

        assert_eq!(
            card.get_post_history_instructions(),
            Some("Always be polite.")
        );
    }

    #[test]
    fn test_optional_fields() {
        let mut card = create_test_card();
        card.data.creator_notes = Some("Test notes".to_string());
        card.data.alternate_greetings =
            Some(vec!["Hi there!".to_string(), "Greetings!".to_string()]);
        card.data.tags = Some(vec!["helpful".to_string(), "friendly".to_string()]);
        card.data.creator = Some("Test Creator".to_string());
        card.data.character_version = Some("1.0".to_string());

        assert_eq!(card.data.creator_notes, Some("Test notes".to_string()));
        assert_eq!(card.data.alternate_greetings.as_ref().unwrap().len(), 2);
        assert_eq!(card.data.tags.as_ref().unwrap().len(), 2);
        assert_eq!(card.data.creator, Some("Test Creator".to_string()));
        assert_eq!(card.data.character_version, Some("1.0".to_string()));
    }

    #[test]
    fn test_serialization_deserialization() {
        let card = create_test_card();
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: CharacterCard = serde_json::from_str(&json).unwrap();

        assert_eq!(card, deserialized);
    }

    #[test]
    fn test_optional_fields_not_serialized_when_none() {
        let card = create_test_card();
        let json = serde_json::to_string(&card).unwrap();

        // Optional fields should not appear in JSON when None
        assert!(!json.contains("creator_notes"));
        assert!(!json.contains("system_prompt"));
        assert!(!json.contains("post_history_instructions"));
        assert!(!json.contains("alternate_greetings"));
        assert!(!json.contains("tags"));
        assert!(!json.contains("creator"));
        assert!(!json.contains("character_version"));
    }

    #[test]
    fn test_greeting_with_substitutions() {
        let mut card = create_test_card();
        card.data.first_mes = "Hello {{user}}! I'm {{char}}, nice to meet you!".to_string();

        // Test with no substitutions (defaults)
        let greeting_default = card.get_greeting_with_substitutions(None, None);
        assert_eq!(greeting_default, "Hello Anon! I'm Alice, nice to meet you!");

        // Test with custom user and character names
        let greeting_custom = card.get_greeting_with_substitutions(Some("Bob"), Some("Assistant"));
        assert_eq!(
            greeting_custom,
            "Hello Bob! I'm Assistant, nice to meet you!"
        );
    }

    #[test]
    fn test_system_prompt_with_substitutions() {
        let mut card = create_test_card();
        card.data.system_prompt = Some("You are {{char}} talking to {{user}}.".to_string());
        card.data.mes_example = "{{user}}: Hi\n{{char}}: Hello {{user}}!".to_string();

        let prompt = card.build_system_prompt_with_substitutions(Some("Alice"), Some("Bot"));

        assert!(prompt.contains("You are Bot talking to Alice."));
        assert!(prompt.contains("Character: Bot"));
        assert!(prompt.contains("Alice: Hi\nBot: Hello Alice!"));
    }

    #[test]
    fn test_post_history_instructions_with_substitutions() {
        let mut card = create_test_card();
        card.data.post_history_instructions =
            Some("Remember that {{user}} is talking to {{char}}.".to_string());

        let instructions =
            card.get_post_history_instructions_with_substitutions(Some("John"), Some("AI"));
        assert_eq!(
            instructions,
            Some("Remember that John is talking to AI.".to_string())
        );

        // Test with None
        card.data.post_history_instructions = None;
        let instructions_none =
            card.get_post_history_instructions_with_substitutions(Some("John"), Some("AI"));
        assert_eq!(instructions_none, None);
    }
}
