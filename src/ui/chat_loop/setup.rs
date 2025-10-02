use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    auth::AuthManager,
    core::{
        app::App, builtin_providers::load_builtin_providers, config::Config,
        providers::ProviderResolutionError,
    },
};

/// Build the application state for the chat loop, including startup picker flows.
///
/// This logic was historically embedded inside `run_chat`; extracting it keeps the event loop
/// focused on UI concerns while centralising provider/model setup policy.
pub async fn bootstrap_app(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
) -> Result<Arc<Mutex<App>>, Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let auth_manager = AuthManager::new();

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

    let app = if open_provider_picker {
        let app = Arc::new(Mutex::new(
            App::new_uninitialized(log.clone()).await.expect("init app"),
        ));
        {
            let mut app_guard = app.lock().await;
            app_guard.picker.startup_requires_provider = true;
            app_guard.picker.startup_multiple_providers_available = multiple_providers_available;
            app_guard.open_provider_picker();
        }
        app
    } else {
        let app = Arc::new(Mutex::new(
            match App::new_with_auth(
                model.clone(),
                log.clone(),
                selected_provider,
                env_only,
                &config,
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
            },
        ));
        let mut need_model_picker = false;
        {
            let app_guard = app.lock().await;
            if app_guard.session.model.is_empty() {
                need_model_picker = true;
            }
        }
        if need_model_picker {
            let mut app_guard = app.lock().await;
            app_guard.picker.startup_requires_model = true;
            app_guard.picker.startup_multiple_providers_available = multiple_providers_available;
            let env_only = has_env_openai && token_providers.is_empty();
            app_guard.session.startup_env_only = env_only;
            if let Err(e) = app_guard.open_model_picker().await {
                app_guard.set_status(format!("Model picker error: {}", e));
            }
        }
        app
    };

    Ok(app)
}
