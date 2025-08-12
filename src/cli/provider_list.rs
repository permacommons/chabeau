//! Provider listing functionality
//!
//! This module handles listing available providers and their authentication status.

use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::Config;
use std::error::Error;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let config = Config::load()?;

    println!("🔗 Available Providers");
    println!("━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // Check built-in providers
    let builtin_providers = load_builtin_providers();

    for provider in builtin_providers {
        let status = match auth_manager.get_token(&provider.id) {
            Ok(Some(_)) => "✅ configured",
            Ok(None) => "❌ not configured",
            Err(_) => "❓ error checking",
        };
        println!("  {} ({}) - {status}", provider.display_name, provider.id);
        println!("    URL: {}", provider.base_url);
        if let Some(mode) = &provider.mode {
            println!("    Auth mode: {mode}");
        }
        println!();
    }

    // Check for custom providers
    match auth_manager.list_custom_providers() {
        Ok(custom_providers) => {
            if custom_providers.is_empty() {
                println!("Custom providers: none configured");
            } else {
                println!("Custom providers:");
                for (name, url, has_token) in custom_providers {
                    let status = if has_token {
                        "✅ configured"
                    } else {
                        "❌ not configured"
                    };
                    println!("  {name} - {status}");
                    println!("    URL: {url}");
                }
            }
        }
        Err(_) => {
            println!("Custom providers: error checking");
        }
    }
    println!();

    // Show which provider would be used by default
    if let Some(default_provider) = &config.default_provider {
        println!("🎯 Default provider: {default_provider} (from config)");
    } else {
        match auth_manager.find_first_available_auth() {
            Some((provider, _)) => {
                println!(
                    "🎯 Default provider: {} ({})",
                    provider.display_name, provider.name
                );
            }
            None => {
                println!("⚠️  No configured providers found");
                println!();
                println!("To configure authentication:");
                println!("  chabeau auth                    # Interactive setup");
                println!();
                println!("Or use environment variables:");
                println!("  export OPENAI_API_KEY=sk-...   # For OpenAI");
            }
        }
    }

    Ok(())
}
