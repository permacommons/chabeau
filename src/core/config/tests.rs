use super::data::suggest_provider_id;
use super::data::{
    path_display, Config, CustomProvider, CustomTheme, McpServerConfig, McpToolPayloadRetention,
    Persona,
};
use super::orchestrator::ConfigOrchestrator;
use crate::core::persona::PersonaManager;
use directories::ProjectDirs;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn config_orchestrator_detects_external_updates() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("config.toml");
    let orchestrator = ConfigOrchestrator::new(config_path.clone());

    orchestrator
        .mutate(|config| {
            config.default_provider = Some("first".to_string());
            Ok(())
        })
        .expect("mutate failed");

    let persisted = Config::load_from_path(&config_path).expect("load failed");
    assert_eq!(persisted.default_provider.as_deref(), Some("first"));

    let cached = orchestrator.load_with_cache().expect("cached load failed");
    assert_eq!(cached.default_provider.as_deref(), Some("first"));

    std::thread::sleep(Duration::from_millis(1100));

    let external = Config {
        default_provider: Some("second".to_string()),
        ..Default::default()
    };
    external
        .save_to_path(&config_path)
        .expect("external save failed");

    let reloaded = orchestrator.load_with_cache().expect("reload failed");
    assert_eq!(reloaded.default_provider.as_deref(), Some("second"));
}

#[test]
fn test_load_nonexistent_config() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("nonexistent_config.toml");

    let config = Config::load_from_path(&config_path).expect("Failed to load config");

    assert_eq!(config.default_provider, None);
}

#[test]
fn test_config_persistence_lifecycle() {
    // Tests the full lifecycle of config persistence: initial save, modification, and unsetting values
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_config.toml");

    // Phase 1: Initial save and load
    let config = Config {
        default_provider: Some("initial-provider".to_string()),
        theme: Some("dark".to_string()),
        ..Default::default()
    };
    config
        .save_to_path(&config_path)
        .expect("Failed to save config");
    let loaded = Config::load_from_path(&config_path).expect("Failed to load config");
    assert_eq!(
        loaded.default_provider,
        Some("initial-provider".to_string())
    );
    assert_eq!(loaded.theme, Some("dark".to_string()));

    // Phase 2: Modify and verify persistence of changes
    let mut config = loaded;
    config.default_provider = Some("changed-provider".to_string());
    config
        .save_to_path(&config_path)
        .expect("Failed to save modified config");
    let loaded = Config::load_from_path(&config_path).expect("Failed to load modified config");
    assert_eq!(
        loaded.default_provider,
        Some("changed-provider".to_string())
    );
    assert_eq!(loaded.theme, Some("dark".to_string())); // Other fields should persist

    // Phase 3: Unset and verify persistence of None
    let mut config = loaded;
    config.default_provider = None;
    config
        .save_to_path(&config_path)
        .expect("Failed to save unset config");
    let loaded = Config::load_from_path(&config_path).expect("Failed to load unset config");
    assert_eq!(loaded.default_provider, None);
    assert_eq!(loaded.theme, Some("dark".to_string())); // Other fields should still persist
}

#[test]
fn test_provider_id_normalization() {
    // Verifies that provider IDs are normalized to lowercase for storage
    // but lookups remain case-insensitive for user convenience
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_config.toml");

    let mut config = Config::default();

    // Set with various cases - should all normalize to lowercase internally
    config.set_default_model("OpenAI".to_string(), "gpt-4".to_string());
    config.set_default_model("AnThRoPiC".to_string(), "claude-3-opus".to_string());
    config.set_default_model("poe".to_string(), "claude-instant".to_string());

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");
    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

    // Verify storage is normalized (internal representation is lowercase)
    assert!(loaded_config.default_models.contains_key("openai"));
    assert!(!loaded_config.default_models.contains_key("OpenAI"));
    assert!(loaded_config.default_models.contains_key("anthropic"));
    assert!(!loaded_config.default_models.contains_key("AnThRoPiC"));
    assert!(loaded_config.default_models.contains_key("poe"));

    // Verify lookups work with any casing
    assert_eq!(
        loaded_config.get_default_model("openai"),
        Some(&"gpt-4".to_string())
    );
    assert_eq!(
        loaded_config.get_default_model("OpenAI"),
        Some(&"gpt-4".to_string())
    );
    assert_eq!(
        loaded_config.get_default_model("OPENAI"),
        Some(&"gpt-4".to_string())
    );

    assert_eq!(
        loaded_config.get_default_model("anthropic"),
        Some(&"claude-3-opus".to_string())
    );
    assert_eq!(
        loaded_config.get_default_model("Anthropic"),
        Some(&"claude-3-opus".to_string())
    );
    assert_eq!(
        loaded_config.get_default_model("AnThRoPiC"),
        Some(&"claude-3-opus".to_string())
    );

    assert_eq!(
        loaded_config.get_default_model("poe"),
        Some(&"claude-instant".to_string())
    );
    assert_eq!(
        loaded_config.get_default_model("POE"),
        Some(&"claude-instant".to_string())
    );
}

