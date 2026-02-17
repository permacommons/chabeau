use super::*;
use crate::character::card::{CharacterCard, CharacterData};
use crate::core::config::data::{Config, McpServerConfig, Persona};
use crate::core::message::TranscriptRole;
use crate::core::persona::PersonaManager;
use crate::utils::test_utils::{
    create_test_app, create_test_message, create_test_message_with_role, with_test_config_env,
};
use rust_mcp_schema::{
    Implementation, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PromptArgument, ServerCapabilities,
};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use tempfile::tempdir;
use toml::Value;

mod test_helpers {
    use super::*;

    pub(super) fn read_config(path: &Path) -> Value {
        let contents = std::fs::read_to_string(path).unwrap();
        toml::from_str(&contents).unwrap()
    }
}

use test_helpers::read_config;

#[test]
fn clear_command_resets_transcript_state() {
    let mut app = create_test_app();
    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Hi there!"));
    app.ui.current_response = "partial".to_string();
    app.session.retrying_message_index = Some(1);
    app.session.is_refining = true;
    app.session.original_refining_content = Some("original".to_string());
    app.session.last_refine_prompt = Some("prompt".to_string());
    app.session.has_received_assistant_message = true;
    app.session.character_greeting_shown = true;

    app.get_prewrapped_lines_cached(80);
    assert!(app.ui.prewrap_cache.is_some());

    let result = process_input(&mut app, "/clear");
    assert!(matches!(result, CommandResult::Continue));
    assert!(app.ui.messages.is_empty());
    assert!(app.ui.current_response.is_empty());
    assert_eq!(app.ui.status.as_deref(), Some("Transcript cleared"));
    assert!(app.ui.prewrap_cache.is_none());
    assert!(app.session.retrying_message_index.is_none());
    assert!(!app.session.is_refining);
    assert!(app.session.original_refining_content.is_none());
    assert!(app.session.last_refine_prompt.is_none());
    assert!(!app.session.has_received_assistant_message);
    assert!(!app.session.character_greeting_shown);
}

#[test]
fn clear_command_shows_character_greeting_when_available() {
    let mut app = create_test_app();
    let greeting_text = "Greetings from TestBot!".to_string();
    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestBot".to_string(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: greeting_text.clone(),
            mes_example: String::new(),
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
    app.session.character_greeting_shown = true;
    app.session.has_received_assistant_message = true;
    app.ui.messages.push_back(create_test_message_with_role(
        TranscriptRole::Assistant,
        &greeting_text,
    ));
    app.ui
        .messages
        .push_back(create_test_message("user", "Hi!"));

    let result = process_input(&mut app, "/clear");
    assert!(matches!(result, CommandResult::Continue));
    assert_eq!(app.ui.status.as_deref(), Some("Transcript cleared"));
    assert_eq!(app.ui.messages.len(), 1);
    let greeting = app.ui.messages.front().unwrap();
    assert_eq!(greeting.role, TranscriptRole::Assistant);
    assert_eq!(greeting.content, greeting_text);
    assert!(app.session.character_greeting_shown);
    assert!(!app.session.has_received_assistant_message);
}

#[test]
fn registry_lists_commands() {
    let commands = super::all_commands();
    assert!(commands.iter().any(|cmd| cmd.name == "help"));
    assert!(commands.iter().any(|cmd| cmd.name == "markdown"));
    assert!(super::registry::find_command("help").is_some());
}

#[test]
fn help_command_includes_registry_metadata() {
    let mut app = create_test_app();
    let result = process_input(&mut app, "/help");
    assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
    let last_message = app.ui.messages.back().expect("help message");
    assert!(last_message
        .content
        .contains("- `/help` — Show available commands"));
}

#[test]
fn commands_dispatch_case_insensitively() {
    with_test_config_env(|_| {
        let mut app = create_test_app();
        app.ui.markdown_enabled = false;
        let result = process_input(&mut app, "/MarkDown On");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app.ui.markdown_enabled);
    });
}

