use crate::core::config::Config;
use crate::core::keyring::KeyringAccessError;
use std::error::Error;
use std::fmt;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const QUICK_FIXES: &[&str] = &[
    "chabeau auth                    # Interactive setup",
    "chabeau -p                      # Check provider status",
    "export OPENAI_API_KEY=sk-...    # Use environment variable (defaults to OpenAI API)",
];

#[derive(Clone, Debug)]
pub struct ProviderMetadata {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
}

#[derive(Clone, Debug)]
pub struct ProviderSession {
    pub api_key: String,
    pub base_url: String,
    pub provider_id: String,
    pub provider_display_name: String,
}

impl ProviderSession {
    pub fn into_tuple(self) -> (String, String, String, String) {
        (
            self.api_key,
            self.base_url,
            self.provider_id,
            self.provider_display_name,
        )
    }
}

#[derive(Debug)]
pub struct ProviderResolutionError {
    message: String,
    quick_fixes: &'static [&'static str],
    exit_code: i32,
}

impl ProviderResolutionError {
    pub fn missing_authentication() -> Self {
        Self::new(
            "❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau auth' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional",
            QUICK_FIXES,
            2,
        )
    }

    pub fn provider_not_configured(provider: &str) -> Self {
        Self::new(
            format!(
                "No authentication found for provider '{provider}'. Run 'chabeau auth' to set up authentication."
            ),
            QUICK_FIXES,
            2,
        )
    }

    pub fn default_provider_missing(provider: &str) -> Self {
        Self::new(
            format!(
                "No authentication found for default provider '{provider}'. Run 'chabeau auth' to set up authentication."
            ),
            QUICK_FIXES,
            2,
        )
    }

    fn new(
        message: impl Into<String>,
        quick_fixes: &'static [&'static str],
        exit_code: i32,
    ) -> Self {
        Self {
            message: message.into(),
            quick_fixes,
            exit_code,
        }
    }

    pub fn quick_fixes(&self) -> &'static [&'static str] {
        self.quick_fixes
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for ProviderResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for ProviderResolutionError {}

pub enum ResolveSessionError {
    Provider(ProviderResolutionError),
    Source(Box<dyn Error>),
}

impl fmt::Debug for ResolveSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveSessionError::Provider(err) => f
                .debug_struct("ResolveSessionError::Provider")
                .field("error", err)
                .finish(),
            ResolveSessionError::Source(err) => f
                .debug_struct("ResolveSessionError::Source")
                .field("error", err)
                .finish(),
        }
    }
}

pub trait ProviderAuthSource {
    fn uses_keyring(&self) -> bool;
    fn find_provider_metadata(&self, provider: &str) -> Option<ProviderMetadata>;
    fn get_auth_for_provider(
        &self,
        provider: &str,
    ) -> Result<Option<(String, String)>, Box<dyn Error>>;
    fn find_first_available_auth(&self) -> Option<(ProviderMetadata, String)>;
}

pub fn resolve_env_session() -> Result<ProviderSession, ProviderResolutionError> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| ProviderResolutionError::missing_authentication())?;

    let base_url =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_OPENAI_BASE_URL.to_string());

    let (provider_id, provider_display_name) = if base_url == DEFAULT_OPENAI_BASE_URL {
        ("openai".to_string(), "OpenAI".to_string())
    } else {
        (
            "openai-compatible".to_string(),
            "OpenAI-compatible".to_string(),
        )
    };

    Ok(ProviderSession {
        api_key,
        base_url,
        provider_id,
        provider_display_name,
    })
}

pub fn resolve_session<S: ProviderAuthSource>(
    source: &S,
    config: &Config,
    provider_override: Option<&str>,
) -> Result<ProviderSession, ResolveSessionError> {
    let provider_override = provider_override.filter(|value| !value.is_empty());

    if let Some(provider_name) = provider_override {
        return resolve_specific_provider(source, provider_name);
    }

    if let Some(default_provider) = config.default_provider.as_deref() {
        match source.get_auth_for_provider(default_provider) {
            Ok(Some((base_url, api_key))) => {
                let metadata = source
                    .find_provider_metadata(default_provider)
                    .unwrap_or_else(|| ProviderMetadata {
                        id: default_provider.to_string(),
                        display_name: default_provider.to_string(),
                        base_url: base_url.clone(),
                    });

                return Ok(build_session(metadata, api_key, base_url));
            }
            Ok(None) => {
                return Err(ResolveSessionError::Provider(
                    ProviderResolutionError::default_provider_missing(default_provider),
                ));
            }
            Err(err) => {
                return handle_keyring_failure(err, Some(default_provider));
            }
        }
    }

    if !source.uses_keyring() {
        return resolve_env_session().map_err(ResolveSessionError::Provider);
    }

    if let Some((metadata, api_key)) = source.find_first_available_auth() {
        return Ok(build_session(metadata, api_key, String::new()));
    }

    resolve_env_session().map_err(ResolveSessionError::Provider)
}

fn resolve_specific_provider<S: ProviderAuthSource>(
    source: &S,
    provider_name: &str,
) -> Result<ProviderSession, ResolveSessionError> {
    match source.get_auth_for_provider(provider_name) {
        Ok(Some((base_url, api_key))) => {
            let metadata = source
                .find_provider_metadata(provider_name)
                .unwrap_or_else(|| ProviderMetadata {
                    id: provider_name.to_string(),
                    display_name: provider_name.to_string(),
                    base_url: base_url.clone(),
                });

            Ok(build_session(metadata, api_key, base_url))
        }
        Ok(None) => Err(ResolveSessionError::Provider(
            ProviderResolutionError::provider_not_configured(provider_name),
        )),
        Err(err) => handle_keyring_failure(err, Some(provider_name)),
    }
}

