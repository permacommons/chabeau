use directories::ProjectDirs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tempfile::NamedTempFile;

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
    pub selection_highlight: Option<String>,
    pub input_border: Option<String>,
    pub input_title: Option<String>,
    pub input_text: Option<String>,
    pub input_cursor_modifiers: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Persona {
    pub id: String,
    pub display_name: String,
    pub bio: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
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
    /// Default character cards for provider/model combinations
    /// Outer key: provider (e.g., "openai")
    /// Inner key: model (e.g., "gpt-4")
    /// Value: character card filename without extension (e.g., "alice" for alice.json or alice.png)
    #[serde(default)]
    pub default_characters: HashMap<String, HashMap<String, String>>,
    /// Default personas for provider/model combinations
    /// Outer key: provider (e.g., "openai")
    /// Inner key: model (e.g., "gpt-4")
    /// Value: persona ID (e.g., "alice-dev")
    #[serde(default)]
    pub default_personas: HashMap<String, HashMap<String, String>>,
    /// User-defined personas for conversation contexts
    #[serde(default)]
    pub personas: Vec<Persona>,
}

/// Get a user-friendly display string for a path
/// Converts absolute paths to use ~ notation on Unix-like systems when possible
///
/// # Examples
/// - Unix: `/home/user/.config/chabeau/cards` → `~/.config/chabeau/cards`
/// - Windows: `C:\Users\user\AppData\Roaming\chabeau\cards` → `C:\Users\user\AppData\Roaming\chabeau\cards`
/// - macOS: `/Users/user/Library/Application Support/...` → `~/Library/Application Support/...`
pub fn path_display<P: AsRef<Path>>(path: P) -> String {
    let path = path.as_ref();

    // Try to use ~ for home directory on Unix-like systems
    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(home);
            if let Ok(relative) = path.strip_prefix(&home_path) {
                return format!("~/{}", relative.display());
            }
        }
    }

    // Fall back to full path
    path.display().to_string()
}

/// Snapshot of the last config we observed on disk.
/// Keeps the parsed value and timestamp so we can avoid redundant reloads.
#[derive(Default)]
struct ConfigCacheState {
    config: Option<Config>,
    modified: Option<SystemTime>,
}

/// Coordinates shared config access across CLI/TUI code paths.
/// The orchestrator caches parsed config files, performs atomic writes, and lets
/// tests swap in isolated config locations without touching user state.
struct ConfigOrchestrator {
    path: PathBuf,
    state: Mutex<ConfigCacheState>,
}

static CONFIG_ORCHESTRATOR: Lazy<ConfigOrchestrator> =
    Lazy::new(|| ConfigOrchestrator::new(Config::get_config_path()));

#[cfg(test)]
static TEST_ORCHESTRATOR: Lazy<Mutex<Option<ConfigOrchestrator>>> = Lazy::new(|| Mutex::new(None));

impl ConfigOrchestrator {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(ConfigCacheState::default()),
        }
    }

    /// Load the config from disk if needed, otherwise return the cached copy.
    /// The cache is invalidated when the file's last-modified timestamp changes.
    fn load_with_cache(&self) -> Result<Config, Box<dyn std::error::Error>> {
        let mut state = self.state.lock().unwrap();
        let disk_modified = Self::modified_time(&self.path);
        if state.config.is_none() || state.modified != disk_modified {
            let config = Config::load_from_path(&self.path)?;
            state.modified = disk_modified;
            state.config = Some(config);
        }
        Ok(state.config.clone().unwrap_or_default())
    }

    /// Persist the provided config and refresh the cache snapshot on success.
    fn persist(&self, config: Config) -> Result<(), Box<dyn std::error::Error>> {
        self.write_config(&config)?;
        let mut state = self.state.lock().unwrap();
        state.modified = Self::modified_time(&self.path);
        state.config = Some(config);
        Ok(())
    }

    /// Apply a mutation closure against the latest config and write the result.
    /// Callers receive the closure's return value, while the orchestrator
    /// ensures a fresh snapshot is loaded and stored atomically.
    fn mutate<F, T>(&self, mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            let disk_modified = Self::modified_time(&self.path);
            if state.config.is_none() || state.modified != disk_modified {
                let config = Config::load_from_path(&self.path)?;
                state.modified = disk_modified;
                state.config = Some(config);
            }
            state.config.clone().unwrap_or_default()
        };

        let mut working = snapshot;
        let result = mutator(&mut working)?;
        self.persist(working)?;
        Ok(result)
    }

    /// Write the config using atomic temp-file persistence to avoid corruption.
    fn write_config(&self, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        config.save_to_path(&self.path)
    }

    fn modified_time(path: &PathBuf) -> Option<SystemTime> {
        fs::metadata(path).ok()?.modified().ok()
    }
}

