use keyring::Entry;
use std::io::{self, Write};

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
        let providers = vec![
            Provider::new(
                "openai".to_string(),
                "https://api.openai.com/v1".to_string(),
                "OpenAI".to_string(),
            ),
            Provider::new(
                "openrouter".to_string(),
                "https://openrouter.ai/api/v1".to_string(),
                "OpenRouter".to_string(),
            ),
            Provider::new(
                "poe".to_string(),
                "https://api.poe.com/v1".to_string(),
                "Poe".to_string(),
            ),
        ];

        Self { providers }
    }

    pub fn find_provider_by_name(&self, name: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn store_token(&self, provider_name: &str, token: &str) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        entry.set_password(token)?;
        Ok(())
    }

    pub fn get_token(&self, provider_name: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn store_custom_provider(&self, name: &str, base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", &format!("custom_provider_{}", name))?;
        entry.set_password(base_url)?;
        Ok(())
    }

    pub fn get_custom_provider(&self, name: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", &format!("custom_provider_{}", name))?;
        match entry.get_password() {
            Ok(base_url) => Ok(Some(base_url)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
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
            let status = if has_token { "✓ configured" } else { "not configured" };
            println!("  {}. {} ({}) - {}", i + 1, provider.display_name, provider.name, status);
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

    fn setup_provider_auth(&self, provider_name: &str, display_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!();
        print!("Enter your {} API token: ", display_name);
        io::stdout().flush()?;

        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim();

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        self.store_token(provider_name, token)?;
        println!("✓ Token stored securely for {}", display_name);

        Ok(())
    }

    fn setup_custom_provider(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!();
        print!("Enter a short name for your custom provider (no spaces): ");
        io::stdout().flush()?;

        let mut name = String::new();
        io::stdin().read_line(&mut name)?;
        let name = name.trim();

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

        print!("Enter your API token for {}: ", name);
        io::stdout().flush()?;

        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim();

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        // Store both the custom provider URL and token
        self.store_custom_provider(name, base_url)?;
        self.store_token(name, token)?;

        println!("✓ Custom provider '{}' configured with URL: {}", name, base_url);

        Ok(())
    }

    pub fn get_auth_for_provider(&self, provider_name: &str) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
        // First check if it's a built-in provider
        if let Some(provider) = self.find_provider_by_name(provider_name) {
            if let Some(token) = self.get_token(provider_name)? {
                return Ok(Some((provider.base_url.clone(), token)));
            }
        } else {
            // Check if it's a custom provider
            if let Some(base_url) = self.get_custom_provider(provider_name)? {
                if let Some(token) = self.get_token(provider_name)? {
                    return Ok(Some((base_url, token)));
                }
            }
        }

        Ok(None)
    }

}
