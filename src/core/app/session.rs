//! Session context and metadata tracking.
//!
//! This module defines [`SessionContext`], which captures runtime state for
//! an active chat session including the selected provider, model, HTTP client,
//! theme, logging configuration, and streaming cancellation tokens.
//!
//! Session metadata allows downstream components to act without re-querying
//! configuration or authentication state during the conversation lifecycle.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use reqwest::Client;
use rust_mcp_schema::CreateMessageRequest;
use tokio_util::sync::CancellationToken;

use crate::api::{ChatMessage, ChatToolCall};
use crate::auth::AuthManager;
use crate::character::card::CharacterCard;
use crate::character::service::CharacterService;
use crate::core::config::data::Config;
#[cfg(test)]
use crate::core::config::data::{DEFAULT_REFINE_INSTRUCTIONS, DEFAULT_REFINE_PREFIX};
use crate::core::providers::{
    resolve_env_session, resolve_session, ProviderResolutionError, ProviderSession,
    ResolveSessionError,
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
    pub is_refining: bool,
    pub original_refining_content: Option<String>,
    pub last_refine_prompt: Option<String>,
    pub refine_instructions: String,
    pub refine_prefix: String,
    pub startup_env_only: bool,
    pub mcp_disabled: bool,
    pub active_character: Option<CharacterCard>,
    pub character_greeting_shown: bool,
    pub has_received_assistant_message: bool,
    pub pending_tool_calls: BTreeMap<u32, PendingToolCall>,
    pub mcp_init_in_progress: bool,
    pub mcp_init_complete: bool,
    pub pending_mcp_message: Option<String>,
    pub pending_tool_queue: VecDeque<ToolCallRequest>,
    pub active_tool_request: Option<ToolCallRequest>,
    pub pending_sampling_queue: VecDeque<McpSamplingRequest>,
    pub active_sampling_request: Option<McpSamplingRequest>,
    pub tool_call_records: Vec<ChatToolCall>,
    pub tool_results: Vec<ChatMessage>,
    pub tool_result_history: Vec<ToolResultRecord>,
    pub tool_payload_history: Vec<ToolPayloadHistoryEntry>,
    pub active_assistant_message_index: Option<usize>,
    pub last_stream_api_messages: Option<Vec<ChatMessage>>,
    pub last_stream_api_messages_base: Option<Vec<ChatMessage>>,
    pub mcp_tools_enabled: bool,
    pub mcp_tools_unsupported: bool,
}

#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultStatus {
    Success,
    Error,
    Denied,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFailureKind {
    ToolError,
    ToolCallFailure,
}

impl ToolFailureKind {
    pub fn label(self) -> &'static str {
        match self {
            ToolFailureKind::ToolError => "tool error",
            ToolFailureKind::ToolCallFailure => "tool call failure",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            ToolFailureKind::ToolError => "Tool error",
            ToolFailureKind::ToolCallFailure => "Tool call failure",
        }
    }
}

