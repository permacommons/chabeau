// Test helpers for character card testing
// This module provides utilities for safe, concurrent testing

#[cfg(test)]
pub(crate) mod helpers {
    use crate::character::card::{CharacterCard, CharacterData};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::{NamedTempFile, TempDir};

    fn base_character_data() -> CharacterData {
        CharacterData {
            name: "Test Character".to_string(),
            description: "A test character for unit tests".to_string(),
            personality: "Friendly and helpful".to_string(),
            scenario: "Testing environment".to_string(),
            first_mes: "Hello! I'm a test character.".to_string(),
            mes_example: "{{user}}: Hi\n{{char}}: Hello!".to_string(),
            creator_notes: None,
            system_prompt: Some("You are Test Character.".to_string()),
            post_history_instructions: Some("Always be polite.".to_string()),
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        }
    }

    /// Create a test character card with the given name and greeting
    pub fn create_test_character(name: &str, greeting: &str) -> CharacterCard {
        let mut data = base_character_data();
        data.name = name.to_string();
        data.description = format!("Test character {}", name);
        data.first_mes = greeting.to_string();
        data.system_prompt = Some(format!("You are {}.", name));

        CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data,
        }
    }

    /// Create a temporary character card file
    pub fn create_temp_card_file(card: &CharacterCard) -> NamedTempFile {
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        let json = serde_json::to_string(card).unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();
        temp_file.flush().unwrap();
        temp_file
    }

    /// Create a temporary cards directory and return the temp dir + cards dir path
    pub fn create_temp_cards_dir() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();
        (temp_dir, cards_dir)
    }

    /// Helper to create a minimal valid character card JSON string
    pub fn create_valid_card_json() -> String {
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: base_character_data(),
        };
        serde_json::to_string(&card).unwrap()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn smoke_test_helper_fixtures_remain_usable() {
            let json = create_valid_card_json();
            let card: CharacterCard = serde_json::from_str(&json).unwrap();
            let temp_file = create_temp_card_file(&card);

            assert!(temp_file.path().exists());
        }
    }
}
