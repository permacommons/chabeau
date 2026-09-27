use super::*;
use crate::core::config::data::{Config, Persona};
use crate::core::message::{self, Message, TranscriptRole};
use crate::core::persona::PersonaManager;
use crate::utils::test_utils::{
    create_test_app, create_test_message, create_test_message_with_role,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_app_messages_excluded_from_api() {
    let mut app = create_test_app();

    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));

    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_app_message(
            AppMessageKind::Info,
            "This is an app message that should not be sent to API".to_string(),
        );
    }

    app.ui
        .messages
        .push_back(create_test_message("assistant", "Hi there!"));

    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_app_message(AppMessageKind::Warning, "Another app message".to_string());
    }

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("How are you?".to_string())
    };

    assert_eq!(api_messages.len(), 3);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "Hello");
    assert_eq!(api_messages[1].role, "assistant");
    assert_eq!(api_messages[1].content, "Hi there!");
    assert_eq!(api_messages[2].role, "user");
    assert_eq!(api_messages[2].content, "How are you?");

    for msg in &api_messages {
        assert!(!message::is_app_message_role(&msg.role));
    }
}

#[test]
fn test_tool_call_argument_values_truncate_long_strings() {
    let long_value = "a".repeat(120);
    let raw = format!(r#"{{"q":"{long_value}","n":1}}"#);

    let summary = summarize_tool_call_arguments(&raw).expect("summary");
    let expected_q = abbreviate_tool_call_value(&serde_json::to_string(&long_value).expect("json"));

    assert!(summary.contains(&format!("q={expected_q}")));
    assert!(summary.contains("n=1"));
}

#[test]
fn add_user_message_omits_trailing_empty_assistant_turns() {
    let mut app = create_test_app();

    app.ui.messages.push_back(create_test_message_with_role(
        TranscriptRole::User,
        "First attempt",
    ));
    app.ui
        .messages
        .push_back(create_test_message_with_role(TranscriptRole::Assistant, ""));

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Try again?".to_string())
    };

    assert_eq!(api_messages.len(), 2);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "First attempt");
    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "Try again?");
    assert!(api_messages
        .iter()
        .all(|msg| msg.role != "assistant" || !msg.content.trim().is_empty()));

    let mut iter = app.ui.messages.iter().rev();
    let last = iter.next().expect("missing assistant placeholder");
    assert_eq!(last.role, TranscriptRole::Assistant);
    assert!(last.content.is_empty());

    let second_last = iter.next().expect("missing user retry message");
    assert_eq!(second_last.role, TranscriptRole::User);
    assert_eq!(second_last.content, "Try again?");

    assert_eq!(
        app.ui
            .messages
            .iter()
            .filter(|msg| msg.role == TranscriptRole::Assistant && msg.content.is_empty())
            .count(),
        1
    );
}

#[test]
fn test_prepare_retry_excludes_system_messages() {
    let mut app = create_test_app();

    app.ui.messages.push_back(Message {
        role: TranscriptRole::User,
        content: "Test question".to_string(),
    });

    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_app_message(
            AppMessageKind::Info,
            "App message between user and assistant".to_string(),
        );
    }

    app.ui.messages.push_back(Message {
        role: TranscriptRole::Assistant,
        content: "Test response".to_string(),
    });

    app.session.retrying_message_index = Some(2);
    app.session.has_received_assistant_message = true;

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.prepare_retry(10, 80).unwrap()
    };

    assert_eq!(api_messages.len(), 1);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "Test question");

    for msg in &api_messages {
        assert!(!message::is_app_message_role(&msg.role));
    }
}

#[test]
fn test_add_user_message_with_character_active() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character with system prompt and post-history instructions
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful and friendly".to_string(),
            scenario: "Testing scenario".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: "Example dialogue".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: Some("Always be polite.".to_string()),
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

    // Add a previous message
    app.ui
        .messages
        .push_back(create_test_message("user", "Previous message"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Previous response"));

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("New message".to_string())
    };

    // Should have: system prompt, previous user, previous assistant, new user, post-history
    assert_eq!(api_messages.len(), 5);

    // First message should be character system prompt
    assert_eq!(api_messages[0].role, "system");
    assert!(api_messages[0].content.contains("You are TestBot."));
    assert!(api_messages[0].content.contains("Character: TestBot"));

    // Middle messages should be conversation history
    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "Previous message");
    assert_eq!(api_messages[2].role, "assistant");
    assert_eq!(api_messages[2].content, "Previous response");
    assert_eq!(api_messages[3].role, "user");
    assert_eq!(api_messages[3].content, "New message");

    // Last message should be post-history instructions
    assert_eq!(api_messages[4].role, "system");
    assert_eq!(api_messages[4].content, "Always be polite.");
}