#[test]
fn dispatch_provides_multi_word_arguments() {
    use super::registry::DispatchOutcome;

    let registry = super::registry::registry();
    match registry.dispatch("/character Jean Luc Picard") {
        DispatchOutcome::Invocation(invocation) => {
            assert_eq!(invocation.command.name, "character");
            assert_eq!(invocation.args_text(), "Jean Luc Picard");
            let args: Vec<_> = invocation.args_iter().collect();
            assert_eq!(args, vec!["Jean", "Luc", "Picard"]);
            assert_eq!(invocation.arg(1), Some("Luc"));
        }
        other => panic!("unexpected dispatch outcome: {:?}", other),
    }
}

#[test]
fn dispatch_reports_unknown_commands() {
    use super::registry::DispatchOutcome;

    let registry = super::registry::registry();
    assert!(matches!(
        registry.dispatch("/does-not-exist"),
        DispatchOutcome::UnknownCommand
    ));
}

#[test]
fn markdown_command_rejects_invalid_argument() {
    with_test_config_env(|_| {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/markdown banana");
        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(
            app.ui.status.as_deref(),
            Some("Usage: /markdown [on|off|toggle]")
        );
    });
}

#[test]
fn test_dump_conversation() {
    // Create a mock app with some messages
    let mut app = create_test_app();

    // Add messages
    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Hi there!"));
    app.ui.messages.push_back(create_test_message_with_role(
        crate::core::message::TranscriptRole::AppInfo,
        "App message",
    ));

    // Create a temporary directory for testing
    let temp_dir = tempdir().unwrap();
    let dump_file_path = temp_dir.path().join("test_dump.txt");

    // Test the dump_conversation function
    assert!(
        crate::commands::handlers::io::dump_conversation_with_overwrite(
            &app,
            dump_file_path.to_str().unwrap(),
            false
        )
        .is_ok()
    );

    // Read the dumped file and verify its contents
    let mut file = File::open(&dump_file_path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    // Check that the contents match what we expect
    assert!(contents.contains("You: Hello"));
    assert!(contents.contains("Hi there!"));
    // App messages should be excluded from dumps
    assert!(!contents.contains("App message"));

    // Clean up
    drop(file);
    fs::remove_file(&dump_file_path).unwrap();
}

#[test]
fn dump_conversation_uses_persona_display_name() {
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

    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));

    let temp_dir = tempdir().unwrap();
    let dump_file_path = temp_dir.path().join("persona_dump.txt");
    dump_conversation_with_overwrite(&app, dump_file_path.to_str().unwrap(), true)
        .expect("failed to dump conversation");

    let contents = fs::read_to_string(&dump_file_path).expect("failed to read dump file");
    assert!(
        contents.contains("Captain: Hello"),
        "Dump should include persona display name, contents: {contents}"
    );
}

#[test]
fn markdown_command_updates_state_and_persists() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut app = create_test_app();
        app.ui.markdown_enabled = true;

        let result = process_input(&mut app, "/markdown off");
        assert!(matches!(result, CommandResult::Continue));
        assert!(!app.ui.markdown_enabled);
        assert_eq!(app.ui.status.as_deref(), Some("Markdown disabled"));

        assert!(config_path.exists());
        let config = read_config(&config_path);
        assert_eq!(config["markdown"].as_bool(), Some(false));

        let result = process_input(&mut app, "/markdown toggle");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app.ui.markdown_enabled);
        assert_eq!(app.ui.status.as_deref(), Some("Markdown enabled"));

        let config = read_config(&config_path);
        assert_eq!(config["markdown"].as_bool(), Some(true));
    });
}

#[test]
fn syntax_command_updates_state_and_persists() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut app = create_test_app();
        app.ui.syntax_enabled = true;

        let result = process_input(&mut app, "/syntax off");
        assert!(matches!(result, CommandResult::Continue));
        assert!(!app.ui.syntax_enabled);
        assert_eq!(app.ui.status.as_deref(), Some("Syntax off"));

        assert!(config_path.exists());
        let config = read_config(&config_path);
        assert_eq!(config["syntax"].as_bool(), Some(false));

        let result = process_input(&mut app, "/syntax toggle");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app.ui.syntax_enabled);
        assert_eq!(app.ui.status.as_deref(), Some("Syntax on"));

        let config = read_config(&config_path);
        assert_eq!(config["syntax"].as_bool(), Some(true));
    });
}

