//! Command-line interface parsing and handling
//!
//! This module handles parsing command-line arguments and executing the appropriate commands.

pub mod model_list;
pub mod pick_default_model;
pub mod pick_default_provider;
pub mod provider_list;

use std::error::Error;

use clap::{Parser, Subcommand};

// Import specific items we need
use crate::auth::AuthManager;
use crate::cli::model_list::list_models;
use crate::cli::pick_default_model::pick_default_model;
use crate::cli::pick_default_provider::pick_default_provider;
use crate::cli::provider_list::list_providers;
use crate::core::config::Config;
use crate::ui::chat_loop::run_chat;

fn print_version_info() {
    println!("chabeau {}", env!("CARGO_PKG_VERSION"));

    let git_describe = env!("VERGEN_GIT_DESCRIBE");
    let git_sha = env!("VERGEN_GIT_SHA");
    let git_branch = env!("VERGEN_GIT_BRANCH");

    // Determine build type
    let build_type = match git_describe {
        "unknown" => "Distribution build",
        desc if desc.starts_with('v') && !desc.contains('-') && !desc.contains("dirty") => "Release build",
        _ => "Development build",
    };
    println!("{}", build_type);

    // Show git information if available
    if git_sha != "unknown" {
        println!("Git commit: {}", &git_sha[..7.min(git_sha.len())]);

        if !git_branch.is_empty() && git_branch != "unknown" {
            println!("Git branch: {}", git_branch);
        }

        if git_describe != git_sha {
            println!("Git describe: {}", git_describe);
        }
    }

    println!("Build timestamp: {}", env!("VERGEN_BUILD_TIMESTAMP"));
    println!("Rust version: {}", env!("VERGEN_RUSTC_SEMVER"));
    println!("Target triple: {}", env!("VERGEN_CARGO_TARGET_TRIPLE"));
    println!("Build profile: {}", if cfg!(debug_assertions) { "debug" } else { "release" });

    println!();
    println!("Chabeau is a Permacommons project and free forever.");
    println!("See https://permacommons.org/ for more information.");
}

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = "A terminal-based chat interface using OpenAI API")]
#[command(disable_version_flag = true)]
#[command(
    long_about = "Chabeau is a full-screen terminal chat interface that connects to various AI APIs \
for real-time conversations. It supports streaming responses and provides a clean, \
responsive interface with color-coded messages.\n\n\
Authentication:\n\
  Use 'chabeau auth' to set up API credentials securely in your system keyring.\n\
  Supports OpenAI, OpenRouter, Poe, Anthropic, and custom providers.\n\n\
Environment Variables (fallback if no auth configured):\n\
  OPENAI_API_KEY    Your OpenAI API key\n\
  OPENAI_BASE_URL   Custom API base URL (optional, defaults to https://api.openai.com/v1)\n\n\
Controls:\n\
  Type              Enter your message in the input field\n\
  Enter             Send the message\n\
  Up/Down/Mouse     Scroll through chat history\n\
  Ctrl+C            Quit the application\n\
  Ctrl+R            Retry the last bot response\n\
  Ctrl+T            Open external editor (requires EDITOR env var)\n\
  Backspace         Delete characters in the input field\n\n\
Commands:\n\
  /help             Show extended help with keyboard shortcuts\n\
  /log <filename>   Enable logging to specified file\n\
  /log              Toggle logging pause/resume"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Model to use for chat, or list available models if no model specified
    #[arg(short = 'm', long, global = true, value_name = "MODEL", num_args = 0..=1, default_missing_value = "")]
    pub model: Option<String>,

    /// Enable logging to specified file
    #[arg(short = 'l', long, global = true)]
    pub log: Option<String>,

    /// Provider to use, or list available providers if no provider specified
    #[arg(short = 'p', long, global = true, value_name = "PROVIDER", num_args = 0..=1, default_missing_value = "")]
    pub provider: Option<String>,

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
    /// Start the chat interface (default)
    Chat,
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

    match args.command.unwrap_or(Commands::Chat) {
        Commands::Auth => {
            let mut auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_auth() {
                eprintln!("❌ Authentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Deauth => {
            let mut auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_deauth(args.provider) {
                eprintln!("❌ Deauthentication failed: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Set { key, value } => {
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
        Commands::Unset { key, value } => {
            let mut config = Config::load()?;
            match key.as_str() {
                "default-provider" => {
                    config.default_provider = None;
                    config.save()?;
                    println!("✅ Unset default-provider");
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
                _ => {
                    eprintln!("❌ Unknown config key: {key}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        Commands::Chat => {
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
                            run_chat(model.to_string(), args.log, provider_for_operations).await
                        }
                        None => {
                            // -m was not provided, use default model for chat
                            run_chat("default".to_string(), args.log, provider_for_operations).await
                        }
                    }
                }
            }
        }
        Commands::PickDefaultModel { provider } => {
            pick_default_model(provider).await?;
            Ok(())
        }
        Commands::PickDefaultProvider => {
            pick_default_provider().await?;
            Ok(())
        }
    }
}
