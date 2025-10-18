//! Model listing functionality
//!
//! This module handles listing available models from various providers.

use crate::api::models::{fetch_models, sort_models};
use crate::auth::AuthManager;
use crate::core::config::data::Config;
use crate::core::providers::ProviderResolutionError;
use chrono::{DateTime, Utc};
use std::error::Error;

pub async fn list_models(provider: Option<String>) -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let config = Config::load()?;

    // Use the shared authentication resolution function
    let (api_key, base_url, provider_internal_name, provider_display_name) =
        match auth_manager.resolve_authentication(provider.as_deref(), &config) {
            Ok(values) => values,
            Err(err) => {
                if let Some(resolution_error) = err.downcast_ref::<ProviderResolutionError>() {
                    eprintln!("{}", resolution_error);
                    let fixes = resolution_error.quick_fixes();
                    if !fixes.is_empty() {
                        eprintln!();
                        eprintln!("💡 Quick fixes:");
                        for fix in fixes {
                            eprintln!("  • {fix}");
                        }
                    }
                    std::process::exit(resolution_error.exit_code());
                }
                return Err(err);
            }
        };

    let provider_name = provider_display_name.clone();

    println!("🤖 Available Models for {provider_name}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // Show default model for this provider if set
    if let Some(default_model) = config.get_default_model(&provider_internal_name) {
        println!("🎯 Default model for this provider: {default_model} (from config)");
        println!();
    }

    let client = reqwest::Client::new();
    let models_response =
        fetch_models(&client, &base_url, &api_key, &provider_internal_name).await?;

    if models_response.data.is_empty() {
        println!("No models found for this provider.");
    } else {
        println!(
            "Found {} models (sorted newest first):",
            models_response.data.len()
        );
        println!();

        // Sort models by creation date (newest first), then by ID for consistent display
        let mut models = models_response.data;
        sort_models(&mut models);

        for model in models {
            println!("  • {}", model.id);
            if let Some(display_name) = &model.display_name {
                if !display_name.is_empty() && display_name != &model.id {
                    println!("    Name: {display_name}");
                }
            }
            if let Some(owned_by) = &model.owned_by {
                if !owned_by.is_empty() && owned_by != "system" {
                    println!("    Owner: {owned_by}");
                }
            }
            // Handle both created (OpenAI-style) and created_at (Anthropic-style) fields
            if let Some(created) = model.created {
                if created > 0 {
                    // Convert Unix timestamp to human-readable date
                    // Some APIs return timestamps in milliseconds, others in seconds
                    let timestamp_secs = if created > 10_000_000_000 {
                        // Likely milliseconds, convert to seconds
                        created / 1000
                    } else {
                        // Already in seconds
                        created
                    };

                    let datetime = DateTime::<Utc>::from_timestamp(timestamp_secs as i64, 0);
                    if let Some(dt) = datetime {
                        println!("    Created: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
                    }
                }
            } else if let Some(created_at) = &model.created_at {
                if !created_at.is_empty() {
                    println!("    Created: {created_at}");
                }
            }
            println!();
        }
    }

    Ok(())
}
