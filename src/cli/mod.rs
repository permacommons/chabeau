//! Command-line interface parsing and handling
//!
//! This module handles parsing command-line arguments and executing the appropriate commands.

pub mod character_list;
pub mod model_list;
pub mod provider_list;
pub mod say;
pub mod theme_list;

use std::error::Error;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};

// Import specific items we need
use crate::auth::AuthManager;
use crate::character::CharacterService;
use crate::cli::character_list::list_characters;
use crate::cli::model_list::list_models;
use crate::cli::provider_list::list_providers;
use crate::cli::theme_list::list_themes;
use crate::core::builtin_providers::find_builtin_provider;
use crate::core::config::data::Config;
use crate::core::persona::PersonaManager;
use crate::ui::builtin_themes::find_builtin_theme;
use crate::ui::chat_loop::run_chat;

fn print_version_info() {
    println!("chabeau {}", env!("CARGO_PKG_VERSION"));

    // Use option_env! to handle missing git environment variables
    let git_describe = option_env!("VERGEN_GIT_DESCRIBE").unwrap_or("unknown");
    let git_sha = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
    let git_branch = option_env!("VERGEN_GIT_BRANCH").unwrap_or("unknown");

    // Check if git information is available
    let has_git_info = git_describe != "unknown" && !git_describe.starts_with("VERGEN_");

    // Determine build type
    let build_type = if !has_git_info {
        "Distribution build"
    } else if git_describe.starts_with('v')
        && !git_describe.contains('-')
        && !git_describe.contains("dirty")
    {
        "Release build"
    } else {
        "Development build"
    };
    println!("{}", build_type);

    // Show git information if available
    if has_git_info {
        println!("Git commit: {}", &git_sha[..7.min(git_sha.len())]);

        if !git_branch.is_empty() && !git_branch.starts_with("VERGEN_") {
            println!("Git branch: {}", git_branch);
        }

        if git_describe != git_sha {
            println!("Git describe: {}", git_describe);
        }
    }

    if let Some(timestamp) = option_env!("VERGEN_BUILD_TIMESTAMP") {
        println!("Build timestamp: {}", timestamp);
    }
    println!("Rust version: {}", env!("VERGEN_RUSTC_SEMVER"));
    println!("Target triple: {}", env!("VERGEN_CARGO_TARGET_TRIPLE"));
    println!(
        "Build profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    println!();
    println!("Chabeau is a Permacommons project and free forever.");
    println!("See https://permacommons.org/ for more information.");
}

// Unified help text used for both short and long help
// Uses LazyLock to compute the cards directory path at runtime
static HELP_ABOUT: LazyLock<String> = LazyLock::new(|| {
    let cards_dir =
        crate::core::config::data::path_display(crate::character::loader::get_cards_dir());
    format!(
        "Chabeau is a full-screen terminal chat interface for OpenAI‑compatible APIs.\n\n\
Authentication:\n\
  Use 'chabeau auth' to set up credentials (OpenAI, OpenRouter, Poe, Anthropic, custom).\n\n\
For one-off use, you can set environment variables (used only if no providers are configured, or with --env):\n\
  OPENAI_API_KEY    API key\n\
  OPENAI_BASE_URL   Base URL (default: https://api.openai.com/v1)\n\n\
Then run 'chabeau --env' (or just 'chabeau' if you have no configured providers).\n\n\
To select providers (e.g., Anthropic, OpenAI) and their models:\n\
  • If only one provider is configured, Chabeau will use it.\n\
  • Otherwise, it will ask you to select the provider.\n\
  • It will then give you a choice of models.\n\n\
Character cards:\n\
  • Import character cards with 'chabeau import <file.json|file.png>'.\n\
  • Use '-c [CHARACTER]' to start a chat with a specific character:\n\
    - By name: '-c alice' (looks in {cards_dir})\n\
    - By path: '-c ./alice.json' or '-c /path/to/alice.json'\n\
  • Inside the TUI, type '/character' to select a character.\n\n\
  Tips:\n\
  • To make a choice the default, select it with [Alt+Enter], or use 'chabeau set'.\n\
  • Inside the TUI, type '/help' for keys and commands.\n\
  • '-p [PROVIDER]' and '-m [MODEL]' select provider/model; '-p' or '-m' alone list them.\n",
        cards_dir = cards_dir
    )
});

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = HELP_ABOUT.as_str())]
#[command(disable_version_flag = true, disable_help_flag = true)]
#[command(long_about = HELP_ABOUT.as_str())]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Print this help
    #[arg(short = 'h', long = "help", action = clap::ArgAction::Help, help = "Print this help")]
    pub help: Option<bool>,

    /// Model to use for chat, or list available models if no model specified
    #[arg(short = 'm', long, global = true, value_name = "MODEL", num_args = 0..=1, default_missing_value = "")]
    pub model: Option<String>,

    /// Enable logging to specified file
    #[arg(short = 'l', long, global = true)]
    pub log: Option<String>,

    /// Provider to use, or list available providers if no provider specified
    #[arg(short = 'p', long, global = true, value_name = "PROVIDER", num_args = 0..=1, default_missing_value = "")]
    pub provider: Option<String>,

    /// Use environment variables for auth (ignore keyring/config)
    #[arg(long = "env", global = true, action = clap::ArgAction::SetTrue)]
    pub env_only: bool,

    /// Character card to use (name from cards dir, or file path), or list available characters if no character specified
    #[arg(short = 'c', long, global = true, value_name = "CHARACTER", num_args = 0..=1, default_missing_value = "")]
    pub character: Option<String>,

    /// Persona to use for this session
    #[arg(long, global = true, value_name = "PERSONA")]
    pub persona: Option<String>,

    /// Preset to use for this session
    #[arg(long, global = true, value_name = "PRESET")]
    pub preset: Option<String>,

    /// Print version information
    #[arg(short = 'v', long = "version", action = clap::ArgAction::SetTrue)]
    pub version: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Set up authentication for API providers
    Auth,
    /// Remove authentication for API providers
    Deauth,
    /// Set configuration values, or show current configuration if no arguments are provided.
    Set {
        /// Configuration key to set. If no key is provided, the current configuration is shown.
        key: Option<String>,
        /// Value to set for the key (e.g., `openai` for `default-provider`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        value: Vec<String>,
    },
    /// Unset configuration values
    Unset {
        /// Configuration key to unset
        key: String,
        /// Value to unset for the key (optional)
        value: Option<String>,
    },
    /// List available themes (built-in and custom)
    Themes,
    /// Import and validate a character card
    Import {
        /// Path to character card file (JSON or PNG)
        #[arg(value_name = "CARD")]
        card: String,
        /// Force overwrite if card already exists
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Send a single-turn message to a model without launching the TUI
    Say {
        /// The prompt to send to the model
        prompt: Vec<String>,
    },
}

