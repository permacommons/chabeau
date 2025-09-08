use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomTheme {
    pub id: String,
    pub display_name: String,
    pub background: Option<String>,
    pub user_prefix: Option<String>,
    pub user_text: Option<String>,
    pub assistant_text: Option<String>,
    pub system_text: Option<String>,
    pub title: Option<String>,
    pub streaming_indicator: Option<String>,
    pub input_border: Option<String>,
    pub input_title: Option<String>,
    pub input_text: Option<String>,
    pub input_cursor_modifiers: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub default_provider: Option<String>,
    #[serde(default)]
    pub default_models: HashMap<String, String>,
    #[serde(default)]
    pub custom_providers: Vec<CustomProvider>,
    /// UI theme name (e.g., "dark", "light", "dracula")
    pub theme: Option<String>,
    #[serde(default)]
    pub custom_themes: Vec<CustomTheme>,
    /// Enable markdown rendering in the chat area
    pub markdown: Option<bool>,
    /// Enable syntax highlighting for fenced code blocks when markdown is enabled
    pub syntax: Option<bool>,
}

impl Config {
    pub fn load() -> Result<Config, Box<dyn std::error::Error>> {
        let config_path = Self::get_config_path();
        Self::load_from_path(&config_path)
    }

    pub fn load_from_path(config_path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
        if config_path.exists() {
            let contents = fs::read_to_string(config_path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_path = Self::get_config_path();
        self.save_to_path(&config_path)
    }

    pub fn save_to_path(&self, config_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        fs::write(config_path, contents)?;
        Ok(())
    }

    fn get_config_path() -> PathBuf {
        let proj_dirs = ProjectDirs::from("org", "permacommons", "chabeau")
            .expect("Failed to determine config directory");
        proj_dirs.config_dir().join("config.toml")
    }

    pub fn print_all(&self) {
        println!("Current configuration:");
        match &self.default_provider {
            Some(provider) => println!("  default-provider: {provider}"),
            None => println!("  default-provider: (unset)"),
        }
        match &self.theme {
            Some(theme) => println!("  theme: {theme}"),
            None => println!("  theme: (unset)"),
        }
        match self.markdown.unwrap_or(true) {
            true => println!("  markdown: on"),
            false => println!("  markdown: off"),
        }
        match self.syntax.unwrap_or(true) {
            true => println!("  syntax: on"),
            false => println!("  syntax: off"),
        }
        if self.default_models.is_empty() {
            println!("  default-models: (none set)");
        } else {
            println!("  default-models:");
            for (provider, model) in &self.default_models {
                println!("    {provider}: {model}");
            }
        }
    }

    pub fn get_default_model(&self, provider: &str) -> Option<&String> {
        self.default_models.get(provider)
    }

    pub fn set_default_model(&mut self, provider: String, model: String) {
        self.default_models.insert(provider, model);
    }

    pub fn unset_default_model(&mut self, provider: &str) {
        self.default_models.remove(provider);
    }

    pub fn add_custom_provider(&mut self, provider: CustomProvider) {
        self.custom_providers.push(provider);
    }

    pub fn remove_custom_provider(&mut self, id: &str) {
        self.custom_providers.retain(|p| p.id != id);
    }

    pub fn get_custom_provider(&self, id: &str) -> Option<&CustomProvider> {
        self.custom_providers.iter().find(|p| p.id == id)
    }

    pub fn list_custom_providers(&self) -> Vec<&CustomProvider> {
        self.custom_providers.iter().collect()
    }

    // Custom themes management
    #[allow(dead_code)]
    pub fn add_custom_theme(&mut self, theme: CustomTheme) {
        self.custom_themes.push(theme);
    }
    #[allow(dead_code)]
    pub fn remove_custom_theme(&mut self, id: &str) {
        self.custom_themes.retain(|t| t.id != id);
    }
    pub fn get_custom_theme(&self, id: &str) -> Option<&CustomTheme> {
        self.custom_themes
            .iter()
            .find(|t| t.id.eq_ignore_ascii_case(id))
    }
    pub fn list_custom_themes(&self) -> Vec<&CustomTheme> {
        self.custom_themes.iter().collect()
    }
}

impl CustomProvider {
    pub fn new(id: String, display_name: String, base_url: String, mode: Option<String>) -> Self {
        Self {
            id,
            display_name,
            base_url,
            mode,
        }
    }
}

/// Generate a suggested ID from a display name
/// Converts to lowercase and keeps only alphanumeric characters
pub fn suggest_provider_id(display_name: &str) -> String {
    display_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_nonexistent_config() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("nonexistent_config.toml");

        let config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Should return default config
        assert_eq!(config.default_provider, None);
    }

    #[test]
    fn test_save_and_load_config() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        // Create a config with a default provider
        let config = Config {
            default_provider: Some("test-provider".to_string()),
            ..Default::default()
        };

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify the loaded config matches what we saved
        assert_eq!(
            loaded_config.default_provider,
            Some("test-provider".to_string())
        );
    }