impl Config {
    /// Load the user's config through the shared orchestrator.
    /// Tests that provide an override orchestrator get isolated state.
    pub fn load() -> Result<Config, Box<dyn std::error::Error>> {
        #[cfg(test)]
        {
            if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
                return orchestrator.load_with_cache();
            }
        }
        CONFIG_ORCHESTRATOR.load_with_cache()
    }

    /// Load config, but return default config when in test mode to avoid side effects
    #[cfg(test)]
    pub fn load_test_safe() -> Config {
        Config::default()
    }

    /// Load config, but return default config when in test mode to avoid side effects
    #[cfg(not(test))]
    pub fn load_test_safe() -> Config {
        Self::load().unwrap_or_default()
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
        #[cfg(test)]
        {
            if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
                return orchestrator.persist(self.clone());
            }
        }
        CONFIG_ORCHESTRATOR.persist(self.clone())
    }

    /// Mutate the config and persist the result atomically.
    /// In normal builds this touches the real config file.
    #[cfg(not(test))]
    pub fn mutate<F, T>(mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        CONFIG_ORCHESTRATOR.mutate(mutator)
    }

    /// Mutate the config in tests.
    /// When a test sets a temporary config path we mutate that file, otherwise
    /// we operate on an in-memory default to avoid polluting user state.
    #[cfg(test)]
    pub fn mutate<F, T>(mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
            orchestrator.mutate(mutator)
        } else {
            let mut config = Config::default();
            let result = mutator(&mut config)?;
            Ok(result)
        }
    }

    #[cfg(test)]
    pub(crate) fn test_config_path() -> PathBuf {
        Self::get_config_path()
    }

    #[cfg(test)]
    /// Point the orchestrator at a test-specific path so integration tests can
    /// exercise real persistence without affecting production config files.
    pub(crate) fn set_test_config_path(path: PathBuf) {
        let mut guard = TEST_ORCHESTRATOR.lock().unwrap();
        *guard = Some(ConfigOrchestrator::new(path));
    }

    /// Remove any test override so subsequent tests fall back to in-memory
    /// defaults, ensuring isolation between suites.
    #[cfg(test)]
    pub(crate) fn clear_test_config_override() {
        let mut guard = TEST_ORCHESTRATOR.lock().unwrap();
        guard.take();
    }

    /// Serialize the config and atomically persist it next to `config_path`.
    /// Tests point the orchestrator at temporary locations, so this helper keeps
    /// the same safe write behavior without touching real user files.
    fn save_to_path(&self, config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let parent = config_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty());

        if let Some(dir) = parent {
            fs::create_dir_all(dir)?;
        }

        let contents = toml::to_string_pretty(self)?;
        let mut temp_file = match parent {
            Some(dir) => NamedTempFile::new_in(dir)?,
            None => NamedTempFile::new()?,
        };

        temp_file.write_all(contents.as_bytes())?;
        temp_file.as_file_mut().sync_all()?;
        temp_file
            .persist(config_path)
            .map_err(|err| -> Box<dyn std::error::Error> { Box::new(err) })?;
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
        self.print_default_characters();
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

    /// Get the default character for a provider/model combination
    /// Returns the character filename (without extension)
    pub fn get_default_character(&self, provider: &str, model: &str) -> Option<&String> {
        self.default_characters
            .get(&provider.to_lowercase())
            .and_then(|models| models.get(model))
    }

    /// Set the default character for a provider/model combination
    /// character_name should be the filename without extension
    pub fn set_default_character(
        &mut self,
        provider: String,
        model: String,
        character_name: String,
    ) {
        let provider_key = provider.to_lowercase();
        self.default_characters
            .entry(provider_key)
            .or_default()
            .insert(model, character_name);
    }

    /// Unset the default character for a provider/model combination
    pub fn unset_default_character(&mut self, provider: &str, model: &str) {
        if let Some(models) = self.default_characters.get_mut(&provider.to_lowercase()) {
            models.remove(model);
            // Clean up empty provider entries
            if models.is_empty() {
                self.default_characters.remove(&provider.to_lowercase());
            }
        }
    }

    /// Set the default persona for a provider/model combination
    /// persona_id should be the persona ID
    pub fn set_default_persona(&mut self, provider: String, model: String, persona_id: String) {
        let provider_key = provider.to_lowercase();
        self.default_personas
            .entry(provider_key)
            .or_default()
            .insert(model, persona_id);
    }

    /// Unset the default persona for a provider/model combination
    pub fn unset_default_persona(&mut self, provider: &str, model: &str) {
        if let Some(models) = self.default_personas.get_mut(&provider.to_lowercase()) {
            models.remove(model);
            // Clean up empty provider entries
            if models.is_empty() {
                self.default_personas.remove(&provider.to_lowercase());
            }
        }
    }

    /// Print all default characters
    pub fn print_default_characters(&self) {
        if self.default_characters.is_empty() {
            println!("  default-characters: (none set)");
        } else {
            println!("  default-characters:");
            let mut providers: Vec<_> = self.default_characters.iter().collect();
            providers.sort_by_key(|(k, _)| *k);
            for (provider, models) in providers {
                let mut model_entries: Vec<_> = models.iter().collect();
                model_entries.sort_by_key(|(k, _)| *k);
                for (model, character) in model_entries {
                    println!("    {}:{}: {}", provider, model, character);
                }
            }
        }
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

    pub fn get_custom_theme(&self, id: &str) -> Option<&CustomTheme> {
        self.custom_themes
            .iter()
            .find(|t| t.id.eq_ignore_ascii_case(id))
    }
    pub fn list_custom_themes(&self) -> Vec<&CustomTheme> {
        self.custom_themes.iter().collect()
    }
}