#[test]
fn test_dump_conversation_file_exists() {
    // Create a mock app with some messages
    let mut app = create_test_app();

    // Add messages
    app.ui
        .messages
        .push_back(create_test_message("user", "Hello"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "Hi there!"));

    // Create a temporary directory for testing
    let temp_dir = tempdir().unwrap();
    let dump_file_path = temp_dir.path().join("test_dump.txt");
    let dump_filename = dump_file_path.to_str().unwrap();

    // Create a file that already exists
    fs::write(&dump_file_path, "existing content").unwrap();

    // Test the dump_conversation function with existing file
    // This should fail because the file already exists
    let result =
        crate::commands::handlers::io::dump_conversation_with_overwrite(&app, dump_filename, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));

    // Check that the existing file content is still there
    let contents = fs::read_to_string(&dump_file_path).unwrap();
    assert_eq!(contents, "existing content");

    // Clean up
    fs::remove_file(&dump_file_path).unwrap();
}

#[test]
fn test_process_input_dump_with_filename() {
    let mut app = create_test_app();

    // Add a message to test dumping
    app.ui
        .messages
        .push_back(create_test_message("user", "Test message"));

    // Create a temporary directory for testing
    let temp_dir = tempdir().unwrap();
    let dump_file_path = temp_dir.path().join("custom_dump.txt");
    let dump_filename = dump_file_path.to_str().unwrap();

    // Process the /dump command
    let result = process_input(&mut app, &format!("/dump {}", dump_filename));

    // Should continue (not process as message)
    assert!(matches!(result, CommandResult::Continue));

    // Should set a status about the dump
    assert!(app.ui.status.is_some());
    assert!(app.ui.status.as_ref().unwrap().starts_with("Dumped: "));

    // Clean up
    fs::remove_file(dump_filename).ok();
}

#[test]
fn test_process_input_dump_empty_conversation() {
    let mut app = create_test_app();

    // Create a temporary directory for testing
    let temp_dir = tempdir().unwrap();
    let dump_file_path = temp_dir.path().join("empty_dump.txt");
    let dump_filename = dump_file_path.to_str().unwrap();

    // Process the /dump command with an empty conversation
    let result = process_input(&mut app, &format!("/dump {}", dump_filename));

    // Should continue (not process as message)
    assert!(matches!(result, CommandResult::Continue));

    // Should set a status with an error
    assert!(app.ui.status.is_some());
    assert!(app.ui.status.as_ref().unwrap().starts_with("Dump error:"));
}

#[test]
fn theme_command_opens_picker() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/theme");
    assert!(matches!(res, CommandResult::OpenThemePicker));
    assert!(app.picker_session().is_none());
}

#[test]
fn model_command_returns_open_picker_result() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/model");
    assert!(matches!(res, CommandResult::OpenModelPicker));
}

#[test]
fn model_command_with_id_sets_model() {
    let mut app = create_test_app();
    let original_model = app.session.model.clone();
    let res = process_input(&mut app, "/model gpt-4");
    assert!(matches!(res, CommandResult::Continue));
    assert_eq!(app.session.model, "gpt-4");
    assert_ne!(app.session.model, original_model);
}

#[test]
fn provider_command_with_same_id_reuses_session() {
    let mut app = create_test_app();
    app.picker.provider_model_transition_state = Some((
        "prev-provider".into(),
        "Prev".into(),
        "prev-model".into(),
        "prev-key".into(),
        "https://prev.example".into(),
    ));
    app.picker.in_provider_model_transition = false;

    let result = process_input(&mut app, "/provider TEST");

    assert!(matches!(result, CommandResult::Continue));
    assert_eq!(app.session.provider_name, "test");
    assert_eq!(app.session.api_key, "test-key");
    assert_eq!(app.ui.status.as_deref(), Some("Provider set: TEST"));
    assert!(!app.picker.in_provider_model_transition);
    assert!(app.picker.provider_model_transition_state.is_none());
}