#[test]
fn test_persona_bio_char_placeholder_with_active_character() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    let config = Config {
        personas: vec![Persona {
            id: "mentor".to_string(),
            display_name: "Mentor".to_string(),
            bio: Some("Guide {{char}} with wisdom.".to_string()),
        }],
        ..Default::default()
    };

    app.persona_manager = PersonaManager::load_personas(&config).unwrap();
    app.persona_manager
        .set_active_persona("mentor")
        .expect("Failed to activate persona");

    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "Aria".to_string(),
            description: "A skilled musician".to_string(),
            personality: "Creative and calm".to_string(),
            scenario: "Guiding apprentices".to_string(),
            first_mes: "Welcome.".to_string(),
            mes_example: "Example.".to_string(),
            creator_notes: None,
            system_prompt: Some("Stay supportive.".to_string()),
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Hello".to_string())
    };

    assert!(!api_messages.is_empty());
    assert_eq!(api_messages[0].role, "system");
    assert!(
        api_messages[0].content.contains("Guide Aria with wisdom."),
        "System prompt should include character name substitution: {}",
        api_messages[0].content
    );
}

#[test]
fn add_user_message_logs_persona_display_name() {
    let mut app = create_test_app();

    let config = Config {
        personas: vec![Persona {
            id: "captain".to_string(),
            display_name: "Captain".to_string(),
            bio: None,
        }],
        ..Default::default()
    };

    app.persona_manager = PersonaManager::load_personas(&config).unwrap();
    app.persona_manager
        .set_active_persona("captain")
        .expect("Failed to activate persona");

    let temp_dir = tempdir().expect("failed to create temp dir for log");
    let log_path = temp_dir.path().join("conversation.log");
    let log_path_string = log_path.to_string_lossy().into_owned();
    app.session
        .logging
        .set_log_file(log_path_string)
        .expect("failed to enable logging");

    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Hello there".to_string());
    }

    let contents = fs::read_to_string(&log_path).expect("failed to read log file");
    assert!(
        contents.contains("Captain: Hello there"),
        "Log should include persona display name, contents: {contents}"
    );
}

#[test]
fn log_rewrite_excludes_app_messages() {
    let mut app = create_test_app();

    let temp_dir = tempdir().expect("failed to create temp dir for log");
    let log_path = temp_dir.path().join("conversation.log");
    let log_path_string = log_path.to_string_lossy().into_owned();
    app.session
        .logging
        .set_log_file(log_path_string)
        .expect("failed to enable logging");

    // Add messages including an app message
    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Hello".to_string());
    }

    app.ui.messages.push_back(create_test_message_with_role(
        TranscriptRole::Assistant,
        "Hi there!",
    ));

    // Add an app message
    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_app_message(
            AppMessageKind::Warning,
            "This is a warning message".to_string(),
        );
    }

    // Trigger a log rewrite (simulating an edit/retry)
    let user_display_name = app.persona_manager.get_display_name();
    app.session
        .logging
        .rewrite_log_without_last_response(&app.ui.messages, &user_display_name)
        .expect("failed to rewrite log");

    let contents = fs::read_to_string(&log_path).expect("failed to read log file");

    // Verify conversation messages are in the log
    assert!(
        contents.contains("You: Hello"),
        "Log should contain user message, contents: {contents}"
    );
    assert!(
        contents.contains("Hi there!"),
        "Log should contain assistant message, contents: {contents}"
    );

    // Verify app message is NOT in the log
    assert!(
        !contents.contains("This is a warning message"),
        "Log should NOT contain app messages, contents: {contents}"
    );
}

#[test]
fn test_add_user_message_with_character_no_post_history() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character without post-history instructions
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
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

    app.session.set_character(character);

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Test message".to_string())
    };

    // Should have: system prompt, user message (no post-history)
    assert_eq!(api_messages.len(), 2);
    assert_eq!(api_messages[0].role, "system");
    assert!(api_messages[0].content.contains("Character: TestBot"));
    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "Test message");
}

#[test]
fn test_add_user_message_without_character() {
    let mut app = create_test_app();

    // No character set
    assert!(app.session.get_character().is_none());

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Test message".to_string())
    };

    // Should only have the user message, no system messages
    assert_eq!(api_messages.len(), 1);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "Test message");
}

