use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    pub app_info_prefix: Option<String>,
    pub app_info_prefix_style: Option<String>,
    pub app_info_text: Option<String>,
    pub app_warning_prefix: Option<String>,
    pub app_warning_prefix_style: Option<String>,
    pub app_warning_text: Option<String>,
    pub app_error_prefix: Option<String>,
    pub app_error_prefix_style: Option<String>,
    pub app_error_text: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Preset {
    pub id: String,
    #[serde(default)]
    pub pre: String,
    #[serde(default)]
    pub post: String,
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
    /// Include built-in presets shipped with the binary
    #[serde(default)]
    pub builtin_presets: Option<bool>,
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
    /// Default presets for provider/model combinations
    /// Outer key: provider (e.g., "openai")
    /// Inner key: model (e.g., "gpt-4")
    /// Value: preset ID (e.g., "concise")
    #[serde(default)]
    pub default_presets: HashMap<String, HashMap<String, String>>,
    /// User-defined personas for conversation contexts
    #[serde(default)]
    pub personas: Vec<Persona>,
    /// User-defined presets for conversation contexts
    #[serde(default)]
    pub presets: Vec<Preset>,
    pub refine_instructions: Option<String>,
    pub refine_prefix: Option<String>,
}

/// Get a user-friendly display string for a path
/// Converts absolute paths to use ~ notation on Unix-like systems when possible
///
/// # Examples
/// - Unix: `/home/user/.config/chabeau/cards` → `~/.config/chabeau/cards`
/// - Windows: `C:\\Users\\user\\AppData\\Roaming\\chabeau\\cards` → `C:\\Users\\user\\AppData\\Roaming\\chabeau\\cards`
/// - macOS: `/Users/user/Library/Application Support/...` → `~/Library/Application Support/...`
pub fn path_display<P: AsRef<Path>>(path: P) -> String {
    let path = path.as_ref();

    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(home);
            if let Ok(relative) = path.strip_prefix(&home_path) {
                return format!("~/{}", relative.display());
            }
        }
    }

    path.display().to_string()
}

impl Config {
    pub fn add_custom_provider(&mut self, provider: CustomProvider) {
        self.custom_providers.push(provider);
    }

    pub fn remove_custom_provider(&mut self, id: &str) {
        self.custom_providers
            .retain(|p| !p.id.eq_ignore_ascii_case(id));
    }

    pub fn get_custom_provider(&self, id: &str) -> Option<&CustomProvider> {
        self.custom_providers
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(id))
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
