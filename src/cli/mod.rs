//! Command-line interface parsing and handling
//!
//! This module handles parsing command-line arguments and executing the appropriate commands.

pub mod character_list;
pub mod model_list;
pub mod provider_list;
pub mod say;
pub mod settings;
pub mod theme_list;

use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};

// Import specific items we need
use crate::auth::prompt_provider_token;
use crate::auth::AuthManager;
use crate::character::CharacterService;
use crate::cli::character_list::list_characters;
use crate::cli::model_list::list_models;
use crate::cli::provider_list::list_providers;
use crate::cli::settings::{SetContext, SettingRegistry};
use crate::cli::theme_list::list_themes;
use crate::core::builtin_providers::{find_builtin_provider, load_builtin_providers};
use crate::core::config::data::{Config, CustomProvider, McpServerConfig};
use crate::core::mcp_auth::{McpOAuthGrant, McpTokenStore};
use crate::core::oauth::{
    apply_oauth_token_response, build_authorization_url, current_unix_epoch_s, exchange_oauth_code,
    open_in_browser, pkce_s256_challenge, probe_oauth_support, random_urlsafe,
    register_oauth_client, wait_for_oauth_callback, AuthorizationUrlParams, OAuthMetadata,
};
use crate::core::persona::PersonaManager;
use crate::ui::chat_loop::run_chat;
use crate::utils::url::normalize_base_url;
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
                            run_chat(
                                model.to_string(),
                                args.log,
                                provider_for_operations,
                                args.env_only,
                                character_for_operations,
                                args.persona,
                                preset_for_operations.clone(),
                                args.disable_mcp,
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
                                args.disable_mcp,
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
        Some(Commands::Mcp { command }) => handle_mcp_command(command, args.env_only).await,
    }
}

#[derive(Clone)]
struct ProviderStatusRow {
    id: String,
    display_name: String,
    has_token: bool,
    kind: &'static str,
}

fn collect_provider_status_rows(
    auth_manager: &AuthManager,
    config: &Config,
) -> (Vec<ProviderStatusRow>, Option<String>) {
    let (providers, default_provider) = auth_manager.get_all_providers_with_auth_status();
    let rows = providers
        .into_iter()
        .map(|provider| {
            let kind = if find_builtin_provider(&provider.id).is_some()
                && config.get_custom_provider(&provider.id).is_none()
            {
                "builtin"
            } else {
                "custom"
            };
            ProviderStatusRow {
                id: provider.id,
                display_name: provider.display_name,
                has_token: provider.has_token,
                kind,
            }
        })
        .collect();
    (rows, default_provider)
}

fn resolve_custom_provider<'a>(config: &'a Config, input: &str) -> Option<&'a CustomProvider> {
    config.get_custom_provider(input)
}

fn resolve_provider_mode(input: &str) -> Result<Option<String>, Box<dyn Error>> {
    let normalized = input.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "openai" {
        return Ok(None);
    }
    if normalized == "anthropic" {
        return Ok(Some(normalized));
    }
    Err("Authentication mode must be 'openai' or 'anthropic'.".into())
}

fn prompt_provider_authentication_mode(
    default_mode: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let mode_input = prompt_optional(&format!(
        "Authentication mode [openai|anthropic] [{default_mode}]: "
    ))?;
    if mode_input.is_empty() {
        if default_mode.eq_ignore_ascii_case("anthropic") {
            Ok(Some("anthropic".to_string()))
        } else {
            Ok(None)
        }
    } else {
        resolve_provider_mode(&mode_input)
    }
}

fn validate_provider_id(input: &str) -> Result<String, Box<dyn Error>> {
    if !input
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return Err("Provider id must contain only letters, numbers, '-' or '_'.".into());
    }
    Ok(input.to_ascii_lowercase())
}

enum ProviderAddMode {
    BuiltinToken,
    CustomProvider,
}

fn print_available_builtin_providers() {
    println!("Available built-in providers:");
    for provider in load_builtin_providers() {
        println!("  - {} ({})", provider.display_name, provider.id);
    }
    println!();
}

fn prompt_provider_add_mode() -> Result<ProviderAddMode, Box<dyn Error>> {
    println!("Select provider setup type:");
    println!("  1) Add token for a built-in provider");
    println!("  2) Add a custom provider");
    loop {
        let input = prompt_optional("Choice [1/2] [1]: ")?;
        match input.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "builtin" | "built-in" => return Ok(ProviderAddMode::BuiltinToken),
            "2" | "custom" => return Ok(ProviderAddMode::CustomProvider),
            _ => println!("Enter 1 for built-in or 2 for custom."),
        }
    }
}