#[test]
fn test_prepare_retry_with_character_active() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: "".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: Some("Be concise.".to_string()),
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

    // Add messages
    app.ui
        .messages
        .push_back(create_test_message("user", "First question"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "First response"));
    app.ui
        .messages
        .push_back(create_test_message("user", "Second question"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Second response to retry"));

    // Set retry index to the last assistant message
    app.session.retrying_message_index = Some(3);
    app.session.has_received_assistant_message = true;

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.prepare_retry(10, 80).unwrap()
    };

    // Should have: system prompt, first user, first assistant, second user, post-history
    assert_eq!(api_messages.len(), 5);

    // First should be character system prompt
    assert_eq!(api_messages[0].role, "system");
    assert!(api_messages[0].content.contains("You are TestBot."));

    // Middle should be conversation history up to retry point
    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "First question");
    assert_eq!(api_messages[2].role, "assistant");
    assert_eq!(api_messages[2].content, "First response");
    assert_eq!(api_messages[3].role, "user");
    assert_eq!(api_messages[3].content, "Second question");

    // Last should be post-history instructions
    assert_eq!(api_messages[4].role, "system");
    assert_eq!(api_messages[4].content, "Be concise.");
}

#[test]
fn test_prepare_retry_without_character() {
    let mut app = create_test_app();

    // No character set
    assert!(app.session.get_character().is_none());

    // Add messages
    app.ui
        .messages
        .push_back(create_test_message("user", "Question"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Response to retry"));

    app.session.retrying_message_index = Some(1);
    app.session.has_received_assistant_message = true;

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.prepare_retry(10, 80).unwrap()
    };

    // Should only have the user message, no system messages
    assert_eq!(api_messages.len(), 1);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "Question");
}

#[test]
fn test_retry_character_greeting_reinserts_locally() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello! I'm TestBot.".to_string(),
            mes_example: "".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

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
    assert_eq!(app.ui.messages[0].role, "assistant");
    assert!(!app.session.has_received_assistant_message);

    app.session.retrying_message_index = Some(0);

    let result = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.prepare_retry(10, 80)
    };

    assert!(result.is_none());
    assert_eq!(app.ui.messages[0].content, "Hello! I'm TestBot.");
    assert!(app.session.retrying_message_index.is_none());
    assert!(!app.session.has_received_assistant_message);
}

#[test]
fn test_retry_character_greeting_updates_after_persona_change() {
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;

    let mut app = create_test_app();

    let config = Config {
        personas: vec![
            Persona {
                id: "first".to_string(),
                display_name: "First".to_string(),
                bio: None,
            },
            Persona {
                id: "second".to_string(),
                display_name: "Second".to_string(),
                bio: None,
            },
        ],
        ..Default::default()
    };
    app.persona_manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    app.persona_manager
        .set_active_persona("first")
        .expect("Failed to activate persona");

    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hi {{user}}! I'm {{char}}.".to_string(),
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

    app.session.set_character(character);

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
    assert_eq!(app.ui.messages[0].role, "assistant");
    assert_eq!(app.ui.messages[0].content, "Hi First! I'm TestBot.");
    assert!(!app.session.has_received_assistant_message);

    app.persona_manager
        .set_active_persona("second")
        .expect("Failed to activate second persona");

    let result = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.prepare_retry(10, 80)
    };

    assert!(result.is_none());
    assert_eq!(app.ui.messages[0].content, "Hi Second! I'm TestBot.");
    assert!(app.session.retrying_message_index.is_none());
    assert!(!app.session.has_received_assistant_message);
}

#[test]
fn test_character_messages_with_transcript_system_messages() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: "".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

    // Add user message
    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));

    // Add transcript app message (should be excluded from API)
    {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_app_message(
            AppMessageKind::Info,
            "Help text displayed in UI".to_string(),
        );
    }

    // Add assistant response
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Hi there!"));

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("How are you?".to_string())
    };

    // Should have: character system prompt, first user, first assistant, new user
    // Transcript app message should be excluded
    assert_eq!(api_messages.len(), 4);

    assert_eq!(api_messages[0].role, "system");
    assert!(api_messages[0].content.contains("You are TestBot."));

    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "Hello");

    assert_eq!(api_messages[2].role, "assistant");
    assert_eq!(api_messages[2].content, "Hi there!");

    assert_eq!(api_messages[3].role, "user");
    assert_eq!(api_messages[3].content, "How are you?");

    // Verify transcript system message is not in API messages
    for msg in &api_messages {
        assert_ne!(msg.content, "Help text displayed in UI");
    }
}

#[test]
fn test_show_character_greeting_if_needed() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character with a greeting
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello! I'm TestBot.".to_string(),
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

    app.session.set_character(character);

    // Initially no messages
    assert_eq!(app.ui.messages.len(), 0);

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

    // Should have added greeting as assistant message
    assert_eq!(app.ui.messages.len(), 1);
    let greeting_msg = app.ui.messages.front().unwrap();
    assert_eq!(greeting_msg.role, "assistant");
    assert_eq!(greeting_msg.content, "Hello! I'm TestBot.");

    // Greeting should be marked as shown
    assert!(app.session.character_greeting_shown);

    // Calling again should not add another greeting
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
}

