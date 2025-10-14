use crate::core::config::Preset;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BuiltinPresetConfig {
    presets: Vec<Preset>,
}

pub fn load_builtin_presets() -> Vec<Preset> {
    const CONFIG_CONTENT: &str = include_str!("../builtins/presets.toml");
    let config: BuiltinPresetConfig =
        toml::from_str(CONFIG_CONTENT).expect("Failed to parse builtins/presets.toml");
    config.presets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_has_expected_builtins() {
        let presets = load_builtin_presets();
        let ids: Vec<String> = presets.iter().map(|p| p.id.clone()).collect();
        assert!(ids.contains(&"short".to_string()));
        assert!(ids.contains(&"roleplay".to_string()));
        assert!(ids.contains(&"casual".to_string()));
    }
}