fn prompt_builtin_provider_choice() -> Result<(String, String), Box<dyn Error>> {
    let builtins = load_builtin_providers();
    println!("Built-in providers:");
    for (index, provider) in builtins.iter().enumerate() {
        println!(
            "  {}) {} ({})",
            index + 1,
            provider.display_name,
            provider.id
        );
    }

    loop {
        let input = prompt_optional("Select provider by number or id: ")?;
        if let Ok(index) = input.parse::<usize>() {
            if index > 0 && index <= builtins.len() {
                let provider = &builtins[index - 1];
                return Ok((provider.id.clone(), provider.display_name.clone()));
            }
        }

        if let Some(provider) = builtins.iter().find(|candidate| {
            candidate.id.eq_ignore_ascii_case(&input)
                || candidate.display_name.eq_ignore_ascii_case(&input)
        }) {
            return Ok((provider.id.clone(), provider.display_name.clone()));
        }
        println!("Unknown provider. Enter a listed number or provider id.");
    }
}

fn prompt_and_store_provider_token(
    auth_manager: &AuthManager,
    provider_id: &str,
    display_name: &str,
) -> Result<(), Box<dyn Error>> {
    let token = prompt_provider_token(display_name).map_err(|err| err.to_string())?;
    auth_manager.store_token(provider_id, &token)?;
    println!("✅ Stored provider token for {display_name}");
    Ok(())
}

fn remove_provider_token_with_message(
    auth_manager: &AuthManager,
    provider_id: &str,
    display_name: &str,
) -> Result<(), Box<dyn Error>> {
    auth_manager.remove_token(provider_id)?;
    println!("✅ Removed provider token for {display_name}");
    Ok(())
}

fn confirm_provider_token_replacement(
    rows: &[ProviderStatusRow],
    provider_id: &str,
    display_name: &str,
) -> Result<bool, Box<dyn Error>> {
    let has_token = rows
        .iter()
        .find(|candidate| candidate.id.eq_ignore_ascii_case(provider_id))
        .is_some_and(|row| row.has_token);
    if has_token {
        prompt_bool_with_default(
            &format!(
                "A token is already configured for {}. Replace it",
                display_name
            ),
            false,
        )
    } else {
        Ok(true)
    }
}

async fn handle_provider_command(command: ProviderCommands) -> Result<(), Box<dyn Error>> {
    match command {
        ProviderCommands::List => list_providers().await,
        ProviderCommands::Add { provider, advanced } => handle_provider_add(provider, advanced),
        ProviderCommands::Edit { provider } => handle_provider_edit(&provider),
        ProviderCommands::Remove { provider } => handle_provider_remove(&provider),
        ProviderCommands::Token { command } => handle_provider_token(command),
    }
}

fn resolve_builtin_provider_choice(input: &str) -> Option<(String, String)> {
    load_builtin_providers()
        .into_iter()
        .find(|provider| {
            provider.id.eq_ignore_ascii_case(input)
                || provider.display_name.eq_ignore_ascii_case(input)
        })
        .map(|provider| (provider.id, provider.display_name))
}

fn add_builtin_provider_token(provider_id: &str, display_name: &str) -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let config = Config::load()?;
    let (rows, _) = collect_provider_status_rows(&auth_manager, &config);
    if !confirm_provider_token_replacement(&rows, provider_id, display_name)? {
        println!("Cancelled.");
        return Ok(());
    }
    prompt_and_store_provider_token(&auth_manager, provider_id, display_name)
}

fn add_custom_provider_interactive(
    advanced: bool,
    seeded_provider_id: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    if !advanced {
        println!(
            "Basic mode: advanced options are hidden (including authentication mode). Re-run with `chabeau provider add -a` for advanced settings."
        );
    }

    let display_name = if let Some(provider_id) = seeded_provider_id.as_deref() {
        let input = prompt_optional(&format!("Display name [{provider_id}]: "))?;
        if input.is_empty() {
            provider_id.to_string()
        } else {
            input
        }
    } else {
        prompt_required("Display name: ")?
    };
    let provider_id = if let Some(provider_id) = seeded_provider_id {
        provider_id
    } else {
        let suggested_id = crate::core::config::data::suggest_provider_id(&display_name);
        let id_input = prompt_optional(&format!("Provider id [{suggested_id}]: "))?;
        if id_input.is_empty() {
            suggested_id
        } else {
            validate_provider_id(&id_input)?
        }
    };

    if find_builtin_provider(&provider_id).is_some()
        || config.get_custom_provider(&provider_id).is_some()
    {
        return Err(format!("Provider '{provider_id}' already exists").into());
    }

    let base_url_input = prompt_required("Base URL: ")?;
    let base_url = normalize_base_url(&base_url_input);
    let mode = if advanced {
        prompt_provider_authentication_mode("openai")?
    } else {
        None
    };

    config.add_custom_provider(CustomProvider::new(
        provider_id.clone(),
        display_name.clone(),
        base_url,
        mode,
    ));
    config.save()?;
    println!("✅ Added provider {display_name} ({provider_id})");

    if prompt_bool_with_default("Add bearer token now", true)? {
        let auth_manager = AuthManager::new()?;
        prompt_and_store_provider_token(&auth_manager, &provider_id, &display_name)?;
    }

    Ok(())
}

