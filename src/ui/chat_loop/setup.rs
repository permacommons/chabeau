use std::sync::Arc;

use tokio::sync::Mutex;

use super::AppHandle;

use crate::{
    auth::AuthManager,
    character::CharacterService,
    core::{
        app,
        app::session::{exit_if_env_only_missing_env, exit_with_provider_resolution_error},
        builtin_providers::load_builtin_providers,
        config::data::Config,
        providers::{resolve_session, ProviderResolutionError, ResolveSessionError},
    },
};

/// Build the application state for the chat loop, including startup picker flows.
///
/// This logic was historically embedded inside `run_chat`; extracting it keeps the event loop
/// focused on UI concerns while centralising provider/model setup policy.
#[allow(clippy::too_many_arguments)]
pub async fn bootstrap_app(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
    character: Option<String>,
    persona: Option<String>,
    preset: Option<String>,
    disable_mcp: bool,
    character_service: CharacterService,
) -> Result<AppHandle, Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let auth_manager = AuthManager::new()?;

    // Lazily gather providers with stored tokens so we only touch the keyring when required
    let mut token_providers: Option<Vec<String>> = None;

    let has_env_openai = std::env::var("OPENAI_API_KEY").is_ok();

    // Provider selection rules mirror the prior implementation.
    let mut selected_provider: Option<String> = None;
    let mut open_provider_picker = false;
    let mut multiple_providers_available = has_env_openai;

    if !env_only {
        if let Some(p) = provider.clone() {
            if !p.is_empty() {
                selected_provider = Some(p);
            }
        }
    }

    if selected_provider.is_none() {
        exit_if_env_only_missing_env(env_only);

        if !env_only {
            if let Some(default_p) = &config.default_provider {
                selected_provider = Some(default_p.clone());
            }
        }
    }

    if selected_provider.is_none() {
        populate_token_providers(&auth_manager, env_only, &mut token_providers);
        let providers_with_tokens = token_providers
            .as_ref()
            .expect("token provider cache should be initialized");
        let total_available = providers_with_tokens.len() + if has_env_openai { 1 } else { 0 };
        multiple_providers_available = total_available > 1;

        if providers_with_tokens.len() == 1 {
            selected_provider = providers_with_tokens.first().cloned();
        } else if total_available > 1 {
            open_provider_picker = true;
        } else if has_env_openai {
            selected_provider = None;
        } else {
            eprintln!(
                "❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau provider add' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
            );
            std::process::exit(2);
        }
    } else if let Some(providers_with_tokens) = token_providers.as_ref() {
        let total_available = providers_with_tokens.len() + if has_env_openai { 1 } else { 0 };
        multiple_providers_available = total_available > 1;
    }

    if !env_only
        && selected_provider.is_none()
        && !has_env_openai
        && token_providers
            .as_ref()
            .map(|providers| providers.is_empty())
            .unwrap_or(true)
    {
        eprintln!(
            "❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau provider add' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
        );
        std::process::exit(2);
    }

    let mut character_service = Some(character_service);

    let app = if open_provider_picker {
        let service = character_service
            .take()
            .expect("character service should be available");
        let mut app = app::new_uninitialized(log.clone(), disable_mcp, service)
            .await
            .expect("init app");
        app.picker.startup_requires_provider = true;
        app.picker.startup_multiple_providers_available = multiple_providers_available;
        app.open_provider_picker();
        app
    } else {
        let provider_override = selected_provider.clone();
        let pre_resolved_session = if env_only {
            None
        } else {
            match resolve_session(&auth_manager, &config, provider_override.as_deref()) {
                Ok(session) => Some(session),
                Err(ResolveSessionError::Provider(err)) => {
                    exit_with_provider_resolution_error(&err);
                }
                Err(ResolveSessionError::Source(err)) => {
                    eprintln!("❌ Error: {err}");
                    std::process::exit(1);
                }
            }
        };

        let service = character_service
            .take()
            .expect("character service should be available");

        let mut app = match app::new_with_auth(
            app::AppInitConfig {
                model: model.clone(),
                log_file: log.clone(),
                provider: provider_override,
                env_only,
                pre_resolved_session,
                character: character.clone(),
                persona,
                preset,
                disable_mcp,
            },
            &config,
            service,
        )
        .await
        {
            Ok(app) => app,
            Err(e) => {
                if let Some(resolution_error) = e.downcast_ref::<ProviderResolutionError>() {
                    exit_with_provider_resolution_error(resolution_error);
                } else {
                    eprintln!("❌ Error: {e}");
                    std::process::exit(1);
                }
            }
        };

        if app.session.model.is_empty() {
            app.picker.startup_requires_model = true;
            app.picker.startup_multiple_providers_available = multiple_providers_available;
            let env_only = has_env_openai
                && token_providers
                    .as_ref()
                    .map(|providers| providers.is_empty())
                    .unwrap_or(false);
            app.session.startup_env_only = env_only;
            if let Err(e) = app.open_model_picker().await {
                app.conversation()
                    .set_status(format!("Model picker error: {}", e));
            }
        }

        app
    };

    let app = Arc::new(Mutex::new(app));

    Ok(AppHandle::new(app))
}

fn populate_token_providers(
    auth_manager: &AuthManager,
    env_only: bool,
    token_providers: &mut Option<Vec<String>>,
) {
    if token_providers.is_some() {
        return;
    }

    let providers = if env_only {
        Vec::new()
    } else {
        let mut providers_with_tokens = Vec::new();
        for bp in load_builtin_providers() {
            if auth_manager.get_token(&bp.id).unwrap_or(None).is_some() {
                providers_with_tokens.push(bp.id);
            }
        }

        for (id, _display, _url, has_token) in auth_manager.list_custom_providers() {
            if has_token {
                providers_with_tokens.push(id);
            }
        }

        providers_with_tokens
    };

    *token_providers = Some(providers);
}
