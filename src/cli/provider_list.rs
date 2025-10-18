//! Provider listing functionality
//!
//! This module handles listing available providers and their authentication status.

use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::data::Config;
use std::error::Error;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
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
    let custom_providers = auth_manager.list_custom_providers();
    if custom_providers.is_empty() {
        println!("Custom providers: none configured");
    } else {
        println!("Custom providers:");
        for (id, display_name, url, has_token) in custom_providers {
            let status = if has_token {
                "✅ configured"
            } else {
                "❌ not configured"
            };
            println!("  {} ({}) - {status}", display_name, id);
            println!("    URL: {url}");
        }
    }
    println!();

    // Show which provider would be used by default
    match auth_manager.resolve_authentication(None, &config) {
        Ok((_, _, provider_id, provider_display_name)) => {
            println!("🎯 Default provider: {provider_display_name} ({provider_id})");
        }
        Err(_) => {
            println!("⚠️  No configured providers found");
            println!();
            println!("To configure authentication:");
            println!("  chabeau auth                    # Interactive setup");
            println!();
            println!("Or use environment variables:");
            println!("  export OPENAI_API_KEY=sk-...   # For OpenAI");
        }
    }

    Ok(())
}