#[test]
fn test_custom_provider_management() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_config.toml");

    let mut config = Config::default();

    let custom_provider = CustomProvider::new(
        "myapi".to_string(),
        "My Custom API".to_string(),
        "https://api.example.com/v1".to_string(),
        Some("anthropic".to_string()),
    );

    config.add_custom_provider(custom_provider);
    config
        .save_to_path(&config_path)
        .expect("Failed to save config");

    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

    let retrieved_provider = loaded_config.get_custom_provider("myapi");
    assert!(retrieved_provider.is_some());

    let provider = retrieved_provider.unwrap();
    assert_eq!(provider.id, "myapi");
    assert_eq!(provider.display_name, "My Custom API");
    assert_eq!(provider.base_url, "https://api.example.com/v1");
    assert_eq!(provider.mode, Some("anthropic".to_string()));

    let uppercase_lookup = loaded_config.get_custom_provider("MYAPI");
    assert!(uppercase_lookup.is_some());
    assert_eq!(uppercase_lookup.unwrap().id, "myapi");

    let providers = loaded_config.list_custom_providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "myapi");

    let mut config = loaded_config;
    config.remove_custom_provider("MYAPI");
    assert!(config.get_custom_provider("myapi").is_none());
    assert!(config.get_custom_provider("MYAPI").is_none());
    assert_eq!(config.list_custom_providers().len(), 0);
}

#[test]
fn test_suggest_provider_id() {
    assert_eq!(suggest_provider_id("OpenAI GPT"), "openaigpt");
    assert_eq!(suggest_provider_id("My Custom API 123"), "mycustomapi123");
    assert_eq!(
        suggest_provider_id("Test-Provider_Name!"),
        "testprovidername"
    );
    assert_eq!(suggest_provider_id("   Spaces   "), "spaces");
    assert_eq!(suggest_provider_id("123Numbers456"), "123numbers456");
    assert_eq!(suggest_provider_id(""), "");
}

#[test]
fn test_custom_provider_auth_modes() {
    let openai_provider = CustomProvider::new(
        "test1".to_string(),
        "Test OpenAI".to_string(),
        "https://api.test.com/v1".to_string(),
        None,
    );

    let anthropic_provider = CustomProvider::new(
        "test2".to_string(),
        "Test Anthropic".to_string(),
        "https://api.test.com/v1".to_string(),
        Some("anthropic".to_string()),
    );

    assert_eq!(openai_provider.mode, None);
    assert_eq!(anthropic_provider.mode, Some("anthropic".to_string()));
}

#[test]
fn test_custom_theme_save_load() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_theme.toml");
    let mut cfg = Config::default();
    cfg.add_custom_theme(CustomTheme {
        id: "mytheme".to_string(),
        display_name: "My Theme".to_string(),
        background: Some("black".to_string()),
        cursor_color: None,
        user_prefix: Some("green,bold".to_string()),
        user_text: Some("green".to_string()),
        assistant_text: Some("white".to_string()),
        system_text: Some("gray".to_string()),
        app_info_prefix: None,
        app_info_prefix_style: None,
        app_info_text: None,
        app_warning_prefix: None,
        app_warning_prefix_style: None,
        app_warning_text: None,
        app_error_prefix: None,
        app_error_prefix_style: None,
        app_error_text: None,
        app_log_prefix: None,
        app_log_prefix_style: None,
        app_log_text: None,
        title: Some("gray".to_string()),
        streaming_indicator: Some("white".to_string()),
        selection_highlight: None,
        input_border: Some("green".to_string()),
        input_title: Some("gray".to_string()),
        input_text: Some("white".to_string()),
        input_cursor_modifiers: Some("reversed".to_string()),
    });
    cfg.save_to_path(&config_path).expect("save failed");

    let loaded = Config::load_from_path(&config_path).expect("load failed");
    let t = loaded
        .get_custom_theme("mytheme")
        .expect("missing custom theme");
    assert_eq!(t.display_name, "My Theme");
    assert_eq!(t.background.as_deref(), Some("black"));
}

