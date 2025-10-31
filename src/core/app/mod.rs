use crate::character::service::CharacterService;
use crate::core::config::data::Config;
use crate::core::message::AppMessageKind;
use crate::core::providers::ProviderSession;

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

/// Configuration parameters for initializing an App with authentication
pub struct AppInitConfig {
    pub model: String,
    pub log_file: Option<String>,
    pub provider: Option<String>,
    pub env_only: bool,
    pub pre_resolved_session: Option<ProviderSession>,
    pub character: Option<String>,
    pub persona: Option<String>,
    pub preset: Option<String>,
}

fn build_app(
    session: SessionContext,
    ui: UiState,
    picker: PickerController,
    character_service: CharacterService,
    persona_manager: crate::core::persona::PersonaManager,
    preset_manager: crate::core::preset::PresetManager,
    config: Config,
) -> App {
    let mut app = App {
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config,
    };

    app.ui.set_input_text(String::new());
    app.configure_textarea_appearance();

    let display_name = app.persona_manager.get_display_name();
    app.ui.update_user_display_name(display_name);

    app
}

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

    let mut app = build_app(
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config.clone(),
    );

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

    let mut app = build_app(
        session,
        ui,
        picker,
        character_service,
        persona_manager,
        preset_manager,
        config.clone(),
    );

    if startup_requires_provider {
        app.picker.startup_requires_provider = true;
    }

    Ok(app)
}

pub struct App {
    pub session: SessionContext,
    pub ui: UiState,
    pub picker: PickerController,
    pub character_service: CharacterService,
    pub persona_manager: crate::core::persona::PersonaManager,
    pub preset_manager: crate::core::preset::PresetManager,
    pub config: Config,
}