    #[test]
    fn test_unset_default_provider() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        // Create a config with a default provider
        let config = Config {
            default_provider: Some("test-provider".to_string()),
            ..Default::default()
        };

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load config again and unset the default provider
        let mut config = Config::load_from_path(&config_path).expect("Failed to load config");
        config.default_provider = None;
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify the default provider is now None
        assert_eq!(loaded_config.default_provider, None);
    }

    #[test]
    fn test_change_default_provider() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        // Create a config with a default provider
        let config = Config {
            default_provider: Some("initial-provider".to_string()),
            ..Default::default()
        };

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load config again and change the default provider
        let mut config = Config::load_from_path(&config_path).expect("Failed to load config");
        config.default_provider = Some("new-provider".to_string());
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify the default provider was changed
        assert_eq!(
            loaded_config.default_provider,
            Some("new-provider".to_string())
        );
    }

    #[test]
    fn test_set_and_load_theme() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("theme_config.toml");

        // Save config with a theme
        let cfg = Config {
            theme: Some("light".to_string()),
            ..Default::default()
        };
        cfg.save_to_path(&config_path).expect("save config failed");

        // Load it back
        let loaded = Config::load_from_path(&config_path).expect("load config failed");
        assert_eq!(loaded.theme, Some("light".to_string()));
    }

    #[test]
    fn test_default_model_lookup_uses_provider_id() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        let mut config = Config::default();

        // Set a default model using the provider ID (as done in pick_default_model.rs)
        config.set_default_model("openai".to_string(), "gpt-4".to_string());
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify we can look up the model using the provider ID (not display name)
        assert_eq!(
            loaded_config.get_default_model("openai"),
            Some(&"gpt-4".to_string())
        );

        // This should return None because "OpenAI" is the display name, not the ID
        assert_eq!(loaded_config.get_default_model("OpenAI"), None);
    }

    #[test]
    fn test_multiple_provider_default_models() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        let mut config = Config::default();

        // Set default models for multiple providers using their IDs
        config.set_default_model("openai".to_string(), "gpt-4".to_string());
        config.set_default_model(
            "anthropic".to_string(),
            "claude-3-opus-20240229".to_string(),
        );
        config.set_default_model("custom-provider".to_string(), "custom-model".to_string());

        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify all models can be looked up using provider IDs
        assert_eq!(
            loaded_config.get_default_model("openai"),
            Some(&"gpt-4".to_string())
        );
        assert_eq!(
            loaded_config.get_default_model("anthropic"),
            Some(&"claude-3-opus-20240229".to_string())
        );
        assert_eq!(
            loaded_config.get_default_model("custom-provider"),
            Some(&"custom-model".to_string())
        );

        // Verify display names don't work (this was the bug)
        assert_eq!(loaded_config.get_default_model("OpenAI"), None);
        assert_eq!(loaded_config.get_default_model("Anthropic"), None);
    }

    #[test]
    fn test_case_insensitive_provider_normalization() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        let mut config = Config::default();

        // Set default models using mixed case provider names (as would happen from CLI)
        config.set_default_model("OpenAI".to_lowercase(), "gpt-4".to_string());
        config.set_default_model("POE".to_lowercase(), "claude-instant".to_string());
        config.set_default_model("AnThRoPiC".to_lowercase(), "claude-3-opus".to_string());

        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify all models can be looked up using lowercase provider names
        assert_eq!(
            loaded_config.get_default_model("openai"),
            Some(&"gpt-4".to_string())
        );
        assert_eq!(
            loaded_config.get_default_model("poe"),
            Some(&"claude-instant".to_string())
        );
        assert_eq!(
            loaded_config.get_default_model("anthropic"),
            Some(&"claude-3-opus".to_string())
        );

        // Verify mixed case lookups don't work (consistent behavior)
        assert_eq!(loaded_config.get_default_model("OpenAI"), None);
        assert_eq!(loaded_config.get_default_model("POE"), None);
        assert_eq!(loaded_config.get_default_model("AnThRoPiC"), None);
    }

    #[test]
    fn test_custom_provider_management() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_config.toml");

        let mut config = Config::default();

        // Add a custom provider
        let custom_provider = CustomProvider::new(
            "myapi".to_string(),
            "My Custom API".to_string(),
            "https://api.example.com/v1".to_string(),
            Some("anthropic".to_string()),
        );

        config.add_custom_provider(custom_provider);
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load the config back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify the custom provider was saved and loaded correctly
        let retrieved_provider = loaded_config.get_custom_provider("myapi");
        assert!(retrieved_provider.is_some());

        let provider = retrieved_provider.unwrap();
        assert_eq!(provider.id, "myapi");
        assert_eq!(provider.display_name, "My Custom API");
        assert_eq!(provider.base_url, "https://api.example.com/v1");
        assert_eq!(provider.mode, Some("anthropic".to_string()));

        // Test listing custom providers
        let providers = loaded_config.list_custom_providers();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "myapi");

        // Test removing custom provider
        let mut config = loaded_config;
        config.remove_custom_provider("myapi");
        assert!(config.get_custom_provider("myapi").is_none());
        assert_eq!(config.list_custom_providers().len(), 0);
    }

    #[test]
    fn test_suggest_provider_id() {
        assert_eq!(suggest_provider_id("OpenAI GPT"), "openaigpt");
        assert_eq!(suggest_provider_id("My Custom API 123"), "mycustomapi123");
        assert_eq!(
            suggest_provider_id("Test-Provider_Name!"),
            "testprovidername"
        );
        assert_eq!(suggest_provider_id("   Spaces   "), "spaces");
        assert_eq!(suggest_provider_id("123Numbers456"), "123numbers456");
        assert_eq!(suggest_provider_id(""), "");
    }

    #[test]
    fn test_custom_provider_auth_modes() {
        let openai_provider = CustomProvider::new(
            "test1".to_string(),
            "Test OpenAI".to_string(),
            "https://api.test.com/v1".to_string(),
            None,
        );

        let anthropic_provider = CustomProvider::new(
            "test2".to_string(),
            "Test Anthropic".to_string(),
            "https://api.test.com/v1".to_string(),
            Some("anthropic".to_string()),
        );

        assert_eq!(openai_provider.mode, None);
        assert_eq!(anthropic_provider.mode, Some("anthropic".to_string()));
    }

    #[test]
    fn test_custom_theme_save_load() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_theme.toml");
        let mut cfg = Config::default();
        cfg.add_custom_theme(CustomTheme {
            id: "mytheme".to_string(),
            display_name: "My Theme".to_string(),
            background: Some("black".to_string()),
            user_prefix: Some("green,bold".to_string()),
            user_text: Some("green".to_string()),
            assistant_text: Some("white".to_string()),
            system_text: Some("gray".to_string()),
            title: Some("gray".to_string()),
            streaming_indicator: Some("white".to_string()),
            input_border: Some("green".to_string()),
            input_title: Some("gray".to_string()),
            input_text: Some("white".to_string()),
            input_cursor_modifiers: Some("reversed".to_string()),
        });
        cfg.save_to_path(&config_path).expect("save failed");

        let loaded = Config::load_from_path(&config_path).expect("load failed");
        let t = loaded
            .get_custom_theme("mytheme")
            .expect("missing custom theme");
        assert_eq!(t.display_name, "My Theme");
        assert_eq!(t.background.as_deref(), Some("black"));
    }
}