impl ToolResultStatus {
    pub fn label(self) -> &'static str {
        match self {
            ToolResultStatus::Success => "success",
            ToolResultStatus::Error => "failed",
            ToolResultStatus::Denied => "denied",
            ToolResultStatus::Blocked => "blocked",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            ToolResultStatus::Success => "Success",
            ToolResultStatus::Error => "Failed",
            ToolResultStatus::Denied => "Denied",
            ToolResultStatus::Blocked => "Blocked",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolResultRecord {
    pub tool_name: String,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub status: ToolResultStatus,
    pub failure_kind: Option<ToolFailureKind>,
    pub content: String,
    pub summary: String,
    pub tool_call_id: Option<String>,
    pub raw_arguments: Option<String>,
    pub assistant_message_index: Option<usize>,
}

#[derive(Clone)]
pub struct ToolPayloadHistoryEntry {
    pub server_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub assistant_message: ChatMessage,
    pub tool_message: ChatMessage,
    pub assistant_message_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub server_id: String,
    pub tool_name: String,
    pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
    pub raw_arguments: String,
    pub tool_call_id: Option<String>,
}

#[derive(Clone)]
pub struct McpSamplingRequest {
    pub server_id: String,
    pub request: CreateMessageRequest,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone)]
pub struct McpPromptRequest {
    pub server_id: String,
    pub prompt_name: String,
    pub arguments: std::collections::HashMap<String, String>,
}

pub struct SessionBootstrap {
    pub session: SessionContext,
    pub theme: Theme,
    pub startup_requires_provider: bool,
    pub startup_errors: Vec<String>,
}

pub struct UninitializedSessionBootstrap {
    pub session: SessionContext,
    pub theme: Theme,
    pub config: Config,
    pub startup_requires_provider: bool,
}

impl SessionContext {
    /// Set the active character card
    pub fn set_character(&mut self, card: CharacterCard) {
        // Check if this is the same character that's already active
        let is_same_character = self
            .active_character
            .as_ref()
            .map(|current| current.data.name == card.data.name)
            .unwrap_or(false);

        self.active_character = Some(card);

        // Only reset greeting flag if this is a different character
        if !is_same_character {
            self.character_greeting_shown = false;
        }
    }

    /// Clear the active character card
    pub fn clear_character(&mut self) {
        self.active_character = None;
        self.character_greeting_shown = false;
    }

    /// Get a reference to the active character card
    pub fn get_character(&self) -> Option<&CharacterCard> {
        self.active_character.as_ref()
    }

    /// Check if the character greeting should be shown
    pub fn should_show_greeting(&self) -> bool {
        if let Some(character) = &self.active_character {
            !self.character_greeting_shown && !character.data.first_mes.trim().is_empty()
        } else {
            false
        }
    }

    /// Mark the character greeting as shown
    pub fn mark_greeting_shown(&mut self) {
        self.character_greeting_shown = true;
    }

    pub fn prune_tool_records_for_assistant_index(&mut self, index: usize) {
        self.prune_tool_records(|candidate| candidate == index);
    }

    pub fn prune_tool_records_from_index(&mut self, start_index: usize) {
        self.prune_tool_records(|candidate| candidate >= start_index);
    }

