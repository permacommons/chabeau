//! Core application state and lifecycle management.
//!
//! This module contains the [`App`] struct, which is the heart of the runtime.
//! It packages the current session, conversation, UI state, pickers, and services
//! into a single owner that can be mutated atomically within the async event loop.
//!
//! Initialization involves loading configuration, resolving authentication,
//! activating personas and presets, and preparing character greetings. The
//! module also provides action dispatching for UI events and background commands.

use crate::character::service::CharacterService;
use crate::core::config::data::Config;
use crate::core::message::AppMessageKind;
use crate::core::providers::ProviderSession;
use crate::mcp::client::McpClientManager;
use crate::mcp::permissions::ToolPermissionStore;

pub mod actions;
pub mod conversation;
pub mod picker;
pub mod session;
pub mod settings;
pub mod ui_state;

#[allow(clippy::module_inception)]
mod app;
mod pickers;
mod streaming;
#[cfg(test)]
mod tests;
mod ui_helpers;

pub use actions::{
    apply_actions, AppAction, AppActionContext, AppActionDispatcher, AppActionEnvelope, AppCommand,
};
#[allow(unused_imports)]
pub use conversation::ConversationController;
#[cfg(test)]
pub use picker::PickerData;
#[allow(unused_imports)]
pub use picker::{
    CharacterPickerState, ModelPickerState, PersonaPickerState, PickerController,
    PickerInspectState, PickerMode, PickerSession, PresetPickerState, ProviderPickerState,
    ThemePickerState,
};
pub use pickers::ModelPickerRequest;
pub use session::{SessionBootstrap, SessionContext, UninitializedSessionBootstrap};
#[allow(unused_imports)]
pub use settings::{ProviderController, ThemeController};
#[allow(unused_imports)]
pub use ui_state::{ActivityKind, UiState, VerticalCursorDirection};

/// Configuration parameters for initializing an App with authentication.
///
/// This structure is passed to [`new_with_auth`] to control session setup,
/// including which provider, model, and character to use, as well as optional
/// persona and preset overrides.
pub struct AppInitConfig {
    /// Model identifier to use for the session.
    pub model: String,

    /// Optional path to a log file for recording API interactions.
    pub log_file: Option<String>,

    /// Provider ID to use (overrides config default if specified).
    pub provider: Option<String>,

    /// If true, use only environment variables for authentication (skip keyring).
    pub env_only: bool,

    /// Pre-resolved provider session (bypasses normal provider resolution).
    pub pre_resolved_session: Option<ProviderSession>,

    /// Character card to load (name or path).
    pub character: Option<String>,

    /// Persona ID to activate for this session.
    pub persona: Option<String>,