fn handle_keyring_failure(
    err: Box<dyn Error>,
    provider_name: Option<&str>,
) -> Result<ProviderSession, ResolveSessionError> {
    match err.downcast::<KeyringAccessError>() {
        Ok(keyring_err) => {
            if keyring_err.is_recoverable() {
                let context = provider_name
                    .map(|name| format!(" for provider '{name}'"))
                    .unwrap_or_default();
                eprintln!(
                    "⚠️  Unable to access stored credentials{context}: {}. Falling back to environment variables if available.",
                    keyring_err
                );
                resolve_env_session().map_err(ResolveSessionError::Provider)
            } else {
                Err(ResolveSessionError::Source(keyring_err))
            }
        }
        Err(original_err) => Err(ResolveSessionError::Source(original_err)),
    }
}

fn build_session(
    metadata: ProviderMetadata,
    api_key: String,
    base_url_from_auth: String,
) -> ProviderSession {
    let base_url = if base_url_from_auth.is_empty() {
        metadata.base_url.clone()
    } else {
        base_url_from_auth
    };

    ProviderSession {
        api_key,
        base_url,
        provider_id: metadata.id.to_lowercase(),
        provider_display_name: metadata.display_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{with_test_config_env, TestEnvVarGuard};
    use std::error::Error as StdError;
    use std::io;

    struct MockSource {}

    impl ProviderAuthSource for MockSource {
        fn uses_keyring(&self) -> bool {
            true
        }

        fn find_provider_metadata(&self, provider: &str) -> Option<ProviderMetadata> {
            Some(ProviderMetadata {
                id: provider.to_string(),
                display_name: provider.to_string(),
                base_url: "https://keyring.example".to_string(),
            })
        }

        fn get_auth_for_provider(
            &self,
            _provider: &str,
        ) -> Result<Option<(String, String)>, Box<dyn StdError>> {
            let backend_error = io::Error::other("mock backend unavailable");
            let keyring_error = keyring::Error::NoStorageAccess(Box::new(backend_error));
            Err(Box::new(KeyringAccessError::from(keyring_error)))
        }

        fn find_first_available_auth(&self) -> Option<(ProviderMetadata, String)> {
            None
        }
    }

    struct PermanentFailureSource;

    impl ProviderAuthSource for PermanentFailureSource {
        fn uses_keyring(&self) -> bool {
            true
        }

        fn find_provider_metadata(&self, provider: &str) -> Option<ProviderMetadata> {
            Some(ProviderMetadata {
                id: provider.to_string(),
                display_name: provider.to_string(),
                base_url: "https://keyring.example".to_string(),
            })
        }

        fn get_auth_for_provider(
            &self,
            _provider: &str,
        ) -> Result<Option<(String, String)>, Box<dyn StdError>> {
            let keyring_error = keyring::Error::BadEncoding(Vec::new());
            Err(Box::new(KeyringAccessError::from(keyring_error)))
        }

        fn find_first_available_auth(&self) -> Option<(ProviderMetadata, String)> {
            None
        }
    }

    #[test]
    fn recoverable_keyring_failure_uses_env_credentials() {
        with_test_config_env(|_| {
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("OPENAI_API_KEY", "sk-env");
            env_guard.set_var("OPENAI_BASE_URL", "https://example.com/v1");

            let config = Config {
                default_provider: Some("openai".to_string()),
                ..Default::default()
            };

            let session = resolve_session(&MockSource {}, &config, None)
                .expect("recoverable error should fall back to env");

            assert_eq!(session.api_key, "sk-env");
            assert_eq!(session.base_url, "https://example.com/v1");
            assert_eq!(session.provider_id, "openai-compatible");
            assert_eq!(session.provider_display_name, "OpenAI-compatible");
        });
    }

    #[test]
    fn provider_override_falls_back_to_env_on_keyring_failure() {
        with_test_config_env(|_| {
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("OPENAI_API_KEY", "sk-env");
            env_guard.set_var("OPENAI_BASE_URL", DEFAULT_OPENAI_BASE_URL);

            let config = Config::default();

            let session = resolve_session(&MockSource {}, &config, Some("openai"))
                .expect("provider override should use env when keyring fails");

            assert_eq!(session.api_key, "sk-env");
            assert_eq!(session.base_url, DEFAULT_OPENAI_BASE_URL);
            assert_eq!(session.provider_id, "openai");
            assert_eq!(session.provider_display_name, "OpenAI");
        });
    }

    #[test]
    fn permanent_keyring_failure_is_propagated() {
        with_test_config_env(|_| {
            let config = Config {
                default_provider: Some("openai".to_string()),
                ..Config::default()
            };

            let err = resolve_session(&PermanentFailureSource, &config, None)
                .expect_err("permanent failures should bubble up");

            match err {
                ResolveSessionError::Source(source_err) => {
                    let keyring_err = source_err
                        .downcast::<KeyringAccessError>()
                        .expect("error should be a KeyringAccessError");
                    assert!(!keyring_err.is_recoverable());
                }
                _ => panic!("unexpected error variant"),
            }
        });
    }
}
