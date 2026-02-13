use super::*;
use crate::core::config::data::{CustomProvider, CustomTheme};
use crate::utils::test_utils::{with_test_config_env, TestEnvVarGuard};
use std::fs;
use tempfile::TempDir;

mod test_helpers {
    use super::*;

    pub(super) fn env_guard(var: &str, value: &str) -> TestEnvVarGuard {
        let mut guard = TestEnvVarGuard::new();
        guard.set_var(var, value);
        guard
    }
}

use test_helpers::env_guard;

#[test]
fn test_character_flag_parsing() {
    // Test short flag
    let args = Args::try_parse_from(["chabeau", "-c", "alice"]).unwrap();
    assert_eq!(args.character, Some("alice".to_string()));

    // Test long flag
    let args = Args::try_parse_from(["chabeau", "--character", "bob"]).unwrap();
    assert_eq!(args.character, Some("bob".to_string()));

    // Test no character flag
    let args = Args::try_parse_from(["chabeau"]).unwrap();
    assert_eq!(args.character, None);

    // Test character flag with path
    let args = Args::try_parse_from(["chabeau", "-c", "path/to/card.json"]).unwrap();
    assert_eq!(args.character, Some("path/to/card.json".to_string()));
}

#[test]
fn test_character_flag_with_other_flags() {
    // Test character flag combined with model and provider
    let args =
        Args::try_parse_from(["chabeau", "-c", "alice", "-m", "gpt-4", "-p", "openai"]).unwrap();
    assert_eq!(args.character, Some("alice".to_string()));
    assert_eq!(args.model, Some("gpt-4".to_string()));
    assert_eq!(args.provider, Some("openai".to_string()));
}

#[test]
fn test_persona_flag_parsing() {
    // Test persona flag
    let args = Args::try_parse_from(["chabeau", "--persona", "alice-dev"]).unwrap();
    assert_eq!(args.persona, Some("alice-dev".to_string()));

    // Test no persona flag
    let args = Args::try_parse_from(["chabeau"]).unwrap();
    assert_eq!(args.persona, None);
}

#[test]
fn test_persona_flag_with_other_flags() {
    // Test persona flag combined with model, provider, and character
    let args = Args::try_parse_from([
        "chabeau",
        "--persona",
        "alice-dev",
        "-m",
        "gpt-4",
        "-p",
        "openai",
        "-c",
        "alice",
    ])
    .unwrap();
    assert_eq!(args.persona, Some("alice-dev".to_string()));
    assert_eq!(args.model, Some("gpt-4".to_string()));
    assert_eq!(args.provider, Some("openai".to_string()));
    assert_eq!(args.character, Some("alice".to_string()));
}

#[test]
fn test_preset_flag_parsing() {
    let args = Args::try_parse_from(["chabeau", "--preset", "focus"]).unwrap();
    assert_eq!(args.preset, Some("focus".to_string()));

    let args = Args::try_parse_from(["chabeau"]).unwrap();
    assert_eq!(args.preset, None);
}

#[test]
fn test_disable_mcp_flag_parsing() {
    let args = Args::try_parse_from(["chabeau", "-d"]).unwrap();
    assert!(args.disable_mcp);

    let args = Args::try_parse_from(["chabeau", "--disable-mcp"]).unwrap();
    assert!(args.disable_mcp);
}

#[test]
fn test_provider_add_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "provider", "add"]).unwrap();
    match args.command {
        Some(Commands::Provider {
            command: ProviderCommands::Add { provider, advanced },
        }) => {
            assert!(provider.is_none());
            assert!(!advanced);
        }
        _ => panic!("Expected provider add subcommand"),
    }
}

#[test]
fn test_provider_add_advanced_flag_parsing() {
    let args = Args::try_parse_from(["chabeau", "provider", "add", "-a"]).unwrap();
    match args.command {
        Some(Commands::Provider {
            command: ProviderCommands::Add { provider, advanced },
        }) => {
            assert!(provider.is_none());
            assert!(advanced);
        }
        _ => panic!("Expected provider add -a subcommand"),
    }
}

#[test]
fn test_provider_add_with_provider_shortcut_parsing() {
    let args = Args::try_parse_from(["chabeau", "provider", "add", "poe"]).unwrap();
    match args.command {
        Some(Commands::Provider {
            command: ProviderCommands::Add { provider, advanced },
        }) => {
            assert_eq!(provider.as_deref(), Some("poe"));
            assert!(!advanced);
        }
        _ => panic!("Expected provider add -a subcommand"),
    }
}

#[test]
fn test_provider_token_add_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "provider", "token", "add", "openai"]).unwrap();
    match args.command {
        Some(Commands::Provider {
            command:
                ProviderCommands::Token {
                    command: ProviderTokenCommands::Add { provider },
                },
        }) => assert_eq!(provider, "openai"),
        _ => panic!("Expected provider token add subcommand"),
    }
}

#[test]
fn test_provider_token_list_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "provider", "token", "list"]).unwrap();
    match args.command {
        Some(Commands::Provider {
            command:
                ProviderCommands::Token {
                    command: ProviderTokenCommands::List { provider },
                },
        }) => assert!(provider.is_none()),
        _ => panic!("Expected provider token list subcommand"),
    }
}

#[test]
fn test_mcp_token_add_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "token", "add", "agpedia"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Token {
                    command: McpTokenCommands::Add { server },
                },
        }) => {
            assert_eq!(server, "agpedia");
        }
        _ => panic!("Expected mcp token add subcommand"),
    }
}