fn handle_provider_add(provider: Option<String>, advanced: bool) -> Result<(), Box<dyn Error>> {
    if let Some(input) = provider {
        if let Some((provider_id, display_name)) = resolve_builtin_provider_choice(&input) {
            println!("Recognized built-in provider: {display_name} ({provider_id}).");
            return add_builtin_provider_token(&provider_id, &display_name);
        }
        let provider_id = validate_provider_id(&input)?;
        println!(
            "'{input}' is not a built-in provider. Treating it as a new custom provider id: {provider_id}"
        );
        return add_custom_provider_interactive(advanced, Some(provider_id));
    }

    print_available_builtin_providers();
    match prompt_provider_add_mode()? {
        ProviderAddMode::BuiltinToken => {
            let (provider_id, display_name) = prompt_builtin_provider_choice()?;
            return add_builtin_provider_token(&provider_id, &display_name);
        }
        ProviderAddMode::CustomProvider => {}
    }

    add_custom_provider_interactive(advanced, None)
}

fn handle_provider_edit(provider_input: &str) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    let existing = resolve_custom_provider(&config, provider_input).cloned();
    let Some(mut provider) = existing else {
        if find_builtin_provider(provider_input).is_some() {
            return Err(
                "Built-in providers cannot be edited. Add a custom provider for overrides.".into(),
            );
        }
        return Err(format!("Provider '{provider_input}' not found").into());
    };

    let display_input = prompt_optional(&format!("Display name [{}]: ", provider.display_name))?;
    if !display_input.is_empty() {
        provider.display_name = display_input;
    }

    let base_input = prompt_optional(&format!("Base URL [{}]: ", provider.base_url))?;
    if !base_input.is_empty() {
        provider.base_url = normalize_base_url(&base_input);
    }

    let mode_default = provider.mode.as_deref().unwrap_or("openai");
    provider.mode = prompt_provider_authentication_mode(mode_default)?;

    for entry in &mut config.custom_providers {
        if entry.id.eq_ignore_ascii_case(&provider.id) {
            *entry = provider.clone();
            break;
        }
    }
    config.save()?;
    println!(
        "✅ Updated provider {} ({})",
        provider.display_name, provider.id
    );
    Ok(())
}

fn handle_provider_remove(provider_input: &str) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    let auth_manager = AuthManager::new()?;
    if let Some(builtin) = find_builtin_provider(provider_input) {
        let confirmed = prompt_bool_with_default(
            &format!(
                "Remove token for built-in provider {} ({})? The provider itself will remain available",
                builtin.display_name, builtin.id
            ),
            false,
        )?;
        if !confirmed {
            println!("Cancelled.");
            return Ok(());
        }
        remove_provider_token_with_message(&auth_manager, &builtin.id, &builtin.display_name)?;
        println!(
            "ℹ️ Built-in provider {} ({}) remains available.",
            builtin.display_name, builtin.id
        );
        return Ok(());
    }
    let provider = if let Some(provider) = resolve_custom_provider(&config, provider_input) {
        provider.clone()
    } else {
        return Err(format!("Provider '{provider_input}' not found").into());
    };

    let confirmed = prompt_bool_with_default(
        &format!(
            "Remove provider {} ({}) from config",
            provider.display_name, provider.id
        ),
        false,
    )?;
    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    config.remove_custom_provider(&provider.id);
    config.save()?;
    let _ = remove_provider_token_with_message(&auth_manager, &provider.id, &provider.display_name);
    println!(
        "✅ Removed provider {} ({})",
        provider.display_name, provider.id
    );
    Ok(())
}

