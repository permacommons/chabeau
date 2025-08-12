use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::{suggest_provider_id, Config, CustomProvider};
use keyring::Entry;
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub display_name: String,
}

impl Provider {
    pub fn new(name: String, base_url: String, display_name: String, _mode: Option<String>) -> Self {
        Self {
            name,
            base_url,
            display_name,
        }
    }
}

pub struct AuthManager {
    providers: Vec<Provider>,
    config: Config,
}

impl AuthManager {
    pub fn new() -> Self {
        // Load config first
        let config = Config::load().unwrap_or_default();

        // Load built-in providers from configuration
        let builtin_providers = load_builtin_providers();
        let mut providers: Vec<Provider> = builtin_providers
            .into_iter()
            .map(|bp| Provider::new(bp.id, bp.base_url, bp.display_name, bp.mode))
            .collect();

        // Add custom providers from config
        for custom_provider in config.list_custom_providers() {
            providers.push(Provider::new(
                custom_provider.id.clone(),
                custom_provider.base_url.clone(),
                custom_provider.display_name.clone(),
                custom_provider.mode.clone(),
            ));
        }

        Self { providers, config }
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
        &mut self,
        provider: CustomProvider,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.config.add_custom_provider(provider);
        self.config.save()?;
        Ok(())
    }

    pub fn get_custom_provider(&self, id: &str) -> Option<&CustomProvider> {
        self.config.get_custom_provider(id)
    }

    pub fn list_custom_providers(&self) -> Vec<(String, String, String, bool)> {
        let mut result = Vec::new();
        for custom_provider in self.config.list_custom_providers() {
            let has_token = self
                .get_token(&custom_provider.id)
                .unwrap_or(None)
                .is_some();
            result.push((
                custom_provider.id.clone(),
                custom_provider.display_name.clone(),
                custom_provider.base_url.clone(),
                has_token,
            ));
        }
        result
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

    pub fn interactive_auth(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        print!("Enter your {display_name} API token (press F2 to reveal last 4 chars): ");
        io::stdout().flush()?;

        let token = self.read_masked_input()?;

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        self.store_token(provider_name, &token)?;
        println!("✓ Token stored securely for {display_name}");

        Ok(())
    }

    fn setup_custom_provider(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!();
        print!("Enter a display name for your custom provider: ");
        io::stdout().flush()?;

        let mut display_name = String::new();
        io::stdin().read_line(&mut display_name)?;
        let display_name = display_name.trim();

        if display_name.is_empty() {
            return Err("Display name cannot be empty".into());
        }

        // Suggest an ID based on the display name
        let suggested_id = suggest_provider_id(display_name);
        print!("Enter an ID for your provider [default: {suggested_id}]: ");
        io::stdout().flush()?;

        let mut id_input = String::new();
        io::stdin().read_line(&mut id_input)?;
        let id = id_input.trim();

        let final_id = if id.is_empty() {
            suggested_id
        } else {
            // Validate the ID - only alphanumeric characters
            if !id.chars().all(|c| c.is_alphanumeric()) {
                return Err("Provider ID can only contain alphanumeric characters".into());
            }
            id.to_lowercase()
        };

        // Check if ID already exists
        if self.find_provider_by_name(&final_id).is_some()
            || self.get_custom_provider(&final_id).is_some()
        {
            return Err(format!("Provider with ID '{final_id}' already exists").into());
        }

        print!("Enter the API base URL (typically, https://some-url.example/api/v1): ");
        io::stdout().flush()?;

        let mut base_url = String::new();
        io::stdin().read_line(&mut base_url)?;
        let base_url = base_url.trim();

        if base_url.is_empty() {
            return Err("Base URL cannot be empty".into());
        }

        let auth_mode = None; // Default to openai mode for now

        print!("Enter your API token for {display_name} (press F2 to reveal last 4 chars): ");
        io::stdout().flush()?;

        let token = self.read_masked_input()?;

        if token.is_empty() {
            return Err("Token cannot be empty".into());
        }

        // Create and store the custom provider
        let custom_provider = CustomProvider::new(
            final_id.clone(),
            display_name.to_string(),
            base_url.to_string(),
            auth_mode,
        );

        self.store_custom_provider(custom_provider)?;
        self.store_token(&final_id, &token)?;

        println!(
            "✓ Custom provider '{display_name}' (ID: {final_id}) configured with URL: {base_url}"
        );

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
            if let Some(custom_provider) = self.get_custom_provider(provider_name) {
                if let Some(token) = self.get_token(provider_name)? {
                    return Ok(Some((custom_provider.base_url.clone(), token)));
                }
            }
        }

        Ok(None)
    }

    pub fn interactive_deauth(
        &mut self,
        provider: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(provider_name) = provider {
            // Provider specified via --provider flag - validate it exists first
            let has_auth = self.get_token(&provider_name)?.is_some();
            let is_custom = self.get_custom_provider(&provider_name).is_some();

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

    fn interactive_deauth_menu(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        let custom_providers = self.list_custom_providers();
        for (id, display_name, _url, has_token) in custom_providers {
            if has_token {
                configured_providers.push((id, display_name, true));
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
        &mut self,
        provider_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.config.remove_custom_provider(provider_id);
        self.config.save()?;
        Ok(())
    }

    /// Read masked input with F2 to reveal last 4 characters
    ///
    /// Features:
    /// - Input is masked with asterisks (*)
    /// - F2 toggles showing the last 4 characters
    /// - Backspace/Delete to remove characters
    /// - Ctrl+U to clear entire line
    /// - Ctrl+W to delete last word
    /// - Ctrl+C or Esc to cancel
    /// - Enter to confirm input
    fn read_masked_input(&self) -> Result<String, Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let mut input = String::new();
        let mut show_last_four = false;

        loop {
            // Clear the line and redraw the prompt with current state
            print!("\r\x1b[K"); // Clear line
            if show_last_four && input.len() >= 4 {
                let masked_part = "*".repeat(input.len() - 4);
                let visible_part = &input[input.len() - 4..];
                print!(
                    "Enter your API token (press F2 to reveal last 4 chars): {}{}",
                    masked_part, visible_part
                );
            } else {
                let masked = "*".repeat(input.len());
                print!(
                    "Enter your API token (press F2 to reveal last 4 chars): {}",
                    masked
                );
            }
            io::stdout().flush()?;

            // Wait for events with a timeout
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            KeyCode::Enter => {
                                disable_raw_mode()?;
                                println!(); // Move to next line
                                return Ok(input);
                            }
                            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                                input.push(c);
                                show_last_four = false; // Hide reveal when typing
                            }
                            KeyCode::Backspace => {
                                if !input.is_empty() {
                                    input.pop();
                                    show_last_four = false; // Hide reveal when editing
                                }
                            }
                            KeyCode::Delete => {
                                // Delete key - same as backspace for single-line input
                                if !input.is_empty() {
                                    input.pop();
                                    show_last_four = false; // Hide reveal when editing
                                }
                            }
                            KeyCode::F(2) => {
                                show_last_four = !show_last_four;
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+U: Clear entire line
                                input.clear();
                                show_last_four = false;
                            }
                            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+W: Delete last word
                                while input.ends_with(' ') {
                                    input.pop();
                                }
                                while !input.is_empty() && !input.ends_with(' ') {
                                    input.pop();
                                }
                                show_last_four = false;
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                disable_raw_mode()?;
                                println!(); // Move to next line
                                return Err("Cancelled by user".into());
                            }
                            KeyCode::Esc => {
                                disable_raw_mode()?;
                                println!(); // Move to next line
                                return Err("Cancelled by user".into());
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