#[test]
fn theme_picker_supports_filtering() {
    let mut app = create_test_app();
    app.open_theme_picker().expect("theme picker opens");

    // Should store all themes for filtering
    assert!(app
        .theme_picker_state()
        .map(|state| !state.all_items.is_empty())
        .unwrap_or(false));

    // Should start with empty filter
    assert!(app
        .theme_picker_state()
        .map(|state| state.search_filter.is_empty())
        .unwrap_or(true));

    // Add a filter and verify filtering works
    if let Some(state) = app.theme_picker_state_mut() {
        state.search_filter.push_str("dark");
    }
    app.filter_themes();

    if let Some(picker) = app.picker_state() {
        // Should have filtered results
        let total = app
            .theme_picker_state()
            .map(|state| state.all_items.len())
            .unwrap_or(0);
        assert!(picker.items.len() <= total);
        // Title should show filter status
        assert!(picker.title.contains("filter: 'dark'"));
    }
}

#[test]
fn picker_supports_home_end_navigation_and_metadata() {
    let mut app = create_test_app();
    app.open_theme_picker().expect("theme picker opens");

    if let Some(picker) = app.picker_state_mut() {
        // Test Home key (move to start)
        picker.selected = picker.items.len() - 1; // Move to last
        picker.move_to_start();
        assert_eq!(picker.selected, 0);

        // Test End key (move to end)
        picker.move_to_end();
        assert_eq!(picker.selected, picker.items.len() - 1);

        // Test metadata is available
        let metadata = picker.get_selected_metadata();
        assert!(metadata.is_some());

        // Test sort mode cycling
        let original_sort = picker.sort_mode.clone();
        picker.cycle_sort_mode();
        assert_ne!(picker.sort_mode, original_sort);

        // Test items have metadata
        assert!(picker.items.iter().any(|item| item.metadata.is_some()));
    }
}

#[test]
fn theme_picker_shows_a_z_sort_indicators() {
    let mut app = create_test_app();

    // Open theme picker - should default to A-Z (Name mode)
    app.open_theme_picker().expect("theme picker opens");

    if let Some(picker) = app.picker_state() {
        // Should default to Name mode (A-Z)
        assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Name);
        // Title should show "Sort by: A-Z"
        assert!(
            picker.title.contains("Sort by: A-Z"),
            "Theme picker should show 'Sort by: A-Z', got: {}",
            picker.title
        );
    }

    // Cycle to Z-A mode
    if let Some(picker) = app.picker_state_mut() {
        picker.cycle_sort_mode();
    }
    app.sort_picker_items();
    app.update_picker_title();

    if let Some(picker) = app.picker_state() {
        // Should now be in Date mode (Z-A for themes)
        assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Date);
        // Title should show "Sort by: Z-A"
        assert!(
            picker.title.contains("Sort by: Z-A"),
            "Theme picker should show 'Sort by: Z-A', got: {}",
            picker.title
        );
    }
}

#[test]
fn character_command_opens_picker() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/character");
    assert!(matches!(res, CommandResult::OpenCharacterPicker));
}

#[test]
fn character_command_with_invalid_name_shows_error() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/character nonexistent_character");
    assert!(matches!(res, CommandResult::Continue));
    assert!(app.ui.status.is_some());
    let status = app.ui.status.as_ref().unwrap();
    assert!(
        status.contains("Character error") || status.contains("not found"),
        "Expected error message, got: {}",
        status
    );
}

#[test]
fn character_command_registered_in_help() {
    let commands = super::all_commands();
    assert!(commands.iter().any(|cmd| cmd.name == "character"));

    let character_cmd = commands.iter().find(|cmd| cmd.name == "character").unwrap();
    assert_eq!(character_cmd.usages.len(), 2);
    assert!(character_cmd.usages[0].syntax.contains("/character"));
    assert!(character_cmd.usages[1].syntax.contains("<name>"));
}