    fn prune_tool_records<F>(&mut self, predicate: F)
    where
        F: Fn(usize) -> bool,
    {
        self.tool_result_history.retain(|record| {
            record
                .assistant_message_index
                .map(|idx| !predicate(idx))
                .unwrap_or(true)
        });
        self.tool_payload_history.retain(|entry| {
            entry
                .assistant_message_index
                .map(|idx| !predicate(idx))
                .unwrap_or(true)
        });
        if let Some(active) = self.active_assistant_message_index {
            if predicate(active) {
                self.active_assistant_message_index = None;
            }
        }
    }
}

/// Result of attempting to load a character during session initialization.
#[derive(Debug)]
pub(crate) struct CharacterLoadOutcome {
    pub character: Option<CharacterCard>,
    pub errors: Vec<String>,
}

pub fn exit_with_provider_resolution_error(err: &ProviderResolutionError) -> ! {
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

pub fn exit_if_env_only_missing_env(env_only: bool) {
    if env_only && std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("❌ --env used but OPENAI_API_KEY is not set");
        std::process::exit(2);
    }
}

/// Load character card for session initialization
/// Priority: CLI flag > default for provider/model > None
pub(crate) fn load_character_for_session(
    cli_character: Option<&str>,
    provider: &str,
    model: &str,
    config: &Config,
    character_service: &mut CharacterService,
) -> Result<CharacterLoadOutcome, Box<dyn std::error::Error>> {
    // If CLI character is specified, use it (highest priority)
    if let Some(character_name) = cli_character {
        let card = character_service
            .resolve(character_name)
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
        return Ok(CharacterLoadOutcome {
            character: Some(card),
            errors: Vec::new(),
        });
    }

    // Otherwise, check for default character for this provider/model
    let mut errors = Vec::new();
    match character_service.load_default_for_session(provider, model, config) {
        Ok(Some((_name, card))) => {
            return Ok(CharacterLoadOutcome {
                character: Some(card),
                errors,
            })
        }
        Ok(None) => {}
        Err(err) => {
            if let Some(default_character) = config.get_default_character(provider, model) {
                errors.push(format!(
                    "Failed to load default character '{}' for {}:{}: {}",
                    default_character, provider, model, err
                ));
            } else {
                errors.push(format!(
                    "Failed to load default character for {}:{}: {}",
                    provider, model, err
                ));
            }
        }
    }

    // No character specified or found
    Ok(CharacterLoadOutcome {
        character: None,
        errors,
    })
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_with_auth(
    model: String,
    log_file: Option<String>,
    provider: Option<String>,
    env_only: bool,
    config: &Config,
    pre_resolved_session: Option<ProviderSession>,
    character: Option<String>,
    character_service: &mut CharacterService,
) -> Result<SessionBootstrap, Box<dyn std::error::Error>> {
    let session = if let Some(session) = pre_resolved_session {
        session
    } else if env_only {
        resolve_env_session().map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?
    } else {
        let auth_manager = AuthManager::new()?;
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

    // Load character card if specified via CLI or config
    let CharacterLoadOutcome {
        character: active_character,
        errors: startup_errors,
    } = load_character_for_session(
        character.as_deref(),
        &provider_name,
        &final_model,
        config,
        character_service,
    )?;

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
        is_refining: false,
        original_refining_content: None,
        last_refine_prompt: None,
        refine_instructions: config.refine_instructions().into_owned(),
        refine_prefix: config.refine_prefix().into_owned(),
        startup_env_only: false,
        mcp_disabled: false,
        active_character,
        character_greeting_shown: false,
        has_received_assistant_message: false,
        pending_tool_calls: BTreeMap::new(),
        mcp_init_in_progress: false,
        mcp_init_complete: false,
        pending_mcp_message: None,
        pending_tool_queue: VecDeque::new(),
        active_tool_request: None,
        pending_sampling_queue: VecDeque::new(),
        active_sampling_request: None,
        tool_call_records: Vec::new(),
        tool_results: Vec::new(),
        tool_result_history: Vec::new(),
        tool_payload_history: Vec::new(),
        active_assistant_message_index: None,
        last_stream_api_messages: None,
        last_stream_api_messages_base: None,
        mcp_tools_enabled: false,
        mcp_tools_unsupported: false,
    };

    Ok(SessionBootstrap {
        session,
        theme: resolved_theme,
        startup_requires_provider: false,
        startup_errors,
    })
}

pub(crate) async fn prepare_uninitialized(
    log_file: Option<String>,
    _character_service: &mut CharacterService,
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
        is_refining: false,
        original_refining_content: None,
        last_refine_prompt: None,
        refine_instructions: config.refine_instructions().into_owned(),
        refine_prefix: config.refine_prefix().into_owned(),
        startup_env_only: false,
        mcp_disabled: false,
        active_character: None,
        character_greeting_shown: false,
        has_received_assistant_message: false,
        pending_tool_calls: BTreeMap::new(),
        mcp_init_in_progress: false,
        mcp_init_complete: false,
        pending_mcp_message: None,
        pending_tool_queue: VecDeque::new(),
        active_tool_request: None,
        pending_sampling_queue: VecDeque::new(),
        active_sampling_request: None,
        tool_call_records: Vec::new(),
        tool_results: Vec::new(),
        tool_result_history: Vec::new(),
        tool_payload_history: Vec::new(),
        active_assistant_message_index: None,
        last_stream_api_messages: None,
        last_stream_api_messages_base: None,
        mcp_tools_enabled: false,
        mcp_tools_unsupported: false,
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
    use crate::core::config::data::Config;
    use crate::core::providers::ProviderSession;
    use crate::utils::test_utils::TestEnvVarGuard;
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
        let mut service = crate::character::CharacterService::new();

        let bootstrap = runtime
            .block_on(super::prepare_with_auth(
                "default".to_string(),
                None,
                None,
                false,
                &config,
                Some(provider_session.clone()),
                None,
                &mut service,
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
        assert!(bootstrap.session.active_character.is_none());
        assert!(!bootstrap.session.character_greeting_shown);
    }

    #[test]
    fn prepare_with_auth_uses_env_session_when_env_only() {
        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("OPENAI_API_KEY", "sk-env");
        env_guard.set_var("OPENAI_BASE_URL", "https://example.com/v1");

        let config = Config::default();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let mut service = crate::character::CharacterService::new();

        let bootstrap = runtime
            .block_on(super::prepare_with_auth(
                "default".to_string(),
                None,
                None,
                true,
                &config,
                None,
                None,
                &mut service,
            ))
            .expect("prepare_with_auth");

        assert_eq!(bootstrap.session.api_key, "sk-env");
        assert_eq!(bootstrap.session.base_url, "https://example.com/v1");
        assert_eq!(bootstrap.session.provider_name, "openai-compatible");
        assert_eq!(bootstrap.session.provider_display_name, "OpenAI-compatible");
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
        // "## Logging started" is an app message added by the command handler, not by initialize_logging
        assert!(contents.contains("Hello from startup"));
    }

    #[test]
    fn session_context_set_character() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
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
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "Test character".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        session.set_character(card.clone());
        assert!(session.active_character.is_some());
        assert_eq!(session.get_character().unwrap().data.name, "Test");
        assert!(!session.character_greeting_shown);
    }

    #[test]
    fn session_context_clear_character() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
            retrying_message_index: None,
            is_refining: false,
            original_refining_content: None,
            last_refine_prompt: None,
            refine_instructions: DEFAULT_REFINE_INSTRUCTIONS.to_string(),
            refine_prefix: DEFAULT_REFINE_PREFIX.to_string(),
            startup_env_only: false,
            mcp_disabled: false,
            active_character: Some(CharacterCard {
                spec: "chara_card_v2".to_string(),
                spec_version: "2.0".to_string(),
                data: CharacterData {
                    name: "Test".to_string(),
                    description: "Test character".to_string(),
                    personality: "Friendly".to_string(),
                    scenario: "Testing".to_string(),
                    first_mes: "Hello!".to_string(),
                    mes_example: String::new(),
                    creator_notes: None,
                    system_prompt: None,
                    post_history_instructions: None,
                    alternate_greetings: None,
                    tags: None,
                    creator: None,
                    character_version: None,
                },
            }),
            character_greeting_shown: true,
            has_received_assistant_message: false,
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        session.clear_character();
        assert!(session.active_character.is_none());
        assert!(!session.character_greeting_shown);
    }

    #[test]
    fn session_context_should_show_greeting() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
            retrying_message_index: None,
            is_refining: false,
            original_refining_content: None,
            last_refine_prompt: None,
            refine_instructions: DEFAULT_REFINE_INSTRUCTIONS.to_string(),
            refine_prefix: DEFAULT_REFINE_PREFIX.to_string(),
            startup_env_only: false,
            mcp_disabled: false,
            active_character: Some(CharacterCard {
                spec: "chara_card_v2".to_string(),
                spec_version: "2.0".to_string(),
                data: CharacterData {
                    name: "Test".to_string(),
                    description: "Test character".to_string(),
                    personality: "Friendly".to_string(),
                    scenario: "Testing".to_string(),
                    first_mes: "Hello!".to_string(),
                    mes_example: String::new(),
                    creator_notes: None,
                    system_prompt: None,
                    post_history_instructions: None,
                    alternate_greetings: None,
                    tags: None,
                    creator: None,
                    character_version: None,
                },
            }),
            character_greeting_shown: false,
            has_received_assistant_message: false,
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        // Should show greeting when character is active and greeting not shown
        assert!(session.should_show_greeting());

        // Should not show greeting after marking as shown
        session.mark_greeting_shown();
        assert!(!session.should_show_greeting());
    }

