//! Command-line interface parsing and handling
//!
//! This module handles parsing command-line arguments and executing the appropriate commands.

pub mod model_list;
pub mod pick_default_model;
pub mod pick_default_provider;
pub mod provider_list;
pub mod theme_list;

use std::error::Error;

use clap::{Parser, Subcommand};

// Import specific items we need
use crate::auth::AuthManager;
use crate::cli::model_list::list_models;
use crate::cli::pick_default_model::pick_default_model;
use crate::cli::pick_default_provider::pick_default_provider;
use crate::cli::provider_list::list_providers;
use crate::cli::theme_list::list_themes;
use crate::core::config::Config;
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
const HELP_ABOUT: &str = "Chabeau is a full-screen terminal chat interface for OpenAI‑compatible APIs.\n\n\
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
  Tips:\n\
  • To make a choice the default, select it with [Alt+Enter], or use the CLI commands below.\n\
  • Inside the TUI, type '/help' for keys and commands.\n\
  • '-p [PROVIDER]' and '-m [MODEL]' select provider/model; '-p' or '-m' alone list them.\n";

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = HELP_ABOUT)]
#[command(disable_version_flag = true, disable_help_flag = true)]
#[command(long_about = HELP_ABOUT)]
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
    /// Interactively select and set a default model
    PickDefaultModel {
        /// Provider to list models for (optional)
        provider: Option<String>,
    },
    /// Interactively select and set a default provider
    PickDefaultProvider,
    /// List available themes (built-in and custom)
    Themes,
    /// Import and validate a character card
    Import {
        /// Path to character card file (JSON or PNG)
        #[arg(short = 'c', long)]
        card: String,
        /// Force overwrite if card already exists
        #[arg(short = 'f', long)]
        force: bool,
    },
}

pub fn main() -> Result<(), Box<dyn Error>> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // Handle version flag
    if args.version {
        print_version_info();
        return Ok(());
    }

    match args.command {
        Some(Commands::Auth) => {
            let mut auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_auth() {
                eprintln!("❌ Authentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Deauth) => {
            let mut auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_deauth(args.provider) {
                eprintln!("❌ Deauthentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Set { key, value }) => {
            let mut config = Config::load()?;
            if let Some(key) = key {
                match key.as_str() {
                    "default-provider" => {
                        if !value.is_empty() {
                            config.default_provider = Some(value.join(" "));
                            config.save()?;
                            println!("✅ Set default-provider to: {}", value.join(" "));
                        } else {
                            config.print_all();
                        }
                    }
                    "theme" => {
                        if !value.is_empty() {
                            let theme_name = value.join(" ");
                            config.theme = Some(theme_name.clone());
                            config.save()?;
                            println!("✅ Set theme to: {}", theme_name);
                        } else {
                            config.print_all();
                        }
                    }
                    "default-model" => {
                        if !value.is_empty() {
                            let val_str = value.join(" ");
                            let parts: Vec<&str> = val_str.splitn(2, ' ').collect();
                            if parts.len() == 2 {
                                let provider = parts[0].to_string();
                                let model = parts[1].to_string();
                                config.set_default_model(provider.clone(), model.clone());
                                config.save()?;
                                println!(
                                    "✅ Set default-model for provider '{}' to: {}",
                                    provider, model
                                );
                            } else {
                                eprintln!(
                                    "⚠️  To set a default model, specify the provider and model:"
                                );
                                eprintln!("Example: chabeau set default-model openai gpt-4o");
                            }
                        } else {
                            config.print_all();
                        }
                    }
                    "default-character" => {
                        if value.len() >= 3 {
                            let provider = value[0].to_string();
                            let model = value[1].to_string();
                            let character = value[2..].join(" ");

                            // Validate that the character exists
                            match crate::character::loader::find_card_by_name(&character) {
                                Ok(_) => {
                                    config.set_default_character(
                                        provider.clone(),
                                        model.clone(),
                                        character.clone(),
                                    );
                                    config.save()?;
                                    println!(
                                        "✅ Set default character for '{}:{}' to: {}",
                                        provider, model, character
                                    );
                                }
                                Err(_) => {
                                    eprintln!(
                                        "❌ Character '{}' not found in cards directory",
                                        character
                                    );
                                    eprintln!(
                                        "   Run 'chabeau import -c <file>' to import a character card first"
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
                config.print_all();
            }
            Ok(())
        }
        Some(Commands::Unset { key, value }) => {
            let mut config = Config::load()?;
            match key.as_str() {
                "default-provider" => {
                    config.default_provider = None;
                    config.save()?;
                    println!("✅ Unset default-provider");
                }
                "theme" => {
                    config.theme = None;
                    config.save()?;
                    println!("✅ Unset theme");
                }
                "default-model" => {
                    if let Some(provider) = value {
                        config.unset_default_model(&provider);
                        config.save()?;
                        println!("✅ Unset default-model for provider: {provider}");
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
                            config.unset_default_character(provider, model);
                            config.save()?;
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

                    match args.model.as_deref() {
                        Some("") => {
                            // -m was provided without a value, list available models
                            list_models(provider_for_operations).await
                        }
                        Some(model) => {
                            // -m was provided with a value, use it for chat
                            run_chat(
                                model.to_string(),
                                args.log,
                                provider_for_operations,
                                args.env_only,
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
                            )
                            .await
                        }
                    }
                }
            }
        }
        Some(Commands::PickDefaultModel { provider }) => {
            pick_default_model(provider).await?;
            Ok(())
        }
        Some(Commands::PickDefaultProvider) => {
            pick_default_provider().await?;
            Ok(())
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
    }
}