#[test]
fn persona_command_opens_picker() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/persona");
    assert!(matches!(res, CommandResult::OpenPersonaPicker));
}

#[test]
fn persona_command_with_invalid_id_shows_error() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/persona nonexistent_persona");
    assert!(matches!(res, CommandResult::Continue));
    assert!(app.ui.status.is_some());
    let status = app.ui.status.as_ref().unwrap();
    assert!(
        status.contains("Persona error") || status.contains("not found"),
        "Expected error message, got: {}",
        status
    );
}

#[test]
fn persona_command_with_valid_id_updates_user_display_name() {
    let mut app = create_test_app();
    let mut config = crate::core::config::data::Config::default();
    config.personas.push(crate::core::config::data::Persona {
        id: "alice-dev".to_string(),
        display_name: "Alice".to_string(),
        bio: Some("A senior software developer".to_string()),
    });
    app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config).unwrap();
    assert_eq!(app.ui.user_display_name, "You");

    let res = process_input(&mut app, "/persona alice-dev");

    assert!(matches!(res, CommandResult::Continue));
    assert_eq!(app.ui.user_display_name, "Alice");
}

#[test]
fn mcp_command_lists_empty_config() {
    let mut app = create_test_app();
    let res = process_input(&mut app, "/mcp");
    assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
    let last = app.ui.messages.back().expect("app message");
    assert!(last.content.contains("MCP servers"));
    assert!(last.content.contains("No MCP servers configured"));
}

#[test]
fn mcp_command_highlights_disabled_state() {
    let mut app = create_test_app();
    app.session.mcp_disabled = true;
    let res = process_input(&mut app, "/mcp");
    assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
    let last = app.ui.messages.back().expect("app message");
    assert!(last.content.contains("MCP: **disabled for this session**"));
}