fn handle_provider_token(command: ProviderTokenCommands) -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let auth_manager = AuthManager::new()?;
    let (rows, default_provider) = collect_provider_status_rows(&auth_manager, &config);

    match command {
        ProviderTokenCommands::List { provider } => {
            if let Some(input) = provider {
                let provider_id = resolve_provider_id(&config, &input)
                    .ok_or_else(|| format!("Provider '{input}' not found"))?;
                let row = rows
                    .iter()
                    .find(|candidate| candidate.id.eq_ignore_ascii_case(&provider_id))
                    .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;
                let status = if row.has_token {
                    "configured"
                } else {
                    "missing"
                };
                println!(
                    "Provider token for {} ({}, {}): {}",
                    row.display_name, row.id, row.kind, status
                );
            } else {
                if rows.is_empty() {
                    println!("No providers configured.");
                    return Ok(());
                }
                println!("Provider token status:");
                for row in rows {
                    let default_mark = if default_provider
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(&row.id))
                    {
                        "*"
                    } else {
                        ""
                    };
                    let status = if row.has_token {
                        "configured"
                    } else {
                        "missing"
                    };
                    println!(
                        "  - {}{} ({}, {}): {}",
                        row.display_name, default_mark, row.id, row.kind, status
                    );
                }
            }
        }
        ProviderTokenCommands::Add { provider } => {
            let provider_id = resolve_provider_id(&config, &provider)
                .ok_or_else(|| format!("Provider '{provider}' not found"))?;
            let row = rows
                .iter()
                .find(|candidate| candidate.id.eq_ignore_ascii_case(&provider_id))
                .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;
            if !confirm_provider_token_replacement(&rows, &provider_id, &row.display_name)? {
                println!("Cancelled.");
                return Ok(());
            }
            prompt_and_store_provider_token(&auth_manager, &provider_id, &row.display_name)?;
        }
        ProviderTokenCommands::Remove { provider } => {
            let provider_id = resolve_provider_id(&config, &provider)
                .ok_or_else(|| format!("Provider '{provider}' not found"))?;
            let row = rows
                .iter()
                .find(|candidate| candidate.id.eq_ignore_ascii_case(&provider_id))
                .ok_or_else(|| format!("Provider '{provider_id}' not found"))?;
            remove_provider_token_with_message(&auth_manager, &provider_id, &row.display_name)?;
        }
    }

    Ok(())
}

async fn handle_mcp_command(command: McpCommands, _env_only: bool) -> Result<(), Box<dyn Error>> {
    match command {
        McpCommands::List => handle_mcp_list(),
        McpCommands::Add { advanced } => handle_mcp_add(advanced).await,
        McpCommands::Edit { server } => handle_mcp_edit(&server),
        McpCommands::Remove { server } => handle_mcp_remove(&server),
        McpCommands::Token { command } => handle_mcp_token(command),
        McpCommands::Oauth { command } => handle_mcp_oauth(command).await,
    }
}

fn handle_mcp_list() -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let servers = config.list_mcp_servers();
    if servers.is_empty() {
        println!(
            "No MCP servers configured. Add `[[mcp_servers]]` to config.toml or run `chabeau mcp add`."
        );
        return Ok(());
    }

    let store = McpTokenStore::new();
    println!("Configured MCP servers:");
    for server in servers {
        let token_status = match store.get_token(&server.id) {
            Ok(Some(_)) => "token configured",
            Ok(None) => "no token",
            Err(_) => "token status unavailable",
        };
        let transport = server.transport.as_deref().unwrap_or("streamable-http");
        let enabled = if server.is_enabled() {
            "enabled"
        } else {
            "disabled"
        };
        println!(
            "  - {} ({}) [{}; {}; {}]",
            server.display_name, server.id, transport, enabled, token_status
        );
    }

    Ok(())
}

