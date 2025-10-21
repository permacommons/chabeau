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
    let has_env_openai = std::env::var("OPENAI_API_KEY").is_ok();

    let selection = match resolve_provider_choice(
        provider.as_deref(),
        env_only,
        has_env_openai,
        &config,
        &auth_manager,
    ) {
        Ok(selection) => selection,
        Err(err) => {
            eprintln!("{}", err.message);
            std::process::exit(err.exit_code);
        }
    };

    let ProviderSelection {
        selected_provider,
        open_provider_picker,
        token_providers,
        multiple_providers_available,
    } = selection;

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
            Some(&auth_manager),
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

#[derive(Debug)]
struct ProviderSelectionError {
    message: String,
    exit_code: i32,
}

struct ProviderSelection {
    selected_provider: Option<String>,
    open_provider_picker: bool,
    token_providers: Option<Vec<String>>,
    multiple_providers_available: bool,
}

fn resolve_provider_choice<T: TokenInventory>(
    provider_override: Option<&str>,
    env_only: bool,
    has_env_openai: bool,
    config: &Config,
    auth_manager: &T,
) -> Result<ProviderSelection, ProviderSelectionError> {
    let mut selected_provider = if !env_only {
        provider_override
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(ToOwned::to_owned)
    } else {
        None
    };

    if selected_provider.is_none() && env_only && !has_env_openai {
        return Err(ProviderSelectionError {
            message: "❌ --env used but OPENAI_API_KEY is not set".into(),
            exit_code: 2,
        });
    }

    if selected_provider.is_none() {
        if let Some(default) = &config.default_provider {
            if !default.is_empty() {
                selected_provider = Some(default.clone());
            }
        }
    }

    let mut open_provider_picker = false;
    let mut token_providers = None;
    let mut multiple_providers_available = false;

    if selected_provider.is_none() && !env_only {
        let providers = auth_manager.gather_token_providers();
        let total_available = providers.len() + if has_env_openai { 1 } else { 0 };
        multiple_providers_available = total_available > 1;

        if providers.len() == 1 {
            selected_provider = providers.first().cloned();
        } else if total_available > 1 {
            open_provider_picker = true;
        } else if !has_env_openai {
            return Err(ProviderSelectionError {
                message: "❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau auth' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\nexport OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional".into(),
                exit_code: 2,
            });
        }

        token_providers = Some(providers);
    }

    Ok(ProviderSelection {
        selected_provider,
        open_provider_picker,
        token_providers,
        multiple_providers_available,
    })
}

trait TokenInventory {
    fn gather_token_providers(&self) -> Vec<String>;
}

impl TokenInventory for AuthManager {
    fn gather_token_providers(&self) -> Vec<String> {
        let mut token_providers = Vec::new();
        for bp in load_builtin_providers() {
            if self.get_token(&bp.id).unwrap_or(None).is_some() {
                token_providers.push(bp.id);
            }
        }
        for (id, _display, _url, has_token) in self.list_custom_providers() {
            if has_token {
                token_providers.push(id);
            }
        }
        token_providers
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_provider_choice, TokenInventory};
    use crate::core::config::data::Config;
    use std::cell::Cell;

    #[derive(Default)]
    struct MockAuth {
        providers: Vec<String>,
        calls: Cell<usize>,
    }

    impl TokenInventory for MockAuth {
        fn gather_token_providers(&self) -> Vec<String> {
            self.calls.set(self.calls.get() + 1);
            self.providers.clone()
        }
    }

    #[test]
    fn provider_override_or_default_skips_token_enumeration() {
        let auth = MockAuth {
            providers: vec!["openai".into()],
            ..Default::default()
        };

        let mut config = Config::default();
        config.default_provider = Some("default-provider".into());

        let override_result =
            resolve_provider_choice(Some("cli-provider"), false, true, &config, &auth)
                .expect("override should succeed");
        assert_eq!(
            override_result.selected_provider.as_deref(),
            Some("cli-provider")
        );
        assert_eq!(
            auth.calls.get(),
            0,
            "token enumeration should not run for overrides"
        );

        let default_result = resolve_provider_choice(None, false, true, &config, &auth)
            .expect("default should succeed");
        assert_eq!(
            default_result.selected_provider.as_deref(),
            Some("default-provider")
        );
        assert_eq!(
            auth.calls.get(),
            0,
            "token enumeration should not run for defaults"
        );
    }
}
