use crate::builtin_providers::load_builtin_providers;
use keyring::Entry;
use std::io::{self, Write};

type CustomProviderInfo = (String, String, bool);

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub display_name: String,
}

impl Provider {
    pub fn new(name: String, base_url: String, display_name: String) -> Self {
        Self {
            name,
            base_url,
            display_name,
        }
    }
}

pub struct AuthManager {
    providers: Vec<Provider>,
}

impl AuthManager {
    pub fn new() -> Self {
        // Load built-in providers from configuration
        let builtin_providers = load_builtin_providers();
        let providers = builtin_providers
            .into_iter()
            .map(|bp| Provider::new(bp.id, bp.base_url, bp.display_name))
            .collect();

        Self { providers }
    }

    pub fn find_provider_by_name(&self, name: &str) -> Option<&Provider> {
        self.providers
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn store_token(
        &self,
        provider_name: &str,
        token: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        entry.set_password(token)?;
        Ok(())
    }

    pub fn get_token(
        &self,
        provider_name: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn store_custom_provider(
        &self,
        name: &str,
        base_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", &format!("custom_provider_{name}"))?;
        entry.set_password(base_url)?;

        // Also add to the list of custom providers
        self.add_to_custom_provider_list(name)?;
        Ok(())
    }

    fn add_to_custom_provider_list(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let list_entry = Entry::new("chabeau", "custom_provider_list")?;
        let current_list = match list_entry.get_password() {
            Ok(list) => list,
            Err(keyring::Error::NoEntry) => String::new(),
            Err(e) => return Err(Box::new(e)),
        };

        let mut providers: Vec<&str> = if current_list.is_empty() {
            Vec::new()
        } else {
            current_list.split(',').collect()
        };

        // Add if not already present
        if !providers.contains(&name) {
            providers.push(name);
            let new_list = providers.join(",");
            list_entry.set_password(&new_list)?;
        }

        Ok(())
    }

    pub fn get_custom_provider(
        &self,
        name: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", &format!("custom_provider_{name}"))?;
        match entry.get_password() {
            Ok(base_url) => Ok(Some(base_url)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn list_custom_providers(
        &self,
    ) -> Result<Vec<CustomProviderInfo>, Box<dyn std::error::Error>> {
        let list_entry = Entry::new("chabeau", "custom_provider_list")?;
        let provider_list = match list_entry.get_password() {
            Ok(list) => list,
            Err(keyring::Error::NoEntry) => return Ok(Vec::new()),
            Err(e) => return Err(Box::new(e)),
        };

        if provider_list.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        for provider_name in provider_list.split(',') {
            let provider_name = provider_name.trim();
            if provider_name.is_empty() {
                continue;
            }

            // Get the base URL for this custom provider
            let base_url = match self.get_custom_provider(provider_name)? {
                Some(url) => url,
                None => continue, // Skip if URL not found
            };

            // Check if token is configured
            let has_token = self.get_token(provider_name)?.is_some();

            result.push((provider_name.to_string(), base_url, has_token));
        }

        Ok(result)
    }

    pub fn find_first_available_auth(&self) -> Option<(Provider, String)> {
        // Try built-in providers in order
        for provider in &self.providers {
            if let Ok(Some(token)) = self.get_token(&provider.name) {
                return Some((provider.clone(), token));
            }
        }
        None
    }

    pub fn interactive_auth(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔐 Chabeau Authentication Setup");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();

        // Show available providers
        println!("Available providers:");
        for (i, provider) in self.providers.iter().enumerate() {
            let has_token = self.get_token(&provider.name)?.is_some();
            let status = if has_token {
                "✓ configured"
            } else {
                "not configured"
            };
            println!(
                "  {}. {} ({}) - {}",
                i + 1,
                provider.display_name,
                provider.name,
                status
            );
        }
        println!("  {}. Custom provider", self.providers.len() + 1);
        println!();

        print!("Select a provider (1-{}): ", self.providers.len() + 1);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let choice: usize = input.trim().parse().map_err(|_| "Invalid choice")?;

        if choice == 0 || choice > self.providers.len() + 1 {
            return Err("Invalid choice".into());
        }

        if choice <= self.providers.len() {
            // Built-in provider
            let provider = &self.providers[choice - 1];
            self.setup_provider_auth(&provider.name, &provider.display_name)?;
        } else {
            // Custom provider
            self.setup_custom_provider()?;
        }

        println!();
        println!("✅ Authentication configured successfully!");
        println!("You can now use Chabeau without setting environment variables.");

        Ok(())
    }

    fn setup_provider_auth(
        &self,
        provider_name: &str,
        display_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!();
        print!("Enter your {display_name} API token: ");
        io::stdout().flush()?;

        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim();

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        self.store_token(provider_name, token)?;
        println!("✓ Token stored securely for {display_name}");

        Ok(())
    }

    fn setup_custom_provider(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!();
        print!("Enter a short name for your custom provider (no spaces): ");
        io::stdout().flush()?;

        let mut name = String::new();
        io::stdin().read_line(&mut name)?;
        let name = name.trim().to_lowercase(); // Normalize to lowercase

        if name.is_empty() || name.contains(' ') {
            return Err("Provider name cannot be empty or contain spaces".into());
        }

        print!("Enter the base URL for your custom provider: ");
        io::stdout().flush()?;

        let mut base_url = String::new();
        io::stdin().read_line(&mut base_url)?;
        let base_url = base_url.trim();

        if base_url.is_empty() {
            return Err("Base URL cannot be empty".into());
        }

        print!("Enter your API token for {name}: ");
        io::stdout().flush()?;

        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim();

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        // Store both the custom provider URL and token
        self.store_custom_provider(&name, base_url)?;
        self.store_token(&name, token)?;

        println!("✓ Custom provider '{name}' configured with URL: {base_url}");

        Ok(())
    }

    pub fn get_auth_for_provider(
        &self,
        provider_name: &str,
    ) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
        // First check if it's a built-in provider (case-insensitive)
        if let Some(provider) = self.find_provider_by_name(provider_name) {
            // Use the canonical provider name for token lookup
            if let Some(token) = self.get_token(&provider.name)? {
                return Ok(Some((provider.base_url.clone(), token)));
            }
        } else {
            // Check if it's a custom provider (case-sensitive for custom names)
            if let Some(base_url) = self.get_custom_provider(provider_name)? {
                if let Some(token) = self.get_token(provider_name)? {
                    return Ok(Some((base_url, token)));
                }
            }
        }

        Ok(None)
    }

    pub fn interactive_deauth(
        &self,
        provider: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(provider_name) = provider {
            // Provider specified via --provider flag - validate it exists first
            let has_auth = self.get_token(&provider_name)?.is_some();
            let is_custom = self.get_custom_provider(&provider_name)?.is_some();

            if !has_auth && !is_custom {
                return Err(format!("Provider '{provider_name}' is not configured. Use 'chabeau providers' to see configured providers.").into());
            }

            if !has_auth {
                return Err(format!(
                    "Provider '{provider_name}' exists but has no authentication configured."
                )
                .into());
            }

            self.remove_provider_auth(&provider_name)?;

            // Check if it's a custom provider and remove it completely
            if is_custom {
                self.remove_custom_provider(&provider_name)?;
            }

            println!("✅ Authentication removed for {provider_name}");
        } else {
            // Interactive mode - show menu of configured providers
            self.interactive_deauth_menu()?;
        }
        Ok(())
    }

    fn interactive_deauth_menu(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🗑️  Chabeau Authentication Removal");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();

        // Collect all configured providers
        let mut configured_providers = Vec::new();

        // Check built-in providers
        for provider in &self.providers {
            if self.get_token(&provider.name)?.is_some() {
                configured_providers.push((
                    provider.name.clone(),
                    provider.display_name.clone(),
                    false,
                ));
            }
        }

        // Check custom providers
        match self.list_custom_providers() {
            Ok(custom_providers) => {
                for (name, _url, has_token) in custom_providers {
                    if has_token {
                        configured_providers.push((name.clone(), name, true));
                    }
                }
            }
            Err(_) => {
                // Ignore errors listing custom providers
            }
        }

        if configured_providers.is_empty() {
            println!("No configured providers found.");
            return Ok(());
        }

        println!("Configured providers:");
        for (i, (_name, display_name, is_custom)) in configured_providers.iter().enumerate() {
            let provider_type = if *is_custom { " (custom)" } else { "" };
            println!("  {}. {}{}", i + 1, display_name, provider_type);
        }
        println!("  {}. Cancel", configured_providers.len() + 1);
        println!();

        print!(
            "Select a provider to remove (1-{}): ",
            configured_providers.len() + 1
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let choice: usize = input.trim().parse().map_err(|_| "Invalid choice")?;

        if choice == 0 || choice > configured_providers.len() + 1 {
            return Err("Invalid choice".into());
        }

        if choice == configured_providers.len() + 1 {
            println!("Cancelled.");
            return Ok(());
        }

        let (provider_name, display_name, is_custom) = &configured_providers[choice - 1];

        // Confirm removal
        print!("Are you sure you want to remove authentication for {display_name}? (y/N): ");
        io::stdout().flush()?;

        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;
        let confirm = confirm.trim().to_lowercase();

        if confirm != "y" && confirm != "yes" {
            println!("Cancelled.");
            return Ok(());
        }

        self.remove_provider_auth(provider_name)?;

        if *is_custom {
            // Also remove the custom provider URL and from the list
            self.remove_custom_provider(provider_name)?;
        }

        println!("✅ Authentication removed for {display_name}");
        Ok(())
    }

    fn remove_provider_auth(&self, provider_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => {
                // Already not configured, that's fine
                Ok(())
            }
            Err(e) => Err(Box::new(e)),
        }
    }

    fn remove_custom_provider(
        &self,
        provider_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Remove the custom provider URL
        let entry = Entry::new("chabeau", &format!("custom_provider_{provider_name}"))?;
        let _ = entry.delete_credential(); // Ignore errors

        // Remove from the custom provider list
        self.remove_from_custom_provider_list(provider_name)?;
        Ok(())
    }

    fn remove_from_custom_provider_list(
        &self,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let list_entry = Entry::new("chabeau", "custom_provider_list")?;
        let current_list = match list_entry.get_password() {
            Ok(list) => list,
            Err(keyring::Error::NoEntry) => return Ok(()), // No list exists
            Err(e) => return Err(Box::new(e)),
        };

        if current_list.is_empty() {
            return Ok(());
        }

        let providers: Vec<&str> = current_list.split(',').collect();
        let filtered_providers: Vec<&str> = providers
            .into_iter()
            .filter(|&p| p.trim() != name)
            .collect();

        if filtered_providers.is_empty() {
            // Remove the entire list entry if empty
            let _ = list_entry.delete_credential();
        } else {
            let new_list = filtered_providers.join(",");
            list_entry.set_password(&new_list)?;
        }

        Ok(())
    }
}