#[test]
fn test_show_character_greeting_empty_greeting() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character with empty greeting
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "   ".to_string(), // Empty/whitespace greeting
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

    // Should not have added any messages (empty greeting)
    assert_eq!(app.ui.messages.len(), 0);
    assert!(!app.session.character_greeting_shown);
}

#[test]
fn test_show_character_greeting_no_character() {
    let mut app = create_test_app();

    // No character set
    assert!(app.session.get_character().is_none());

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

    // Should not have added any messages
    assert_eq!(app.ui.messages.len(), 0);
    assert!(!app.session.character_greeting_shown);
}

#[test]
fn test_character_greeting_with_persona_substitutions() {
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;

    let mut app = create_test_app();

    // Set up persona
    let config = Config {
        personas: vec![Persona {
            id: "alice-dev".to_string(),
            display_name: "Alice".to_string(),
            bio: Some("You are talking to {{user}}, a senior developer.".to_string()),
        }],
        ..Default::default()
    };
    app.persona_manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    app.persona_manager
        .set_active_persona("alice-dev")
        .expect("Failed to activate persona");

    // Set up character with substitution placeholders in greeting
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello {{user}}! I'm {{char}}, ready to help!".to_string(),
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

    // Verify greeting was added with substitutions applied
    assert_eq!(app.ui.messages.len(), 1);
    let greeting_msg = &app.ui.messages[0];
    assert_eq!(greeting_msg.role, "assistant");
    assert_eq!(
        greeting_msg.content,
        "Hello Alice! I'm TestBot, ready to help!"
    );
    assert!(app.session.character_greeting_shown);
}

#[test]
fn test_persona_system_prompt_integration_with_character() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character with a system prompt
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: "".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Test message".to_string())
    };

    // Should have character system prompt (potentially modified by persona)
    assert_eq!(api_messages.len(), 2);
    assert_eq!(api_messages[0].role, "system");
    // The content should contain the character system prompt
    assert!(api_messages[0].content.contains("You are TestBot."));
    assert_eq!(api_messages[1].role, "user");
    assert_eq!(api_messages[1].content, "Test message");
}

#[test]
fn test_persona_system_prompt_integration_without_character() {
    let mut app = create_test_app();

    // No character set
    assert!(app.session.get_character().is_none());

    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Test message".to_string())
    };

    // Should only have the user message since no persona is active
    // (PersonaManager is created fresh each time with no active persona)
    assert_eq!(api_messages.len(), 1);
    assert_eq!(api_messages[0].role, "user");
    assert_eq!(api_messages[0].content, "Test message");
}

#[test]
fn test_character_greeting_included_in_api_messages() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    // Set up a character with a greeting
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: "A test character".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Greetings!".to_string(),
            mes_example: "".to_string(),
            creator_notes: None,
            system_prompt: Some("You are TestBot.".to_string()),
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

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

    // Add user message
    let api_messages = {
        let mut conversation = ConversationController::new(
            &mut app.session,
            &mut app.ui,
            &app.persona_manager,
            &app.preset_manager,
        );
        conversation.add_user_message("Hello".to_string())
    };

    // Should have: system prompt, greeting (assistant), user message
    assert_eq!(api_messages.len(), 3);

    assert_eq!(api_messages[0].role, "system");
    assert!(api_messages[0].content.contains("You are TestBot."));

    assert_eq!(api_messages[1].role, "assistant");
    assert_eq!(api_messages[1].content, "Greetings!");

    assert_eq!(api_messages[2].role, "user");
    assert_eq!(api_messages[2].content, "Hello");
}

#[test]
fn test_persona_with_blank_bio_does_not_add_system_message() {
    let cases = [
        ("empty", "Empty", ""),
        ("whitespace", "Whitespace", "   \n\t"),
    ];

    for (persona_id, display_name, bio) in cases {
        let config = Config {
            personas: vec![Persona {
                id: persona_id.to_string(),
                display_name: display_name.to_string(),
                bio: Some(bio.to_string()),
            }],
            ..Default::default()
        };

        let persona_manager = PersonaManager::load_personas(&config).unwrap();

        let mut app = create_test_app();
        app.persona_manager = persona_manager;
        app.persona_manager
            .set_active_persona(persona_id)
            .expect("persona activation");

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Hello".to_string())
        };

        assert!(
            api_messages.iter().all(|msg| msg.role != "system"),
            "system message injected for persona {persona_id}"
        );
    }
}
