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
    ListResourcesResult, ListToolsResult, PromptArgument, ServerCapabilities, Tool,
    ToolInputSchema,
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
    assert!(app.active_picker().is_none());
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
fn mcp_command_reports_disabled_client_side_tool_validation() {
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

    let input_schema = ToolInputSchema::new(Vec::new(), None, None);
    let tool = Tool {
        annotations: None,
        description: Some("Search".to_string()),
        execution: None,
        icons: Vec::new(),
        input_schema,
        meta: None,
        name: "search".to_string(),
        output_schema: None,
        title: None,
    };

    if let Some(server) = app.mcp.server_mut("alpha") {
        server.set_cached_tools(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: vec![tool],
        });
        server.cached_tool_validators.insert(
            "search".to_string(),
            crate::mcp::client::CachedToolSchemaValidator {
                validator: None,
                compile_error: Some("unsupported root schema".to_string()),
            },
        );
    } else {
        panic!("missing MCP server state");
    }

    let token_store = crate::core::mcp_auth::McpTokenStore::new_with_keyring(false);
    let output = crate::commands::build_mcp_server_output(
        app.mcp.server("alpha").expect("missing MCP server"),
        false,
        &token_store,
    );
    assert!(output.contains("**Client-side tool validation:** partially disabled."));
    assert!(output.contains("search: schema compilation failed (unsupported root schema)"));
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
        server.set_cached_tools(ListToolsResult {
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
    assert!(server.cached_tool_validators.is_empty());
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

mod session_tests {
    use super::*;
    use crate::core::config::data::{Config, CustomProvider};
    use crate::core::message::Message;
    use crate::core::session_store::save_session;
    use crate::utils::test_utils::{
        create_test_app, create_test_message, with_test_config_env, TestEnvVarGuard,
    };

    struct TestAuthTokenGuard;

    impl TestAuthTokenGuard {
        fn new() -> Self {
            crate::auth::clear_test_tokens();
            Self
        }
    }

    impl Drop for TestAuthTokenGuard {
        fn drop(&mut self) {
            crate::auth::clear_test_tokens();
        }
    }

    fn with_session_dir<F, R>(f: F) -> R
    where
        F: FnOnce(&std::path::Path) -> R,
    {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let mut guard = TestEnvVarGuard::new();
        guard.set_var("CHABEAU_DATA_DIR", temp_dir.path());
        let _auth_guard = TestAuthTokenGuard::new();
        f(temp_dir.path())
    }

    fn save_session_fixture(
        session_id: &str,
        name: &str,
        provider: &str,
        model: &str,
        base_url: &str,
    ) {
        save_session(
            session_id,
            name,
            provider,
            model,
            base_url,
            None,
            None,
            None,
            &[],
            &[],
            &[],
            &crate::core::app::session::McpInitState::default(),
            "",
            "",
            false,
            false,
        )
        .expect("save session fixture");
    }

    fn save_openai_session(session_id: &str, name: &str) {
        crate::auth::set_test_token("openai", "test-openai-key");
        save_session_fixture(
            session_id,
            name,
            "openai",
            "test-model",
            "https://api.openai.com/v1",
        );
    }

    fn assert_loaded_context(
        app: &crate::core::app::App,
        api_key: &str,
        provider: &str,
        provider_display: &str,
        base_url: &str,
        model: &str,
    ) {
        assert_eq!(app.session.api_key, api_key);
        assert_eq!(app.session.provider_name, provider);
        assert_eq!(app.session.provider_display_name, provider_display);
        assert_eq!(app.session.base_url, base_url);
        assert_eq!(app.session.model, model);
    }

    fn assert_status_contains(app: &crate::core::app::App, expected: &str) {
        assert!(app.ui.status.as_deref().unwrap_or("").contains(expected));
    }

    fn configure_custom_provider(id: &str, display_name: &str, base_url: &str) {
        Config::mutate(|config| {
            config.add_custom_provider(CustomProvider::new(
                id.to_string(),
                display_name.to_string(),
                base_url.to_string(),
                None,
            ));
            Ok(())
        })
        .expect("configure custom provider");
    }

    fn save_test_session(session_id: &str, name: &str) {
        save_openai_session(session_id, name);
    }

    fn save_settings_session() {
        save_session(
            "sess-settings",
            "Settings Test",
            "openai",
            "mod",
            "https://api.openai.com/v1",
            None,
            None,
            None,
            &[],
            &[],
            &[],
            &crate::core::app::session::McpInitState::default(),
            "custom refine instructions",
            "custom refine prefix",
            true,
            false,
        )
        .expect("save settings session");
    }

    #[test]
    fn save_command_without_name_uses_first_user_message() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            app.session.session_id = "sess-001".to_string();
            app.ui.messages.push_back(create_test_message(
                "user",
                "This is a very long message that should be truncated because it exceeds fifty characters easily",
            ));

            let result = process_input(&mut app, "/save");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let status = app.ui.status.as_deref().unwrap_or("");
            assert!(status.contains("This is a very long message"));
            assert!(status.contains("sess-001"));
        });
    }

    #[test]
    fn save_command_with_explicit_name() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            app.session.session_id = "sess-002".to_string();
            app.ui
                .messages
                .push_back(create_test_message("user", "Hello"));

            let result = process_input(&mut app, "/save MySession");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let status = app.ui.status.as_deref().unwrap_or("");
            assert!(status.contains("MySession"));
            assert!(status.contains("sess-002"));
        });
    }

    #[test]
    fn save_command_rejects_too_many_args() {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/save arg1 arg2");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app
            .ui
            .status
            .as_deref()
            .unwrap_or("")
            .contains("Usage: /save"));
    }

    #[test]
    fn save_command_sets_confirmation_status() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            app.session.session_id = "sess-003".to_string();
            app.ui
                .messages
                .push_back(create_test_message("user", "Test"));

            let result = process_input(&mut app, "/save ConfirmTest");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let status = app.ui.status.as_deref().unwrap_or("");
            assert!(status.contains("Session saved:"));
            assert!(status.contains("ConfirmTest"));
            assert!(status.contains("sess-003"));
        });
    }

    #[test]
    fn load_command_by_id_restores_messages() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            app.session.session_id = "sess-load-msgs".to_string();
            app.session.provider_name = "openai".to_string();
            app.session.api_key = "message-openai-key".to_string();
            app.session.base_url = "https://api.openai.com/v1".to_string();
            app.ui
                .messages
                .push_back(create_test_message("user", "Before save"));
            app.ui
                .messages
                .push_back(create_test_message("assistant", "Response"));

            process_input(&mut app, "/save LoadMsgsTest");
            crate::auth::set_test_token("openai", "message-openai-key");

            let mut app2 = create_test_app();
            app2.session.session_id = "new-session".to_string();

            let result = super::handlers::session::do_load_session(&mut app2, "sess-load-msgs");
            assert!(result.is_ok());
            assert_eq!(app2.ui.messages.len(), 2);
            assert_eq!(app2.ui.messages[0].content, "Before save");
            assert_eq!(app2.ui.messages[1].content, "Response");
        });
    }

    #[test]
    fn load_command_by_id_restores_session_context() {
        with_session_dir(|_| {
            save_test_session("sess-ctx", "Context Test");

            let mut app = create_test_app();
            let result = super::handlers::session::do_load_session(&mut app, "sess-ctx");
            assert!(result.is_ok());
            assert_eq!(app.session.session_id, "sess-ctx");
            assert_loaded_context(
                &app,
                "test-openai-key",
                "openai",
                "OpenAI",
                "https://api.openai.com/v1",
                "test-model",
            );
        });
    }

    #[test]
    fn load_command_by_id_resolves_provider_credentials() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                crate::auth::set_test_token("openai", "loaded-openai-key");

                save_session_fixture(
                    "sess-auth",
                    "Auth Test",
                    "openai",
                    "gpt-4o",
                    "https://api.openai.com/v1",
                );

                let mut app = create_test_app();
                app.session.api_key = "stale-provider-key".to_string();
                app.session.provider_display_name = "Stale Provider".to_string();

                let result = super::handlers::session::do_load_session(&mut app, "sess-auth");

                assert!(result.is_ok());
                assert_loaded_context(
                    &app,
                    "loaded-openai-key",
                    "openai",
                    "OpenAI",
                    "https://api.openai.com/v1",
                    "gpt-4o",
                );
            });
        });
    }

    #[test]
    fn load_command_by_id_preserves_snapshot_base_url_for_configured_provider() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                configure_custom_provider(
                    "snapshot-provider",
                    "Snapshot Provider",
                    "https://current.example/v1",
                );
                crate::auth::set_test_token("snapshot-provider", "snapshot-provider-key");

                save_session_fixture(
                    "sess-snapshot-base",
                    "Snapshot Base Test",
                    "snapshot-provider",
                    "custom-model",
                    "https://saved.example/v1",
                );

                let mut app = create_test_app();
                let result =
                    super::handlers::session::do_load_session(&mut app, "sess-snapshot-base");

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "snapshot-provider-key",
                    "snapshot-provider",
                    "Snapshot Provider",
                    "https://saved.example/v1",
                    "custom-model",
                );
            });
        });
    }

    #[test]
    fn load_command_by_id_resolves_env_openai_compatible_credentials() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                let mut env_guard = TestEnvVarGuard::new();
                env_guard.set_var("OPENAI_API_KEY", "env-openai-compatible-key");
                env_guard.remove_var("OPENAI_BASE_URL");

                save_session_fixture(
                    "sess-env-compatible",
                    "Env Compatible Test",
                    "openai-compatible",
                    "custom-model",
                    "https://compatible.example/v1",
                );

                let mut app = create_test_app();
                app.session.api_key = "stale-provider-key".to_string();
                app.session.base_url = "https://stale.example/v1".to_string();
                app.session.provider_display_name = "Stale Provider".to_string();

                let result =
                    super::handlers::session::do_load_session(&mut app, "sess-env-compatible");

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "env-openai-compatible-key",
                    "openai-compatible",
                    "OpenAI-compatible",
                    "https://compatible.example/v1",
                    "custom-model",
                );
                assert_status_contains(&app, "using OPENAI_API_KEY fallback");
            });
        });
    }

    #[test]
    fn load_command_by_id_resolves_env_openai_credentials() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                let mut env_guard = TestEnvVarGuard::new();
                env_guard.set_var("OPENAI_API_KEY", "env-openai-key");
                env_guard.remove_var("OPENAI_BASE_URL");

                save_session_fixture(
                    "sess-env-openai",
                    "Env OpenAI Test",
                    "openai",
                    "gpt-4o",
                    "https://api.openai.com/v1",
                );

                let mut app = create_test_app();
                app.session.api_key = "stale-provider-key".to_string();
                app.session.base_url = "https://stale.example/v1".to_string();
                app.session.provider_display_name = "Stale Provider".to_string();

                let result = super::handlers::session::do_load_session(&mut app, "sess-env-openai");

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "env-openai-key",
                    "openai",
                    "OpenAI",
                    "https://api.openai.com/v1",
                    "gpt-4o",
                );
            });
        });
    }

    #[test]
    fn load_command_by_id_warns_when_using_current_credentials_fallback() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                save_session_fixture(
                    "sess-current-fallback",
                    "Current Fallback Test",
                    "runtime-provider",
                    "runtime-model",
                    "https://saved-runtime.example/v1",
                );

                let mut app = create_test_app();
                app.session.provider_name = "runtime-provider".to_string();
                app.session.provider_display_name = "Runtime Provider".to_string();
                app.session.api_key = "runtime-key".to_string();
                app.session.base_url = "https://current-runtime.example/v1".to_string();

                let result =
                    super::handlers::session::do_load_session(&mut app, "sess-current-fallback");

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "runtime-key",
                    "runtime-provider",
                    "Runtime Provider",
                    "https://saved-runtime.example/v1",
                    "runtime-model",
                );
                assert_status_contains(&app, "using current credentials");
            });
        });
    }

    #[test]
    fn load_command_by_id_falls_back_to_current_provider_when_saved_provider_unavailable() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                save_session_fixture(
                    "sess-different-provider-fallback",
                    "Different Provider Fallback Test",
                    "missing-provider",
                    "saved-model",
                    "https://saved-missing.example/v1",
                );

                let mut app = create_test_app();
                app.session.provider_name = "current-provider".to_string();
                app.session.provider_display_name = "Current Provider".to_string();
                app.session.api_key = "current-key".to_string();
                app.session.base_url = "https://current.example/v1".to_string();
                app.session.model = "current-model".to_string();

                let result = super::handlers::session::do_load_session(
                    &mut app,
                    "sess-different-provider-fallback",
                );

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "current-key",
                    "current-provider",
                    "Current Provider",
                    "https://current.example/v1",
                    "current-model",
                );
                assert_status_contains(
                    &app,
                    "provider 'missing-provider' unavailable; using current provider 'current-provider'",
                );
            });
        });
    }

    #[test]
    fn load_command_by_id_rejects_wrong_env_provider_fallback() {
        with_test_config_env(|_| {
            with_session_dir(|_| {
                let mut env_guard = TestEnvVarGuard::new();
                env_guard.set_var("OPENAI_API_KEY", "env-openai-key");
                env_guard.remove_var("OPENAI_BASE_URL");
                crate::auth::set_test_recoverable_keyring_error("anthropic");

                save_session_fixture(
                    "sess-wrong-env-fallback",
                    "Wrong Env Fallback Test",
                    "anthropic",
                    "claude-sonnet-4-20250514",
                    "https://api.anthropic.com",
                );

                let mut app = create_test_app();
                app.session.provider_name = "current-provider".to_string();
                app.session.provider_display_name = "Current Provider".to_string();
                app.session.api_key = "current-key".to_string();
                app.session.base_url = "https://current.example/v1".to_string();
                app.session.model = "current-model".to_string();

                let result =
                    super::handlers::session::do_load_session(&mut app, "sess-wrong-env-fallback");

                assert!(result.is_ok(), "load failed: {result:?}");
                assert_loaded_context(
                    &app,
                    "current-key",
                    "current-provider",
                    "Current Provider",
                    "https://current.example/v1",
                    "current-model",
                );
                assert_status_contains(
                    &app,
                    "provider 'anthropic' unavailable; using current provider 'current-provider'",
                );
            });
        });
    }

    #[test]
    fn load_command_by_id_restores_ui_settings() {
        with_session_dir(|_| {
            save_settings_session();

            let mut app = create_test_app();
            app.ui.markdown_enabled = false;
            app.ui.syntax_enabled = true;
            crate::auth::set_test_token("openai", "test-openai-key");

            let result = super::handlers::session::do_load_session(&mut app, "sess-settings");
            assert!(result.is_ok());
            assert!(app.ui.markdown_enabled);
            assert!(!app.ui.syntax_enabled);
            assert_eq!(
                app.session.refine_instructions,
                "custom refine instructions"
            );
            assert_eq!(app.session.refine_prefix, "custom refine prefix");
        });
    }

    #[test]
    fn load_command_no_sessions_shows_message() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            let result = process_input(&mut app, "/load");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            assert!(app
                .ui
                .status
                .as_deref()
                .unwrap_or("")
                .contains("No saved sessions"));
        });
    }

    #[test]
    fn load_command_invalid_id_shows_error() {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/load nonexistent-session-id");
        assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
        let status = app.ui.status.as_deref().unwrap_or("");
        assert!(status.contains("Load error:"));
    }

    #[test]
    fn load_command_rejects_too_many_args() {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/load id1 id2");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app
            .ui
            .status
            .as_deref()
            .unwrap_or("")
            .contains("Usage: /load"));
    }

    #[test]
    fn sessions_command_no_sessions_shows_message() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            let result = process_input(&mut app, "/sessions");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            assert!(app
                .ui
                .status
                .as_deref()
                .unwrap_or("")
                .contains("No saved sessions"));
        });
    }

    #[test]
    fn sessions_command_with_sessions_opens_picker() {
        with_session_dir(|_| {
            save_test_session("sess-picker", "Picker Test");

            let mut app = create_test_app();
            let result = process_input(&mut app, "/sessions");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.active_picker().is_some());
        });
    }

    #[test]
    fn generate_session_name_truncates_long_messages() {
        use crate::commands::handlers::session::handle_save;
        use crate::commands::registry::CommandInvocation;
        use std::collections::VecDeque;

        let long_content = "a".repeat(100);
        let mut messages = VecDeque::new();
        messages.push_back(Message::new(TranscriptRole::User, long_content));

        let mut app = create_test_app();
        app.session.session_id = "sess-trunc".to_string();
        app.ui.messages = messages;

        let save_cmd = super::registry::find_command("save").expect("save command exists");
        let invocation = CommandInvocation::new_for_test(save_cmd, "save", "", vec![]);

        with_session_dir(|_| {
            let result = handle_save(&mut app, invocation);
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let status = app.ui.status.as_deref().unwrap_or("");
            // 47 'a's + "..." = 50 chars
            assert!(status.contains("aaa"));
        });
    }

    #[test]
    fn generate_session_name_fallback_no_user_messages() {
        use crate::commands::handlers::session::handle_save;
        use crate::commands::registry::CommandInvocation;
        use std::collections::VecDeque;

        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("assistant", "Only assistant"));

        let mut app = create_test_app();
        app.session.session_id = "sess-fallback".to_string();
        app.ui.messages = messages;

        let save_cmd = super::registry::find_command("save").expect("save command exists");
        let invocation = CommandInvocation::new_for_test(save_cmd, "save", "", vec![]);

        with_session_dir(|_| {
            let result = handle_save(&mut app, invocation);
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let status = app.ui.status.as_deref().unwrap_or("");
            assert!(status.contains("Untitled session"));
        });
    }

    #[test]
    fn full_save_load_cycle_integration() {
        with_session_dir(|_| {
            let mut app = create_test_app();
            app.session.session_id = "sess-integration".to_string();
            app.session.provider_name = "openai".to_string();
            app.session.api_key = "integration-openai-key".to_string();
            app.session.model = "gpt-4".to_string();
            app.session.base_url = "https://api.openai.com/v1".to_string();
            app.ui.markdown_enabled = true;
            app.ui.syntax_enabled = false;

            app.ui
                .messages
                .push_back(create_test_message("user", "What is Rust?"));
            app.ui.messages.push_back(create_test_message(
                "assistant",
                "Rust is a systems programming language.",
            ));

            process_input(&mut app, "/save IntegrationTest");
            crate::auth::set_test_token("openai", "integration-openai-key");

            let mut app2 = create_test_app();
            app2.session.session_id = "fresh-session".to_string();
            app2.ui.markdown_enabled = false;
            app2.ui.syntax_enabled = true;

            let result = super::handlers::session::do_load_session(&mut app2, "sess-integration");
            assert!(result.is_ok());

            assert_eq!(app2.session.session_id, "sess-integration");
            assert_eq!(app2.session.provider_name, "openai");
            assert_eq!(app2.session.model, "gpt-4");
            assert_eq!(app2.session.base_url, "https://api.openai.com/v1");
            assert!(app2.ui.markdown_enabled);
            assert!(!app2.ui.syntax_enabled);
            assert_eq!(app2.ui.messages.len(), 2);
            assert_eq!(app2.ui.messages[0].content, "What is Rust?");
            assert_eq!(
                app2.ui.messages[1].content,
                "Rust is a systems programming language."
            );
        });
    }
}
