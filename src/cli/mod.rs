//! Command-line interface parsing and handling
//!
//! This module handles parsing command-line arguments and executing the appropriate commands.

pub mod character_list;
mod mcp;
pub mod model_list;
mod prompts;
mod provider;
pub mod provider_list;
pub mod say;
pub mod settings;
pub mod theme_list;

use std::error::Error;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};

// Import specific items we need
use crate::character::CharacterService;
use crate::cli::character_list::list_characters;
use crate::cli::mcp::handle_mcp_command;
use crate::cli::model_list::list_models;
use crate::cli::provider::handle_provider_command;
use crate::cli::provider_list::list_providers;
use crate::cli::settings::{SetContext, SettingRegistry};
use crate::cli::theme_list::list_themes;
use crate::core::builtin_providers::find_builtin_provider;
use crate::core::config::data::Config;
use crate::core::persona::PersonaManager;
use crate::ui::chat_loop::run_chat;
use tracing_subscriber::EnvFilter;

#[cfg(test)]
use crate::ui::builtin_themes::find_builtin_theme;

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
  Use 'chabeau provider add' and 'chabeau provider token add <id>' to set up credentials.\n\n\
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
#[command(disable_version_flag = true)]
#[command(long_about = HELP_ABOUT.as_str())]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Model to use for chat, or list available models if no model specified
    #[arg(short = 'm', long, value_name = "MODEL", num_args = 0..=1, default_missing_value = "")]
    pub model: Option<String>,

    /// Enable logging to specified file
    #[arg(short = 'l', long)]
    pub log: Option<String>,

    /// Provider to use, or list available providers if no provider specified
    #[arg(short = 'p', long, value_name = "PROVIDER", num_args = 0..=1, default_missing_value = "")]
    pub provider: Option<String>,

    /// Use environment variables for auth (ignore keyring/config)
    #[arg(long = "env", action = clap::ArgAction::SetTrue)]
    pub env_only: bool,

    /// Character card to use (name from cards dir, or file path), or list available characters if no character specified
    #[arg(short = 'c', long, value_name = "CHARACTER", num_args = 0..=1, default_missing_value = "")]
    pub character: Option<String>,

    /// Persona to use for this session
    #[arg(long, value_name = "PERSONA")]
    pub persona: Option<String>,

    /// Preset to use for this session
    #[arg(long, value_name = "PRESET")]
    pub preset: Option<String>,

    /// Print version information
    #[arg(short = 'v', long = "version", action = clap::ArgAction::SetTrue)]
    pub version: bool,

    /// Enable verbose MCP debug logging
    #[arg(long = "debug-mcp", action = clap::ArgAction::SetTrue)]
    pub debug_mcp: bool,

    /// Disable MCP even if configured
    #[arg(short = 'd', long = "disable-mcp", action = clap::ArgAction::SetTrue)]
    pub disable_mcp: bool,

    /// Session ID to load on startup
    #[arg(short = 's', long, value_name = "SESSION")]
    pub session: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage API providers and credentials
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },
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
    /// Send a single-turn message to a model without launching the TUI (MCP is disabled in this mode)
    Say {
        /// The prompt to send to the model
        prompt: Vec<String>,
    },
    /// Manage MCP servers and authentication
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// List configured providers and token status
    List,
    /// Add provider credentials or a custom provider interactively
    Add {
        /// Built-in provider id/name shortcut, or custom provider id seed
        provider: Option<String>,
        /// Show optional provider settings, including authentication mode
        #[arg(short = 'a', long = "advanced", action = clap::ArgAction::SetTrue)]
        advanced: bool,
    },
    /// Edit a custom provider configuration interactively
    Edit {
        /// Provider id from config.toml
        provider: String,
    },
    /// Remove a custom provider, or remove token for a built-in provider
    Remove {
        /// Provider id from config.toml
        provider: String,
    },
    /// Manage provider bearer tokens
    Token {
        #[command(subcommand)]
        command: ProviderTokenCommands,
    },
}

#[derive(Subcommand)]
pub enum ProviderTokenCommands {
    /// Show token status for one or all providers
    List {
        /// Provider id
        provider: Option<String>,
    },
    /// Store or update the bearer token for a provider
    Add {
        /// Provider id
        provider: String,
    },
    /// Remove the bearer token for a provider
    Remove {
        /// Provider id
        provider: String,
    },
}

#[derive(Subcommand)]
pub enum McpCommands {
    /// List configured MCP servers and token status
    List,
    /// Add a new MCP server configuration interactively
    Add {
        /// Show optional MCP settings in the add flow
        #[arg(short = 'a', long = "advanced", action = clap::ArgAction::SetTrue)]
        advanced: bool,
    },
    /// Edit an existing MCP server configuration interactively
    Edit {
        /// MCP server id from config.toml
        server: String,
        /// Show optional MCP settings in the edit flow
        #[arg(short = 'a', long = "advanced", action = clap::ArgAction::SetTrue)]
        advanced: bool,
    },
    /// Remove an MCP server configuration
    Remove {
        /// MCP server id from config.toml
        server: String,
    },
    /// Manage bearer tokens for MCP servers
    Token {
        #[command(subcommand)]
        command: McpTokenCommands,
    },
    /// Manage OAuth grants for MCP servers
    Oauth {
        #[command(subcommand)]
        command: McpOauthCommands,
    },
}