async fn handle_mcp_add(advanced: bool) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    if !advanced {
        println!(
            "Basic mode: advanced options are hidden. Re-run with `chabeau mcp add -a` for advanced settings."
        );
    }
    let display_name = prompt_required("Display name: ")?;
    let suggested_id = crate::core::config::data::suggest_provider_id(&display_name);
    let id_input = prompt_optional(&format!("Server id [{suggested_id}]: "))?;
    let server_id = if id_input.is_empty() {
        suggested_id
    } else {
        validate_mcp_server_id(&id_input)?
    };

    if config.get_mcp_server(&server_id).is_some() {
        return Err(format!("MCP server '{server_id}' already exists").into());
    }

    let mut server = McpServerConfig {
        id: server_id,
        display_name,
        base_url: None,
        command: None,
        args: None,
        env: None,
        transport: Some(prompt_transport(None)?.to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(true),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: Some(false),
    };
    configure_mcp_transport_fields(&mut server, false, advanced)?;
    if advanced {
        server.enabled = Some(prompt_bool_with_default("Enabled", server.is_enabled())?);
        server.yolo = Some(prompt_bool_with_default(
            "YOLO auto-approve",
            server.is_yolo(),
        )?);
    }

    config.mcp_servers.push(server.clone());
    config.save()?;
    println!(
        "✅ Added MCP server {} ({})",
        server.display_name, server.id
    );

    let is_http_transport = !matches!(server.transport.as_deref(), Some("stdio"));
    if is_http_transport
        && server
            .base_url
            .as_deref()
            .is_some_and(|url| url.starts_with("http://") || url.starts_with("https://"))
    {
        if let Some(metadata) = probe_oauth_support(&server).await? {
            println!("Detected OAuth metadata for {}.", server.display_name);
            if let Err(err) =
                add_oauth_grant_for_server(&server, Some(metadata), false, advanced).await
            {
                eprintln!("⚠️ OAuth setup skipped: {err}");
            }
        } else if prompt_bool_with_default(
            "No OAuth metadata detected. Add bearer token now",
            false,
        )? {
            let token =
                prompt_provider_token(&server.display_name).map_err(|err| err.to_string())?;
            McpTokenStore::new().set_token(&server.id, &token)?;
            println!("✅ Stored MCP token for {}", server.display_name);
        }
    }
    Ok(())
}

fn handle_mcp_edit(server_input: &str) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    let current = resolve_mcp_server(&config, server_input)?.clone();
    let mut server = current.clone();

    let display_name = prompt_optional(&format!("Display name [{}]: ", current.display_name))?;
    if !display_name.is_empty() {
        server.display_name = display_name;
    }

    let current_transport = current
        .transport
        .as_deref()
        .unwrap_or("streamable-http")
        .to_string();
    let transport = prompt_transport(Some(&current_transport))?;
    server.transport = Some(transport.to_string());
    configure_mcp_transport_fields(&mut server, true, true)?;

    server.enabled = Some(prompt_bool_with_default("Enabled", current.is_enabled())?);
    server.yolo = Some(prompt_bool_with_default(
        "YOLO auto-approve",
        current.is_yolo(),
    )?);

    if let Some(existing) = config
        .mcp_servers
        .iter_mut()
        .find(|candidate| candidate.id.eq_ignore_ascii_case(&server.id))
    {
        *existing = server.clone();
    }
    config.save()?;
    println!(
        "✅ Updated MCP server {} ({})",
        server.display_name, server.id
    );
    Ok(())
}

fn handle_mcp_remove(server_input: &str) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    let server = resolve_mcp_server(&config, server_input)?.clone();
    let confirmed = prompt_bool_with_default(
        &format!(
            "Remove MCP server {} ({}) from config",
            server.display_name, server.id
        ),
        false,
    )?;
    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    config
        .mcp_servers
        .retain(|candidate| !candidate.id.eq_ignore_ascii_case(&server.id));
    config.save()?;
    println!(
        "✅ Removed MCP server {} ({})",
        server.display_name, server.id
    );
    Ok(())
}

fn handle_mcp_token(command: McpTokenCommands) -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let store = McpTokenStore::new();
    match command {
        McpTokenCommands::Add { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            let token = prompt_provider_token(&server_config.display_name)
                .map_err(|err| err.to_string())?;
            store.set_token(&server_config.id, &token)?;
            println!("✅ Stored MCP token for {}", server_config.display_name);
        }
        McpTokenCommands::Remove { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            let removed = store.remove_token(&server_config.id)?;
            if removed {
                println!("✅ Removed MCP token for {}", server_config.display_name);
            } else {
                println!("No MCP token was stored for {}", server_config.display_name);
            }
        }
        McpTokenCommands::List { server } => {
            if let Some(server) = server {
                let server_config = resolve_mcp_server(&config, &server)?;
                let status = if store.get_token(&server_config.id)?.is_some() {
                    "configured"
                } else {
                    "missing"
                };
                println!(
                    "MCP token for {} ({}): {}",
                    server_config.display_name, server_config.id, status
                );
            } else {
                let servers = config.list_mcp_servers();
                if servers.is_empty() {
                    println!("No MCP servers configured.");
                    return Ok(());
                }
                println!("MCP token status:");
                for server in servers {
                    let status = if store.get_token(&server.id)?.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    println!("  - {} ({}): {}", server.display_name, server.id, status);
                }
            }
        }
    }

    Ok(())
}