#[cfg(test)]
impl Config {
    // Custom themes management (used by tests)
    pub fn add_custom_theme(&mut self, theme: CustomTheme) {
        self.custom_themes.push(theme);
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
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn config_orchestrator_detects_external_updates() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("config.toml");
        let orchestrator = ConfigOrchestrator::new(config_path.clone());

        orchestrator
            .mutate(|config| {
                config.default_provider = Some("first".to_string());
                Ok(())
            })
            .expect("mutate failed");

        let persisted = Config::load_from_path(&config_path).expect("load failed");
        assert_eq!(persisted.default_provider.as_deref(), Some("first"));

        let cached = orchestrator.load_with_cache().expect("cached load failed");
        assert_eq!(cached.default_provider.as_deref(), Some("first"));

        std::thread::sleep(Duration::from_millis(1100));

        let mut external = Config::default();
        external.default_provider = Some("second".to_string());
        external
            .save_to_path(&config_path)
            .expect("external save failed");

        let reloaded = orchestrator.load_with_cache().expect("reload failed");
        assert_eq!(reloaded.default_provider.as_deref(), Some("second"));
    }

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

        // Set a default model using the provider ID (matching how defaults are persisted)
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
            selection_highlight: None,
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

    #[test]
    fn test_set_and_get_default_character() {
        let mut config = Config::default();

        // Set a default character for openai/gpt-4
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );

