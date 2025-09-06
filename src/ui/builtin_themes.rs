use crate::core::config::CustomTheme;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeSpec {
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

#[derive(Debug, Deserialize)]
struct BuiltinThemesConfig {
    themes: Vec<ThemeSpec>,
}

pub fn load_builtin_themes() -> Vec<ThemeSpec> {
    const CONFIG_CONTENT: &str = include_str!("../builtin_themes.toml");
    let config: BuiltinThemesConfig =
        toml::from_str(CONFIG_CONTENT).expect("Failed to parse builtin_themes.toml");
    config.themes
}

pub fn find_builtin_theme(id: &str) -> Option<ThemeSpec> {
    load_builtin_themes()
        .into_iter()
        .find(|t| t.id.eq_ignore_ascii_case(id))
}

/// Convert a `CustomTheme` from config into a `ThemeSpec` compatible with UI theming.
pub fn theme_spec_from_custom(ct: &CustomTheme) -> ThemeSpec {
    ThemeSpec {
        id: ct.id.clone(),
        display_name: ct.display_name.clone(),
        background: ct.background.clone(),
        user_prefix: ct.user_prefix.clone(),
        user_text: ct.user_text.clone(),
        assistant_text: ct.assistant_text.clone(),
        system_text: ct.system_text.clone(),
        title: ct.title.clone(),
        streaming_indicator: ct.streaming_indicator.clone(),
        input_border: ct.input_border.clone(),
        input_title: ct.input_title.clone(),
        input_text: ct.input_text.clone(),
        input_cursor_modifiers: ct.input_cursor_modifiers.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_has_expected_builtins() {
        let themes = load_builtin_themes();
        let ids: Vec<String> = themes.iter().map(|t| t.id.clone()).collect();
        assert!(ids.contains(&"dark".to_string()));
        assert!(ids.contains(&"light".to_string()));
        assert!(ids.contains(&"dracula".to_string()));
        assert!(ids.contains(&"solarized-dark".to_string()));
        assert!(ids.contains(&"solarized-light".to_string()));
        assert!(ids.contains(&"high-contrast-dark".to_string()));
        assert!(ids.contains(&"paper".to_string()));
    }

    #[test]
    fn find_builtin_theme_works_case_insensitive() {
        let t = find_builtin_theme("DaRk").expect("should find 'dark'");
        assert_eq!(t.id, "dark");
    }
}