    #[test]
    fn session_context_should_not_show_empty_greeting() {
        use crate::character::card::{CharacterCard, CharacterData};

        let session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
            retrying_message_index: None,
            is_refining: false,
            original_refining_content: None,
            last_refine_prompt: None,
            refine_instructions: DEFAULT_REFINE_INSTRUCTIONS.to_string(),
            refine_prefix: DEFAULT_REFINE_PREFIX.to_string(),
            startup_env_only: false,
            mcp_disabled: false,
            active_character: Some(CharacterCard {
                spec: "chara_card_v2".to_string(),
                spec_version: "2.0".to_string(),
                data: CharacterData {
                    name: "Test".to_string(),
                    description: "Test character".to_string(),
                    personality: "Friendly".to_string(),
                    scenario: "Testing".to_string(),
                    first_mes: "   ".to_string(), // Empty/whitespace greeting
                    mes_example: String::new(),
                    creator_notes: None,
                    system_prompt: None,
                    post_history_instructions: None,
                    alternate_greetings: None,
                    tags: None,
                    creator: None,
                    character_version: None,
                },
            }),
            character_greeting_shown: false,
            has_received_assistant_message: false,
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        // Should not show empty/whitespace greeting
        assert!(!session.should_show_greeting());
    }

    #[test]
    fn load_character_for_session_no_character() {
        let config = Config::default();
        let mut service = crate::character::CharacterService::new();
        let outcome =
            super::load_character_for_session(None, "openai", "gpt-4", &config, &mut service)
                .expect("load_character_for_session");

        assert!(outcome.character.is_none());
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn load_character_for_session_cli_takes_precedence() {
        use crate::character::card::{CharacterCard, CharacterData};
        use std::collections::HashMap;
        use std::fs;

        let temp_dir = tempdir().expect("tempdir");
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).expect("create cards dir");

        // Create a test card
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestChar".to_string(),
                description: "Test".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let card_path = cards_dir.join("testchar.json");
        let card_json = serde_json::to_string(&card).expect("serialize card");
        fs::write(&card_path, card_json).expect("write card");

        // Create config with a different default character
        let mut default_chars = HashMap::new();
        let mut openai_models = HashMap::new();
        openai_models.insert("gpt-4".to_string(), "other-char".to_string());
        default_chars.insert("openai".to_string(), openai_models);

        let config = Config {
            default_characters: default_chars,
            ..Default::default()
        };

        // CLI character should take precedence (but we can't test this without
        // setting up the full cards directory structure, so we'll just verify
        // the logic exists in the function)
        // This test verifies the function signature and basic behavior
        let mut service = crate::character::CharacterService::new();
        let result = super::load_character_for_session(
            Some(card_path.to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );
        let outcome = result.expect("cli load");
        assert!(outcome.errors.is_empty());
        assert_eq!(
            outcome.character.expect("character loaded").data.name,
            "TestChar"
        );
    }

    #[test]
    fn load_character_for_session_filepath_fallback() {
        use crate::character::card::{CharacterCard, CharacterData};
        use std::fs;

        let temp_dir = tempdir().expect("tempdir");

        // Create a character card file outside the cards directory
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "FilePathChar".to_string(),
                description: "Loaded from file path".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello from file!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let card_path = temp_dir.path().join("external_card.json");
        let card_json = serde_json::to_string(&card).expect("serialize card");
        fs::write(&card_path, card_json).expect("write card");

        let config = Config::default();
        let mut service = crate::character::CharacterService::new();

        // Load character by file path (should work as fallback)
        let result = super::load_character_for_session(
            Some(card_path.to_str().unwrap()),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(outcome.character.is_some());
        assert_eq!(outcome.character.unwrap().data.name, "FilePathChar");
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn load_character_for_session_cards_dir_priority() {
        use crate::character::card::{CharacterCard, CharacterData};
        use std::fs;

        let temp_dir = tempdir().expect("tempdir");

        // Create a character card file in current directory with name "data"
        let wrong_card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "WrongChar".to_string(),
                description: "Should not be loaded".to_string(),
                personality: "Wrong".to_string(),
                scenario: "Wrong".to_string(),
                first_mes: "Wrong!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let wrong_path = temp_dir.path().join("data.json");
        let wrong_json = serde_json::to_string(&wrong_card).expect("serialize card");
        fs::write(&wrong_path, wrong_json).expect("write card");

        let config = Config::default();

        // Try to load character named "data" - should fail because it's not in cards dir
        // and we're not providing the full path
        let mut service = crate::character::CharacterService::new();
        let result = super::load_character_for_session(
            Some("data"),
            "openai",
            "gpt-4",
            &config,
            &mut service,
        );

        // Should fail because "data" is not found in cards directory
        // and "data" as a relative path doesn't exist
        assert!(result.is_err());
    }

    #[test]
    fn session_context_get_character_returns_none_initially() {
        let session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
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
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        assert!(session.get_character().is_none());
        assert!(!session.should_show_greeting());
    }

    #[test]
    fn session_context_greeting_lifecycle() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
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
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        // Initially no greeting
        assert!(!session.should_show_greeting());

        // Set character with greeting
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "Test character".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello there!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        session.set_character(card);

        // Should show greeting now
        assert!(session.should_show_greeting());

        // Mark as shown
        session.mark_greeting_shown();

        // Should not show greeting anymore
        assert!(!session.should_show_greeting());

        // Clear character
        session.clear_character();

        // Should not show greeting after clearing
        assert!(!session.should_show_greeting());
    }

    #[test]
    fn session_context_reselecting_same_character_preserves_greeting_flag() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
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
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "Test character".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello there!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        // Set character and mark greeting as shown
        session.set_character(card.clone());
        assert!(session.should_show_greeting());
        session.mark_greeting_shown();
        assert!(!session.should_show_greeting());

        // Re-select the same character
        session.set_character(card);

        // Greeting flag should still be true (greeting already shown)
        assert!(!session.should_show_greeting());
        assert!(session.character_greeting_shown);
    }

    #[test]
    fn session_context_selecting_different_character_resets_greeting_flag() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut session = SessionContext {
            client: Client::new(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: String::new(),
            provider_display_name: String::new(),
            logging: LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
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
            pending_tool_calls: BTreeMap::new(),
            mcp_init_in_progress: false,
            mcp_init_complete: false,
            pending_mcp_message: None,
            pending_tool_queue: VecDeque::new(),
            active_tool_request: None,
            pending_sampling_queue: VecDeque::new(),
            active_sampling_request: None,
            tool_call_records: Vec::new(),
            tool_results: Vec::new(),
            tool_result_history: Vec::new(),
            tool_payload_history: Vec::new(),
            active_assistant_message_index: None,
            last_stream_api_messages: None,
            last_stream_api_messages_base: None,
            mcp_tools_enabled: false,
            mcp_tools_unsupported: false,
        };

        let card1 = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test1".to_string(),
                description: "Test character 1".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello from Test1!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let card2 = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test2".to_string(),
                description: "Test character 2".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello from Test2!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        // Set first character and mark greeting as shown
        session.set_character(card1);
        session.mark_greeting_shown();
        assert!(!session.should_show_greeting());

        // Select a different character
        session.set_character(card2);

        // Greeting flag should be reset (new character, should show greeting)
        assert!(session.should_show_greeting());
        assert!(!session.character_greeting_shown);
    }
}
