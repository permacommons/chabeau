use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::data::{suggest_provider_id, Config, CustomProvider};
use crate::core::keyring::{KeyringAccessError, SharedKeyringAccessError};
use crate::core::providers::{
    resolve_session, ProviderAuthSource, ProviderMetadata, ResolveSessionError,
};
use crate::utils::url::normalize_base_url;
use keyring::Entry;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

mod ui;

use self::ui::{
    prompt_auth_menu, prompt_custom_provider_details, prompt_deauth_menu, prompt_provider_token,
    AuthMenuSelection, CustomProviderInput, DeauthMenuItem, ProviderMenuItem, UiError,
};
// Constants for repeated strings
const KEYRING_SERVICE: &str = "chabeau";

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub display_name: String,
}

type ConfiguredProviderEntry = (String, String, bool);

fn map_ui_result<T>(result: Result<T, UiError>) -> Result<T, Box<dyn std::error::Error>> {
    result.map_err(|err| Box::new(err) as Box<dyn std::error::Error>)
}

struct CustomProviderSummary {
    provider_id: String,
    display_name: String,
    base_url: String,
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
    use_keyring: bool,
}

#[derive(Clone, Debug)]
enum KeyringCacheEntry {
    Present(String),
    Missing,
    Error(SharedKeyringAccessError),
}