#[test]
fn mcp_command_highlights_yolo_servers() {
    let mut app = create_test_app();
    app.config.mcp_servers.push(McpServerConfig {
        id: "alpha".to_string(),
        display_name: "Alpha".to_string(),
        base_url: Some("https://mcp.example.com".to_string()),
        command: None,
        args: None,
        env: None,
        headers: None,
        transport: Some("streamable-http".to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(true),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: Some(true),
    });
    app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

    let res = process_input(&mut app, "/mcp");
    assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
    let last = app.ui.messages.back().expect("app message");
    assert!(last.content.contains("**YOLO**"));
}

#[test]
fn mcp_command_highlights_disabled_servers() {
    let mut app = create_test_app();
    app.config.mcp_servers.push(McpServerConfig {
        id: "alpha".to_string(),
        display_name: "Alpha".to_string(),
        base_url: Some("https://mcp.example.com".to_string()),
        command: None,
        args: None,
        env: None,
        headers: None,
        transport: Some("streamable-http".to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(false),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: None,
    });
    app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

    let res = process_input(&mut app, "/mcp");
    assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
    let last = app.ui.messages.back().expect("app message");
    assert!(last.content.contains("**disabled**"));
}

#[test]
fn mcp_command_skips_refresh_for_disabled_server() {
    let mut app = create_test_app();
    app.config
        .mcp_servers
        .push(crate::core::config::data::McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(false),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
    app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

    let res = process_input(&mut app, "/mcp alpha");
    assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
    let last = app.ui.messages.back().expect("app message");
    assert!(last.content.contains("MCP: **disabled**"));
}

#[test]
fn mcp_command_includes_allowed_tools() {
    let mut app = create_test_app();
    app.config
        .mcp_servers
        .push(crate::core::config::data::McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: Some(vec!["weather.lookup".to_string(), "time.now".to_string()]),
            protocol_version: Some("2024-11-05".to_string()),
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
    app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

    let res = process_input(&mut app, "/mcp alpha");
    assert!(matches!(
        res,
        CommandResult::RefreshMcp {
            server_id: ref id
        } if id == "alpha"
    ));
    assert_eq!(app.ui.status.as_deref(), Some("Refreshing MCP data..."));
    assert_eq!(
        app.ui.activity_indicator,
        Some(crate::core::app::ActivityKind::McpRefresh)
    );
}

#[test]
fn yolo_command_shows_and_persists() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut config = Config::default();
        config.mcp_servers.push(McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        config.save().expect("save config");

        let mut app = create_test_app();
        app.config = config.clone();
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let result = process_input(&mut app, "/yolo alpha");
        assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
        let last = app.ui.messages.back().expect("app message");
        assert!(last.content.contains("YOLO: disabled"));

        let result = process_input(&mut app, "/yolo alpha on");
        assert!(matches!(result, CommandResult::Continue));
        let status = app.ui.status.as_deref().unwrap_or_default();
        assert!(status.contains("YOLO enabled"));
        assert!(status.contains("saved to config.toml"));

        let config = read_config(&config_path);
        let yolo = config
            .get("mcp_servers")
            .and_then(|servers| servers.as_array())
            .and_then(|servers| servers.first())
            .and_then(|server| server.get("yolo"))
            .and_then(|value| value.as_bool());
        assert_eq!(yolo, Some(true));
    });
}

#[test]
fn mcp_command_toggle_enabled_persists() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut config = Config::default();
        config.mcp_servers.push(McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        config.save().expect("save config");

        let mut app = create_test_app();
        app.config = config.clone();
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let result = process_input(&mut app, "/mcp alpha off");
        assert!(matches!(result, CommandResult::Continue));
        let status = app.ui.status.as_deref().unwrap_or_default();
        assert!(status.contains("MCP disabled"));
        assert!(status.contains("saved to config.toml"));

        let config = read_config(&config_path);
        let enabled = config
            .get("mcp_servers")
            .and_then(|servers| servers.as_array())
            .and_then(|servers| servers.first())
            .and_then(|server| server.get("enabled"))
            .and_then(|value| value.as_bool());
        assert_eq!(enabled, Some(false));
    });
}

#[test]
fn mcp_command_toggle_on_triggers_refresh() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut config = Config::default();
        config.mcp_servers.push(McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(false),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        config.save().expect("save config");

        let mut app = create_test_app();
        app.config = config.clone();
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let result = process_input(&mut app, "/mcp alpha on");
        assert!(matches!(
            result,
            CommandResult::RefreshMcp {
                server_id: ref id
            } if id == "alpha"
        ));
        assert!(app
            .ui
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("Refreshing MCP data for alpha"));
        assert_eq!(
            app.ui.activity_indicator,
            Some(crate::core::app::ActivityKind::McpRefresh)
        );

        let config = read_config(&config_path);
        let enabled = config
            .get("mcp_servers")
            .and_then(|servers| servers.as_array())
            .and_then(|servers| servers.first())
            .and_then(|server| server.get("enabled"))
            .and_then(|value| value.as_bool());
        assert_eq!(enabled, Some(true));
    });
}