async fn handle_mcp_oauth(command: McpOauthCommands) -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let store = McpTokenStore::new();
    match command {
        McpOauthCommands::List { server } => {
            if let Some(server) = server {
                let server_config = resolve_mcp_server(&config, &server)?;
                if let Some(grant) = store.get_oauth_grant(&server_config.id)? {
                    println!(
                        "MCP OAuth for {} ({}): configured",
                        server_config.display_name, server_config.id
                    );
                    let scope = grant.scope.as_deref().unwrap_or("n/a");
                    println!("  scope: {scope}");
                    let expires = grant
                        .expires_at_epoch_s
                        .map(|epoch| epoch.to_string())
                        .unwrap_or_else(|| "n/a".to_string());
                    println!("  expires_at_epoch_s: {expires}");
                } else {
                    println!(
                        "MCP OAuth for {} ({}): missing",
                        server_config.display_name, server_config.id
                    );
                }
            } else {
                let servers = config.list_mcp_servers();
                if servers.is_empty() {
                    println!("No MCP servers configured.");
                    return Ok(());
                }
                println!("MCP OAuth grant status:");
                for server in servers {
                    let status = if store.get_oauth_grant(&server.id)?.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    println!("  - {} ({}): {}", server.display_name, server.id, status);
                }
            }
        }
        McpOauthCommands::Add { server, advanced } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            add_oauth_grant_for_server(server_config, None, true, advanced).await?;
        }
        McpOauthCommands::Remove { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            remove_oauth_grant_for_server(server_config).await?;
        }
    }
    Ok(())
}

async fn add_oauth_grant_for_server(
    server: &McpServerConfig,
    metadata: Option<OAuthMetadata>,
    check_existing: bool,
    advanced: bool,
) -> Result<(), Box<dyn Error>> {
    let store = McpTokenStore::new();

    if check_existing && store.get_oauth_grant(&server.id)?.is_some() {
        println!(
            "OAuth grant already exists for {} ({}).",
            server.display_name, server.id
        );
        if prompt_bool_with_default("Remove existing grant now", false)? {
            remove_oauth_grant_for_server(server).await?;
        } else {
            println!("Cancelled.");
        }
        return Ok(());
    }

    let metadata = if let Some(metadata) = metadata {
        metadata
    } else {
        match probe_oauth_support(server).await? {
            Some(metadata) => metadata,
            None => {
                return Err(format!(
                    "No OAuth metadata discovered for {} ({})",
                    server.display_name, server.id
                )
                .into());
            }
        }
    };

    if let Some(authorization_endpoint) = metadata.authorization_endpoint.as_deref() {
        let token_endpoint = metadata
            .token_endpoint
            .as_deref()
            .ok_or("OAuth metadata is missing token_endpoint.")?;
        let mut client_id = if advanced {
            let client_id_input = prompt_optional("OAuth client id (optional): ")?;
            if client_id_input.is_empty() {
                None
            } else {
                Some(client_id_input)
            }
        } else {
            println!(
                "Basic mode: trying automatic OAuth client registration. Re-run with `-a` to provide a client id manually."
            );
            None
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");

        if client_id.is_none() {
            if let Some(registration_endpoint) = metadata.registration_endpoint.as_deref() {
                match register_oauth_client(registration_endpoint, &redirect_uri).await {
                    Ok(registered_id) => {
                        println!("Registered OAuth client automatically.");
                        client_id = Some(registered_id);
                    }
                    Err(err) => {
                        eprintln!("⚠️ OAuth client registration failed: {err}");
                    }
                }
            } else if !advanced {
                println!(
                    "OAuth metadata does not advertise dynamic registration. Re-run with `-a` to provide a client id if authorization fails."
                );
            }
        }

        let scope = metadata.scopes_supported.as_ref().and_then(|scopes| {
            let joined = scopes
                .iter()
                .map(|scope| scope.trim())
                .filter(|scope| !scope.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        });

        let state = random_urlsafe(24);
        let code_verifier = random_urlsafe(64);
        let code_challenge = pkce_s256_challenge(&code_verifier);
        let authorization_url = build_authorization_url(AuthorizationUrlParams {
            authorization_endpoint,
            client_id: client_id.as_deref(),
            redirect_uri: &redirect_uri,
            state: &state,
            code_challenge: &code_challenge,
            code_challenge_method: "S256",
            issuer: metadata.issuer.as_deref(),
            scope: scope.as_deref(),
        })?;

        if open_in_browser(authorization_url.as_str()).is_ok() {
            println!("Opened OAuth authorization URL in your browser.");
        } else {
            eprintln!(
                "⚠️ Could not launch browser automatically. Open this URL manually:
{}",
                authorization_url
            );
        }

        println!("Waiting for OAuth redirect on {redirect_uri} ...");
        let auth_code = wait_for_oauth_callback(listener, &state).await?;
        let token = exchange_oauth_code(
            token_endpoint,
            client_id.as_deref(),
            &redirect_uri,
            &auth_code,
            &code_verifier,
        )
        .await?;

        let now_epoch_s = current_unix_epoch_s().unwrap_or_default();
        let grant_seed = McpOAuthGrant {
            access_token: String::new(),
            refresh_token: None,
            token_type: None,
            scope: None,
            expires_at_epoch_s: None,
            client_id,
            redirect_uri: Some(redirect_uri),
            authorization_endpoint: metadata.authorization_endpoint.clone(),
            token_endpoint: metadata.token_endpoint.clone(),
            revocation_endpoint: metadata.revocation_endpoint.clone(),
            issuer: metadata.issuer.clone(),
        };
        let grant = apply_oauth_token_response(&grant_seed, token, now_epoch_s);
        store.set_oauth_grant(&server.id, &grant)?;
        store.set_token(&server.id, &grant.access_token)?;
        println!("✅ Stored OAuth grant for {}", server.display_name);
        Ok(())
    } else {
        Err("OAuth metadata is missing authorization_endpoint.".into())
    }
}

async fn remove_oauth_grant_for_server(server: &McpServerConfig) -> Result<(), Box<dyn Error>> {
    let store = McpTokenStore::new();
    let grant = match store.get_oauth_grant(&server.id)? {
        Some(grant) => grant,
        None => {
            println!("No OAuth grant stored for {}.", server.display_name);
            return Ok(());
        }
    };

    if let Some(revocation_endpoint) = grant.revocation_endpoint.as_deref() {
        let client = reqwest::Client::new();
        match client
            .post(revocation_endpoint)
            .form(&[("token", grant.access_token.as_str())])
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                println!("OAuth token revoked at server endpoint.");
            }
            Ok(response) => {
                eprintln!(
                    "⚠️ OAuth revocation returned HTTP {}. Removing local grant anyway.",
                    response.status()
                );
            }
            Err(err) => {
                eprintln!("⚠️ OAuth revocation failed ({err}). Removing local grant anyway.");
            }
        }
    } else {
        eprintln!("⚠️ No revocation endpoint in OAuth metadata. Removing local grant only.");
    }

    store.remove_oauth_grant(&server.id)?;
    let _ = store.remove_token(&server.id)?;
    println!("✅ Removed OAuth grant for {}", server.display_name);
    Ok(())
}

