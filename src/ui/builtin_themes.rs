use crate::core::config::data::CustomTheme;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeSpec {
    pub id: String,
    pub display_name: String,
    pub background: Option<String>,
    pub cursor_color: Option<String>,
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
    pub app_log_prefix: Option<String>,
    pub app_log_prefix_style: Option<String>,
    pub app_log_text: Option<String>,
    pub title: Option<String>,
    pub streaming_indicator: Option<String>,
    pub selection_highlight: Option<String>,
    pub input_border: Option<String>,
    pub input_title: Option<String>,
    pub input_text: Option<String>,
    pub input_cursor_modifiers: Option<String>,
    // Markdown extensions (all optional)
    pub md_h1: Option<String>,
    pub md_h2: Option<String>,
    pub md_h3: Option<String>,
    pub md_h4: Option<String>,
    pub md_h5: Option<String>,
    pub md_h6: Option<String>,
    pub md_paragraph: Option<String>,
    pub md_inline_code: Option<String>,
    pub md_link: Option<String>,
    pub md_rule: Option<String>,
    pub md_blockquote_text: Option<String>,
    pub md_list_marker: Option<String>,
    pub md_codeblock_text: Option<String>,
    pub md_codeblock_bg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BuiltinThemesConfig {
    themes: Vec<ThemeSpec>,
}

pub fn load_builtin_themes() -> Vec<ThemeSpec> {
    const CONFIG_CONTENT: &str = include_str!("../builtins/themes.toml");
    let config: BuiltinThemesConfig =
        toml::from_str(CONFIG_CONTENT).expect("Failed to parse builtins/themes.toml");
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
        cursor_color: ct.cursor_color.clone(),
        user_prefix: ct.user_prefix.clone(),
        user_text: ct.user_text.clone(),
        assistant_text: ct.assistant_text.clone(),
        system_text: ct.system_text.clone(),
        app_info_prefix: ct.app_info_prefix.clone(),
        app_info_prefix_style: ct.app_info_prefix_style.clone(),
        app_info_text: ct.app_info_text.clone(),
        app_warning_prefix: ct.app_warning_prefix.clone(),
        app_warning_prefix_style: ct.app_warning_prefix_style.clone(),
        app_warning_text: ct.app_warning_text.clone(),
        app_error_prefix: ct.app_error_prefix.clone(),
        app_error_prefix_style: ct.app_error_prefix_style.clone(),
        app_error_text: ct.app_error_text.clone(),
        app_log_prefix: ct.app_log_prefix.clone(),
        app_log_prefix_style: ct.app_log_prefix_style.clone(),
        app_log_text: ct.app_log_text.clone(),
        title: ct.title.clone(),
        streaming_indicator: ct.streaming_indicator.clone(),
        selection_highlight: ct.selection_highlight.clone(),
        input_border: ct.input_border.clone(),
        input_title: ct.input_title.clone(),
        input_text: ct.input_text.clone(),
        input_cursor_modifiers: ct.input_cursor_modifiers.clone(),
        md_h1: None,
        md_h2: None,
        md_h3: None,
        md_h4: None,
        md_h5: None,
        md_h6: None,
        md_paragraph: None,
        md_inline_code: None,
        md_link: None,
        md_rule: None,
        md_blockquote_text: None,
        md_list_marker: None,
        md_codeblock_text: None,
        md_codeblock_bg: None,
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