#[test]
fn mcp_command_toggle_off_clears_runtime_state() {
    let mut app = create_test_app();
    app.config.mcp_servers.push(McpServerConfig {
        id: "alpha".to_string(),
        display_name: "Alpha".to_string(),
        base_url: Some("https://mcp.example.com".to_string()),
        command: None,
        args: None,
        env: None,
        headers: None,
        transport: Some("streamable-http".to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(true),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: None,
    });
    app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

    if let Some(server) = app.mcp.server_mut("alpha") {
        server.connected = true;
        server.last_error = Some("boom".to_string());
        server.cached_tools = Some(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: Vec::new(),
        });
        server.cached_resources = Some(ListResourcesResult {
            meta: None,
            next_cursor: None,
            resources: Vec::new(),
        });
        server.cached_resource_templates = Some(ListResourceTemplatesResult {
            meta: None,
            next_cursor: None,
            resource_templates: Vec::new(),
        });
        server.cached_prompts = Some(ListPromptsResult {
            meta: None,
            next_cursor: None,
            prompts: Vec::new(),
        });
        server.session_id = Some("session".to_string());
        server.auth_header = Some("Bearer token".to_string());
        server.server_details = Some(InitializeResult {
            capabilities: ServerCapabilities::default(),
            instructions: None,
            meta: None,
            protocol_version: "2025-11-25".to_string(),
            server_info: Implementation {
                name: "server".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                description: None,
                icons: Vec::new(),
                website_url: None,
            },
        });
        server.streamable_http_request_id = 5;
        server.event_listener_started = true;
    } else {
        panic!("missing MCP server state");
    }

    let result = process_input(&mut app, "/mcp alpha off");
    assert!(matches!(result, CommandResult::Continue));

    let server = app.mcp.server("alpha").expect("missing MCP server");
    assert!(!server.connected);
    assert!(server.last_error.is_none());
    assert!(server.cached_tools.is_none());
    assert!(server.cached_resources.is_none());
    assert!(server.cached_resource_templates.is_none());
    assert!(server.cached_prompts.is_none());
    assert!(server.session_id.is_none());
    assert!(server.auth_header.is_none());
    assert!(server.server_details.is_none());
    assert_eq!(server.streamable_http_request_id, 0);
    assert!(!server.event_listener_started);
}

