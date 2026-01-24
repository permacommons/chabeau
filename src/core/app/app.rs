use super::conversation::ConversationController;
use super::settings::{ProviderController, ThemeController};
use super::App;

#[cfg(any(test, feature = "bench"))]
use super::inspect::InspectController;
#[cfg(any(test, feature = "bench"))]
use super::picker::PickerController;
#[cfg(any(test, feature = "bench"))]
use super::session::SessionContext;
#[cfg(any(test, feature = "bench"))]
use super::ui_state::UiState;
#[cfg(any(test, feature = "bench"))]
use crate::character::service::CharacterService;
#[cfg(any(test, feature = "bench"))]
use crate::core::config::data::{DEFAULT_REFINE_INSTRUCTIONS, DEFAULT_REFINE_PREFIX};
#[cfg(any(test, feature = "bench"))]
use crate::mcp::client::McpClientManager;
#[cfg(any(test, feature = "bench"))]
use crate::ui::theme::Theme;

impl App {
    /// Returns a controller for theme-related operations.
    ///
    /// The theme controller provides methods to switch themes, open the
    /// theme picker, and manage theme state.
    pub fn theme_controller(&mut self) -> ThemeController<'_> {
        ThemeController::new(&mut self.ui, &mut self.picker)
    }

    /// Returns a controller for provider-related operations.
    ///
    /// The provider controller handles switching providers, opening the
    /// provider picker, and managing provider/model state.
    pub fn provider_controller(&mut self) -> ProviderController<'_> {
        ProviderController::new(&mut self.session, &mut self.picker)
    }

    /// Returns a controller for conversation operations.
    ///
    /// The conversation controller provides methods to add messages, clear
    /// the conversation, apply personas and presets, and manage the message
    /// history.
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
            last_refine_prompt: None,
            refine_instructions: DEFAULT_REFINE_INSTRUCTIONS.to_string(),
            refine_prefix: DEFAULT_REFINE_PREFIX.to_string(),
            startup_env_only: false,
            mcp_disabled: false,
            active_character: None,
            character_greeting_shown: false,
            has_received_assistant_message: false,
            pending_tool_calls: std::collections::BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: std::collections::VecDeque::new(),
            active_tool_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        let ui = UiState::new_basic(theme, markdown_enabled, syntax_enabled, None);

        // Create a test PersonaManager with empty config
        let test_config = crate::core::config::data::Config::default();
        let persona_manager = crate::core::persona::PersonaManager::load_personas(&test_config)
            .expect("Failed to create test PersonaManager");
        let preset_manager = crate::core::preset::PresetManager::load_presets(&test_config)
            .expect("Failed to create test PresetManager");
        let mcp = McpClientManager::from_config(&test_config);

        App {
            session,
            ui,
            picker: PickerController::new(),
            inspect: InspectController::new(),
            character_service: CharacterService::new(),
            persona_manager,
            preset_manager,
            config: test_config,
            mcp,
            mcp_permissions: crate::mcp::permissions::ToolPermissionStore::default(),
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
