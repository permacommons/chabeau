// Integration tests for persona workflows
// These tests verify end-to-end functionality across multiple modules

#[cfg(test)]
mod integration_tests {
    use crate::commands::{process_input, CommandResult};
    use crate::core::app::conversation::ConversationController;
    use crate::core::config::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::create_test_app;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create test personas for integration tests
    fn create_test_personas() -> Vec<Persona> {
        vec![
            Persona {
                id: "alice-dev".to_string(),
                display_name: "Alice".to_string(),
                bio: Some("You are talking to Alice, a senior software developer with 10 years of experience in {{char}} development.".to_string()),
            },
            Persona {
                id: "bob-student".to_string(),
                display_name: "Bob".to_string(),
                bio: Some("You are talking to {{user}}, a computer science student learning about AI.".to_string()),
            },
            Persona {
                id: "charlie-no-bio".to_string(),
                display_name: "Charlie".to_string(),
                bio: None,
            },
        ]
    }

    /// Helper to create a test config with personas
    fn create_test_config_with_personas() -> Config {
        Config {
            personas: create_test_personas(),
            ..Default::default()
        }
    }

    #[test]
    fn test_cli_persona_activation_workflow() {
        // Test CLI argument processing with persona activation
        // This simulates: start app with --persona flag → verify persona is active

        let config = create_test_config_with_personas();
        let mut persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Simulate CLI persona selection
        let cli_persona_id = "alice-dev";
        let result = persona_manager.set_active_persona(cli_persona_id);
        assert!(result.is_ok(), "Failed to set CLI persona");

        // Verify persona is active
        let active_persona = persona_manager.get_active_persona();
        assert!(
            active_persona.is_some(),
            "No persona is active after CLI selection"
        );
        assert_eq!(active_persona.unwrap().id, cli_persona_id);
        assert_eq!(active_persona.unwrap().display_name, "Alice");

        // Verify display name is updated
        assert_eq!(persona_manager.get_display_name(), "Alice");

        // Verify system prompt modification
        let base_prompt = "You are a helpful assistant.";
        let modified_prompt = persona_manager.get_modified_system_prompt(base_prompt);
        assert!(modified_prompt.contains("Alice, a senior software developer"));
        assert!(modified_prompt.contains(base_prompt));
    }

    #[test]
    fn test_cli_invalid_persona_error_handling() {
        // Test CLI error handling for invalid persona IDs

        let config = create_test_config_with_personas();
        let mut persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Try to set invalid persona
        let result = persona_manager.set_active_persona("nonexistent-persona");
        assert!(result.is_err(), "Should fail for invalid persona ID");

        // Verify no persona is active
        assert!(persona_manager.get_active_persona().is_none());
        assert_eq!(persona_manager.get_display_name(), "You");
    }