impl AuthManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_keyring(true)
    }

    /// Construct an AuthManager, optionally disabling keyring access (useful for tests)
    pub fn new_with_keyring(use_keyring: bool) -> Result<Self, Box<dyn std::error::Error>> {
        // Load config first
        let config = Config::load()?;

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

        Ok(Self {
            providers,
            config,
            use_keyring,
        })
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
        match resolve_session(self, config, provider) {
            Ok(session) => Ok(session.into_tuple()),
            Err(ResolveSessionError::Provider(err)) => Err(Box::new(err)),
            Err(ResolveSessionError::Source(err)) => Err(err),
        }
    }

    pub fn store_token(
        &self,
        provider_name: &str,
        token: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.use_keyring {
            return Ok(());
        }
        let entry = Entry::new(KEYRING_SERVICE, provider_name)?;
        entry.set_password(token)?;
        self.cache_lookup(provider_name, KeyringCacheEntry::Present(token.to_string()));
        Ok(())
    }

    pub fn get_token(
        &self,
        provider_name: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if !self.use_keyring {
            return Ok(None);
        }
        if let Some(cached) = get_cached_entry(provider_name) {
            return match cached {
                KeyringCacheEntry::Present(token) => Ok(Some(token.clone())),
                KeyringCacheEntry::Missing => Ok(None),
                KeyringCacheEntry::Error(err) => Err(Box::new(err.clone())),
            };
        }
        self.log_lookup(provider_name, "attempt");
        let entry = match Entry::new(KEYRING_SERVICE, provider_name) {
            Ok(entry) => entry,
            Err(err) => {
                let keyring_err = KeyringAccessError::from(err);
                let shared_err = SharedKeyringAccessError::new(keyring_err);
                self.log_lookup(
                    provider_name,
                    &format!("error=entry_new message={}", shared_err),
                );
                self.cache_lookup(provider_name, KeyringCacheEntry::Error(shared_err.clone()));
                return Err(Box::new(shared_err));
            }
        };
        match entry.get_password() {
            Ok(token) => {
                self.log_lookup(provider_name, &format!("result=success token={token}"));
                self.cache_lookup(provider_name, KeyringCacheEntry::Present(token.clone()));
                Ok(Some(token))
            }
            Err(keyring::Error::NoEntry) => {
                self.log_lookup(provider_name, "result=missing");
                self.cache_lookup(provider_name, KeyringCacheEntry::Missing);
                Ok(None)
            }
            Err(err) => {
                let keyring_err = KeyringAccessError::from(err);
                let shared_err = SharedKeyringAccessError::new(keyring_err);
                self.log_lookup(
                    provider_name,
                    &format!("result=error message={}", shared_err),
                );
                self.cache_lookup(provider_name, KeyringCacheEntry::Error(shared_err.clone()));
                Err(Box::new(shared_err))
            }
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
        let mut menu_items = Vec::new();
        for provider in &self.providers {
            let configured = self.get_token(&provider.name)?.is_some();
            menu_items.push(ProviderMenuItem {
                id: provider.name.clone(),
                display_name: provider.display_name.clone(),
                configured,
            });
        }

        let selection = map_ui_result(prompt_auth_menu(&menu_items))?;

        match selection {
            AuthMenuSelection::Provider(index) => {
                let provider = &self.providers[index];
                let token = map_ui_result(prompt_provider_token(&provider.display_name))?;
                if token.is_empty() {
                    return Err("Token cannot be empty".into());
                }
                self.store_token(&provider.name, &token)?;
                println!("✓ Token stored securely for {}", provider.display_name);
            }
            AuthMenuSelection::Custom => {
                let existing_ids = self.collect_existing_provider_ids();
                let custom_input =
                    map_ui_result(prompt_custom_provider_details(&existing_ids, |name| {
                        suggest_provider_id(name)
                    }))?;
                let summary = self.setup_custom_provider(custom_input)?;
                println!(
                    "✓ Custom provider '{}' (ID: {}) configured with URL: {}",
                    summary.display_name, summary.provider_id, summary.base_url
                );
            }
            AuthMenuSelection::Cancel => {
                println!("Cancelled.");
                return Ok(());
            }
        }

        println!();
        println!("✅ Authentication configured successfully!");
        println!("You can now use Chabeau without setting environment variables.");

        Ok(())
    }

    fn setup_custom_provider(
        &mut self,
        details: CustomProviderInput,
    ) -> Result<CustomProviderSummary, Box<dyn std::error::Error>> {
        let CustomProviderInput {
            display_name,
            provider_id,
            base_url,
            token,
        } = details;

        let normalized_base_url = normalize_base_url(&base_url);
        let custom_provider = CustomProvider::new(
            provider_id.clone(),
            display_name.clone(),
            normalized_base_url.clone(),
            None,
        );

        self.store_custom_provider(custom_provider)?;
        self.store_token(&provider_id, &token)?;

        if self
            .providers
            .iter()
            .all(|existing| existing.name != provider_id)
        {
            self.providers.push(Provider::new(
                provider_id.clone(),
                normalized_base_url.clone(),
                display_name.clone(),
                None,
            ));
        }

        Ok(CustomProviderSummary {
            provider_id,
            display_name,
            base_url: normalized_base_url,
        })
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
            let provider_msg = provider_name.clone();
            let (resolved_provider, is_custom) = self.resolve_deauth_target(&provider_msg)?;

            if self.get_token(&resolved_provider)?.is_none() {
                return Err(format!(
                    "Provider '{provider_msg}' exists but has no authentication configured."
                )
                .into());
            }

            self.remove_provider_auth(&resolved_provider)?;

            // Check if it's a custom provider and remove it completely
            if is_custom {
                self.remove_custom_provider(&resolved_provider)?;
            }

            println!("✅ Authentication removed for {provider_msg}");
        } else {
            // Interactive mode - show menu of configured providers
            self.interactive_deauth_menu()?;
        }
        Ok(())
    }

    fn resolve_deauth_target(
        &self,
        provider_name: &str,
    ) -> Result<(String, bool), Box<dyn std::error::Error>> {
        if let Some(custom) = self.get_custom_provider(provider_name) {
            return Ok((custom.id.clone(), true));
        }

        let normalized = provider_name.to_lowercase();
        if let Some(custom) = self.get_custom_provider(&normalized) {
            return Ok((custom.id.clone(), true));
        }

        if let Some(provider) = self.find_provider_by_name(provider_name) {
            return Ok((provider.name.clone(), false));
        }

        Err(format!(
            "Provider '{provider_name}' is not configured. Use 'chabeau providers' to see configured providers."
        )
        .into())
    }

    fn interactive_deauth_menu(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let configured_providers = self.collect_configured_providers(|name| {
            self.get_token(name).map(|token| token.is_some())
        })?;

        let menu_items: Vec<DeauthMenuItem> = configured_providers
            .iter()
            .map(|(name, display_name, is_custom)| DeauthMenuItem {
                id: name.clone(),
                display_name: display_name.clone(),
                is_custom: *is_custom,
            })
            .collect();

        if let Some(selection) = map_ui_result(prompt_deauth_menu(&menu_items))? {
            self.remove_provider_auth(&selection.provider_id)?;

            if selection.is_custom {
                self.remove_custom_provider(&selection.provider_id)?;
            }

            println!("✅ Authentication removed for {}", selection.display_name);
        }

        Ok(())
    }

    fn collect_configured_providers<F>(
        &self,
        mut has_token: F,
    ) -> Result<Vec<ConfiguredProviderEntry>, Box<dyn std::error::Error>>
    where
        F: FnMut(&str) -> Result<bool, Box<dyn std::error::Error>>,
    {
        let mut configured_providers: Vec<ConfiguredProviderEntry> = Vec::new();

        for provider in &self.providers {
            if self.get_custom_provider(&provider.name).is_some() {
                continue;
            }

            if has_token(&provider.name)? {
                configured_providers.push((
                    provider.name.clone(),
                    provider.display_name.clone(),
                    false,
                ));
            }
        }

        for custom_provider in self.config.list_custom_providers() {
            if has_token(&custom_provider.id)? {
                configured_providers.push((
                    custom_provider.id.clone(),
                    custom_provider.display_name.clone(),
                    true,
                ));
            }
        }

        Ok(configured_providers)
    }

    fn collect_existing_provider_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        for provider in &self.providers {
            ids.insert(provider.name.clone());
        }
        for custom in self.config.list_custom_providers() {
            ids.insert(custom.id.clone());
        }
        ids
    }

    fn remove_provider_auth(&self, provider_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let entry = Entry::new("chabeau", provider_name)?;
        match entry.delete_credential() {
            Ok(()) => {
                self.cache_lookup(provider_name, KeyringCacheEntry::Missing);
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                self.cache_lookup(provider_name, KeyringCacheEntry::Missing);
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
}

impl AuthManager {
    fn cache_lookup(&self, provider_name: &str, entry: KeyringCacheEntry) {
        if !self.use_keyring {
            return;
        }

        if let Ok(mut cache) = token_cache().lock() {
            cache.insert(provider_name.to_string(), entry);
        }
    }

    fn log_lookup(&self, provider_name: &str, detail: &str) {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("lookups.log")
        {
            let _ = writeln!(file, "provider={provider_name} {detail}");
        }
    }
}

fn get_cached_entry(provider_name: &str) -> Option<KeyringCacheEntry> {
    let cache = token_cache().lock().ok()?;
    cache.get(provider_name).cloned()
}

fn token_cache() -> &'static Mutex<HashMap<String, KeyringCacheEntry>> {
    static TOKEN_CACHE: OnceLock<Mutex<HashMap<String, KeyringCacheEntry>>> = OnceLock::new();
    TOKEN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl ProviderAuthSource for AuthManager {
    fn uses_keyring(&self) -> bool {
        self.use_keyring
    }

    fn find_provider_metadata(&self, provider: &str) -> Option<ProviderMetadata> {
        if let Some(builtin) = self.find_provider_by_name(provider) {
            return Some(ProviderMetadata {
                id: builtin.name.clone(),
                display_name: builtin.display_name.clone(),
                base_url: builtin.base_url.clone(),
            });
        }

        self.get_custom_provider(provider)
            .map(|custom| ProviderMetadata {
                id: custom.id.clone(),
                display_name: custom.display_name.clone(),
                base_url: custom.base_url.clone(),
            })
    }

    fn get_auth_for_provider(
        &self,
        provider: &str,
    ) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
        AuthManager::get_auth_for_provider(self, provider)
    }

    fn find_first_available_auth(&self) -> Option<(ProviderMetadata, String)> {
        AuthManager::find_first_available_auth(self).map(|(provider, token)| {
            (
                ProviderMetadata {
                    id: provider.name,
                    display_name: provider.display_name,
                    base_url: provider.base_url,
                },
                token,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{with_test_config_env, TestEnvVarGuard};

    #[test]
    fn collect_configured_providers_skips_duplicate_custom_entries() {
        let mut config = Config::default();
        config.add_custom_provider(CustomProvider::new(
            "custom".to_string(),
            "Custom Provider".to_string(),
            "https://example.com".to_string(),
            None,
        ));

        let providers = vec![
            Provider::new(
                "anthropic".to_string(),
                "https://api.anthropic.com".to_string(),
                "Anthropic".to_string(),
                None,
            ),
            Provider::new(
                "custom".to_string(),
                "https://example.com".to_string(),
                "Custom Provider".to_string(),
                None,
            ),
        ];

        let manager = AuthManager {
            providers,
            config,
            use_keyring: false,
        };

        let configured = manager
            .collect_configured_providers(|name| {
                Ok::<bool, Box<dyn std::error::Error>>(matches!(name, "anthropic" | "custom"))
            })
            .expect("configured providers should be collected");

        assert_eq!(configured.len(), 2);
        assert_eq!(
            configured[0],
            ("anthropic".to_string(), "Anthropic".to_string(), false)
        );
        assert_eq!(
            configured[1],
            ("custom".to_string(), "Custom Provider".to_string(), true,)
        );
    }

    #[test]
    fn env_fallback_sets_openai_provider_for_default_base() {
        with_test_config_env(|_| {
            // Ensure no default provider in config and no keyring; set explicit default base
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("OPENAI_API_KEY", "sk-test");
            env_guard.set_var("OPENAI_BASE_URL", "https://api.openai.com/v1");
            let am = AuthManager::new_with_keyring(false).expect("auth manager loads");
            let cfg = Config::default();
            let (_key, base, prov, display) = am
                .resolve_authentication(None, &cfg)
                .expect("env fallback should work");
            assert_eq!(base, "https://api.openai.com/v1");
            assert_eq!(prov, "openai");
            assert_eq!(display, "OpenAI");
            env_guard.remove_var("OPENAI_API_KEY");
            env_guard.remove_var("OPENAI_BASE_URL");
        });
    }

    #[test]
    fn env_fallback_sets_openai_compatible_for_custom_base() {
        with_test_config_env(|_| {
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("OPENAI_API_KEY", "sk-test");
            env_guard.set_var("OPENAI_BASE_URL", "https://example.com/v1");
            let am = AuthManager::new_with_keyring(false).expect("auth manager loads");
            let cfg = Config::default();
            let (_key, base, prov, display) = am
                .resolve_authentication(None, &cfg)
                .expect("env fallback should work");
            assert_eq!(base, "https://example.com/v1");
            assert_eq!(prov, "openai-compatible");
            assert_eq!(display, "OpenAI-compatible");
            env_guard.remove_var("OPENAI_API_KEY");
            env_guard.remove_var("OPENAI_BASE_URL");
        });
    }

    #[test]
    fn resolve_deauth_target_normalizes_builtin_provider() {
        with_test_config_env(|_| {
            let manager = AuthManager::new_with_keyring(false).expect("auth manager loads");
            let (resolved, is_custom) = manager
                .resolve_deauth_target("OpenAI")
                .expect("provider should resolve");
            assert_eq!(resolved, "openai");
            assert!(!is_custom);
        });
    }

    #[test]
    fn resolve_deauth_target_normalizes_custom_provider() {
        with_test_config_env(|_| {
            Config::mutate(|config| {
                config.add_custom_provider(CustomProvider::new(
                    "mycustom".to_string(),
                    "My Custom".to_string(),
                    "https://example.com".to_string(),
                    None,
                ));
                Ok(())
            })
            .expect("custom provider persisted");

            let mut manager = AuthManager::new_with_keyring(false).expect("auth manager loads");
            let (resolved, is_custom) = manager
                .resolve_deauth_target("MYCUSTOM")
                .expect("provider should resolve");
            assert_eq!(resolved, "mycustom");
            assert!(is_custom);

            manager
                .remove_custom_provider("MYCUSTOM")
                .expect("custom provider removed");
            assert!(manager.get_custom_provider("mycustom").is_none());
            assert!(manager.get_custom_provider("MYCUSTOM").is_none());
        });
    }

    // Note: We can't easily test the full read_masked_input function without mocking
    // the terminal input, but we can test the helper functions that contain the logic.
    // For integration testing of the full masked input functionality, manual testing
    // or more complex test harnesses would be needed.
}
