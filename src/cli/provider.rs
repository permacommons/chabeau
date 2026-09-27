//! `chabeau provider` subcommands: listing, adding, editing and removing
//! providers, and managing their stored tokens.

use std::error::Error;

use super::prompts::{prompt_bool_with_default, prompt_optional, prompt_required};
use super::{resolve_provider_id, ProviderCommands, ProviderTokenCommands};
use crate::auth::prompt_provider_token;
use crate::auth::AuthManager;
use crate::cli::provider_list::list_providers;
use crate::core::builtin_providers::{find_builtin_provider, load_builtin_providers};
use crate::core::config::data::{Config, CustomProvider};
use crate::utils::url::normalize_base_url;

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

pub(super) async fn handle_provider_command(
    command: ProviderCommands,
) -> Result<(), Box<dyn Error>> {
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