    #[test]
    fn test_interactive_persona_command_workflow() {
        // Test interactive command execution and picker workflow
        // This simulates: /persona command → picker opens → selection made

        let mut app = create_test_app();

        // Add test personas to the app's persona manager
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Execute /persona command
        let result = process_input(&mut app, "/persona");
        assert!(
            matches!(result, CommandResult::OpenPersonaPicker),
            "Persona command should open picker"
        );

        // Test direct persona activation via command
        let result = process_input(&mut app, "/persona alice-dev");
        assert!(
            matches!(result, CommandResult::Continue),
            "Direct persona activation should continue"
        );

        // Verify persona is activated
        let active_persona = app.persona_manager.get_active_persona();
        assert!(
            active_persona.is_some(),
            "Persona should be active after direct command"
        );
        assert_eq!(active_persona.unwrap().id, "alice-dev");

        // Test invalid persona ID
        let result = process_input(&mut app, "/persona nonexistent");
        assert!(
            matches!(result, CommandResult::Continue),
            "Invalid persona should continue with error"
        );

        // Verify original persona is still active
        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "alice-dev");
    }

    #[test]
    fn test_persona_picker_with_active_persona() {
        // Test picker behavior when a persona is already active

        let mut app = create_test_app();

        // Set up personas and activate one
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");
        app.persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        // Execute /persona command
        let result = process_input(&mut app, "/persona");
        assert!(matches!(result, CommandResult::OpenPersonaPicker));

        // Test switching to another persona
        let result = process_input(&mut app, "/persona bob-student");
        assert!(matches!(result, CommandResult::Continue));

        // Verify persona switched
        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "bob-student");
        assert_eq!(active_persona.unwrap().display_name, "Bob");
    }

    #[test]
    fn test_persona_deactivation_workflow() {
        // Test persona deactivation through picker

        let mut app = create_test_app();

        // Set up personas and activate one
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");
        app.persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        // Verify persona is active
        assert!(app.persona_manager.get_active_persona().is_some());
        assert_eq!(app.persona_manager.get_display_name(), "Alice");

        // Deactivate persona
        app.persona_manager.clear_active_persona();

        // Verify persona is deactivated
        assert!(app.persona_manager.get_active_persona().is_none());
        assert_eq!(app.persona_manager.get_display_name(), "You");
    }

    #[test]
    fn test_system_prompt_modification_with_active_persona() {
        // Test system prompt modification with active personas

        let config = create_test_config_with_personas();
        let mut persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        let base_prompt = "You are a helpful assistant.";

        // Test without persona
        let prompt_no_persona = persona_manager.get_modified_system_prompt(base_prompt);
        assert_eq!(
            prompt_no_persona, base_prompt,
            "Prompt should be unchanged without persona"
        );

        // Test with persona that has bio
        persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");
        let prompt_with_persona = persona_manager.get_modified_system_prompt(base_prompt);

        assert!(prompt_with_persona.contains("Alice, a senior software developer"));
        assert!(prompt_with_persona.contains(base_prompt));
        assert!(
            prompt_with_persona.len() > base_prompt.len(),
            "Modified prompt should be longer"
        );

        // Test with persona without bio
        persona_manager
            .set_active_persona("charlie-no-bio")
            .expect("Failed to activate persona");
        let prompt_no_bio = persona_manager.get_modified_system_prompt(base_prompt);
        assert_eq!(
            prompt_no_bio, base_prompt,
            "Prompt should be unchanged for persona without bio"
        );
    }

    #[test]
    fn test_persona_substitution_in_conversation() {
        // Test character and user substitution with personas

        let config = create_test_config_with_personas();
        let mut persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Test substitution without persona
        let text_with_placeholders = "Hello {{user}}, I am {{char}}!";
        let result_no_persona =
            persona_manager.apply_substitutions(text_with_placeholders, Some("TestBot"));
        assert_eq!(result_no_persona, "Hello Anon, I am TestBot!");

        // Test substitution with persona
        persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");
        let result_with_persona =
            persona_manager.apply_substitutions(text_with_placeholders, Some("TestBot"));
        assert_eq!(result_with_persona, "Hello Alice, I am TestBot!");

        // Test substitution in persona bio
        let active_persona = persona_manager.get_active_persona().unwrap();
        let bio_with_substitution = active_persona.bio.as_ref().unwrap();
        let substituted_bio =
            persona_manager.apply_substitutions(bio_with_substitution, Some("TestBot"));
        assert!(substituted_bio.contains("Alice, a senior software developer"));
        assert!(substituted_bio.contains("TestBot development"));
    }

    #[test]
    fn test_default_persona_loading_from_config() {
        // Test automatic default persona loading based on provider/model

        let mut config = create_test_config_with_personas();

        // Set default persona in config
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );

        let persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Verify default is loaded
        let default = persona_manager.get_default_for_provider_model("openai_gpt-4");
        assert!(default.is_some());
        assert_eq!(default.unwrap(), "alice-dev");
    }

    #[test]
    fn test_cli_persona_overrides_default() {
        // Test that CLI persona selection overrides default persona

        let mut config = create_test_config_with_personas();
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "bob-student".to_string(),
        );

        let mut persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Verify default is set
        assert_eq!(
            persona_manager
                .get_default_for_provider_model("openai_gpt-4")
                .unwrap(),
            "bob-student"
        );

        // Simulate CLI override
        persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to set CLI persona");

        // Verify CLI persona is active (not default)
        let active = persona_manager.get_active_persona().unwrap();
        assert_eq!(active.id, "alice-dev");
        assert_ne!(active.id, "bob-student");
    }

    #[test]
    fn test_persona_display_name_in_conversation() {
        // Test persona display name integration with conversation UI

        let mut app = create_test_app();

        // Set up personas
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Test without persona - should use "You"
        assert_eq!(app.persona_manager.get_display_name(), "You");

        // Activate persona
        app.persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        // Test with persona - should use persona display name
        assert_eq!(app.persona_manager.get_display_name(), "Alice");

        // Add a user message and verify display name is used
        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_user_message("Hello!".to_string());
        }

        // Verify message was added (we can't easily test the exact display without UI rendering)
        assert!(!app.ui.messages.is_empty());
    }

    #[test]
    fn test_persona_config_persistence() {
        // Test persona configuration persistence across save/load

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_personas.toml");

        // Create config with personas and defaults
        let mut config = create_test_config_with_personas();
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );
        config.set_default_persona(
            "anthropic".to_string(),
            "claude-3-opus".to_string(),
            "bob-student".to_string(),
        );

        // Save config
        let toml_str = toml::to_string(&config).unwrap();
        fs::write(&config_path, toml_str).unwrap();

        // Load config
        let loaded_toml = fs::read_to_string(&config_path).unwrap();
        let loaded_config: Config = toml::from_str(&loaded_toml).unwrap();

        // Verify personas are preserved
        assert_eq!(loaded_config.personas.len(), 3);
        assert!(loaded_config.personas.iter().any(|p| p.id == "alice-dev"));
        assert!(loaded_config.personas.iter().any(|p| p.id == "bob-student"));
        assert!(loaded_config
            .personas
            .iter()
            .any(|p| p.id == "charlie-no-bio"));

        // Verify defaults are preserved
        let persona_manager =
            PersonaManager::load_personas(&loaded_config).expect("Failed to load personas");
        assert_eq!(
            persona_manager
                .get_default_for_provider_model("openai_gpt-4")
                .unwrap(),
            "alice-dev"
        );
        assert_eq!(
            persona_manager
                .get_default_for_provider_model("anthropic_claude-3-opus")
                .unwrap(),
            "bob-student"
        );
    }

    #[test]
    fn test_end_to_end_persona_workflow() {
        // Test complete end-to-end persona workflow
        // This simulates: config load → CLI selection → conversation → picker change → conversation

        let mut app = create_test_app();

        // Step 1: Load personas from config
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Step 2: Simulate CLI persona selection
        app.persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to set CLI persona");

        // Verify initial state
        assert_eq!(app.persona_manager.get_display_name(), "Alice");
        let initial_prompt = app
            .persona_manager
            .get_modified_system_prompt("You are helpful.");
        assert!(initial_prompt.contains("Alice, a senior software developer"));

        // Step 3: Add user message with persona active
        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_user_message("What's your experience?".to_string());
        }

        // Step 4: Switch persona via picker simulation
        app.persona_manager
            .set_active_persona("bob-student")
            .expect("Failed to switch persona");

        // Verify persona switch
        assert_eq!(app.persona_manager.get_display_name(), "Bob");
        let switched_prompt = app
            .persona_manager
            .get_modified_system_prompt("You are helpful.");
        assert!(switched_prompt.contains("Bob, a computer science student"));
        assert!(!switched_prompt.contains("Alice"));

        // Step 5: Add another message with new persona
        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_user_message("I'm learning AI.".to_string());
        }

        // Step 6: Deactivate persona
        app.persona_manager.clear_active_persona();

        // Verify deactivation
        assert_eq!(app.persona_manager.get_display_name(), "You");
        let final_prompt = app
            .persona_manager
            .get_modified_system_prompt("You are helpful.");
        assert_eq!(final_prompt, "You are helpful.");
    }

    #[test]
    fn test_persona_error_recovery() {
        // Test error handling and recovery in persona workflows

        let mut app = create_test_app();

        // Test with empty persona list
        let empty_config = Config::default();
        app.persona_manager =
            PersonaManager::load_personas(&empty_config).expect("Failed to load empty personas");

        // Should handle empty persona list gracefully
        assert!(app.persona_manager.list_personas().is_empty());
        assert!(app.persona_manager.get_active_persona().is_none());
        assert_eq!(app.persona_manager.get_display_name(), "You");

        // Test /persona command with no personas
        let result = process_input(&mut app, "/persona");
        assert!(matches!(result, CommandResult::OpenPersonaPicker)); // Should not crash

        // Test invalid persona activation
        let result = app.persona_manager.set_active_persona("nonexistent");
        assert!(result.is_err());
        assert!(app.persona_manager.get_active_persona().is_none());

        // Add personas and test recovery
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Should work normally after recovery
        assert!(app.persona_manager.set_active_persona("alice-dev").is_ok());
        assert!(app.persona_manager.get_active_persona().is_some());
    }

    #[test]
    fn test_persona_command_variations() {
        // Test various persona command variations

        let mut app = create_test_app();

        // Set up personas
        let config = create_test_config_with_personas();
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");

        // Test opening picker
        let result = process_input(&mut app, "/persona");
        assert!(matches!(result, CommandResult::OpenPersonaPicker));

        // Test activating each persona
        let result = process_input(&mut app, "/persona alice-dev");
        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(
            app.persona_manager.get_active_persona().unwrap().id,
            "alice-dev"
        );

        let result = process_input(&mut app, "/persona bob-student");
        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(
            app.persona_manager.get_active_persona().unwrap().id,
            "bob-student"
        );

        let result = process_input(&mut app, "/persona charlie-no-bio");
        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(
            app.persona_manager.get_active_persona().unwrap().id,
            "charlie-no-bio"
        );

        // Test error handling for invalid persona
        let result = process_input(&mut app, "/persona invalid-persona");
        assert!(matches!(result, CommandResult::Continue));
        // Should still have the last valid persona active
        assert_eq!(
            app.persona_manager.get_active_persona().unwrap().id,
            "charlie-no-bio"
        );
    }
}
