use super::*;

fn picker_item(id: &str, label: &str, metadata: Option<&str>) -> PickerItem {
    let metadata_string = metadata.map(|value| value.to_string());
    PickerItem {
        id: id.to_string(),
        label: label.to_string(),
        metadata: metadata_string.clone(),
        inspect_metadata: metadata_string,
        sort_key: None,
    }
}

#[test]
fn test_filter_models_resets_selection_and_matches_case_insensitively() {
    let mut controller = PickerController::new();
    let items = vec![
        picker_item("gpt-4", "GPT-4", None),
        picker_item("gpt-3.5", "GPT-3.5", None),
        picker_item("claude-3", "Claude 3", None),
    ];

    let mut picker_state = PickerState::new("Pick Model", items.clone(), 2);
    picker_state.sort_mode = SortMode::Name;

    let mut session = ActivePicker {
        state: picker_state,
        data: PickerData::Model(Box::new(ModelPickerState {
            search_filter: "GPT".to_string(),
            all_items: items,
            before_model: None,
            has_dates: false,
        })),
    };
    session.state.sort_mode = session.default_sort_mode();

    controller.active_picker = Some(session);
    controller.filter_models();

    let session = controller.active().expect("model picker session");
    assert_eq!(session.state.selected, 0);
    assert_eq!(session.state.items.len(), 2);
    let ids: Vec<&str> = session
        .state
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert!(ids.contains(&"gpt-4"));
    assert!(ids.contains(&"gpt-3.5"));
}

#[test]
fn test_filter_characters_preserves_special_entry_and_selection_bounds() {
    let mut controller = PickerController::new();
    let items = vec![
        picker_item(
            TURN_OFF_CHARACTER_ID,
            "[Turn off character mode]",
            Some("Disable character"),
        ),
        picker_item("alice", "Alice", Some("Friendly adventurer")),
        picker_item("gamma", "Gamma", Some("Galactic explorer")),
    ];

    let mut picker_state = PickerState::new("Pick Character", items.clone(), 2);
    picker_state.sort_mode = SortMode::Name;

    let mut session = ActivePicker {
        state: picker_state,
        data: PickerData::Character(CharacterPickerState {
            search_filter: "GAMMA".to_string(),
            all_items: items,
        }),
    };
    session.state.sort_mode = session.default_sort_mode();

    controller.active_picker = Some(session);
    controller.filter_characters();

    let session = controller.active().expect("character picker session");
    assert_eq!(session.state.selected, 0);
    assert_eq!(session.state.items.len(), 2);
    assert_eq!(session.state.items[0].id, TURN_OFF_CHARACTER_ID);
    assert!(session.state.items.iter().any(|item| item.id == "gamma"));
}

#[test]
fn test_filter_personas_preserves_special_entry_and_selection_bounds() {
    let mut controller = PickerController::new();
    let items = vec![
        picker_item(
            TURN_OFF_PERSONA_ID,
            "[Turn off persona]",
            Some("Deactivate persona"),
        ),
        picker_item("mentor", "Mentor", Some("Helpful adviser")),
        picker_item("artist", "Artist", Some("Creative mind")),
    ];

    let mut picker_state = PickerState::new("Pick Persona", items.clone(), 2);
    picker_state.sort_mode = SortMode::Name;

    let mut session = ActivePicker {
        state: picker_state,
        data: PickerData::Persona(PersonaPickerState {
            search_filter: "ADVISER".to_string(),
            all_items: items,
        }),
    };
    session.state.sort_mode = session.default_sort_mode();

    controller.active_picker = Some(session);
    controller.filter_personas();

    let session = controller.active().expect("persona picker session");
    assert_eq!(session.state.selected, 0);
    assert_eq!(session.state.items.len(), 2);
    assert_eq!(session.state.items[0].id, TURN_OFF_PERSONA_ID);
    assert!(session.state.items.iter().any(|item| item.id == "mentor"));
}