fn validate_mcp_server_id(input: &str) -> Result<String, Box<dyn Error>> {
    if !input
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return Err("Server id must contain only letters, numbers, '-' or '_'.".into());
    }
    Ok(input.to_ascii_lowercase())
}

fn prompt_required(prompt: &str) -> Result<String, Box<dyn Error>> {
    loop {
        let value = prompt_optional(prompt)?;
        if value.is_empty() {
            println!("Value cannot be empty.");
            continue;
        }
        return Ok(value);
    }
}

fn prompt_optional(prompt: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_bool_with_default(label: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    let default_hint = if default { "Y/n" } else { "y/N" };
    loop {
        let input = prompt_optional(&format!("{label} [{default_hint}]: "))?;
        if input.is_empty() {
            return Ok(default);
        }
        match input.to_ascii_lowercase().as_str() {
            "y" | "yes" | "on" | "true" => return Ok(true),
            "n" | "no" | "off" | "false" => return Ok(false),
            _ => println!("Please enter yes or no."),
        }
    }
}

fn prompt_transport(current: Option<&str>) -> Result<&'static str, Box<dyn Error>> {
    let default = current.unwrap_or("streamable-http");
    loop {
        let input = prompt_optional(&format!("Transport [streamable-http|stdio] [{default}]: "))?;
        let normalized = if input.is_empty() {
            default.to_ascii_lowercase()
        } else {
            input.to_ascii_lowercase()
        };
        match normalized.as_str() {
            "streamable-http" | "streamable_http" | "http" => return Ok("streamable-http"),
            "stdio" => return Ok("stdio"),
            _ => println!("Unsupported transport. Enter streamable-http or stdio."),
        }
    }
}

