use super::picker::{
    self, CharacterPickerState, ModelPickerState, PersonaPickerState, PickerMode, PickerSession,
    PresetPickerState, ProviderPickerState, ThemePickerState,
};
use super::ui_state::ActivityKind;
use super::App;
use crate::api::ModelsResponse;
use crate::core::config::data::Config;
use crate::ui::picker::PickerState;
use reqwest::Client;

#[derive(Clone)]
pub struct ModelPickerRequest {
    pub client: Client,
    pub base_url: String,
    pub api_key: String,
    pub provider_name: String,
    pub default_model_for_provider: Option<String>,
}

impl App {
    pub fn picker_session(&self) -> Option<&PickerSession> {
        self.picker.session()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn picker_session_mut(&mut self) -> Option<&mut PickerSession> {
        self.picker.session_mut()
    }

    pub fn current_picker_mode(&self) -> Option<PickerMode> {
        self.picker.current_mode()
    }

    pub fn picker_state(&self) -> Option<&PickerState> {
        self.picker.state()
    }

    pub fn picker_state_mut(&mut self) -> Option<&mut PickerState> {
        self.picker.state_mut()
    }

    pub fn theme_picker_state(&self) -> Option<&ThemePickerState> {
        self.picker.session().and_then(PickerSession::theme_state)
    }

    pub fn theme_picker_state_mut(&mut self) -> Option<&mut ThemePickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::theme_state_mut)
    }

    pub fn model_picker_state(&self) -> Option<&ModelPickerState> {
        self.picker.session().and_then(PickerSession::model_state)
    }

    pub fn model_picker_state_mut(&mut self) -> Option<&mut ModelPickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::model_state_mut)
    }

    pub fn provider_picker_state(&self) -> Option<&ProviderPickerState> {
        self.picker
            .session()
            .and_then(PickerSession::provider_state)
    }

    pub fn provider_picker_state_mut(&mut self) -> Option<&mut ProviderPickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::provider_state_mut)
    }

    pub fn close_picker(&mut self) {
        self.picker.close();
        self.close_inspect();
    }

    /// Open a theme picker modal with built-in and custom themes
    pub fn open_theme_picker(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.close_inspect();
        self.picker.open_theme_picker(&mut self.ui)
    }

    /// Apply theme temporarily for preview (does not persist config)
    pub fn preview_theme_by_id(&mut self, id: &str) {
        let mut controller = self.theme_controller();
        controller.preview_theme_by_id(id);
    }

    /// Revert theme to the one before opening picker (on cancel)
    pub fn revert_theme_preview(&mut self) {
        let mut controller = self.theme_controller();
        controller.revert_theme_preview();
    }

    /// Open a model picker modal with available models from current provider
    pub async fn open_model_picker(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.close_inspect();
        let request = self.prepare_model_picker_request()?;
        let ModelPickerRequest {
            client,
            base_url,
            api_key,
            provider_name,
            default_model_for_provider,
        } = request;

        let models_response =
            crate::api::models::fetch_models(&client, &base_url, &api_key, &provider_name).await?;

        self.complete_model_picker_request(default_model_for_provider, models_response)
    }

    pub fn prepare_model_picker_request(
        &mut self,
    ) -> Result<ModelPickerRequest, Box<dyn std::error::Error>> {
        self.ui.begin_activity(ActivityKind::ModelRequest);
        let cfg = Config::load_test_safe()?;
        let default_model_for_provider =
            cfg.get_default_model(&self.session.provider_name).cloned();

        Ok(ModelPickerRequest {
            client: self.session.client.clone(),
            base_url: self.session.base_url.clone(),
            api_key: self.session.api_key.clone(),
            provider_name: self.session.provider_name.clone(),
            default_model_for_provider,
        })
    }

    pub fn complete_model_picker_request(
        &mut self,
        default_model_for_provider: Option<String>,
        models_response: ModelsResponse,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.picker.populate_model_picker_from_response(
            &self.session,
            default_model_for_provider,
            models_response,
        );
        self.ui.end_activity(ActivityKind::ModelRequest);
        result
    }

    pub fn fail_model_picker_request(&mut self) {
        self.ui.end_activity(ActivityKind::ModelRequest);
    }

    /// Filter models based on search term and update picker
    pub fn filter_models(&mut self) {
        self.picker.filter_models();
    }

    /// Filter themes based on search term and update picker
    pub fn filter_themes(&mut self) {
        self.picker.filter_themes();
    }

    /// Filter providers based on search term and update picker
    pub fn filter_providers(&mut self) {
        self.picker.filter_providers();
    }

    /// Sort picker items based on current sort mode
    pub fn sort_picker_items(&mut self) {
        self.picker.sort_items();
    }

    /// Update picker title to show sort mode
    pub fn update_picker_title(&mut self) {
        self.picker.update_title();
    }

    /// Revert model to the one before opening picker (on cancel)
    pub fn revert_model_preview(&mut self) {
        self.picker.revert_model_preview(&mut self.session);
    }

    /// Open a provider picker modal with available providers
    pub fn open_provider_picker(&mut self) {
        self.close_inspect();
        if let Err(message) = self.picker.open_provider_picker(&self.session) {
            self.conversation().set_status(message);
        }
    }

    /// Revert provider to the one before opening picker (on cancel)
    pub fn revert_provider_preview(&mut self) {
        self.picker.revert_provider_preview(&mut self.session);
    }

    /// Clear provider->model transition state when model is successfully selected
    pub fn complete_provider_model_transition(&mut self) {
        self.picker.complete_provider_model_transition();
    }

    /// Open a character picker modal with available character cards
    pub fn open_character_picker(&mut self) {
        self.close_inspect();
        match self.character_service.list_metadata() {
            Ok(metadata) => {
                let mut cards = Vec::with_capacity(metadata.len());
                for entry in metadata {
                    match self.character_service.resolve_by_name(&entry.name) {
                        Ok(card) => cards.push(card),
                        Err(err) => {
                            self.conversation().set_status(format!(
                                "Error loading character '{}': {}",
                                entry.name, err
                            ));
                            return;
                        }
                    }
                }

                if let Err(message) = self.picker.open_character_picker(cards, &self.session) {
                    self.conversation().set_status(message);
                }
            }
            Err(err) => {
                self.conversation()
                    .set_status(format!("Error loading characters: {}", err));
            }
        }
    }

    /// Open a persona picker modal with available personas
    pub fn open_persona_picker(&mut self) {
        self.close_inspect();
        if let Err(message) = self
            .picker
            .open_persona_picker(&self.persona_manager, &self.session)
        {
            self.conversation().set_status(message);
        }
    }

    /// Apply the selected character from the picker
    pub fn apply_selected_character(&mut self, set_as_default: bool) {
        let character_name = self
            .picker
            .session()
            .and_then(|picker| picker.state.selected_id())
            .map(|s| s.to_string());

        if let Some(character_name) = character_name {
            // Check if user selected "turn off character mode"
            if character_name == picker::TURN_OFF_CHARACTER_ID {
                self.session.clear_character();
                self.conversation()
                    .set_status("Character mode disabled".to_string());
                self.close_picker();
                return;
            }

            match self.character_service.resolve_by_name(&character_name) {
                Ok(card) => {
                    let card_name = card.data.name.clone();
                    self.session.set_character(card);

                    // Show character greeting if present (won't show if already shown)
                    self.conversation().show_character_greeting_if_needed();

                    if set_as_default {
                        // Set as default for current provider/model
                        let provider = self.session.provider_name.clone();
                        let model = self.session.model.clone();

                        match Config::load() {
                            Ok(mut config) => {
                                config.set_default_character(
                                    provider.clone(),
                                    model.clone(),
                                    character_name.to_string(),
                                );
                                if let Err(e) = config.save() {
                                    self.conversation().set_status(format!(
                                        "Character set: {} (failed to save as default: {})",
                                        card_name, e
                                    ));
                                } else {
                                    self.conversation().set_status(format!(
                                        "Character set: {} (saved as default for {}:{})",
                                        card_name, provider, model
                                    ));
                                }
                            }
                            Err(e) => {
                                self.conversation().set_status(format!(
                                    "Character set: {} (failed to load config: {})",
                                    card_name, e
                                ));
                            }
                        }
                    } else {
                        self.conversation()
                            .set_status(format!("Character set: {}", card_name));
                    }
                }
                Err(e) => {
                    self.conversation()
                        .set_status(format!("Error loading character: {}", e));
                }
            }
        }
        self.close_picker();
    }

    /// Filter characters based on search term and update picker
    pub fn filter_characters(&mut self) {
        self.picker.filter_characters();
    }

    /// Apply the selected persona from the picker
    pub fn apply_selected_persona(&mut self, set_as_default: bool) {
        let persona_id = self
            .picker
            .session()
            .and_then(|picker| picker.state.selected_id())
            .map(|s| s.to_string());

        if let Some(persona_id) = persona_id {
            // Check if user selected "turn off persona"
            if persona_id == "[turn_off_persona]" {
                self.persona_manager.clear_active_persona();
                self.ui.update_user_display_name("You".to_string());
                self.conversation()
                    .set_status("Persona deactivated".to_string());
                self.close_picker();
                return;
            }

            match self.persona_manager.set_active_persona(&persona_id) {
                Ok(()) => {
                    let persona_name = self
                        .persona_manager
                        .get_active_persona()
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());
                    self.ui.update_user_display_name(persona_name.clone());

                    if set_as_default {
                        // Set as default for current provider/model
                        let provider = self.session.provider_name.clone();
                        let model = self.session.model.clone();

                        match self
                            .persona_manager
                            .set_default_for_provider_model_persistent(
                                &provider,
                                &model,
                                &persona_id,
                            ) {
                            Ok(()) => {
                                self.conversation().set_status(format!(
                                    "Persona activated: {} (saved as default for {}:{})",
                                    persona_name, provider, model
                                ));
                            }
                            Err(e) => {
                                self.conversation().set_status(format!(
                                    "Persona activated: {} (failed to save as default: {})",
                                    persona_name, e
                                ));
                            }
                        }
                    } else {
                        self.conversation()
                            .set_status(format!("Persona activated: {}", persona_name));
                    }
                }
                Err(e) => {
                    self.conversation()
                        .set_status(format!("Error activating persona: {}", e));
                }
            }
        }
        self.close_picker();
    }

    /// Filter personas based on search term and update picker
    pub fn filter_personas(&mut self) {
        self.picker.filter_personas();
    }

    /// Open a preset picker modal with available presets
    pub fn open_preset_picker(&mut self) {
        self.close_inspect();
        if let Err(message) = self
            .picker
            .open_preset_picker(&self.preset_manager, &self.session)
        {
            self.conversation().set_status(message);
        }
    }

    /// Apply the selected preset from the picker
    pub fn apply_selected_preset(&mut self, set_as_default: bool) {
        let preset_id = self
            .picker
            .session()
            .and_then(|picker| picker.state.selected_id())
            .map(|s| s.to_string());

        if let Some(preset_id) = preset_id {
            if preset_id == picker::TURN_OFF_PRESET_ID {
                self.preset_manager.clear_active_preset();
                self.conversation()
                    .set_status("Preset deactivated".to_string());
                self.close_picker();
                return;
            }

            match self.preset_manager.set_active_preset(&preset_id) {
                Ok(()) => {
                    if set_as_default {
                        let provider = self.session.provider_name.clone();
                        let model = self.session.model.clone();
                        match self
                            .preset_manager
                            .set_default_for_provider_model_persistent(
                                &provider, &model, &preset_id,
                            ) {
                            Ok(()) => {
                                self.conversation().set_status(format!(
                                    "Preset activated: {} (saved as default for {}:{})",
                                    preset_id, provider, model
                                ));
                            }
                            Err(e) => {
                                self.conversation().set_status(format!(
                                    "Preset activated: {} (failed to save as default: {})",
                                    preset_id, e
                                ));
                            }
                        }
                    } else {
                        self.conversation()
                            .set_status(format!("Preset activated: {}", preset_id));
                    }
                }
                Err(e) => {
                    self.conversation()
                        .set_status(format!("Preset error: {}", e));
                }
            }
        }

        self.close_picker();
    }

    /// Filter presets based on search term and update picker
    pub fn filter_presets(&mut self) {
        self.picker.filter_presets();
    }

    /// Get character picker state accessor
    pub fn character_picker_state(&self) -> Option<&CharacterPickerState> {
        self.picker
            .session()
            .and_then(PickerSession::character_state)
    }

    /// Get mutable character picker state accessor
    pub fn character_picker_state_mut(&mut self) -> Option<&mut CharacterPickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::character_state_mut)
    }

    /// Get persona picker state accessor
    pub fn persona_picker_state(&self) -> Option<&PersonaPickerState> {
        self.picker.session().and_then(PickerSession::persona_state)
    }

    /// Get mutable persona picker state accessor
    pub fn persona_picker_state_mut(&mut self) -> Option<&mut PersonaPickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::persona_state_mut)
    }

    /// Get preset picker state accessor
    pub fn preset_picker_state(&self) -> Option<&PresetPickerState> {
        self.picker.session().and_then(PickerSession::preset_state)
    }

    /// Get mutable preset picker state accessor
    pub fn preset_picker_state_mut(&mut self) -> Option<&mut PresetPickerState> {
        self.picker
            .session_mut()
            .and_then(PickerSession::preset_state_mut)
    }
}
