use super::{picker::PickerController, session::SessionContext, ui_state::UiState};
use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::ui::builtin_themes::{find_builtin_theme, theme_spec_from_custom};
use crate::ui::theme::Theme;

pub struct ThemeController<'a> {
    ui: &'a mut UiState,
    picker: &'a mut PickerController,
}

impl<'a> ThemeController<'a> {
    pub fn new(ui: &'a mut UiState, picker: &'a mut PickerController) -> Self {
        Self { ui, picker }
    }

    fn apply_theme(&mut self, theme: Theme) {
        self.ui.theme = crate::utils::color::quantize_theme_for_current_terminal(theme);
        self.ui.configure_textarea();
    }

    fn resolve_theme(id: &str, config: &Config) -> Result<Theme, String> {
        if let Some(custom) = config.get_custom_theme(id) {
            Ok(Theme::from_spec(&theme_spec_from_custom(custom)))
        } else if let Some(spec) = find_builtin_theme(id) {
            Ok(Theme::from_spec(&spec))
        } else {
            Err(format!("Unknown theme: {}", id))
        }
    }

    pub fn apply_theme_by_id(&mut self, id: &str) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        let theme = Self::resolve_theme(id, &config)?;
        self.apply_theme(theme);
        self.ui.current_theme_id = Some(id.to_string());

        config.theme = Some(id.to_string());
        config.save_test_safe().map_err(|e| e.to_string())?;

        if let Some(state) = self.picker.theme_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
        }

        Ok(())
    }

    pub fn apply_theme_by_id_session_only(&mut self, id: &str) -> Result<(), String> {
        let config = Config::load_test_safe();
        let theme = Self::resolve_theme(id, &config)?;
        self.apply_theme(theme);
        self.ui.current_theme_id = Some(id.to_string());

        if let Some(state) = self.picker.theme_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
        }

        Ok(())
    }

    pub fn preview_theme_by_id(&mut self, id: &str) {
        let config = Config::load_test_safe();
        if let Ok(theme) = Self::resolve_theme(id, &config) {
            self.apply_theme(theme);
        }
    }

    pub fn revert_theme_preview(&mut self) {
        let previous_theme = self
            .picker
            .theme_state()
            .and_then(|state| state.before_theme.clone());

        if let Some(state) = self.picker.theme_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
            state.search_filter.clear();
            state.all_items.clear();
        }

        if let Some(theme) = previous_theme {
            self.apply_theme(theme);
        }
    }

    pub fn unset_default_theme(&mut self) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.theme = None;
        config.save_test_safe().map_err(|e| e.to_string())
    }
}

pub struct ProviderController<'a> {
    session: &'a mut SessionContext,
    picker: &'a mut PickerController,
}

impl<'a> ProviderController<'a> {
    pub fn new(session: &'a mut SessionContext, picker: &'a mut PickerController) -> Self {
        Self { session, picker }
    }

    pub fn apply_model_by_id(&mut self, model_id: &str) {
        self.session.model = model_id.to_string();
        if let Some(state) = self.picker.model_state_mut() {
            state.before_model = None;
        }
        if self.picker.in_provider_model_transition {
            self.picker.in_provider_model_transition = false;
            self.picker.provider_model_transition_state = None;
        }
    }

    pub fn apply_model_by_id_persistent(&mut self, model_id: &str) -> Result<(), String> {
        self.apply_model_by_id(model_id);
        let mut config = Config::load_test_safe();
        config.set_default_model(self.session.provider_name.clone(), model_id.to_string());
        config.save_test_safe().map_err(|e| e.to_string())
    }

    pub fn apply_provider_by_id(&mut self, provider_id: &str) -> (Result<(), String>, bool) {
        let auth_manager = AuthManager::new();
        let config = Config::load_test_safe();

        match auth_manager.resolve_authentication(Some(provider_id), &config) {
            Ok((api_key, base_url, provider_name, provider_display_name)) => {
                let open_model_picker =
                    if let Some(default_model) = config.get_default_model(&provider_name) {
                        self.picker.in_provider_model_transition = false;
                        self.picker.provider_model_transition_state = None;
                        self.session.api_key = api_key;
                        self.session.base_url = base_url;
                        self.session.provider_name = provider_name.clone();
                        self.session.provider_display_name = provider_display_name;
                        self.session.model = default_model.clone();
                        false
                    } else {
                        self.picker.in_provider_model_transition = true;
                        self.picker.provider_model_transition_state = Some((
                            self.session.provider_name.clone(),
                            self.session.provider_display_name.clone(),
                            self.session.model.clone(),
                            self.session.api_key.clone(),
                            self.session.base_url.clone(),
                        ));

                        self.session.api_key = api_key;
                        self.session.base_url = base_url;
                        self.session.provider_name = provider_name.clone();
                        self.session.provider_display_name = provider_display_name;
                        true
                    };

                if let Some(state) = self.picker.provider_state_mut() {
                    state.before_provider = None;
                }

                (Ok(()), open_model_picker)
            }
            Err(e) => (Err(e.to_string()), false),
        }
    }

    pub fn apply_provider_by_id_persistent(
        &mut self,
        provider_id: &str,
    ) -> (Result<(), String>, bool) {
        let (result, should_open_model_picker) = self.apply_provider_by_id(provider_id);
        if let Err(e) = result {
            return (Err(e), false);
        }

        let mut config = Config::load_test_safe();
        config.default_provider = Some(provider_id.to_string());
        match config.save_test_safe() {
            Ok(_) => (Ok(()), should_open_model_picker),
            Err(e) => (Err(e.to_string()), false),
        }
    }

    pub fn unset_default_model(&mut self, provider: &str) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.unset_default_model(provider);
        config.save_test_safe().map_err(|e| e.to_string())
    }

    pub fn unset_default_provider(&mut self) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.default_provider = None;
        config.save_test_safe().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::create_test_app;

    #[test]
    fn apply_theme_session_only_updates_ui() {
        let mut app = create_test_app();
        let previous_background = app.ui.theme.background_color;
        let mut controller = ThemeController::new(&mut app.ui, &mut app.picker);
        controller
            .apply_theme_by_id_session_only("light")
            .expect("theme should apply");

        assert_eq!(app.ui.current_theme_id.as_deref(), Some("light"));
        assert_ne!(app.ui.theme.background_color, previous_background);
    }

    #[test]
    fn preview_theme_preserves_current_theme_id() {
        let mut app = create_test_app();
        app.ui.current_theme_id = Some("dark".to_string());
        let mut controller = ThemeController::new(&mut app.ui, &mut app.picker);

        controller.preview_theme_by_id("light");

        assert_eq!(app.ui.current_theme_id.as_deref(), Some("dark"));
    }

    #[test]
    fn apply_model_clears_transition_state() {
        let mut app = create_test_app();
        app.picker.in_provider_model_transition = true;
        app.picker.provider_model_transition_state = Some((
            "prev-provider".into(),
            "Prev".into(),
            "prev-model".into(),
            "prev-key".into(),
            "https://prev.example".into(),
        ));

        let mut controller = ProviderController::new(&mut app.session, &mut app.picker);
        controller.apply_model_by_id("new-model");

        assert_eq!(app.session.model, "new-model");
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
    }
}