#[test]
fn test_mcp_token_list_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "token", "list"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Token {
                    command: McpTokenCommands::List { server },
                },
        }) => {
            assert!(server.is_none());
        }
        _ => panic!("Expected mcp token list subcommand"),
    }
}

#[test]
fn test_mcp_edit_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "edit", "agpedia"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command: McpCommands::Edit { server },
        }) => {
            assert_eq!(server, "agpedia");
        }
        _ => panic!("Expected mcp edit subcommand"),
    }
}

#[test]
fn test_mcp_add_advanced_flag_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "add", "--advanced"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command: McpCommands::Add { advanced },
        }) => {
            assert!(advanced);
        }
        _ => panic!("Expected mcp add --advanced"),
    }
}

#[test]
fn test_mcp_oauth_add_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "oauth", "add", "agpedia"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Oauth {
                    command: McpOauthCommands::Add { server, advanced },
                },
        }) => {
            assert_eq!(server, "agpedia");
            assert!(!advanced);
        }
        _ => panic!("Expected mcp oauth add subcommand"),
    }
}

#[test]
fn test_mcp_oauth_list_command_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "oauth", "list"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Oauth {
                    command: McpOauthCommands::List { server },
                },
        }) => {
            assert!(server.is_none());
        }
        _ => panic!("Expected mcp oauth list subcommand"),
    }
}

#[test]
fn test_mcp_oauth_add_advanced_flag_parsing() {
    let args = Args::try_parse_from(["chabeau", "mcp", "oauth", "add", "agpedia", "-a"]).unwrap();
    match args.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Oauth {
                    command: McpOauthCommands::Add { server, advanced },
                },
        }) => {
            assert_eq!(server, "agpedia");
            assert!(advanced);
        }
        _ => panic!("Expected mcp oauth add subcommand"),
    }
}

#[test]
fn test_resolve_provider_id_builtin_and_custom() {
    with_test_config_env(|_| {
        Config::mutate(|config| {
            config.add_custom_provider(CustomProvider::new(
                "custom".to_string(),
                "Custom".to_string(),
                "https://example.com".to_string(),
                None,
            ));
            Ok(())
        })
        .unwrap();

        let config = Config::load().unwrap();
        assert_eq!(
            resolve_provider_id(&config, "OpenAI"),
            Some("openai".to_string())
        );
        assert_eq!(
            resolve_provider_id(&config, "CUSTOM"),
            Some("custom".to_string())
        );
        assert!(resolve_provider_id(&config, "unknown").is_none());
    });
}

#[test]
fn test_resolve_theme_id_builtin_and_custom() {
    with_test_config_env(|_| {
        Config::mutate(|config| {
            config.custom_themes.push(CustomTheme {
                id: "sunset".to_string(),
                display_name: "Sunset".to_string(),
                background: None,
                cursor_color: None,
                user_prefix: None,
                user_text: None,
                assistant_text: None,
                system_text: None,
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
                title: None,
                streaming_indicator: None,
                selection_highlight: None,
                input_border: None,
                input_title: None,
                input_text: None,
                input_cursor_modifiers: None,
            });
            Ok(())
        })
        .unwrap();

        let config = Config::load().unwrap();
        assert_eq!(resolve_theme_id(&config, "Dark"), Some("dark".to_string()));
        assert_eq!(
            resolve_theme_id(&config, "sunset"),
            Some("sunset".to_string())
        );
        assert!(resolve_theme_id(&config, "unknown").is_none());
    });
}

#[test]
fn test_persona_validation_with_valid_config() {
    use crate::core::config::data::{Config, Persona};

    // Create a config with test personas
    let config = Config {
        personas: vec![
            Persona {
                id: "alice-dev".to_string(),
                display_name: "Alice".to_string(),
                bio: Some("You are talking to Alice, a senior developer.".to_string()),
            },
            Persona {
                id: "bob-student".to_string(),
                display_name: "Bob".to_string(),
                bio: None,
            },
        ],
        ..Default::default()
    };

    // Test valid persona validation
    assert!(validate_persona("alice-dev", &config).is_ok());
    assert!(validate_persona("bob-student", &config).is_ok());
}

#[test]
fn test_cli_set_default_model_with_mixed_case_provider() {
    with_test_config_env(|_| {
        let args =
            Args::try_parse_from(["chabeau", "set", "default-model", "OpenAI", "gpt-4o"]).unwrap();

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_args(args))
            .expect("CLI command should succeed");

        let config = Config::load().expect("config should load");
        assert_eq!(
            config.get_default_model("openai"),
            Some(&"gpt-4o".to_string())
        );
        assert_eq!(
            config.get_default_model("OpenAI"),
            Some(&"gpt-4o".to_string())
        );
    });
}

#[test]
fn test_cli_set_default_character_with_cached_service() {
    with_test_config_env(|_| {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "Test character",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello from cache!",
                "mes_example": ""
            }
        });

        fs::write(cards_dir.join("alice.json"), card_json.to_string()).unwrap();

        let env_guard = env_guard("CHABEAU_CARDS_DIR", cards_dir.to_str().unwrap());

        let args = Args::try_parse_from([
            "chabeau",
            "set",
            "default-character",
            "openai",
            "gpt-4",
            "Alice",
        ])
        .unwrap();

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_args(args))
            .expect("CLI command should succeed");

        drop(env_guard);

        let config = Config::load().expect("config should load");
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"Alice".to_string())
        );
    });
}