#[test]
fn test_filter_presets_preserves_special_entry_and_selection_bounds() {
    let mut controller = PickerController::new();
    let items = vec![
        picker_item(
            TURN_OFF_PRESET_ID,
            "[Turn off preset]",
            Some("Deactivate preset"),
        ),
        picker_item("focus", "Focus Mode", Some("Deep work profile")),
        picker_item("chatty", "Chatty", Some("Casual conversation")),
    ];

    let mut picker_state = PickerState::new("Pick Preset", items.clone(), 2);
    picker_state.sort_mode = SortMode::Name;

    let mut session = ActivePicker {
        state: picker_state,
        data: PickerData::Preset(PresetPickerState {
            search_filter: "FOCUS".to_string(),
            all_items: items,
        }),
    };
    session.state.sort_mode = session.default_sort_mode();

    controller.active_picker = Some(session);
    controller.filter_presets();

    let session = controller.active().expect("preset picker session");
    assert_eq!(session.state.selected, 0);
    assert_eq!(session.state.items.len(), 2);
    assert_eq!(session.state.items[0].id, TURN_OFF_PRESET_ID);
    assert!(session.state.items.iter().any(|item| item.id == "focus"));
}

#[test]
fn test_filter_sessions_updates_rendered_items_from_source_list() {
    let mut controller = PickerController::new();
    let items = vec![
        picker_item("sess-alpha", "Alpha Session", Some("openai | gpt-4")),
        picker_item("sess-beta", "Beta Session", Some("anthropic | claude")),
        picker_item("sess-gamma", "Gamma Session", Some("local | llama")),
    ];

    let mut picker_state = PickerState::new("Load Session", items.clone(), 2);
    picker_state.sort_mode = SortMode::Name;

    let mut session = ActivePicker {
        state: picker_state,
        data: PickerData::SavedSession(SavedSessionPickerState {
            sessions: Vec::new(),
            selected_index: 2,
            search_filter: "claude".to_string(),
            all_items: items,
        }),
    };
    session.state.sort_mode = session.default_sort_mode();

    controller.active_picker = Some(session);
    controller.filter_saved_sessions();

    let session = controller.active().expect("session load picker");
    assert_eq!(session.state.selected, 0);
    assert_eq!(session.state.items.len(), 1);
    assert_eq!(session.state.items[0].id, "sess-beta");

    let session = controller.active_mut().expect("session load picker");
    session
        .saved_session_state_mut()
        .expect("session load state")
        .search_filter
        .clear();

    controller.filter_saved_sessions();

    let session = controller.active().expect("session load picker");
    assert_eq!(session.state.items.len(), 3);
}

#[test]
fn test_picker_data_variant_footprint_is_normalized() {
    use std::mem::size_of;

    let small_inline = size_of::<CharacterPickerState>();
    assert!(size_of::<ModelPickerState>() > small_inline);
    assert!(size_of::<ProviderPickerState>() > small_inline);
    assert!(size_of::<ThemePickerState>() > small_inline);
    // PickerData should still be reasonably compact (enum with large variants)
    assert!(size_of::<PickerData>() < size_of::<ModelPickerState>() + size_of::<usize>());
}

#[test]
fn test_sanitize_picker_metadata_removes_newlines() {
    let input = "Line 1\nLine 2\nLine 3";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Line 1 Line 2 Line 3");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Line 1\nLine 2\nLine 3");
}

#[test]
fn test_sanitize_picker_metadata_removes_carriage_returns() {
    let input = "Line 1\r\nLine 2\r\nLine 3";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Line 1 Line 2 Line 3");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Line 1\nLine 2\nLine 3");
}

#[test]
fn test_sanitize_picker_metadata_collapses_whitespace() {
    let input = "Too    many     spaces";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Too many spaces");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Too    many     spaces");
}

#[test]
fn test_sanitize_picker_metadata_removes_control_chars() {
    let input = "Text\twith\ttabs\x00and\x01control\x02chars";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Text with tabs and control chars");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Textwithtabsandcontrolchars");
}

