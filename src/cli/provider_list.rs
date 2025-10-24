use std::error::Error;

use crate::auth::AuthManager;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let (providers, default_provider) = auth_manager.get_all_providers_with_auth_status();

    if providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    println!(
        "{:<20} {:<30} {:<15} {}",
        "Provider", "Display Name", "Authenticated", "Default"
    );
    println!(
        "{:<20} {:<30} {:<15} {}",
        "--------", "------------", "-------------", "-------"
    );

    for (id, display_name, has_token) in providers {
        let auth_status = if has_token { "✅" } else { "❌" };
        let is_default = default_provider
            .as_ref()
            .map_or(false, |d| d.eq_ignore_ascii_case(&id));
        let default_status = if is_default { "✓" } else { "" };
        println!(
            "{:<20} {:<30} {:<15} {}",
            id, display_name, auth_status, default_status
        );
    }

    Ok(())
}