#[derive(Subcommand)]
pub enum McpTokenCommands {
    /// Show token status for one or all MCP servers
    List {
        /// MCP server id from config.toml
        server: Option<String>,
    },
    /// Store or update the bearer token for an MCP server
    Add {
        /// MCP server id from config.toml
        server: String,
    },
    /// Remove the bearer token for an MCP server
    Remove {
        /// MCP server id from config.toml
        server: String,
    },
}

#[derive(Subcommand)]
pub enum McpOauthCommands {
    /// Show OAuth grant status for one or all MCP servers
    List {
        /// MCP server id from config.toml
        server: Option<String>,
    },
    /// Add an OAuth grant for an MCP server
    Add {
        /// MCP server id from config.toml
        server: String,
        /// Show optional OAuth prompts
        #[arg(short = 'a', long = "advanced", action = clap::ArgAction::SetTrue)]
        advanced: bool,
    },
    /// Remove (revoke + delete) OAuth grant for an MCP server
    Remove {
        /// MCP server id from config.toml
        server: String,
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
#[cfg(test)]
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

/// Print all settings using the registry's format methods.
fn print_all_settings(config: &Config, registry: &SettingRegistry) {
    println!("Current configuration:");
    for key in registry.keys_display_order() {
        if let Some(handler) = registry.get(key) {
            println!("{}", handler.format(config));
        }
    }
}

async fn async_main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    init_mcp_debugging(args.debug_mcp);
    handle_args(args).await
}

fn init_mcp_debugging(enabled: bool) {
    if !enabled {
        return;
    }

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var(
            "RUST_LOG",
            "chabeau::mcp=trace,chabeau::core::app::actions::streaming=debug,chabeau::ui::chat_loop::event_loop=debug,chabeau::ui::chat_loop::keybindings::handlers=debug,rust_mcp_schema=trace,reqwest=debug",
        );
    }
    std::env::set_var("CHABEAU_MCP_DEBUG", "1");

    let log_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("mcp.log");
    let file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        Ok(file) => file,
        Err(err) => {
            eprintln!(
                "❌ Failed to open MCP log file {}: {err}",
                log_path.display()
            );
            return;
        }
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_ansi(false)
        .with_writer(tracing_subscriber::fmt::writer::BoxMakeWriter::new(file))
        .try_init();
}

async fn handle_args(args: Args) -> Result<(), Box<dyn Error>> {
    // Handle version flag
    if args.version {
        print_version_info();
        return Ok(());
    }

    let mut character_service = CharacterService::new();

    match args.command {
        Some(Commands::Provider { command }) => handle_provider_command(command).await,
        Some(Commands::Set { key, value }) => {
            let registry = SettingRegistry::new();
            let config = Config::load()?;

            if let Some(key) = key {
                let mut ctx = SetContext {
                    config: &config,
                    character_service: &mut character_service,
                };

                match registry.get(&key) {
                    Some(handler) => {
                        if value.is_empty() {
                            // No value provided, show current config
                            print_all_settings(&config, &registry);
                        } else {
                            match handler.set(&value, &mut ctx) {
                                Ok(msg) => println!("{msg}"),
                                Err(e) => {
                                    e.print();
                                    std::process::exit(e.exit_code());
                                }
                            }
                        }
                    }
                    None => {
                        eprintln!("❌ Unknown config key: {key}");
                        eprintln!("   Available keys: {}", registry.keys_sorted().join(", "));
                        std::process::exit(1);
                    }
                }
            } else {
                print_all_settings(&config, &registry);
            }
            Ok(())
        }
        Some(Commands::Unset { key, value }) => {
            let registry = SettingRegistry::new();
            let config = Config::load()?;
            let mut ctx = SetContext {
                config: &config,
                character_service: &mut character_service,
            };

            match registry.get(&key) {
                Some(handler) => match handler.unset(value.as_deref(), &mut ctx) {
                    Ok(msg) => println!("{msg}"),
                    Err(e) => {
                        e.print();
                        std::process::exit(e.exit_code());
                    }
                },
                None => {
                    eprintln!("❌ Unknown config key: {key}");
                    eprintln!("   Available keys: {}", registry.keys_sorted().join(", "));
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
                            run_chat(crate::ui::chat_loop::RunChatOptions {
                                model: model.to_string(),
                                log: args.log,
                                provider: provider_for_operations,
                                env_only: args.env_only,
                                character: character_for_operations,
                                persona: args.persona,
                                preset: preset_for_operations.clone(),
                                disable_mcp: args.disable_mcp,
                                character_service: service_for_run
                                    .take()
                                    .expect("character service available for run_chat"),
                                session: args.session.clone(),
                            })
                            .await
                        }
                        None => {
                            // -m was not provided, use default model for chat
                            run_chat(crate::ui::chat_loop::RunChatOptions {
                                model: "default".to_string(),
                                log: args.log,
                                provider: provider_for_operations,
                                env_only: args.env_only,
                                character: character_for_operations,
                                persona: args.persona,
                                preset: preset_for_operations,
                                disable_mcp: args.disable_mcp,
                                character_service: service_for_run
                                    .take()
                                    .expect("character service available for run_chat"),
                                session: args.session.clone(),
                            })
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
            say::run_say(say::RunSayOptions {
                prompt,
                model: args.model,
                provider: args.provider,
                env_only: args.env_only,
                character: args.character,
                persona: args.persona,
                preset: args.preset,
            })
            .await
        }
        Some(Commands::Mcp { command }) => handle_mcp_command(command, args.env_only).await,
    }
}

#[cfg(test)]
mod tests;