#[test]
fn mcp_command_forget_clears_permissions_and_history() {
    with_test_config_env(|config_root| {
        let config_path = config_root.join("chabeau").join("config.toml");
        let mut config = Config::default();
        config.mcp_servers.push(McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        config.mcp_servers.push(McpServerConfig {
            id: "beta".to_string(),
            display_name: "Beta".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        config.save().expect("save config");

        let mut app = create_test_app();
        app.config = config.clone();
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        app.mcp_permissions.record(
            "alpha",
            "tool-a",
            crate::mcp::permissions::ToolPermissionDecision::Block,
        );
        app.mcp_permissions.record(
            "beta",
            "tool-b",
            crate::mcp::permissions::ToolPermissionDecision::Block,
        );

        app.session.tool_pipeline.tool_result_history.push(
            crate::core::app::session::ToolResultRecord {
                tool_name: "tool-a".to_string(),
                server_name: Some("Alpha".to_string()),
                server_id: Some("alpha".to_string()),
                status: crate::core::app::session::ToolResultStatus::Success,
                failure_kind: None,
                content: "ok".to_string(),
                summary: "ok".to_string(),
                tool_call_id: None,
                raw_arguments: None,
                assistant_message_index: None,
            },
        );
        app.session.tool_pipeline.tool_result_history.push(
            crate::core::app::session::ToolResultRecord {
                tool_name: "tool-b".to_string(),
                server_name: Some("Beta".to_string()),
                server_id: Some("beta".to_string()),
                status: crate::core::app::session::ToolResultStatus::Success,
                failure_kind: None,
                content: "ok".to_string(),
                summary: "ok".to_string(),
                tool_call_id: None,
                raw_arguments: None,
                assistant_message_index: None,
            },
        );
        app.session.tool_pipeline.tool_payload_history.push(
            crate::core::app::session::ToolPayloadHistoryEntry {
                server_id: Some("alpha".to_string()),
                tool_call_id: Some("1".to_string()),
                assistant_message: crate::api::ChatMessage {
                    role: "assistant".to_string(),
                    content: "call".to_string(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                tool_message: crate::api::ChatMessage {
                    role: "tool".to_string(),
                    content: "result".to_string(),
                    name: None,
                    tool_call_id: Some("1".to_string()),
                    tool_calls: None,
                },
                assistant_message_index: None,
            },
        );
        app.session.tool_pipeline.tool_payload_history.push(
            crate::core::app::session::ToolPayloadHistoryEntry {
                server_id: Some("beta".to_string()),
                tool_call_id: Some("2".to_string()),
                assistant_message: crate::api::ChatMessage {
                    role: "assistant".to_string(),
                    content: "call".to_string(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                tool_message: crate::api::ChatMessage {
                    role: "tool".to_string(),
                    content: "result".to_string(),
                    name: None,
                    tool_call_id: Some("2".to_string()),
                    tool_calls: None,
                },
                assistant_message_index: None,
            },
        );

        let result = process_input(&mut app, "/mcp alpha forget");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app
            .mcp_permissions
            .decision_for("alpha", "tool-a")
            .is_none());
        assert!(app.mcp_permissions.decision_for("beta", "tool-b").is_some());
        assert_eq!(app.session.tool_pipeline.tool_result_history.len(), 1);
        assert_eq!(app.session.tool_pipeline.tool_payload_history.len(), 1);
        assert_eq!(
            app.session.tool_pipeline.tool_result_history[0]
                .server_id
                .as_deref(),
            Some("beta")
        );
        assert_eq!(
            app.session.tool_pipeline.tool_payload_history[0]
                .server_id
                .as_deref(),
            Some("beta")
        );

        let config = read_config(&config_path);
        let alpha_enabled = config
            .get("mcp_servers")
            .and_then(|servers| servers.as_array())
            .and_then(|servers| {
                servers
                    .iter()
                    .find(|server| server.get("id").and_then(|id| id.as_str()) == Some("alpha"))
            })
            .and_then(|server| server.get("enabled"))
            .and_then(|value| value.as_bool());
        assert_eq!(alpha_enabled, Some(false));
    });
}

#[test]
fn parse_kv_args_supports_quotes() {
    let args =
        super::mcp_prompt_parser::parse_kv_args("topic=\"soil health\" lang=en").expect("parse");
    assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
    assert_eq!(args.get("lang").map(String::as_str), Some("en"));
}

#[test]
fn parse_kv_args_rejects_missing_equals() {
    let err = super::mcp_prompt_parser::parse_kv_args("topic").unwrap_err();
    assert!(err.contains("key=value"));
}

#[test]
fn parse_prompt_args_single_argument_accepts_bare_value() {
    let prompt_args = vec![PromptArgument {
        name: "topic".to_string(),
        title: None,
        description: None,
        required: Some(true),
    }];
    let args = super::mcp_prompt_parser::parse_prompt_args("soil", &prompt_args).expect("parse");
    assert_eq!(args.get("topic").map(String::as_str), Some("soil"));
}

#[test]
fn parse_prompt_args_single_argument_accepts_quoted_value() {
    let prompt_args = vec![PromptArgument {
        name: "topic".to_string(),
        title: None,
        description: None,
        required: Some(true),
    }];
    let args = super::mcp_prompt_parser::parse_prompt_args("\"soil health\"", &prompt_args)
        .expect("parse");
    assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
}

#[test]
fn parse_prompt_args_single_argument_accepts_unquoted_spaces() {
    let prompt_args = vec![PromptArgument {
        name: "topic".to_string(),
        title: None,
        description: None,
        required: Some(true),
    }];
    let args =
        super::mcp_prompt_parser::parse_prompt_args("soil health", &prompt_args).expect("parse");
    assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
}

#[test]
fn parse_prompt_args_multiple_arguments_requires_key_value() {
    let prompt_args = vec![
        PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        },
        PromptArgument {
            name: "lang".to_string(),
            title: None,
            description: None,
            required: Some(true),
        },
    ];
    let err = super::mcp_prompt_parser::parse_prompt_args("soil", &prompt_args).unwrap_err();
    assert!(err.contains("key=value"));
}

#[test]
fn validate_prompt_args_rejects_unknown_keys() {
    let prompt_args = vec![PromptArgument {
        name: "topic".to_string(),
        title: None,
        description: None,
        required: Some(true),
    }];
    let mut args = HashMap::new();
    args.insert("foo".to_string(), "bar".to_string());
    let err = super::mcp_prompt_parser::validate_prompt_args(&args, &prompt_args).unwrap_err();
    assert!(err.contains("Unknown prompt argument"));
    assert!(err.contains("topic"));
}
