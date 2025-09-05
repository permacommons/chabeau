use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::{suggest_provider_id, Config, CustomProvider};
use crate::utils::input::sanitize_text_input;
use crate::utils::url::normalize_base_url;
use keyring::Entry;
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::time::Duration;

// Constants for repeated strings
const KEYRING_SERVICE: &str = "chabeau";
const MASKED_INPUT_PROMPT: &str = "Enter your API token (press F2 to reveal last 4 chars): ";
const INVALID_CHOICE_MSG: &str = "Invalid choice";
const TOKEN_EMPTY_ERROR: &str = "Token cannot be empty";

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub display_name: String,
}

impl Provider {
    pub fn new(
        name: String,
        base_url: String,
        display_name: String,
        _mode: Option<String>,
    ) -> Self {
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

    /// Resolve authentication information for a provider
    ///
    /// This function consolidates the common authentication resolution logic:
    /// 1. Finding authentication for a specified provider
    /// 2. Using config default provider if available
    /// 3. Falling back to first available authentication
    /// 4. Using environment variables as last resort
    ///
    /// Returns: (api_key, base_url, provider_name, provider_display_name)
    pub fn resolve_authentication(
        &self,
        provider: Option<&str>,
        config: &Config,
    ) -> Result<(String, String, String, String), Box<dyn std::error::Error>> {
        // Handle the case where provider is specified but empty (""), meaning use config default
        if let Some(provider_name) = provider {
            if provider_name.is_empty() {
                // User specified -p without a value, use config default if available
                if let Some(ref default_provider) = config.default_provider {
                    if let Some((base_url, api_key)) =
                        self.get_auth_for_provider(default_provider)?
                    {
                        let display_name = self
                            .find_provider_by_name(default_provider)
                            .map(|p| p.display_name.clone())
                            .unwrap_or_else(|| default_provider.clone());
                        Ok((
                            api_key,
                            base_url,
                            default_provider.to_lowercase(),
                            display_name,
                        ))
                    } else {
                        Err(format!("No authentication found for default provider '{default_provider}'. Run 'chabeau auth' to set up authentication.").into())
                    }
                } else {
                    // Try to find any available authentication
                    if let Some((provider, api_key)) = self.find_first_available_auth() {
                        Ok((
                            api_key,
                            provider.base_url,
                            provider.name.to_lowercase(),
                            provider.display_name,
                        ))
                    } else {
                        // Fall back to environment variables
                        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                            "❌ No authentication configured and OPENAI_API_KEY environment variable not set

Please either:
1. Run 'chabeau auth' to set up authentication, or
2. Set environment variables:
   export OPENAI_API_KEY=\"your-api-key-here\"
   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
                        })?;

                        let base_url = std::env::var("OPENAI_BASE_URL")
                            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

                        Ok((
                            api_key,
                            base_url,
                            "openai".to_string(),
                            "OpenAI".to_string(),
                        ))
                    }
                }
            } else {
                // User specified a provider - normalize to lowercase for consistent config lookup
                let normalized_provider_name = provider_name.to_lowercase();
                if let Some((base_url, api_key)) = self.get_auth_for_provider(provider_name)? {
                    let display_name = self
                        .find_provider_by_name(provider_name)
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| provider_name.to_string());
                    Ok((api_key, base_url, normalized_provider_name, display_name))
                } else {
                    Err(format!("No authentication found for provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into())
                }
            }
        } else if let Some(ref provider_name) = config.default_provider {
            // Config specifies a default provider
            if let Some((base_url, api_key)) = self.get_auth_for_provider(provider_name)? {
                let display_name = self
                    .find_provider_by_name(provider_name)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| provider_name.clone());
                Ok((
                    api_key,
                    base_url,
                    provider_name.to_lowercase(),
                    display_name,
                ))
            } else {
                Err(format!("No authentication found for default provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into())
            }
        } else {
            // Try to find any available authentication
            if let Some((provider, api_key)) = self.find_first_available_auth() {
                Ok((
                    api_key,
                    provider.base_url,
                    provider.name.to_lowercase(),
                    provider.display_name,
                ))
            } else {
                // Fall back to environment variables
                let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    "❌ No authentication configured and OPENAI_API_KEY environment variable not set

Please either:
1. Run 'chabeau auth' to set up authentication, or
2. Set environment variables:
   export OPENAI_API_KEY=\"your-api-key-here\"
   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
                })?;

                let base_url = std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

                Ok((
                    api_key,
                    base_url,
                    "openai".to_string(),
                    "OpenAI".to_string(),
                ))
            }
        }
    }

    pub fn store_token(
        &self,
        provider_name: &str,
        token: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new(KEYRING_SERVICE, provider_name)?;
        entry.set_password(token)?;
        Ok(())
    }

    pub fn get_token(
        &self,
        provider_name: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let entry = Entry::new(KEYRING_SERVICE, provider_name)?;
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
        let choice: usize = input.trim().parse().map_err(|_| INVALID_CHOICE_MSG)?;

        if choice == 0 || choice > self.providers.len() + 1 {
            return Err(INVALID_CHOICE_MSG.into());
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
        println!("Selected provider: {display_name}");
        print!("{}", MASKED_INPUT_PROMPT);
        io::stdout().flush()?;

        let token = self.read_masked_input()?;

        if token.is_empty() {
            return Err(TOKEN_EMPTY_ERROR.into());
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

        // Normalize the base URL to remove trailing slashes
        let normalized_base_url = normalize_base_url(base_url);

        let auth_mode = None; // Default to openai mode for now

        print!("{}", MASKED_INPUT_PROMPT);
        io::stdout().flush()?;

        let token = self.read_masked_input()?;

        if token.is_empty() {
            return Err(TOKEN_EMPTY_ERROR.into());
        }

        // Create and store the custom provider
        let custom_provider = CustomProvider::new(
            final_id.clone(),
            display_name.to_string(),
            normalized_base_url.clone(),
            auth_mode,
        );

        self.store_custom_provider(custom_provider)?;
        self.store_token(&final_id, &token)?;

        println!(
            "✓ Custom provider '{display_name}' (ID: {final_id}) configured with URL: {normalized_base_url}"
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
    /// - Enter or newlines (\n, \r) to confirm input
    /// - Proper paste handling with sanitization
    fn read_masked_input(&self) -> Result<String, Box<dyn std::error::Error>> {
        enable_raw_mode()?;

        // Enable bracketed paste mode to handle paste events properly
        let mut stdout = io::stdout();
        execute!(stdout, event::EnableBracketedPaste)?;

        let mut input = String::new();
        let mut show_last_four = false;
        let mut needs_redraw = true;

        let result = loop {
            // Only redraw when necessary
            if needs_redraw {
                self.display_masked_prompt(&input, show_last_four)?;
                needs_redraw = false;
            }

            // Wait for events with a timeout
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            KeyCode::Enter => {
                                break Ok(input);
                            }
                            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Check for newline characters and submit immediately
                                if c == '\n' || c == '\r' {
                                    break Ok(input);
                                }
                                input.push(c);
                                show_last_four = false; // Hide reveal when typing
                                needs_redraw = true;
                            }
                            KeyCode::Backspace | KeyCode::Delete => {
                                if !input.is_empty() {
                                    input.pop();
                                    show_last_four = false; // Hide reveal when editing
                                    needs_redraw = true;
                                }
                            }
                            KeyCode::F(2) => {
                                show_last_four = !show_last_four;
                                needs_redraw = true;
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+U: Clear entire line
                                input.clear();
                                show_last_four = false;
                                needs_redraw = true;
                            }
                            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+W: Delete last word
                                self.delete_last_word(&mut input);
                                show_last_four = false;
                                needs_redraw = true;
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                break Err("Cancelled by user".into());
                            }
                            KeyCode::Esc => {
                                break Err("Cancelled by user".into());
                            }
                            _ => {}
                        }
                    }
                    Event::Paste(text) => {
                        // Handle paste events with sanitization
                        let sanitized_text = sanitize_text_input(&text);

                        // Check if the sanitized text contains newlines - if so, submit immediately
                        if sanitized_text.contains('\n') {
                            // Take everything before the first newline as the input
                            let before_newline = sanitized_text.split('\n').next().unwrap_or("");
                            input.push_str(before_newline);

                            // Show the masked input before submitting
                            self.display_masked_prompt(&input, false)?;

                            break Ok(input);
                        } else {
                            // No newlines, just add the sanitized text
                            input.push_str(&sanitized_text);
                            show_last_four = false; // Hide reveal when pasting
                            needs_redraw = true;
                        }
                    }
                    _ => {}
                }
            }
        };

        // Cleanup: disable bracketed paste mode and raw mode
        disable_raw_mode()?;
        execute!(stdout, event::DisableBracketedPaste)?;
        println!(); // Move to next line

        result
    }

    /// Display the masked input prompt with optional reveal of last 4 characters
    fn display_masked_prompt(
        &self,
        input: &str,
        show_last_four: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        print!("\r\x1b[K"); // Clear line
        if show_last_four && input.len() >= 4 {
            let masked_part = "*".repeat(input.len() - 4);
            let visible_part = &input[input.len() - 4..];
            print!("{}{}{}", MASKED_INPUT_PROMPT, masked_part, visible_part);
        } else {
            let masked = "*".repeat(input.len());
            print!("{}{}", MASKED_INPUT_PROMPT, masked);
        }
        io::stdout().flush()?;
        Ok(())
    }

    /// Delete the last word from the input string (Ctrl+W functionality)
    fn delete_last_word(&self, input: &mut String) {
        // Remove trailing spaces
        while input.ends_with(' ') {
            input.pop();
        }
        // Remove the last word
        while !input.is_empty() && !input.ends_with(' ') {
            input.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_auth_manager() -> AuthManager {
        AuthManager::new()
    }

    #[test]
    fn test_delete_last_word_single_word() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::from("hello");
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "");
    }

    #[test]
    fn test_delete_last_word_multiple_words() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::from("hello world test");
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "hello world ");
    }

    #[test]
    fn test_delete_last_word_trailing_spaces() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::from("hello world   ");
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "hello ");
    }

    #[test]
    fn test_delete_last_word_empty_string() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::new();
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "");
    }

    #[test]
    fn test_delete_last_word_only_spaces() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::from("   ");
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "");
    }

    #[test]
    fn test_delete_last_word_mixed_spaces() {
        let auth_manager = create_test_auth_manager();
        let mut input = String::from("hello  world  test  ");
        auth_manager.delete_last_word(&mut input);
        assert_eq!(input, "hello  world  ");
    }

    // Note: We can't easily test the full read_masked_input function without mocking
    // the terminal input, but we can test the helper functions that contain the logic.
    // For integration testing of the full masked input functionality, manual testing
    // or more complex test harnesses would be needed.
}