#[test]
fn test_set_and_get_default_character() {
    let mut config = Config::default();

    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );

    assert_eq!(
        config.get_default_character("openai", "gpt-3.5-turbo"),
        None
    );
    assert_eq!(
        config.get_default_character("anthropic", "claude-3-opus"),
        None
    );
}

#[test]
fn test_set_multiple_default_characters() {
    let mut config = Config::default();

    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );
    config.set_default_character(
        "openai".to_string(),
        "gpt-4o".to_string(),
        "alice".to_string(),
    );
    config.set_default_character(
        "anthropic".to_string(),
        "claude-3-opus-20240229".to_string(),
        "bob".to_string(),
    );
    config.set_default_character(
        "anthropic".to_string(),
        "claude-3-5-sonnet-20241022".to_string(),
        "charlie".to_string(),
    );

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );
    assert_eq!(
        config.get_default_character("openai", "gpt-4o"),
        Some(&"alice".to_string())
    );
    assert_eq!(
        config.get_default_character("anthropic", "claude-3-opus-20240229"),
        Some(&"bob".to_string())
    );
    assert_eq!(
        config.get_default_character("anthropic", "claude-3-5-sonnet-20241022"),
        Some(&"charlie".to_string())
    );
}

#[test]
fn test_unset_default_character() {
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

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );
    assert_eq!(
        config.get_default_character("openai", "gpt-4o"),
        Some(&"bob".to_string())
    );

    config.unset_default_character("openai", "gpt-4");

    assert_eq!(config.get_default_character("openai", "gpt-4"), None);
    assert_eq!(
        config.get_default_character("openai", "gpt-4o"),
        Some(&"bob".to_string())
    );
}

#[test]
fn test_unset_last_character_cleans_up_provider() {
    let mut config = Config::default();

    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );

    assert!(config.default_characters.contains_key("openai"));

    config.unset_default_character("openai", "gpt-4");

    assert!(!config.default_characters.contains_key("openai"));
}

#[test]
fn test_default_character_case_insensitive_provider() {
    let mut config = Config::default();

    config.set_default_character(
        "OpenAI".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );

    assert_eq!(
        config.get_default_character("OpenAI", "gpt-4"),
        Some(&"alice".to_string())
    );

    assert_eq!(
        config.get_default_character("OPENAI", "gpt-4"),
        Some(&"alice".to_string())
    );
}

#[test]
fn test_overwrite_default_character() {
    let mut config = Config::default();

    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );

    config.set_default_character("openai".to_string(), "gpt-4".to_string(), "bob".to_string());

    assert_eq!(
        config.get_default_character("openai", "gpt-4"),
        Some(&"bob".to_string())
    );
}

#[test]
fn test_save_and_load_default_characters() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_characters.toml");

    let mut config = Config::default();

    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );
    config.set_default_character(
        "anthropic".to_string(),
        "claude-3-opus-20240229".to_string(),
        "bob".to_string(),
    );

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");

    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

    assert_eq!(
        loaded_config.get_default_character("openai", "gpt-4"),
        Some(&"alice".to_string())
    );
    assert_eq!(
        loaded_config.get_default_character("anthropic", "claude-3-opus-20240229"),
        Some(&"bob".to_string())
    );
}

#[test]
fn test_format_default_characters_empty() {
    let config = Config::default();

    let output = config.format_default_characters();

    assert_eq!(output, "  default-characters: (none set)");
}

#[test]
fn test_format_default_characters_sorting_and_format() {
    let mut config = Config::default();

    // Set up in non-alphabetical order to verify sorting
    config.set_default_character(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice".to_string(),
    );
    config.set_default_character(
        "anthropic".to_string(),
        "claude-3-opus".to_string(),
        "bob".to_string(),
    );
    config.set_default_character(
        "openai".to_string(),
        "gpt-3.5-turbo".to_string(),
        "charlie".to_string(),
    );

    let output = config.format_default_characters();

    // Verify header
    assert!(output.contains("  default-characters:"));

    // Verify all entries are present with correct format (provider:model: character)
    assert!(output.contains("anthropic:claude-3-opus: bob"));
    assert!(output.contains("openai:gpt-3.5-turbo: charlie"));
    assert!(output.contains("openai:gpt-4: alice"));

    // Verify alphabetical sorting: anthropic should come before openai
    let anthropic_pos = output.find("anthropic:claude-3-opus").unwrap();
    let openai_pos = output.find("openai:gpt-3.5-turbo").unwrap();
    assert!(
        anthropic_pos < openai_pos,
        "Providers should be alphabetically sorted (anthropic before openai)"
    );

    // Verify models within same provider are sorted: gpt-3.5-turbo before gpt-4
    let gpt_35_pos = output.find("openai:gpt-3.5-turbo").unwrap();
    let gpt_4_pos = output.find("openai:gpt-4").unwrap();
    assert!(
        gpt_35_pos < gpt_4_pos,
        "Models should be alphabetically sorted (gpt-3.5-turbo before gpt-4)"
    );

    // Verify indentation (should have 4 spaces before each entry)
    let lines: Vec<&str> = output.lines().collect();
    for line in lines.iter().skip(1) {
        // Skip header line
        if !line.is_empty() {
            assert!(
                line.starts_with("    "),
                "Entry lines should be indented with 4 spaces: '{}'",
                line
            );
        }
    }

    // Verify the exact output format
    let expected = "  default-characters:\n    anthropic:claude-3-opus: bob\n    openai:gpt-3.5-turbo: charlie\n    openai:gpt-4: alice";
    assert_eq!(output, expected);
}

