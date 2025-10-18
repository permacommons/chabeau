use std::sync::Arc;

use tokio::sync::Mutex;

use super::AppHandle;

use crate::{
    auth::AuthManager,
    character::CharacterService,
    core::{
        app,
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
    character_service: CharacterService,
) -> Result<AppHandle, Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let auth_manager = AuthManager::new()?;

    // Gather providers with stored tokens (ignored in env-only mode)
    let mut token_providers: Vec<String> = Vec::new();
    if !env_only {
        for bp in load_builtin_providers() {
            if auth_manager.get_token(&bp.id).unwrap_or(None).is_some() {
                token_providers.push(bp.id);
            }
        }
        for (id, _display, _url, has_token) in auth_manager.list_custom_providers() {
            if has_token {
                token_providers.push(id);
            }
        }
    }

    let has_env_openai = std::env::var("OPENAI_API_KEY").is_ok();

    // Provider selection rules mirror the prior implementation.
    let mut selected_provider: Option<String> = None;
    let mut open_provider_picker = false;
    let total_available = token_providers.len() + if has_env_openai { 1 } else { 0 };
    let multiple_providers_available = total_available > 1;

    if !env_only {
        if let Some(p) = provider.clone() {
            if !p.is_empty() {
                selected_provider = Some(p);
            }
        }
    }

    if selected_provider.is_none() {
        if env_only && !has_env_openai {
            eprintln!("❌ --env used but OPENAI_API_KEY is not set");
            std::process::exit(2);
        }
        if let Some(default_p) = &config.default_provider {
            selected_provider = Some(default_p.clone());
        } else if token_providers.len() == 1 {
            selected_provider = token_providers.first().cloned();
        } else if total_available > 1 {
            open_provider_picker = true;
        } else if has_env_openai {
            selected_provider = None;
        } else {
            eprintln!("❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau auth' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional");
            std::process::exit(2);
        }
    }

    let mut character_service = Some(character_service);

    let app = if open_provider_picker {
        let service = character_service
            .take()
            .expect("character service should be available");
        let mut app = app::new_uninitialized(log.clone(), service)
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
                    eprintln!("{}", err);
                    let fixes = err.quick_fixes();
                    if !fixes.is_empty() {
                        eprintln!();
                        eprintln!("💡 Quick fixes:");
                        for fix in fixes {
                            eprintln!("  • {fix}");
                        }
                    }
                    std::process::exit(err.exit_code());
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
            },
            &config,
            service,
        )
        .await
        {
            Ok(app) => app,
            Err(e) => {
                if let Some(resolution_error) = e.downcast_ref::<ProviderResolutionError>() {
                    eprintln!("{}", resolution_error);
                    let fixes = resolution_error.quick_fixes();
                    if !fixes.is_empty() {
                        eprintln!();
                        eprintln!("💡 Quick fixes:");
                        for fix in fixes {
                            eprintln!("  • {fix}");
                        }
                    }
                    std::process::exit(resolution_error.exit_code());
                } else {
                    eprintln!("❌ Error: {e}");
                    std::process::exit(1);
                }
            }
        };

        if app.session.model.is_empty() {
            app.picker.startup_requires_model = true;
            app.picker.startup_multiple_providers_available = multiple_providers_available;
            let env_only = has_env_openai && token_providers.is_empty();
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