#[test]
fn test_sanitize_picker_metadata_handles_mixed_whitespace() {
    let input = "Mixed\n\r\t  whitespace\n\n\nhere";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Mixed whitespace here");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Mixed\n  whitespace\n\n\nhere");
}

#[test]
fn test_sanitize_picker_metadata_preserves_normal_text() {
    let input = "Normal text with spaces";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "Normal text with spaces");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "Normal text with spaces");
}

#[test]
fn test_sanitize_picker_metadata_handles_empty_string() {
    let input = "";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "");
}

#[test]
fn test_sanitize_picker_metadata_handles_only_whitespace() {
    let input = "\n\r\t   \n";
    let result = sanitize_picker_metadata(input);
    assert_eq!(result, "");
    let inspect = sanitize_picker_metadata_for_inspect(input);
    assert_eq!(inspect, "");
}

#[test]
fn test_turn_off_character_entry_added_when_character_active() {
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::character::service::CharacterService;
    use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
    use std::fs;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();
    let cards_dir = temp_dir.path().join("cards");
    fs::create_dir_all(&cards_dir).unwrap();

    let card_json = serde_json::json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "TestChar",
            "description": "Test",
            "personality": "Friendly",
            "scenario": "Testing",
            "first_mes": "Hello!",
            "mes_example": ""
        }
    });

    fs::write(cards_dir.join("test.json"), card_json.to_string()).unwrap();

    let mut app = create_test_app();
    let mut service = CharacterService::new();

    app.session.set_character(CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestChar".to_string(),
            description: "Test".to_string(),
            personality: "Friendly".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: String::new(),
            creator_notes: None,
            system_prompt: None,
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    });

    let mut env_guard = TestEnvVarGuard::new();
    env_guard.set_var("CHABEAU_CONFIG_DIR", temp_dir.path().as_os_str());

    let cards = service
        .list_metadata()
        .expect("metadata")
        .into_iter()
        .map(|meta| service.resolve_by_name(&meta.name).expect("card"))
        .collect();
    let result = app.picker.open_character_picker(cards, &app.session);

    assert!(result.is_ok());

    let picker_items = &app.picker.active().unwrap().state.items;
    assert!(picker_items.len() >= 2);
    assert_eq!(picker_items[0].id, TURN_OFF_CHARACTER_ID);
    assert_eq!(picker_items[0].label, "[Turn off character mode]");
}

#[test]
fn test_turn_off_character_entry_not_added_when_no_character() {
    use crate::character::service::CharacterService;
    use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
    use std::fs;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();
    let cards_dir = temp_dir.path().join("cards");
    fs::create_dir_all(&cards_dir).unwrap();

    let card_json = serde_json::json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "TestChar",
            "description": "Test",
            "personality": "Friendly",
            "scenario": "Testing",
            "first_mes": "Hello!",
            "mes_example": ""
        }
    });

    fs::write(cards_dir.join("test.json"), card_json.to_string()).unwrap();

    let mut app = create_test_app();
    let mut service = CharacterService::new();

    assert!(app.session.active_character.is_none());

    let mut env_guard = TestEnvVarGuard::new();
    env_guard.set_var("CHABEAU_CONFIG_DIR", temp_dir.path().as_os_str());

    let cards = service
        .list_metadata()
        .expect("metadata")
        .into_iter()
        .map(|meta| service.resolve_by_name(&meta.name).expect("card"))
        .collect();
    let result = app.picker.open_character_picker(cards, &app.session);

    assert!(result.is_ok());

    let picker_items = &app.picker.active().unwrap().state.items;
    assert!(!picker_items
        .iter()
        .any(|item| item.id == TURN_OFF_CHARACTER_ID));
}

