//! Provider listing functionality
//!
//! This module handles listing available providers and their authentication status.

use crate::auth::AuthManager;
use crate::core::config::Config;
use std::error::Error;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let config = Config::load()?;

    println!("🔗 Available Providers");
    println!("━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // Check built-in providers
    let builtin_providers = vec![
        ("openai", "OpenAI", "https://api.openai.com/v1"),
        ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
        ("poe", "Poe", "https://api.poe.com/v1"),
        ("anthropic", "Anthropic", "https://api.anthropic.com/v1"),
    ];

    for (name, display_name, url) in builtin_providers {
        let status = match auth_manager.get_token(name) {
            Ok(Some(_)) => "✅ configured",
            Ok(None) => "❌ not configured",
            Err(_) => "❓ error checking",
        };
        println!("  {display_name} ({name}) - {status}");
        println!("    URL: {url}");
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