pub fn main() -> Result<(), Box<dyn Error>> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async_main())
}

/// Validate persona argument against available personas in config
fn validate_persona(persona_id: &str, config: &Config) -> Result<(), Box<dyn Error>> {
    let persona_manager = PersonaManager::load_personas(config)?;

    if persona_manager.find_persona_by_id(persona_id).is_none() {
        let available_personas: Vec<String> = persona_manager
            .list_personas()
            .iter()
            .map(|p| format!("{} ({})", p.display_name, p.id))
            .collect();

        if available_personas.is_empty() {
            eprintln!(
                "❌ Persona '{}' not found. No personas are configured.",
                persona_id
            );
            eprintln!("   Add personas to your config.toml file in the [[personas]] section.");
        } else {
            eprintln!("❌ Persona '{}' not found. Available personas:", persona_id);
            for persona in available_personas {
                eprintln!("   {}", persona);
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

/// Resolve a provider identifier against built-in and custom providers.
/// Returns the canonical provider ID if found.
fn resolve_provider_id(config: &Config, input: &str) -> Option<String> {
    if let Some(provider) = find_builtin_provider(input) {
        return Some(provider.id);
    }

    config
        .get_custom_provider(input)
        .map(|provider| provider.id.clone())
}

/// Resolve a theme identifier against built-in and custom themes.
/// Returns the canonical theme ID if found.
fn resolve_theme_id(config: &Config, input: &str) -> Option<String> {
    if let Some(theme) = find_builtin_theme(input) {
        return Some(theme.id);
    }

    config.get_custom_theme(input).map(|theme| theme.id.clone())
}

/// Validate preset argument against available presets in config
fn validate_preset(preset_id: &str, config: &Config) -> Result<(), Box<dyn Error>> {
    let preset_manager = crate::core::preset::PresetManager::load_presets(config)?;

    if preset_manager.find_preset_by_id(preset_id).is_none() {
        let available_presets: Vec<String> = preset_manager
            .list_presets()
            .iter()
            .map(|p| p.id.clone())
            .collect();

        if available_presets.is_empty() {
            eprintln!(
                "❌ Preset '{}' not found. No presets are configured.",
                preset_id
            );
            eprintln!("   Add presets to your config.toml file in the [[presets]] section.");
        } else {
            eprintln!("❌ Preset '{}' not found. Available presets:", preset_id);
            for preset in available_presets {
                eprintln!("   {}", preset);
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

async fn async_main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    handle_args(args).await
}

async fn handle_args(args: Args) -> Result<(), Box<dyn Error>> {
    // Handle version flag
    if args.version {
        print_version_info();
        return Ok(());
    }

    let mut character_service = CharacterService::new();

    match args.command {
        Some(Commands::Auth) => {
            let mut auth_manager = match AuthManager::new() {
                Ok(manager) => manager,
                Err(err) => {
                    eprintln!("❌ Failed to load configuration: {err}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = auth_manager.interactive_auth() {
                eprintln!("❌ Authentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Deauth) => {
            let mut auth_manager = match AuthManager::new() {
                Ok(manager) => manager,
                Err(err) => {
                    eprintln!("❌ Failed to load configuration: {err}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = auth_manager.interactive_deauth(args.provider) {
                eprintln!("❌ Deauthentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Set { key, value }) => {
            if let Some(key) = key {
                match key.as_str() {
                    "default-provider" => {
                        if !value.is_empty() {
                            let provider_input = value.join(" ");
                            let config_snapshot = Config::load()?;

                            match resolve_provider_id(&config_snapshot, &provider_input) {
                                Some(resolved_provider) => {
                                    let provider_msg = resolved_provider.clone();
                                    Config::mutate(move |config| {
                                        config.default_provider = Some(resolved_provider);
                                        Ok(())
                                    })?;
                                    println!("✅ Set default-provider to: {provider_msg}");
                                }
                                None => {
                                    eprintln!(
                                        "❌ Unknown provider: {provider_input}. Run 'chabeau providers' to list available providers."
                                    );
                                    std::process::exit(1);
                                }
                            }
                        } else {
                            Config::load()?.print_all();
                        }
                    }
                    "theme" => {
                        if !value.is_empty() {
                            let theme_input = value.join(" ");
                            let config_snapshot = Config::load()?;

                            match resolve_theme_id(&config_snapshot, &theme_input) {
                                Some(resolved_theme) => {
                                    let theme_msg = resolved_theme.clone();
                                    Config::mutate(move |config| {
                                        config.theme = Some(resolved_theme);
                                        Ok(())
                                    })?;
                                    println!("✅ Set theme to: {theme_msg}");
                                }
                                None => {
                                    eprintln!(
                                        "❌ Unknown theme: {theme_input}. Run 'chabeau themes' to list available themes."
                                    );
                                    std::process::exit(1);
                                }
                            }
                        } else {
                            Config::load()?.print_all();
                        }
                    }
                    "default-model" => {
                        if !value.is_empty() {
                            let val_str = value.join(" ");
                            let parts: Vec<&str> = val_str.splitn(2, ' ').collect();
                            if parts.len() == 2 {
                                let provider_input = parts[0].to_string();
                                let model = parts[1].to_string();
                                let config_snapshot = Config::load()?;

                                let Some(resolved_provider) =
                                    resolve_provider_id(&config_snapshot, &provider_input)
                                else {
                                    eprintln!(
                                        "❌ Unknown provider: {provider_input}. Run 'chabeau providers' to list available providers."
                                    );
                                    std::process::exit(1);
                                };

                                let provider_msg = resolved_provider.clone();
                                let model_msg = model.clone();
                                Config::mutate(move |config| {
                                    config.set_default_model(resolved_provider, model);
                                    Ok(())
                                })?;
                                println!(
                                    "✅ Set default-model for provider '{}' to: {}",
                                    provider_msg, model_msg
                                );
                            } else {
                                eprintln!(
                                    "⚠️  To set a default model, specify the provider and model:"
                                );
                                eprintln!("Example: chabeau set default-model openai gpt-4o");
                            }
                        } else {
                            Config::load()?.print_all();
                        }
                    }
                    "default-character" => {
                        if value.len() >= 3 {
                            let provider_input = value[0].to_string();
                            let model = value[1].to_string();
                            let character = value[2..].join(" ");

                            let config_snapshot = Config::load()?;
                            let Some(resolved_provider) =
                                resolve_provider_id(&config_snapshot, &provider_input)
                            else {
                                eprintln!(
                                    "❌ Unknown provider: {provider_input}. Run 'chabeau providers' to list available providers."
                                );
                                std::process::exit(1);
                            };

                            match character_service.resolve_by_name(&character) {
                                Ok(_) => {
                                    let provider_msg = resolved_provider.clone();
                                    let model_msg = model.clone();
                                    let character_msg = character.clone();
                                    Config::mutate(move |config| {
                                        config.set_default_character(
                                            resolved_provider,
                                            model,
                                            character,
                                        );
                                        Ok(())
                                    })?;
                                    println!(
                                        "✅ Set default character for '{}:{}' to: {}",
                                        provider_msg, model_msg, character_msg
                                    );
                                }
                                Err(err) => {
                                    eprintln!("❌ {}", err);
                                    eprintln!(
                                        "   Run 'chabeau import <file>' to import a character card first"
                                    );
                                    std::process::exit(1);
                                }
                            }
                        } else {
                            eprintln!(
                                "⚠️  To set a default character, specify provider, model, and character:"
                            );
                            eprintln!("Example: chabeau set default-character openai gpt-4 alice");
                        }
                    }
                    _ => {
                        eprintln!("❌ Unknown config key: {key}");
                        std::process::exit(1);
                    }
                }
            } else {
                Config::load()?.print_all();
            }
            Ok(())
        }
        Some(Commands::Unset { key, value }) => {
            match key.as_str() {
                "default-provider" => {
                    Config::mutate(|config| {
                        config.default_provider = None;
                        Ok(())
                    })?;
                    println!("✅ Unset default-provider");
                }
                "theme" => {
                    Config::mutate(|config| {
                        config.theme = None;
                        Ok(())
                    })?;
                    println!("✅ Unset theme");
                }
                "default-model" => {
                    if let Some(provider) = value {
                        let provider_msg = provider.clone();
                        Config::mutate(|config| {
                            config.unset_default_model(&provider);
                            Ok(())
                        })?;
                        println!("✅ Unset default-model for provider: {provider_msg}");
                    } else {
                        eprintln!("⚠️  To unset a default model, specify the provider:");
                        eprintln!("Example: chabeau unset default-model openai");
                    }
                }
                "default-character" => {
                    if let Some(val) = value {
                        let parts: Vec<&str> = val.splitn(2, ' ').collect();
                        if parts.len() == 2 {
                            let provider = parts[0];
                            let model = parts[1];
                            let provider_owned = provider.to_string();
                            let model_owned = model.to_string();
                            Config::mutate(move |config| {
                                config.unset_default_character(&provider_owned, &model_owned);
                                Ok(())
                            })?;
                            println!("✅ Unset default character for '{}:{}'", provider, model);
                        } else {
                            eprintln!(
                                "⚠️  To unset a default character, specify provider and model:"
                            );
                            eprintln!("Example: chabeau unset default-character openai gpt-4");
                        }
                    } else {
                        eprintln!("⚠️  To unset a default character, specify provider and model:");
                        eprintln!("Example: chabeau unset default-character openai gpt-4");
                    }
                }
                _ => {
                    eprintln!("❌ Unknown config key: {key}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        None => {
            // Check if -c was provided without a character name (empty string)
            if args.character.as_deref() == Some("") {
                // -c was provided without a value, list available characters
                return list_characters(&mut character_service).await;
            }

            if args.persona.is_some() || args.preset.is_some() {
                let config = Config::load()?;
                if let Some(persona_id) = &args.persona {
                    validate_persona(persona_id, &config)?;
                }
                if let Some(preset_id) = &args.preset {
                    validate_preset(preset_id, &config)?;
                }
            }

            // Check if -p was provided without a provider name (empty string)
            match args.provider.as_deref() {
                Some("") => {
                    // -p was provided without a value, list available providers
                    list_providers().await
                }
                _ => {
                    // Normal flow: check -m flag behavior
                    let provider_for_operations = if args.provider.as_deref() == Some("") {
                        None // Don't pass empty string provider to other operations
                    } else {
                        args.provider
                    };

                    let character_for_operations = if args.character.as_deref() == Some("") {
                        None // Don't pass empty string character to other operations
                    } else {
                        args.character
                    };
                    let preset_for_operations = args.preset.clone();

                    let mut service_for_run = Some(character_service);

                    match args.model.as_deref() {
                        Some("") => {
                            // -m was provided without a value, list available models
                            let result = list_models(provider_for_operations).await;
                            drop(service_for_run.take());
                            result
                        }
                        Some(model) => {
                            // -m was provided with a value, use it for chat
                            run_chat(
                                model.to_string(),
                                args.log,
                                provider_for_operations,
                                args.env_only,
                                character_for_operations,
                                args.persona,
                                preset_for_operations.clone(),
                                service_for_run
                                    .take()
                                    .expect("character service available for run_chat"),
                            )
                            .await
                        }
                        None => {
                            // -m was not provided, use default model for chat
                            run_chat(
                                "default".to_string(),
                                args.log,
                                provider_for_operations,
                                args.env_only,
                                character_for_operations,
                                args.persona,
                                preset_for_operations,
                                service_for_run
                                    .take()
                                    .expect("character service available for run_chat"),
                            )
                            .await
                        }
                    }
                }
            }
        }
        Some(Commands::Themes) => {
            list_themes().await?;
            Ok(())
        }
        Some(Commands::Import { card, force }) => {
            match crate::character::import::import_card(&card, force) {
                Ok(message) => {
                    println!("{}", message);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("❌ Import failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Say { prompt }) => {
            say::run_say(
                prompt,
                args.model,
                args.provider,
                args.env_only,
                args.character,
                args.persona,
                args.preset,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::data::{CustomProvider, CustomTheme};
    use crate::utils::test_utils::{with_test_config_env, TestEnvVarGuard};
    use std::fs;
    use tempfile::TempDir;

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
        let args = Args::try_parse_from(["chabeau", "-c", "alice", "-m", "gpt-4", "-p", "openai"])
            .unwrap();
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
                Args::try_parse_from(["chabeau", "set", "default-model", "OpenAI", "gpt-4o"])
                    .unwrap();

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

            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("CHABEAU_CARDS_DIR", &cards_dir);

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
}
