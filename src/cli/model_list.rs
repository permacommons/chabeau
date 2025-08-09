//! Model listing functionality
//!
//! This module handles listing available models from various providers.

use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::api::models::{fetch_models, sort_models};
use chrono::{DateTime, Utc};
use std::error::Error;

pub async fn list_models(provider: Option<String>) -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let config = Config::load()?;

    let (api_key, base_url, provider_name) = if let Some(provider_name) = provider {
        // User specified a provider
        if let Some((base_url, api_key)) = auth_manager.get_auth_for_provider(&provider_name)? {
            (api_key, base_url, provider_name.clone())
        } else {
            return Err(format!("No authentication found for provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into());
        }
    } else if let Some(ref provider_name) = config.default_provider {
        // Config specifies a default provider
        if let Some((base_url, api_key)) = auth_manager.get_auth_for_provider(provider_name)? {
            // Get the proper display name for the provider
            let display_name =
                if let Some(provider) = auth_manager.find_provider_by_name(provider_name) {
                    provider.display_name.clone()
                } else {
                    // For custom providers, use the provider name as display name
                    provider_name.clone()
                };
            (api_key, base_url, display_name)
        } else {
            return Err(format!("No authentication found for default provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into());
        }
    } else {
        // Try to find any available authentication
        if let Some((provider, api_key)) = auth_manager.find_first_available_auth() {
            (api_key, provider.base_url, provider.display_name)
        } else {
            // Fall back to environment variables
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                "❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau auth' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
            })?;

            let base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

            (api_key, base_url, "Environment Variables".to_string())
        }
    };

    println!("🤖 Available Models for {provider_name}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // Show default model for this provider if set
    if let Some(default_model) = config.get_default_model(&provider_name) {
        println!("🎯 Default model for this provider: {default_model} (from config)");
        println!();
    }

    let client = reqwest::Client::new();
    let models_response = fetch_models(&client, &base_url, &api_key, &provider_name).await?;

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