        // Verify we can retrieve it
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );

        // Verify it returns None for non-existent combinations
        assert_eq!(
            config.get_default_character("openai", "gpt-3.5-turbo"),
            None
        );
        assert_eq!(
            config.get_default_character("anthropic", "claude-3-opus"),
            None
        );
    }

    #[test]
    fn test_set_multiple_default_characters() {
        let mut config = Config::default();

        // Set default characters for multiple provider/model combinations
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "openai".to_string(),
            "gpt-4o".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "anthropic".to_string(),
            "claude-3-opus-20240229".to_string(),
            "bob".to_string(),
        );
        config.set_default_character(
            "anthropic".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "charlie".to_string(),
        );

        // Verify all can be retrieved
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );
        assert_eq!(
            config.get_default_character("openai", "gpt-4o"),
            Some(&"alice".to_string())
        );
        assert_eq!(
            config.get_default_character("anthropic", "claude-3-opus-20240229"),
            Some(&"bob".to_string())
        );
        assert_eq!(
            config.get_default_character("anthropic", "claude-3-5-sonnet-20241022"),
            Some(&"charlie".to_string())
        );
    }

    #[test]
    fn test_unset_default_character() {
        let mut config = Config::default();

        // Set default characters
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "openai".to_string(),
            "gpt-4o".to_string(),
            "bob".to_string(),
        );

        // Verify they're set
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );
        assert_eq!(
            config.get_default_character("openai", "gpt-4o"),
            Some(&"bob".to_string())
        );

        // Unset one
        config.unset_default_character("openai", "gpt-4");

        // Verify it's gone but the other remains
        assert_eq!(config.get_default_character("openai", "gpt-4"), None);
        assert_eq!(
            config.get_default_character("openai", "gpt-4o"),
            Some(&"bob".to_string())
        );
    }

    #[test]
    fn test_unset_last_character_cleans_up_provider() {
        let mut config = Config::default();

        // Set a single default character
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );

        // Verify the provider exists in the map
        assert!(config.default_characters.contains_key("openai"));

        // Unset the character
        config.unset_default_character("openai", "gpt-4");

        // Verify the provider entry is cleaned up
        assert!(!config.default_characters.contains_key("openai"));
    }

    #[test]
    fn test_default_character_case_insensitive_provider() {
        let mut config = Config::default();

        // Set with mixed case provider name
        config.set_default_character(
            "OpenAI".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );

        // Verify we can retrieve with lowercase
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );

        // Verify we can retrieve with mixed case
        assert_eq!(
            config.get_default_character("OpenAI", "gpt-4"),
            Some(&"alice".to_string())
        );

        // Verify we can retrieve with uppercase
        assert_eq!(
            config.get_default_character("OPENAI", "gpt-4"),
            Some(&"alice".to_string())
        );
    }

    #[test]
    fn test_overwrite_default_character() {
        let mut config = Config::default();

        // Set a default character
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );

        // Verify it's set
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );

        // Overwrite with a different character
        config.set_default_character("openai".to_string(), "gpt-4".to_string(), "bob".to_string());

        // Verify it's updated
        assert_eq!(
            config.get_default_character("openai", "gpt-4"),
            Some(&"bob".to_string())
        );
    }

    #[test]
    fn test_save_and_load_default_characters() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_characters.toml");

        let mut config = Config::default();

        // Set multiple default characters
        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "anthropic".to_string(),
            "claude-3-opus-20240229".to_string(),
            "bob".to_string(),
        );

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load it back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify the characters are preserved
        assert_eq!(
            loaded_config.get_default_character("openai", "gpt-4"),
            Some(&"alice".to_string())
        );
        assert_eq!(
            loaded_config.get_default_character("anthropic", "claude-3-opus-20240229"),
            Some(&"bob".to_string())
        );
    }

    #[test]
    fn test_print_default_characters_empty() {
        let config = Config::default();

        // This should not panic and should print "(none set)"
        config.print_default_characters();
    }

    #[test]
    fn test_print_default_characters_with_data() {
        let mut config = Config::default();

        config.set_default_character(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice".to_string(),
        );
        config.set_default_character(
            "anthropic".to_string(),
            "claude-3-opus-20240229".to_string(),
            "bob".to_string(),
        );

        // This should not panic and should print the characters
        config.print_default_characters();
    }

    #[test]
    fn test_persona_serialization() {
        let persona = Persona {
            id: "test-persona".to_string(),
            display_name: "Test User".to_string(),
            bio: Some("A test persona for unit testing".to_string()),
        };

        // Test that persona can be serialized and deserialized
        let serialized = toml::to_string(&persona).expect("Failed to serialize persona");
        let deserialized: Persona =
            toml::from_str(&serialized).expect("Failed to deserialize persona");

        assert_eq!(deserialized.id, "test-persona");
        assert_eq!(deserialized.display_name, "Test User");
        assert_eq!(
            deserialized.bio,
            Some("A test persona for unit testing".to_string())
        );
    }

    #[test]
    fn test_persona_optional_bio() {
        let persona = Persona {
            id: "minimal-persona".to_string(),
            display_name: "Minimal User".to_string(),
            bio: None,
        };

        // Test that persona with no bio can be serialized and deserialized
        let serialized = toml::to_string(&persona).expect("Failed to serialize persona");
        let deserialized: Persona =
            toml::from_str(&serialized).expect("Failed to deserialize persona");

        assert_eq!(deserialized.id, "minimal-persona");
        assert_eq!(deserialized.display_name, "Minimal User");
        assert_eq!(deserialized.bio, None);
    }

    #[test]
    fn test_config_with_personas() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_personas.toml");

        let config = Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    display_name: "Alice".to_string(),
                    bio: Some("A senior software developer".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    display_name: "Bob".to_string(),
                    bio: None,
                },
            ],
            ..Default::default()
        };

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Load it back
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");

        // Verify personas were preserved
        assert_eq!(loaded_config.personas.len(), 2);

        let alice = &loaded_config.personas[0];
        assert_eq!(alice.id, "alice-dev");
        assert_eq!(alice.display_name, "Alice");
        assert_eq!(alice.bio, Some("A senior software developer".to_string()));

        let bob = &loaded_config.personas[1];
        assert_eq!(bob.id, "bob-student");
        assert_eq!(bob.display_name, "Bob");
        assert_eq!(bob.bio, None);
    }

    #[test]
    fn test_empty_personas_array() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config_path = temp_dir.path().join("test_empty_personas.toml");

        let config = Config::default();
        assert!(config.personas.is_empty());

        // Save and load to ensure empty array is handled correctly
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");
        let loaded_config = Config::load_from_path(&config_path).expect("Failed to load config");
        assert!(loaded_config.personas.is_empty());
    }

    #[test]
    fn test_path_display() {
        // Test with a simple path
        let path = PathBuf::from("/some/absolute/path");
        let display = path_display(&path);
        assert!(!display.is_empty());

        // On Unix, test home directory substitution
        #[cfg(unix)]
        {
            if let Some(home) = std::env::var_os("HOME") {
                let home_path = PathBuf::from(&home);
                let subpath = home_path.join("test/path");
                let display = path_display(&subpath);
                assert!(
                    display.starts_with("~/"),
                    "Expected path to start with ~/, got: {}",
                    display
                );
                assert!(display.contains("test/path"));
            }
        }

        // Test that non-home paths are not modified
        let abs_path = PathBuf::from("/usr/local/bin");
        let display = path_display(&abs_path);
        assert_eq!(display, "/usr/local/bin");
    }

    #[test]
    fn test_path_display_with_config_dir() {
        // Test with actual config directory
        let proj_dirs = ProjectDirs::from("org", "permacommons", "chabeau")
            .expect("Failed to determine config directory");
        let config_dir = proj_dirs.config_dir();
        let display = path_display(config_dir);

        // Should not be empty
        assert!(!display.is_empty());

        // Should contain "chabeau"
        assert!(display.contains("chabeau"));

        // On Unix with HOME set, should use tilde
        #[cfg(unix)]
        {
            if std::env::var_os("HOME").is_some() {
                assert!(display.starts_with('~') || display.starts_with('/'));
            }
        }
    }

    #[test]
    fn test_set_and_get_default_persona() {
        use crate::core::persona::PersonaManager;

        let mut config = Config::default();

        // Initially no default persona - test through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert!(manager
            .get_default_for_provider_model("openai", "gpt-4")
            .is_none());

        // Set a default persona
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );

        // Verify it's set through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4"),
            Some("alice-dev")
        );

        // Case insensitive provider lookup - test through PersonaManager
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("OPENAI", "gpt-4"),
            Some("alice-dev")
        );

        // Different model should return None
        assert!(manager
            .get_default_for_provider_model("openai", "gpt-3.5-turbo")
            .is_none());
    }

    #[test]
    fn test_unset_default_persona() {
        use crate::core::persona::PersonaManager;

        let mut config = Config::default();

        // Set default personas
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4o".to_string(),
            "bob-student".to_string(),
        );

        // Verify they're set through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4"),
            Some("alice-dev")
        );
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4o"),
            Some("bob-student")
        );

        // Unset one
        config.unset_default_persona("openai", "gpt-4");

        // Verify it's gone but the other remains through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert!(manager
            .get_default_for_provider_model("openai", "gpt-4")
            .is_none());
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4o"),
            Some("bob-student")
        );
    }

    #[test]
    fn test_unset_default_persona_cleans_up_empty_provider() {
        use crate::core::persona::PersonaManager;

        let mut config = Config::default();

        // Set a single default persona
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );

        // Verify it's set through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4"),
            Some("alice-dev")
        );

        // Unset the persona
        config.unset_default_persona("openai", "gpt-4");

        // Verify the provider entry is cleaned up
        assert!(config.default_personas.is_empty());

        // Also verify through PersonaManager that no defaults exist
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert!(manager
            .get_default_for_provider_model("openai", "gpt-4")
            .is_none());
    }

    #[test]
    fn test_overwrite_default_persona() {
        use crate::core::persona::PersonaManager;

        let mut config = Config::default();

        // Set a default persona
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "alice-dev".to_string(),
        );

        // Verify it's set through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4"),
            Some("alice-dev")
        );

        // Overwrite with a different persona
        config.set_default_persona(
            "openai".to_string(),
            "gpt-4".to_string(),
            "bob-student".to_string(),
        );

        // Verify it's updated through PersonaManager (production path)
        let manager = PersonaManager::load_personas(&config).expect("Failed to load personas");
        assert_eq!(
            manager.get_default_for_provider_model("openai", "gpt-4"),
            Some("bob-student")
        );
    }
}
