use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub default_provider: Option<String>,
    #[serde(default)]
    pub default_models: HashMap<String, String>,
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
        let mut config = Config::default();
        config.default_provider = Some("test-provider".to_string());

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
        let mut config = Config::default();
        config.default_provider = Some("test-provider".to_string());

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Unset the default provider
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
        let mut config = Config::default();
        config.default_provider = Some("initial-provider".to_string());

        // Save the config
        config
            .save_to_path(&config_path)
            .expect("Failed to save config");

        // Change the default provider
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
}
