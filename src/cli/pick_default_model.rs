//! Default model configuration
//!
//! This module handles the interactive selection and configuration of default models for providers.

use crate::api::models::{fetch_models, sort_models};
use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::Config;
use chrono::{DateTime, Utc};
use std::error::Error;

pub async fn pick_default_model(provider: Option<String>) -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let mut config = Config::load()?;

    // If no provider specified, prompt user to select one
    let provider_name = if let Some(provider_name) = provider {
        provider_name
    } else {
        // Get list of available providers that have authentication
        let mut providers = Vec::new();

        // Add built-in providers that have authentication
        let builtin_providers = load_builtin_providers();

        for builtin_provider in builtin_providers {
            if auth_manager.get_token(&builtin_provider.id)?.is_some() {
                providers.push((builtin_provider.id, builtin_provider.display_name));
            }
        }

        // Add custom providers that have authentication
        let custom_providers = auth_manager.list_custom_providers();
        for (id, display_name, _, has_token) in custom_providers {
            if has_token {
                providers.push((id, display_name));
            }
        }

        if providers.is_empty() {
            return Err(
                "No configured providers found. Run 'chabeau auth' to set up authentication."
                    .into(),
            );
        }

        println!("Select a provider to set default model for:");
        for (i, (name, display_name)) in providers.iter().enumerate() {
            println!("  {}. {} ({})", i + 1, display_name, name);
        }

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let choice: usize = input.trim().parse().map_err(|_| "Invalid choice")?;

        if choice == 0 || choice > providers.len() {
            return Err("Invalid choice".into());
        }

        providers[choice - 1].0.clone()
    };

    // Use the shared authentication resolution function
    let (api_key, base_url, _, display_name) =
        auth_manager.resolve_authentication(Some(&provider_name), &config)?;

    println!("🤖 Available Models for {display_name}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let client = reqwest::Client::new();
    let models_response = fetch_models(&client, &base_url, &api_key, &provider_name).await?;

    if models_response.data.is_empty() {
        println!("No models found for this provider.");
        return Ok(());
    }

    println!(
        "Found {} models (sorted newest first):",
        models_response.data.len()
    );
    println!();

    // Sort models by creation date (newest first), then by ID for consistent display
    let mut models = models_response.data;
    sort_models(&mut models);

    // Display models with indices
    for (i, model) in models.iter().enumerate() {
        println!("  {}. {}", i + 1, model.id);
        if let Some(display_name) = &model.display_name {
            if !display_name.is_empty() && display_name != &model.id {
                println!("     Name: {display_name}");
            }
        }
        if let Some(owned_by) = &model.owned_by {
            if !owned_by.is_empty() && owned_by != "system" {
                println!("     Owner: {owned_by}");
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
                    println!("     Created: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
                }
            }
        } else if let Some(created_at) = &model.created_at {
            if !created_at.is_empty() {
                println!("     Created: {created_at}");
            }
        }
        println!();
    }

    // Prompt user to select a model
    println!("Select a model to set as default (enter the number):");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().map_err(|_| "Invalid choice")?;

    if choice == 0 || choice > models.len() {
        return Err("Invalid choice".into());
    }

    let selected_model = &models[choice - 1].id;
    config.set_default_model(provider_name.to_lowercase(), selected_model.clone());
    config.save()?;

    println!("✅ Set default model for provider to: {selected_model}");
    Ok(())
}
