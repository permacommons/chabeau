// Integration tests for character card workflows
// These tests verify end-to-end functionality across multiple modules

#[cfg(test)]
mod integration_tests {

    use crate::character::card::{CharacterCard, CharacterData};
    use crate::character::import::{import_card, ImportError};
    use crate::core::app::conversation::ConversationController;
    use crate::core::app::session::load_character_for_session;
    use crate::core::config::Config;
    use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
    use std::fs;
    use std::io::Write;

    use tempfile::{NamedTempFile, TempDir};

    /// Helper to create a test character card
    fn create_test_character(name: &str, greeting: &str) -> CharacterCard {
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

    /// Helper to create a temporary character card file
    fn create_temp_card_file(card: &CharacterCard) -> NamedTempFile {
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        let json = serde_json::to_string(card).unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();
        temp_file.flush().unwrap();
        temp_file
    }

    #[test]
    fn test_import_then_select_via_cli_workflow() {
        // This test simulates: import card → start session with CLI flag → verify character loaded

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Step 1: Create a character card file
        let character = create_test_character("TestCLI", "Hello from CLI!");
        let temp_card = create_temp_card_file(&character);

        // Step 2: Simulate import (copy to cards directory)
        let dest_path = cards_dir.join("testcli.json");
        fs::copy(temp_card.path(), &dest_path).unwrap();

        // Step 3: Load character by name (simulating CLI flag)
        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(dest_path.to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_ok());
        let loaded_card = result.unwrap();
        assert!(loaded_card.is_some());

        let card = loaded_card.unwrap();
        assert_eq!(card.data.name, "TestCLI");
        assert_eq!(card.data.first_mes, "Hello from CLI!");
    }

    #[test]
    fn test_import_then_select_via_picker_workflow() {
        // This test simulates: import card → list cards for picker → select card

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Step 1: Import multiple cards
        let char1 = create_test_character("PickerChar1", "Hello 1!");
        let char2 = create_test_character("PickerChar2", "Hello 2!");

        let card1_json = serde_json::to_string(&char1).unwrap();
        let card2_json = serde_json::to_string(&char2).unwrap();

        fs::write(cards_dir.join("picker1.json"), card1_json).unwrap();
        fs::write(cards_dir.join("picker2.json"), card2_json).unwrap();

        // Step 2: List available cards (simulating picker opening)
        // Note: This would normally use list_available_cards(), but that uses the real cards dir
        // For this test, we'll directly verify the cards exist
        assert!(cards_dir.join("picker1.json").exists());
        assert!(cards_dir.join("picker2.json").exists());

        // Step 3: Load a specific card by name (simulating picker selection)
        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(cards_dir.join("picker1.json").to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_ok());
        let loaded_card = result.unwrap();
        assert!(loaded_card.is_some());
        assert_eq!(loaded_card.unwrap().data.name, "PickerChar1");
    }

    #[test]
    fn test_import_set_default_start_session_workflow() {
        // This test simulates: import card → set as default → start session → verify auto-loaded

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Step 1: Import a card
        let character = create_test_character("DefaultChar", "Hello by default!");
        let card_json = serde_json::to_string(&character).unwrap();
        let card_path = cards_dir.join("defaultchar.json");
        fs::write(&card_path, card_json).unwrap();

        // Step 2: Set as default in config
        let mut config = Config::default();
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "defaultchar".to_string(),
        );

        // Verify default is set
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"defaultchar".to_string())
        );

        // Step 3: Start session without CLI flag (should load default)
        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", &cards_dir);
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(None, "openai", "gpt-4", &config, &mut service);

        let loaded_card = result.expect("default load result");
        assert!(loaded_card.is_some());
        assert_eq!(loaded_card.unwrap().data.name, "DefaultChar");
    }

    #[test]
    fn test_character_switching_during_session() {
        // This test simulates: start with character A → switch to character B → verify state

        let mut app = create_test_app();

        // Start with character A
        let char_a = create_test_character("CharA", "Hello from A!");
        app.session.set_character(char_a.clone());

        assert_eq!(app.session.get_character().unwrap().data.name, "CharA");
        assert!(!app.session.character_greeting_shown);

        // Show greeting for character A
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        assert_eq!(app.ui.messages.len(), 1);
        assert_eq!(app.ui.messages.front().unwrap().content, "Hello from A!");
        assert!(app.session.character_greeting_shown);

        // Switch to character B
        let char_b = create_test_character("CharB", "Hello from B!");
        app.session.set_character(char_b.clone());

        // Greeting flag should be reset for new character
        assert_eq!(app.session.get_character().unwrap().data.name, "CharB");
        assert!(!app.session.character_greeting_shown);

        // Show greeting for character B
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Should have both greetings now
        assert_eq!(app.ui.messages.len(), 2);
        assert_eq!(app.ui.messages.back().unwrap().content, "Hello from B!");
        assert!(app.session.character_greeting_shown);
    }

    #[test]
    fn test_error_case_missing_card_file() {
        // Test loading a non-existent card file

        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some("/nonexistent/path/to/card.json"),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_str = err.to_string();
        // The error message should indicate the file wasn't found
        assert!(
            err_str.contains("File not found")
                || err_str.contains("No such file")
                || err_str.contains("not found")
                || err_str.contains("Failed to load"),
            "Unexpected error message: {}",
            err_str
        );
    }

    #[test]
    fn test_error_case_invalid_card_format() {
        // Test loading a card with invalid JSON

        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file.write_all(b"{ invalid json }").unwrap();
        temp_file.flush().unwrap();

        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(temp_file.path().to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid JSON") || err.to_string().contains("expected"));
    }

    #[test]
    fn test_error_case_missing_required_fields() {
        // Test loading a card missing required fields

        let invalid_card = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "",  // Empty name should fail validation
                "description": "Test",
                "personality": "Test",
                "scenario": "Test",
                "first_mes": "Test",
                "mes_example": "Test"
            }
        });

        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file
            .write_all(invalid_card.to_string().as_bytes())
            .unwrap();
        temp_file.flush().unwrap();

        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(temp_file.path().to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("name") || err.to_string().contains("validation"));
    }

    #[test]
    fn test_error_case_wrong_spec_version() {
        // Test loading a card with wrong spec version

        let wrong_spec = serde_json::json!({
            "spec": "wrong_spec",
            "spec_version": "1.0",
            "data": {
                "name": "Test",
                "description": "Test",
                "personality": "Test",
                "scenario": "Test",
                "first_mes": "Test",
                "mes_example": "Test"
            }
        });

        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file
            .write_all(wrong_spec.to_string().as_bytes())
            .unwrap();
        temp_file.flush().unwrap();

        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(temp_file.path().to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("spec") || err.to_string().contains("validation"));
    }

    #[test]
    fn test_full_conversation_flow_with_character() {
        // Test a complete conversation flow with character active

        let mut app = create_test_app();

        // Set up character
        let character = create_test_character("ConvoBot", "Hello! How can I help?");
        app.session.set_character(character);

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        assert_eq!(app.ui.messages.len(), 1);
        assert_eq!(
            app.ui.messages.front().unwrap().content,
            "Hello! How can I help?"
        );

        // User sends first message
        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("What's the weather?".to_string())
        };

        // Verify API messages include character system prompt, greeting, and user message
        assert!(api_messages.len() >= 3);
        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("You are ConvoBot"));
        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[1].content, "Hello! How can I help?");
        assert_eq!(api_messages[2].role, "user");
        assert_eq!(api_messages[2].content, "What's the weather?");

        // Last message should be post-history instructions
        assert_eq!(api_messages.last().unwrap().role, "system");
        assert_eq!(api_messages.last().unwrap().content, "Always be polite.");
    }

    #[test]
    fn test_config_persistence_with_multiple_defaults() {
        // Test that default characters persist across config save/load

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");

        // Create config with multiple default characters
        let mut config = Config::default();
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "openai".to_string(),
            "gpt-4o".to_string(),
            "bob".to_string(),
        );
        config.set_default_character(
            "anthropic".to_string(),
            "claude-3-opus".to_string(),
            "charlie".to_string(),
        );

        // Save config
        let toml_str = toml::to_string(&config).unwrap();
        fs::write(&config_path, toml_str).unwrap();

        // Load config
        let loaded_toml = fs::read_to_string(&config_path).unwrap();
        let loaded_config: Config = toml::from_str(&loaded_toml).unwrap();

        // Verify all defaults are preserved
        assert_eq!(
            loaded_config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );
        assert_eq!(
            loaded_config.get_default_character("openai", "gpt-4o"),
            Some(&"bob".to_string())
        );
        assert_eq!(
            loaded_config.get_default_character("anthropic", "claude-3-opus"),
            Some(&"charlie".to_string())
        );
    }

    #[test]
    fn test_character_precedence_cli_over_default() {
        // Test that CLI-specified character takes precedence over default

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Create two cards
        let default_char = create_test_character("DefaultChar", "I'm the default");
        let cli_char = create_test_character("CLIChar", "I'm from CLI");

        let default_json = serde_json::to_string(&default_char).unwrap();
        let cli_json = serde_json::to_string(&cli_char).unwrap();

        let default_path = cards_dir.join("default.json");
        let cli_path = cards_dir.join("cli.json");

        fs::write(&default_path, default_json).unwrap();
        fs::write(&cli_path, cli_json).unwrap();

        // Set up config with default
        let mut config = Config::default();
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "default".to_string(),
        );

        // Load with CLI override
        let mut service = crate::character::CharacterService::new();
        let result = load_character_for_session(
            Some(cli_path.to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        assert!(result.is_ok());
        let loaded = result.unwrap();
        assert!(loaded.is_some());

        // Should load CLI character, not default
        assert_eq!(loaded.unwrap().data.name, "CLIChar");
    }

    #[test]
    fn test_character_with_empty_optional_fields() {
        // Test that characters with empty optional fields work correctly

        let mut app = create_test_app();

        let minimal_char = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "MinimalChar".to_string(),
                description: "Minimal".to_string(),
                personality: "Simple".to_string(),
                scenario: "Test".to_string(),
                first_mes: "Hi".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(minimal_char);

        // Should work without errors
        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Test".to_string())
        };

        // Should have system prompt and user message (no post-history)
        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "system");
        assert_eq!(api_messages[1].role, "user");
    }

    #[test]
    fn test_import_overwrite_protection() {
        // Test that import prevents overwriting without force flag

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        // Create initial card
        let char1 = create_test_character("OverwriteTest", "Version 1");
        let card1_json = serde_json::to_string(&char1).unwrap();
        let card_path = cards_dir.join("overwrite.json");
        fs::write(&card_path, card1_json).unwrap();

        // Try to import a different card with same filename
        let char2 = create_test_character("OverwriteTest", "Version 2");
        let temp_card_path = temp_dir.path().join("overwrite.json");
        fs::write(&temp_card_path, serde_json::to_string(&char2).unwrap()).unwrap();

        let result = {
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());

            // Import without force should fail and keep existing file intact
            let result = import_card(&temp_card_path, false);
            drop(env_guard);
            result
        };
        assert!(matches!(result, Err(ImportError::AlreadyExists(_))));

        // Verify original content is preserved
        let content = fs::read_to_string(&card_path).unwrap();
        assert!(content.contains("Version 1"));
    }
}
