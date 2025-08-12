//! Built-in provider configuration
//!
//! This module handles loading and managing built-in provider configurations
//! from the builtin_models.toml file at build time.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BuiltinProvidersConfig {
    providers: Vec<BuiltinProvider>,
}

impl BuiltinProvider {
    /// Get the authentication mode for this provider
    pub fn auth_mode(&self) -> &str {
        self.mode.as_deref().unwrap_or("openai")
    }

    /// Check if this provider uses Anthropic-style authentication
    pub fn is_anthropic_mode(&self) -> bool {
        self.auth_mode() == "anthropic"
    }
}

/// Load built-in providers from the embedded configuration
pub fn load_builtin_providers() -> Vec<BuiltinProvider> {
    const CONFIG_CONTENT: &str = include_str!("../builtin_models.toml");

    let config: BuiltinProvidersConfig =
        toml::from_str(CONFIG_CONTENT).expect("Failed to parse builtin_models.toml");

    config.providers
}

/// Find a built-in provider by ID (case-insensitive)
pub fn find_builtin_provider(id: &str) -> Option<BuiltinProvider> {
    load_builtin_providers()
        .into_iter()
        .find(|p| p.id.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_providers() {
        let providers = load_builtin_providers();
        assert!(!providers.is_empty());

        // Check that we have the expected providers
        let provider_ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert!(provider_ids.contains(&"openai"));
        assert!(provider_ids.contains(&"anthropic"));
        assert!(provider_ids.contains(&"openrouter"));
        assert!(provider_ids.contains(&"poe"));
    }

    #[test]
    fn test_find_builtin_provider() {
        // Test case-insensitive lookup
        let provider = find_builtin_provider("OpenAI");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().id, "openai");

        // Test exact match
        let provider = find_builtin_provider("anthropic");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().display_name, "Anthropic");

        // Test non-existent provider
        let provider = find_builtin_provider("nonexistent");
        assert!(provider.is_none());
    }

    #[test]
    fn test_anthropic_mode() {
        let anthropic = find_builtin_provider("anthropic").unwrap();
        assert!(anthropic.is_anthropic_mode());
        assert_eq!(anthropic.auth_mode(), "anthropic");

        let openai = find_builtin_provider("openai").unwrap();
        assert!(!openai.is_anthropic_mode());
        assert_eq!(openai.auth_mode(), "openai");
    }

    #[test]
    fn test_provider_properties() {
        let providers = load_builtin_providers();

        for provider in providers {
            // All providers should have non-empty required fields
            assert!(!provider.id.is_empty());
            assert!(!provider.display_name.is_empty());
            assert!(!provider.base_url.is_empty());

            // Base URL should be a valid URL format
            assert!(provider.base_url.starts_with("https://"));
        }
    }
}