#[test]
fn test_turn_off_character_stays_at_top_after_sort() {
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::character::service::CharacterService;
    use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
    use std::fs;
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();
    let cards_dir = temp_dir.path().join("cards");
    fs::create_dir_all(&cards_dir).unwrap();

    for name in &["Alice", "Bob", "Charlie"] {
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": name,
                "description": "Test",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": ""
            }
        });
        fs::write(
            cards_dir.join(format!("{}.json", name.to_lowercase())),
            card_json.to_string(),
        )
        .unwrap();
    }

    let mut app = create_test_app();
    let mut service = CharacterService::new();

    app.session.set_character(CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "Alice".to_string(),
            description: "Test".to_string(),
            personality: "Friendly".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: String::new(),
            creator_notes: None,
            system_prompt: None,
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    });

    let mut env_guard = TestEnvVarGuard::new();
    env_guard.set_var("CHABEAU_CONFIG_DIR", temp_dir.path().as_os_str());

    let cards = service
        .list_metadata()
        .expect("metadata")
        .into_iter()
        .map(|meta| service.resolve_by_name(&meta.name).expect("card"))
        .collect();
    let result = app.picker.open_character_picker(cards, &app.session);

    assert!(result.is_ok());

    let items_before_sort = app.picker.active().unwrap().state.items.len();
    assert!(items_before_sort >= 4); // turn off + 3 characters

    app.picker.sort_items();

    let picker_items = &app.picker.active().unwrap().state.items;

    // First item should always be the turn off entry, regardless of sort
    assert_eq!(picker_items[0].id, TURN_OFF_CHARACTER_ID);
    assert_eq!(picker_items[0].label, "[Turn off character mode]");

    // Verify we still have all items after sorting
    assert_eq!(picker_items.len(), items_before_sort);
}

#[test]
fn test_turn_off_persona_stays_at_top_after_sort() {
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::create_test_app;

    let config = Config {
        personas: vec![
            Persona {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                bio: Some("Alpha bio".to_string()),
            },
            Persona {
                id: "beta".to_string(),
                display_name: "Beta".to_string(),
                bio: Some("Beta bio".to_string()),
            },
            Persona {
                id: "gamma".to_string(),
                display_name: "Gamma".to_string(),
                bio: Some("Gamma bio".to_string()),
            },
        ],
        ..Default::default()
    };

    let mut persona_manager = PersonaManager::load_personas(&config).unwrap();
    persona_manager.set_active_persona("alpha").unwrap();

    let mut app = create_test_app();
    app.persona_manager = persona_manager;

    let result = app
        .picker
        .open_persona_picker(&app.persona_manager, &app.session);

    assert!(result.is_ok());

    let items_before_sort = app.picker.active().unwrap().state.items.len();
    assert!(items_before_sort >= 2);

    app.picker.sort_items();

    let picker_items = &app.picker.active().unwrap().state.items;

    // First item should always be the turn off persona entry, regardless of sort
    assert_eq!(picker_items[0].id, TURN_OFF_PERSONA_ID);
    assert_eq!(picker_items[0].label, "[Turn off persona]");

    // Verify we still have all items after sorting
    assert_eq!(picker_items.len(), items_before_sort);
}

#[test]
fn test_character_picker_highlights_active_character() {
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::utils::test_utils::create_test_app;

    let mut app = create_test_app();
    let active_card = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "Beta".to_string(),
            description: "Test character Beta".to_string(),
            personality: "Helpful".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello".to_string(),
            mes_example: "{{user}}: Hi\n{{char}}: Hello".to_string(),
            creator_notes: None,
            system_prompt: None,
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(active_card);

    let cards = vec![
        CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alpha".to_string(),
                description: "Alpha description".to_string(),
                personality: "Curious".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello".to_string(),
                mes_example: "{{user}}: Hi\n{{char}}: Hello".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        },
        CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Beta".to_string(),
                description: "Beta description".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello".to_string(),
                mes_example: "{{user}}: Hi\n{{char}}: Hello".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        },
    ];

    app.picker
        .open_character_picker(cards, &app.session)
        .unwrap();

    let session = app.picker.active().expect("character picker session");
    let selected_item = &session.state.items[session.state.selected];
    assert_eq!(selected_item.id, "Beta");
}

