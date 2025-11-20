// Test helpers for character card testing
// This module provides utilities for safe, concurrent testing

#[cfg(test)]
pub(crate) mod helpers {
    use crate::character::card::{CharacterCard, CharacterData};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::{NamedTempFile, TempDir};

    /// Create a test character card with the given name and greeting
    pub fn create_test_character(name: &str, greeting: &str) -> CharacterCard {
        CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: name.to_string(),
                description: format!("Test character {}", name),
                personality: "Friendly and helpful".to_string(),
                scenario: "Testing environment".to_string(),
                first_mes: greeting.to_string(),
                mes_example: "{{user}}: Hi\n{{char}}: Hello!".to_string(),
                creator_notes: None,
                system_prompt: Some(format!("You are {}.", name)),
                post_history_instructions: Some("Always be polite.".to_string()),
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
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

    /// Create a temporary cards directory with test cards
    /// Returns the temp directory and the cards directory path
    /// The directory will be automatically cleaned up when the TempDir is dropped
    pub fn create_temp_cards_dir() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();
        (temp_dir, cards_dir)
    }

    /// Create a temporary cards directory with pre-populated test cards
    pub fn create_temp_cards_dir_with_cards(cards: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let (temp_dir, cards_dir) = create_temp_cards_dir();

        for (name, greeting) in cards {
            let card = create_test_character(name, greeting);
            let card_json = serde_json::to_string(&card).unwrap();
            let filename = format!("{}.json", name.to_lowercase().replace(' ', "_"));
            fs::write(cards_dir.join(filename), card_json).unwrap();
        }

        (temp_dir, cards_dir)
    }

    /// Helper to create a minimal valid character card JSON string
    pub fn create_valid_card_json() -> String {
        serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Character",
                "description": "A test character for unit tests",
                "personality": "Friendly and helpful",
                "scenario": "Testing environment",
                "first_mes": "Hello! I'm a test character.",
                "mes_example": "{{user}}: Hi\n{{char}}: Hello!"
            }
        })
        .to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_create_temp_card_file() {
            let card = create_test_character("TempTest", "Hi!");
            let temp_file = create_temp_card_file(&card);
            assert!(temp_file.path().exists());

            // Verify the file contains valid JSON
            let contents = fs::read_to_string(temp_file.path()).unwrap();
            let loaded: CharacterCard = serde_json::from_str(&contents).unwrap();
            assert_eq!(loaded.data.name, "TempTest");
        }

        #[test]
        fn test_create_temp_cards_dir_with_cards() {
            let cards = vec![("Alice", "Hello, I'm Alice!"), ("Bob", "Hi, I'm Bob!")];
            let (_temp_dir, cards_dir) = create_temp_cards_dir_with_cards(&cards);

            assert!(cards_dir.join("alice.json").exists());
            assert!(cards_dir.join("bob.json").exists());

            // Verify card contents
            let alice_json = fs::read_to_string(cards_dir.join("alice.json")).unwrap();
            let alice: CharacterCard = serde_json::from_str(&alice_json).unwrap();
            assert_eq!(alice.data.name, "Alice");
        }

        #[test]
        fn test_valid_card_json() {
            let json = create_valid_card_json();
            let card: CharacterCard = serde_json::from_str(&json).unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Test Character");
        }
    }
}
