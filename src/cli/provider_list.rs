use std::error::Error;

use crate::auth::AuthManager;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let providers = auth_manager.get_all_providers_with_auth_status();

    if providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    println!("{:<20} {:<30} {}", "Provider", "Display Name", "Authenticated");
    println!("{:<20} {:<30} {}", "--------", "------------", "-------------");

    for (id, display_name, has_token) in providers {
        let auth_status = if has_token { "✅" } else { "❌" };
        println!("{:<20} {:<30} {}", id, display_name, auth_status);
    }

    Ok(())
}