#[test]
fn test_persona_picker_highlights_active_persona() {
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::create_test_app;

    let config = Config {
        personas: vec![
            Persona {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                bio: Some("Alpha bio".to_string()),
            },
            Persona {
                id: "beta".to_string(),
                display_name: "Beta".to_string(),
                bio: Some("Beta bio".to_string()),
            },
        ],
        ..Default::default()
    };

    let mut persona_manager = PersonaManager::load_personas(&config).unwrap();
    persona_manager.set_active_persona("beta").unwrap();

    let mut app = create_test_app();
    app.persona_manager = persona_manager;
    app.persona_manager
        .set_active_persona("beta")
        .expect("active persona available");

    app.picker
        .open_persona_picker(&app.persona_manager, &app.session)
        .unwrap();

    let session = app.picker.active().expect("persona picker session");
    let selected_item = &session.state.items[session.state.selected];
    assert_eq!(selected_item.id, "beta");
}

#[test]
fn test_preset_picker_highlights_active_preset() {
    use crate::core::config::data::{Config, Preset};
    use crate::core::preset::PresetManager;
    use crate::utils::test_utils::create_test_app;

    let config = Config {
        builtin_presets: Some(false),
        presets: vec![
            Preset {
                id: "focus".to_string(),
                pre: "Focus".to_string(),
                post: String::new(),
            },
            Preset {
                id: "casual".to_string(),
                pre: "Casual".to_string(),
                post: String::new(),
            },
        ],
        ..Default::default()
    };

    let mut preset_manager = PresetManager::load_presets(&config).unwrap();
    preset_manager.set_active_preset("casual").unwrap();

    let mut app = create_test_app();
    app.preset_manager = preset_manager;
    app.preset_manager
        .set_active_preset("casual")
        .expect("active preset available");

    app.picker
        .open_preset_picker(&app.preset_manager, &app.session)
        .unwrap();

    let session = app.picker.active().expect("preset picker session");
    let selected_item = &session.state.items[session.state.selected];
    assert_eq!(selected_item.id, "casual");
}

#[test]
fn test_persona_picker_sanitizes_bio_metadata() {
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::create_test_app;

    let config = Config {
        personas: vec![Persona {
            id: "neat".to_string(),
            display_name: "Neat".to_string(),
            bio: Some("First line\nSecond\tline".to_string()),
        }],
        ..Default::default()
    };

    let persona_manager = PersonaManager::load_personas(&config).unwrap();

    let mut app = create_test_app();
    app.persona_manager = persona_manager;

    app.picker
        .open_persona_picker(&app.persona_manager, &app.session)
        .unwrap();

    let picker_items = &app.picker.active().unwrap().state.items;
    let persona_item = picker_items
        .iter()
        .find(|item| item.id == "neat")
        .expect("persona entry present");

    assert_eq!(
        persona_item.metadata.as_deref(),
        Some("First line Second line")
    );
}

#[test]
fn test_persona_picker_metadata_defaults_to_no_bio_when_empty() {
    use crate::core::config::data::{Config, Persona};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::create_test_app;

    let config = Config {
        personas: vec![Persona {
            id: "blank".to_string(),
            display_name: "Blank".to_string(),
            bio: Some("   \n\t".to_string()),
        }],
        ..Default::default()
    };

    let persona_manager = PersonaManager::load_personas(&config).unwrap();

    let mut app = create_test_app();
    app.persona_manager = persona_manager;

    app.picker
        .open_persona_picker(&app.persona_manager, &app.session)
        .unwrap();

    let picker_items = &app.picker.active().unwrap().state.items;
    let persona_item = picker_items
        .iter()
        .find(|item| item.id == "blank")
        .expect("persona entry present");

    assert_eq!(persona_item.metadata.as_deref(), Some("No bio"));
}