#[test]
fn test_persona_serialization() {
    let persona = Persona {
        id: "test-persona".to_string(),
        display_name: "Test User".to_string(),
        bio: Some("A test persona for unit testing".to_string()),
    };

    let serialized = toml::to_string(&persona).expect("Failed to serialize persona");
    let deserialized: Persona = toml::from_str(&serialized).expect("Failed to deserialize persona");

    assert_eq!(deserialized.id, "test-persona");
    assert_eq!(deserialized.display_name, "Test User");
    assert_eq!(
        deserialized.bio,
        Some("A test persona for unit testing".to_string())
    );
}

#[test]
fn test_persona_optional_bio() {
    let persona = Persona {
        id: "minimal-persona".to_string(),
        display_name: "Minimal User".to_string(),
        bio: None,
    };

    let serialized = toml::to_string(&persona).expect("Failed to serialize persona");
    let deserialized: Persona = toml::from_str(&serialized).expect("Failed to deserialize persona");

    assert_eq!(deserialized.id, "minimal-persona");
    assert_eq!(deserialized.display_name, "Minimal User");
    assert_eq!(deserialized.bio, None);
}

#[test]
fn test_config_with_personas() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_personas.toml");

    let config = Config {
        personas: vec![
            Persona {
                id: "alice-dev".to_string(),
                display_name: "Alice".to_string(),
                bio: Some("A senior software developer".to_string()),
            },
            Persona {
                id: "bob-student".to_string(),
                display_name: "Bob".to_string(),
                bio: None,
            },
        ],
        ..Default::default()
    };

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");

    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

    assert_eq!(loaded_config.personas.len(), 2);

    let alice = &loaded_config.personas[0];
    assert_eq!(alice.id, "alice-dev");
    assert_eq!(alice.display_name, "Alice");
    assert_eq!(alice.bio, Some("A senior software developer".to_string()));

    let bob = &loaded_config.personas[1];
    assert_eq!(bob.id, "bob-student");
    assert_eq!(bob.display_name, "Bob");
    assert_eq!(bob.bio, None);
}

#[test]
fn test_empty_personas_array() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_empty_personas.toml");

    let config = Config::default();
    assert!(config.personas.is_empty());

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");
    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");
    assert!(loaded_config.personas.is_empty());
}

#[test]
fn test_mcp_server_config_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_mcp.toml");

    let config = Config {
        mcp_servers: vec![McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha MCP".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: Some(vec!["alpha.tool".to_string()]),
            protocol_version: Some("2024-11-05".to_string()),
            enabled: Some(false),
            tool_payloads: Some(McpToolPayloadRetention::Window),
            tool_payload_window: Some(4),
            yolo: Some(true),
        }],
        ..Default::default()
    };

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");

    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");
    assert_eq!(loaded_config.mcp_servers.len(), 1);

    let server = &loaded_config.mcp_servers[0];
    assert_eq!(server.id, "alpha");
    assert_eq!(server.display_name, "Alpha MCP");
    assert_eq!(server.base_url.as_deref(), Some("https://mcp.example.com"));
    assert_eq!(server.transport.as_deref(), Some("streamable-http"));
    assert_eq!(
        server.allowed_tools.as_deref(),
        Some(&["alpha.tool".to_string()][..])
    );
    assert_eq!(server.protocol_version.as_deref(), Some("2024-11-05"));
    assert_eq!(server.enabled, Some(false));
    assert_eq!(server.tool_payloads, Some(McpToolPayloadRetention::Window));
    assert_eq!(server.tool_payload_window, Some(4));
    assert_eq!(server.yolo, Some(true));
}

