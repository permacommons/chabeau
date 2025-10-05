use std::time::Instant;

use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::core::providers::{
    resolve_env_session, resolve_session, ProviderSession, ResolveSessionError,
};
use crate::ui::appearance::{detect_preferred_appearance, Appearance};
use crate::ui::builtin_themes::{find_builtin_theme, theme_spec_from_custom};
use crate::ui::theme::Theme;
use crate::utils::color::quantize_theme_for_current_terminal;
use crate::utils::logging::LoggingState;
use crate::utils::url::construct_api_url;

pub struct SessionContext {
    pub client: Client,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub provider_name: String,
    pub provider_display_name: String,
    pub logging: LoggingState,
    pub stream_cancel_token: Option<CancellationToken>,
    pub current_stream_id: u64,
    pub last_retry_time: Instant,
    pub retrying_message_index: Option<usize>,
    pub startup_env_only: bool,
}

pub struct SessionBootstrap {
    pub session: SessionContext,
    pub theme: Theme,
    pub startup_requires_provider: bool,
}

pub struct UninitializedSessionBootstrap {
    pub session: SessionContext,
    pub theme: Theme,
    pub config: Config,
    pub startup_requires_provider: bool,
}

pub(crate) fn initialize_logging(
    log_file: Option<String>,
) -> Result<LoggingState, Box<dyn std::error::Error>> {
    let mut logging = LoggingState::new(log_file.clone())?;
    if let Some(log_path) = log_file {
        if let Err(e) = logging.set_log_file(log_path.clone()) {
            eprintln!(
                "Warning: Failed to enable startup logging ({}): {}",
                log_path, e
            );
        }
    }
    Ok(logging)
}

fn theme_from_appearance(appearance: Appearance) -> Theme {
    match appearance {
        Appearance::Light => Theme::light(),
        Appearance::Dark => Theme::dark_default(),
    }
}

pub(crate) fn resolve_theme(config: &Config) -> Theme {
    let resolved_theme = match &config.theme {
        Some(name) => {
            if let Some(ct) = config.get_custom_theme(name) {
                Theme::from_spec(&theme_spec_from_custom(ct))
            } else if let Some(spec) = find_builtin_theme(name) {
                Theme::from_spec(&spec)
            } else {
                Theme::from_name(name)
            }
        }
        None => detect_preferred_appearance()
            .map(theme_from_appearance)
            .unwrap_or_else(Theme::dark_default),
    };

    quantize_theme_for_current_terminal(resolved_theme)
}

pub(crate) async fn prepare_with_auth(
    model: String,
    log_file: Option<String>,
    provider: Option<String>,
    env_only: bool,
    config: &Config,
    pre_resolved_session: Option<ProviderSession>,
) -> Result<SessionBootstrap, Box<dyn std::error::Error>> {
    let session = if let Some(session) = pre_resolved_session {
        session
    } else if env_only {
        resolve_env_session().map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?
    } else {
        let auth_manager = AuthManager::new();
        match resolve_session(&auth_manager, config, provider.as_deref()) {
            Ok(session) => session,
            Err(ResolveSessionError::Provider(err)) => return Err(Box::new(err)),
            Err(ResolveSessionError::Source(err)) => return Err(err),
        }
    };

    let (api_key, base_url, provider_name, provider_display_name) = session.into_tuple();

    let final_model = if model != "default" {
        model
    } else if let Some(default_model) = config.get_default_model(&provider_name) {
        default_model.clone()
    } else {
        String::new()
    };

    let _api_endpoint = construct_api_url(&base_url, "chat/completions");

    let logging = initialize_logging(log_file)?;
    let resolved_theme = resolve_theme(config);

    let session = SessionContext {
        client: Client::new(),
        model: final_model,
        api_key,
        base_url,
        provider_name: provider_name.to_string(),
        provider_display_name,
        logging,
        stream_cancel_token: None,
        current_stream_id: 0,
        last_retry_time: Instant::now(),
        retrying_message_index: None,
        startup_env_only: false,
    };

    Ok(SessionBootstrap {
        session,
        theme: resolved_theme,
        startup_requires_provider: false,
    })
}

pub(crate) async fn prepare_uninitialized(
    log_file: Option<String>,
) -> Result<UninitializedSessionBootstrap, Box<dyn std::error::Error>> {
    let config = Config::load()?;

    let logging = initialize_logging(log_file)?;
    let resolved_theme = resolve_theme(&config);

    let session = SessionContext {
        client: Client::new(),
        model: String::new(),
        api_key: String::new(),
        base_url: String::new(),
        provider_name: String::new(),
        provider_display_name: "(no provider selected)".to_string(),
        logging,
        stream_cancel_token: None,
        current_stream_id: 0,
        last_retry_time: Instant::now(),
        retrying_message_index: None,
        startup_env_only: false,
    };

    Ok(UninitializedSessionBootstrap {
        session,
        theme: resolved_theme,
        config,
        startup_requires_provider: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Config;
    use crate::core::providers::ProviderSession;
    use tempfile::tempdir;

    #[test]
    fn theme_from_appearance_matches_light_theme() {
        let theme = theme_from_appearance(Appearance::Light);
        assert_eq!(theme.background_color, Theme::light().background_color);
    }

    #[test]
    fn theme_from_appearance_matches_dark_theme() {
        let theme = theme_from_appearance(Appearance::Dark);
        assert_eq!(
            theme.background_color,
            Theme::dark_default().background_color
        );
    }

    #[test]
    fn resolve_theme_prefers_configured_theme() {
        let config = Config {
            theme: Some("light".to_string()),
            ..Default::default()
        };

        let resolved_theme = resolve_theme(&config);
        let expected_theme = quantize_theme_for_current_terminal(Theme::light());
        assert_eq!(
            resolved_theme.background_color,
            expected_theme.background_color
        );
    }

    #[test]
    fn prepare_with_auth_uses_pre_resolved_session() {
        let provider_session = ProviderSession {
            api_key: "test-key".to_string(),
            base_url: "https://example.invalid".to_string(),
            provider_id: "test-provider".to_string(),
            provider_display_name: "Test Provider".to_string(),
        };

        let config = Config::default();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");

        let bootstrap = runtime
            .block_on(super::prepare_with_auth(
                "default".to_string(),
                None,
                None,
                false,
                &config,
                Some(provider_session.clone()),
            ))
            .expect("prepare_with_auth");

        assert_eq!(bootstrap.session.api_key, provider_session.api_key);
        assert_eq!(bootstrap.session.base_url, provider_session.base_url);
        assert_eq!(
            bootstrap.session.provider_name,
            provider_session.provider_id
        );
        assert_eq!(
            bootstrap.session.provider_display_name,
            provider_session.provider_display_name
        );
        assert!(!bootstrap.startup_requires_provider);
        assert!(!bootstrap.session.startup_env_only);
    }

    #[test]
    fn initialize_logging_with_file_writes_initial_entry() {
        let temp_dir = tempdir().expect("tempdir");
        let log_path = temp_dir.path().join("startup.log");
        let log_file = log_path.to_string_lossy().to_string();

        let logging = initialize_logging(Some(log_file.clone())).expect("logging initialized");
        logging
            .log_message("Hello from startup")
            .expect("log message");

        let contents = std::fs::read_to_string(&log_path).expect("read log file");
        assert!(contents.contains("## Logging started"));
        assert!(contents.contains("Hello from startup"));
    }
}