    /// Preset ID to activate for this session.
    pub preset: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn build_app(
    session: SessionContext,
    ui: UiState,
    picker: PickerController,
    character_service: CharacterService,
    persona_manager: crate::core::persona::PersonaManager,
    preset_manager: crate::core::preset::PresetManager,
    config: Config,
    mcp: McpClientManager,
) -> App {
    let mut app = App {
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config,
        mcp,
        mcp_permissions: ToolPermissionStore::default(),
    };

    app.ui.set_input_text(String::new());
    app.configure_textarea_appearance();

    let display_name = app.persona_manager.get_display_name();
    app.ui.update_user_display_name(display_name);

    app
}

/// Creates a new authenticated application instance.
///
/// This initializes the full application state including session authentication,
/// personas, presets, and character configuration. Use this for normal interactive
/// chat sessions where credentials have been configured.
///
/// The function resolves authentication, loads the specified or default persona
/// and preset for the provider/model combination, and displays any startup errors
/// in the conversation transcript.
///
/// # Arguments
///
/// * `init_config` - Configuration controlling session initialization
/// * `config` - User configuration loaded from disk
/// * `character_service` - Service for loading and caching character cards
///
/// # Errors
///
/// Returns an error if:
/// - Authentication resolution fails (no credentials found)
/// - Persona or preset loading encounters an error
/// - Character card loading fails
/// - Session bootstrap fails
///
/// # Examples
///
/// ```no_run
/// # use chabeau::core::app::{new_with_auth, AppInitConfig};
/// # use chabeau::core::config::data::Config;
/// # use chabeau::character::service::CharacterService;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Config::load()?;
/// let character_service = CharacterService::new();
/// let init_config = AppInitConfig {
///     model: "gpt-4".to_string(),
///     log_file: None,
///     provider: None,
///     env_only: false,
///     pre_resolved_session: None,
///     character: None,
///     persona: None,
///     preset: None,
/// };
///
/// let app = new_with_auth(init_config, &config, character_service).await?;
/// # Ok(())
/// # }
/// ```
pub async fn new_with_auth(
    init_config: AppInitConfig,
    config: &Config,
    mut character_service: CharacterService,
) -> Result<App, Box<dyn std::error::Error>> {
    let SessionBootstrap {
        session,
        theme,
        startup_requires_provider,
        mut startup_errors,
    } = session::prepare_with_auth(
        init_config.model,
        init_config.log_file,
        init_config.provider,
        init_config.env_only,
        config,
        init_config.pre_resolved_session,
        init_config.character,
        &mut character_service,
    )
    .await?;

    // Initialize PersonaManager and apply CLI persona if provided
    let mut persona_manager = crate::core::persona::PersonaManager::load_personas(config)?;
    if let Some(persona_id) = init_config.persona {
        persona_manager.set_active_persona(&persona_id)?;
    } else {
        // Load default persona for current provider/model if no CLI persona specified
        if let Some(default_persona_id) =
            persona_manager.get_default_for_provider_model(&session.provider_name, &session.model)
        {
            let default_persona_id = default_persona_id.to_string(); // Clone to avoid borrow issues
            if let Err(e) = persona_manager.set_active_persona(&default_persona_id) {
                startup_errors.push(format!(
                    "Could not load default persona '{}': {}",
                    default_persona_id, e
                ));
            }
        }
    }

    // Initialize PresetManager and apply CLI preset if provided
    let mut preset_manager = crate::core::preset::PresetManager::load_presets(config)?;
    if let Some(preset_id) = init_config.preset {
        preset_manager.set_active_preset(&preset_id)?;
    } else if let Some(default_preset_id) =
        preset_manager.get_default_for_provider_model(&session.provider_name, &session.model)
    {
        let default_preset_id = default_preset_id.to_string();
        if let Err(e) = preset_manager.set_active_preset(&default_preset_id) {
            startup_errors.push(format!(
                "Could not load default preset '{}': {}",
                default_preset_id, e
            ));
        }
    }

    let ui = UiState::from_config(theme, config);
    let picker = PickerController::new();
    let mcp = McpClientManager::from_config(config);

    let mut app = build_app(
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config.clone(),
        mcp,
    );

    // Add log startup message if logging is active
    if app.session.logging.is_active() {
        let timestamp = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string();
        let log_message = format!("Logging started at {}", timestamp);
        app.conversation()
            .add_app_message(AppMessageKind::Log, log_message);
    }

    if startup_requires_provider {
        app.picker.startup_requires_provider = true;
    }

    if !startup_errors.is_empty() {
        let mut conversation = app.conversation();
        for error in startup_errors {
            conversation.add_app_message(AppMessageKind::Error, error);
        }
    }

    Ok(app)
}

/// Creates an uninitialized application instance without authentication.
///
/// This creates an app in a state where no provider or model is configured,
/// typically used when no credentials are available and the user needs to
/// select a provider interactively. The UI will show a provider picker on
/// startup.
///
/// Use this instead of [`new_with_auth`] when authentication cannot be
/// resolved (e.g., no keyring credentials and no environment variables).
///
/// # Arguments
///
/// * `log_file` - Optional path to a log file for recording API interactions
/// * `character_service` - Service for loading and caching character cards
///
/// # Errors
///
/// Returns an error if session bootstrap or configuration loading fails.
pub async fn new_uninitialized(
    log_file: Option<String>,
    mut character_service: CharacterService,
) -> Result<App, Box<dyn std::error::Error>> {
    let UninitializedSessionBootstrap {
        session,
        theme,
        config,
        startup_requires_provider,
    } = session::prepare_uninitialized(log_file, &mut character_service).await?;

    // Initialize PersonaManager (no CLI persona for uninitialized app)
    let persona_manager = crate::core::persona::PersonaManager::load_personas(&config)?;
    let preset_manager = crate::core::preset::PresetManager::load_presets(&config)?;

    let ui = UiState::from_config(theme, &config);
    let picker = PickerController::new();
    let mcp = McpClientManager::from_config(&config);

    let mut app = build_app(
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config.clone(),
        mcp,
    );

    if startup_requires_provider {
        app.picker.startup_requires_provider = true;
    }

    Ok(app)
}

/// The main application state container.
///
/// This struct is the heart of the runtime, packaging all session state,
/// UI state, pickers, and services into a single owner that can be mutated
/// atomically within the async event loop.
///
/// The app is typically created via [`new_with_auth`] for authenticated
/// sessions or [`new_uninitialized`] when credentials are not available.
/// Once created, the app is wrapped in an async mutex and passed to the
/// UI event loop.
///
/// Access to specific controllers is provided through methods like
/// [`theme_controller`](crate::core::app::App::theme_controller),
/// [`provider_controller`](crate::core::app::App::provider_controller), and
/// [`conversation`](crate::core::app::App::conversation).
pub struct App {
    /// Active session context (provider, model, API client, theme).
    pub session: SessionContext,

    /// UI state (messages, input, scroll, streaming status).
    pub ui: UiState,

    /// Picker controller (theme, provider, model, character pickers).
    pub picker: PickerController,

    /// Character card service for loading and caching character cards.
    pub character_service: CharacterService,

    /// Persona manager for user identity and system prompts.
    pub persona_manager: crate::core::persona::PersonaManager,

    /// Preset manager for prompt templates and refinement settings.
    pub preset_manager: crate::core::preset::PresetManager,

    /// User configuration loaded from disk.
    pub config: Config,

    /// MCP client runtime and cached server listings.
    pub mcp: McpClientManager,

    /// Tool permission decisions for MCP tools.
    pub mcp_permissions: ToolPermissionStore,
}
