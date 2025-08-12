//! Default provider configuration
//!
//! This module handles the interactive selection and configuration of default providers.

use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::Config;
use std::error::Error;

pub async fn pick_default_provider() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let mut config = Config::load()?;

    // Get list of available providers
    let mut providers = Vec::new();

    // Add built-in providers that have authentication
    let builtin_providers = load_builtin_providers();

    for builtin_provider in builtin_providers {
        if auth_manager.get_token(&builtin_provider.id)?.is_some() {
            providers.push((builtin_provider.id, builtin_provider.display_name));
        }
    }

    // Add custom providers
    let custom_providers = auth_manager.list_custom_providers();
    for (id, display_name, _, has_token) in custom_providers {
        if has_token {
            providers.push((id, display_name));
        }
    }

    if providers.is_empty() {
        return Err(
            "No configured providers found. Run 'chabeau auth' to set up authentication.".into(),
        );
    }

    println!("🔧 Select Default Provider");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("Available configured providers:");
    for (i, (name, display_name)) in providers.iter().enumerate() {
        let current_marker = if config.default_provider.as_ref() == Some(name) {
            " (current default)"
        } else {
            ""
        };
        println!("  {}. {} ({}){}", i + 1, display_name, name, current_marker);
    }
    println!();

    print!("Select a provider to set as default (enter the number): ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().map_err(|_| "Invalid choice")?;

    if choice == 0 || choice > providers.len() {
        return Err("Invalid choice".into());
    }

    let selected_provider = &providers[choice - 1].0;
    config.default_provider = Some(selected_provider.to_lowercase());
    config.save()?;

    println!("✅ Set default provider to: {selected_provider}");
    Ok(())
}