fn configure_mcp_transport_fields(
    server: &mut McpServerConfig,
    is_edit: bool,
    advanced: bool,
) -> Result<(), Box<dyn Error>> {
    match server.transport.as_deref().unwrap_or("streamable-http") {
        "stdio" => {
            server.base_url = None;
            let command_prompt = if is_edit {
                format!(
                    "Command [{}]: ",
                    server.command.as_deref().unwrap_or_default()
                )
            } else {
                "Command: ".to_string()
            };
            let command_input = prompt_optional(&command_prompt)?;
            if is_edit {
                if !command_input.is_empty() {
                    server.command = Some(command_input);
                }
            } else if command_input.is_empty() {
                return Err("Command cannot be empty for stdio transport.".into());
            } else {
                server.command = Some(command_input);
            }

            let args_default = server
                .args
                .as_ref()
                .map(|args| args.join(" "))
                .unwrap_or_default();
            let args_input =
                prompt_optional(&format!("Args (space-separated) [{}]: ", args_default))?;
            if !args_input.is_empty() {
                server.args = Some(
                    args_input
                        .split_whitespace()
                        .map(ToString::to_string)
                        .collect(),
                );
            } else if !is_edit {
                server.args = None;
            }

            if is_edit || advanced {
                let env_default = server
                    .env
                    .as_ref()
                    .map(|env| {
                        let mut pairs: Vec<String> = env
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect();
                        pairs.sort();
                        pairs.join(",")
                    })
                    .unwrap_or_default();
                let env_input = prompt_optional(&format!(
                    "Env (KEY=VALUE, comma-separated) [{}]: ",
                    env_default
                ))?;
                if !env_input.is_empty() {
                    server.env = Some(parse_env_pairs(&env_input)?);
                } else if !is_edit {
                    server.env = None;
                }
            }
        }
        _ => {
            server.command = None;
            server.args = None;
            server.env = None;
            let base_prompt = if is_edit {
                format!(
                    "Base URL [{}]: ",
                    server.base_url.as_deref().unwrap_or_default()
                )
            } else {
                "Base URL: ".to_string()
            };
            let base_input = prompt_optional(&base_prompt)?;
            if is_edit {
                if !base_input.is_empty() {
                    server.base_url = Some(base_input);
                }
                if server.base_url.as_deref().unwrap_or_default().is_empty() {
                    return Err("Base URL cannot be empty for HTTP transport.".into());
                }
            } else if base_input.is_empty() {
                return Err("Base URL cannot be empty for HTTP transport.".into());
            } else {
                server.base_url = Some(base_input);
            }
        }
    }

    if is_edit || advanced {
        let protocol_default = server.protocol_version.as_deref().unwrap_or_default();
        let protocol_input = prompt_optional(&format!(
            "Protocol version (optional) [{}]: ",
            protocol_default
        ))?;
        if !protocol_input.is_empty() {
            server.protocol_version = Some(protocol_input);
        } else if !is_edit {
            server.protocol_version = None;
        }

        let tools_default = server
            .allowed_tools
            .as_ref()
            .map(|tools| tools.join(","))
            .unwrap_or_default();
        let tools_input = prompt_optional(&format!(
            "Allowed tools (comma-separated, optional) [{}]: ",
            tools_default
        ))?;
        if !tools_input.is_empty() {
            let tools: Vec<String> = tools_input
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect();
            server.allowed_tools = if tools.is_empty() { None } else { Some(tools) };
        } else if !is_edit {
            server.allowed_tools = None;
        }
    }

    Ok(())
}

fn parse_env_pairs(
    input: &str,
) -> Result<std::collections::HashMap<String, String>, Box<dyn Error>> {
    let mut env = std::collections::HashMap::new();
    for pair in input
        .split(',')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!("Invalid env entry '{pair}'. Expected KEY=VALUE.").into());
        };
        let key = key.trim();
        if key.is_empty() {
            return Err("Environment variable name cannot be empty.".into());
        }
        env.insert(key.to_string(), value.trim().to_string());
    }
    Ok(env)
}

fn resolve_mcp_server<'a>(
    config: &'a Config,
    input: &str,
) -> Result<&'a McpServerConfig, Box<dyn Error>> {
    if let Some(server) = config.get_mcp_server(input) {
        return Ok(server);
    }

    let known: Vec<String> = config
        .list_mcp_servers()
        .iter()
        .map(|server| format!("{} ({})", server.display_name, server.id))
        .collect();

    eprintln!("❌ MCP server '{}' not found.", input);
    if known.is_empty() {
        eprintln!("   Add MCP servers to config.toml first.");
    } else {
        eprintln!("   Available MCP servers:");
        for server in known {
            eprintln!("   {}", server);
        }
    }
    std::process::exit(1);
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
        let args =
            Args::try_parse_from(["chabeau", "mcp", "oauth", "add", "agpedia", "-a"]).unwrap();
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