#[test]
fn test_mcp_server_stdio_config_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("test_mcp_stdio.toml");

    let mut env = HashMap::new();
    env.insert("MCP_TOKEN".to_string(), "local-dev".to_string());

    let config = Config {
        mcp_servers: vec![McpServerConfig {
            id: "stdio".to_string(),
            display_name: "Stdio MCP".to_string(),
            base_url: None,
            command: Some("mcp-server".to_string()),
            args: Some(vec!["--mode".to_string(), "stdio".to_string()]),
            env: Some(env),
            transport: Some("stdio".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        }],
        ..Default::default()
    };

    config
        .save_to_path(&config_path)
        .expect("Failed to save config");

    let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");
    assert_eq!(loaded_config.mcp_servers.len(), 1);

    let server = &loaded_config.mcp_servers[0];
    assert_eq!(server.id, "stdio");
    assert_eq!(server.display_name, "Stdio MCP");
    assert!(server.base_url.is_none());
    assert_eq!(server.command.as_deref(), Some("mcp-server"));
    assert_eq!(
        server.args.as_deref(),
        Some(&["--mode".to_string(), "stdio".to_string()][..])
    );
    assert_eq!(
        server.env.as_ref().and_then(|env| env.get("MCP_TOKEN")),
        Some(&"local-dev".to_string())
    );
    assert_eq!(server.transport.as_deref(), Some("stdio"));
    assert_eq!(server.enabled, Some(true));
}

#[test]
fn test_path_display() {
    let path = PathBuf::from("/some/absolute/path");
    let display = path_display(&path);
    assert!(!display.is_empty());

    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(&home);
            let subpath = home_path.join("test/path");
            let display = path_display(&subpath);
            assert!(
                display.starts_with("~/"),
                "Expected path to start with ~/, got: {}",
                display
            );
            assert!(display.contains("test/path"));
        }
    }

    let abs_path = PathBuf::from("/usr/local/bin");
    let display = path_display(&abs_path);
    assert_eq!(display, "/usr/local/bin");
}

#[test]
fn test_path_display_with_config_dir() {
    let proj_dirs = ProjectDirs::from("org", "permacommons", "chabeau")
        .expect("Failed to determine config directory");
    let config_dir = proj_dirs.config_dir();
    let display = path_display(config_dir);

    assert!(!display.is_empty());
    assert!(display.contains("chabeau"));

    #[cfg(unix)]
    {
        if std::env::var_os("HOME").is_some() {
            assert!(display.starts_with('~') || display.starts_with('/'));
        }
    }
}

#[test]
fn test_set_and_get_default_persona() {
    let mut config = Config::default();

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert!(manager
        .get_default_for_provider_model("openai", "gpt-4")
        .is_none());

    config.set_default_persona(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice-dev".to_string(),
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4"),
        Some("alice-dev")
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("OPENAI", "gpt-4"),
        Some("alice-dev")
    );

    assert!(manager
        .get_default_for_provider_model("openai", "gpt-3.5-turbo")
        .is_none());
}

#[test]
fn test_unset_default_persona() {
    let mut config = Config::default();

    config.set_default_persona(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice-dev".to_string(),
    );
    config.set_default_persona(
        "openai".to_string(),
        "gpt-4o".to_string(),
        "bob-student".to_string(),
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4"),
        Some("alice-dev")
    );
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4o"),
        Some("bob-student")
    );

    config.unset_default_persona("openai", "gpt-4");

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert!(manager
        .get_default_for_provider_model("openai", "gpt-4")
        .is_none());
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4o"),
        Some("bob-student")
    );
}

#[test]
fn test_unset_default_persona_cleans_up_empty_provider() {
    let mut config = Config::default();

    config.set_default_persona(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice-dev".to_string(),
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4"),
        Some("alice-dev")
    );

    config.unset_default_persona("openai", "gpt-4");

    assert!(config.default_personas.is_empty());

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert!(manager
        .get_default_for_provider_model("openai", "gpt-4")
        .is_none());
}

#[test]
fn test_overwrite_default_persona() {
    let mut config = Config::default();

    config.set_default_persona(
        "openai".to_string(),
        "gpt-4".to_string(),
        "alice-dev".to_string(),
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4"),
        Some("alice-dev")
    );

    config.set_default_persona(
        "openai".to_string(),
        "gpt-4".to_string(),
        "bob-student".to_string(),
    );

    let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
    assert_eq!(
        manager.get_default_for_provider_model("openai", "gpt-4"),
        Some("bob-student")
    );
}
