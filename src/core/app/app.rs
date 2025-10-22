use super::conversation::ConversationController;
use super::settings::{ProviderController, ThemeController};
use super::App;

#[cfg(any(test, feature = "bench"))]
use super::picker::PickerController;
#[cfg(any(test, feature = "bench"))]
use super::session::SessionContext;
#[cfg(any(test, feature = "bench"))]
use super::ui_state::UiState;
#[cfg(any(test, feature = "bench"))]
use crate::character::service::CharacterService;
#[cfg(any(test, feature = "bench"))]
use crate::ui::theme::Theme;

impl App {
    pub fn theme_controller(&mut self) -> ThemeController<'_> {
        ThemeController::new(&mut self.ui, &mut self.picker)
    }

    pub fn provider_controller(&mut self) -> ProviderController<'_> {
        ProviderController::new(&mut self.session, &mut self.picker)
    }

    pub fn conversation(&mut self) -> ConversationController<'_> {
        ConversationController::new(
            &mut self.session,
            &mut self.ui,
            &self.persona_manager,
            &self.preset_manager,
        )
    }

    #[cfg(any(test, feature = "bench"))]
    pub fn new_test_app(theme: Theme, markdown_enabled: bool, syntax_enabled: bool) -> Self {
        let session = SessionContext {
            client: reqwest::Client::new(),
            model: "bench".into(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: "bench".into(),
            provider_display_name: "Bench".into(),
            logging: crate::utils::logging::LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            is_refining: false,
            original_refining_content: None,
            startup_env_only: false,
            active_character: None,
            character_greeting_shown: false,
            has_received_assistant_message: false,
        };

        let ui = UiState::new_basic(theme, markdown_enabled, syntax_enabled, None);

        // Create a test PersonaManager with empty config
        let test_config = crate::core::config::data::Config::default();
        let persona_manager = crate::core::persona::PersonaManager::load_personas(&test_config)
            .expect("Failed to create test PersonaManager");
        let preset_manager = crate::core::preset::PresetManager::load_presets(&test_config)
            .expect("Failed to create test PresetManager");

        App {
            session,
            ui,
            picker: PickerController::new(),
            character_service: CharacterService::new(),
            persona_manager,
            preset_manager,
            config: test_config,
        }
    }

    // Used by Criterion benches in `benches/`.
    #[cfg(feature = "bench")]
    #[allow(dead_code)]
    pub fn new_bench(theme: Theme, markdown_enabled: bool, syntax_enabled: bool) -> Self {
        Self::new_test_app(theme, markdown_enabled, syntax_enabled)
    }
}

#[cfg(all(feature = "bench", not(test)))]
const _: fn(Theme, bool, bool) -> App = App::new_bench;
