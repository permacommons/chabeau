use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::core::message::Message;
use crate::ui::appearance::{detect_preferred_appearance, Appearance};
use crate::ui::builtin_themes::{find_builtin_theme, theme_spec_from_custom};
use crate::ui::picker::PickerState;
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use crate::utils::logging::LoggingState;
use crate::utils::scroll::ScrollCalculator;
use crate::utils::url::construct_api_url;
use chrono::Utc;
use ratatui::text::Line;
use reqwest::Client;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tui_textarea::TextArea;

pub mod picker;
pub mod ui_state;

#[cfg_attr(not(test), allow(unused_imports))]
pub use picker::{
    ModelPickerState, PickerController, PickerData, PickerSession, ProviderPickerState,
    ThemePickerState,
};
pub use ui_state::{FilePrompt, FilePromptKind, UiMode, UiState};

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

pub struct App {
    pub session: SessionContext,
    pub ui: UiState,
    pub picker: PickerController,
}

fn initialize_logging(
    log_file: Option<String>,
) -> Result<LoggingState, Box<dyn std::error::Error>> {
    let logging = LoggingState::new(log_file.clone())?;
    if let Some(_log_path) = log_file {
        let timestamp = Utc::now().to_rfc3339();
        if let Err(e) = logging.log_message(&format!("## Logging started at {}", timestamp)) {
            eprintln!("Warning: Failed to write initial log timestamp: {}", e);
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

fn resolve_theme(config: &Config) -> Theme {
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

    crate::utils::color::quantize_theme_for_current_terminal(resolved_theme)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Theme,
    Model,
    Provider,
}

impl App {
    pub fn is_input_active(&self) -> bool {
        matches!(
            self.ui.mode,
            UiMode::Typing | UiMode::InPlaceEdit { .. } | UiMode::FilePrompt(_)
        )
    }

    pub fn in_edit_select_mode(&self) -> bool {
        matches!(self.ui.mode, UiMode::EditSelect { .. })
    }

    pub fn selected_user_message_index(&self) -> Option<usize> {
        if let UiMode::EditSelect { selected_index } = self.ui.mode {
            Some(selected_index)
        } else {
            None
        }
    }

    pub fn set_selected_user_message_index(&mut self, index: usize) {
        if let UiMode::EditSelect { selected_index } = &mut self.ui.mode {
            *selected_index = index;
        }
    }

    pub fn in_block_select_mode(&self) -> bool {
        matches!(self.ui.mode, UiMode::BlockSelect { .. })
    }

    pub fn selected_block_index(&self) -> Option<usize> {
        if let UiMode::BlockSelect { block_index } = self.ui.mode {
            Some(block_index)
        } else {
            None
        }
    }

    pub fn set_selected_block_index(&mut self, index: usize) {
        if let UiMode::BlockSelect { block_index } = &mut self.ui.mode {
            *block_index = index;
        }
    }

    pub fn in_place_edit_index(&self) -> Option<usize> {
        if let UiMode::InPlaceEdit { index } = self.ui.mode {
            Some(index)
        } else {
            None
        }
    }

    fn set_mode(&mut self, mode: UiMode) {
        self.ui.mode = mode;
    }

    pub fn file_prompt(&self) -> Option<&FilePrompt> {
        if let UiMode::FilePrompt(ref prompt) = self.ui.mode {
            Some(prompt)
        } else {
            None
        }
    }

    pub fn take_in_place_edit_index(&mut self) -> Option<usize> {
        if let UiMode::InPlaceEdit { index } = self.ui.mode {
            self.set_mode(UiMode::Typing);
            Some(index)
        } else {
            None
        }
    }

    pub fn toggle_compose_mode(&mut self) {
        self.ui.compose_mode = !self.ui.compose_mode;
    }

    pub fn picker_session(&self) -> Option<&PickerSession> {
        self.picker.session()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn picker_session_mut(&mut self) -> Option<&mut PickerSession> {
        self.picker.session_mut()
    }

    pub fn current_picker_mode(&self) -> Option<PickerMode> {
        self.picker.current_mode()
    }

    pub fn picker_state(&self) -> Option<&PickerState> {
        self.picker.state()
    }

    pub fn picker_state_mut(&mut self) -> Option<&mut PickerState> {
        self.picker.state_mut()
    }

    pub fn theme_picker_state(&self) -> Option<&ThemePickerState> {
        self.picker.theme_state()
    }

    pub fn theme_picker_state_mut(&mut self) -> Option<&mut ThemePickerState> {
        self.picker.theme_state_mut()
    }

    pub fn model_picker_state(&self) -> Option<&ModelPickerState> {
        self.picker.model_state()
    }

    pub fn model_picker_state_mut(&mut self) -> Option<&mut ModelPickerState> {
        self.picker.model_state_mut()
    }

    pub fn provider_picker_state(&self) -> Option<&ProviderPickerState> {
        self.picker.provider_state()
    }

    pub fn provider_picker_state_mut(&mut self) -> Option<&mut ProviderPickerState> {
        self.picker.provider_state_mut()
    }

    /// Close any active picker session.
    pub fn close_picker(&mut self) {
        self.picker.close();
    }

    /// Returns true if current picker should use alphabetical sorting (A–Z / Z–A)
    pub async fn new_with_auth(
        model: String,
        log_file: Option<String>,
        provider: Option<String>,
        env_only: bool,
        config: &Config,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use crate::core::providers::{resolve_env_session, resolve_session, ResolveSessionError};

        let auth_manager = AuthManager::new();

        // Resolve authentication: if env_only, force env vars; otherwise use shared resolution
        let session = if env_only {
            resolve_env_session().map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?
        } else {
            match resolve_session(&auth_manager, config, provider.as_deref()) {
                Ok(session) => session,
                Err(ResolveSessionError::Provider(err)) => return Err(Box::new(err)),
                Err(ResolveSessionError::Source(err)) => return Err(err),
            }
        };

        let (api_key, base_url, provider_name, provider_display_name) = session.into_tuple();

        // Determine the model to use:
        // 1. If a specific model was requested (not "default"), use that
        // 2. If a default model is set for this provider in config, use that
        // 3. Otherwise, leave it unset and let the UI open the model picker
        let final_model = if model != "default" {
            model
        } else if let Some(default_model) = config.get_default_model(&provider_name) {
            default_model.clone()
        } else {
            String::new()
        };

        // Build API endpoint for internal use (no noisy startup prints)
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

        let mut app = App {
            session,
            ui: UiState::from_config(resolved_theme, config),
            picker: PickerController::new(),
        };

        // Keep textarea state in sync with the string input initially
        app.set_input_text(String::new());
        app.configure_textarea_appearance();

        Ok(app)
    }

    /// Create an app without a selected provider or model. Used when multiple providers
    /// are available but none is configured as default; the UI will open the provider picker.
    pub async fn new_uninitialized(
        log_file: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::load()?;

        // Quiet startup; no noisy prints here

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

        let mut picker = PickerController::new();
        picker.startup_requires_provider = true;

        let mut app = App {
            session,
            ui: UiState::from_config(resolved_theme, &config),
            picker,
        };

        app.set_input_text(String::new());
        app.configure_textarea_appearance();
        Ok(app)
    }

    /// Return prewrapped chat lines for current state, caching across frames when safe.
    /// Cache is used only in normal mode (no selection/highlight) and invalidated when
    /// width, flags, theme signature, message count, or last message content hash changes.
    pub fn get_prewrapped_lines_cached(&mut self, width: u16) -> &Vec<Line<'static>> {
        self.ui.get_prewrapped_lines_cached(width)
    }

    pub fn get_prewrapped_span_metadata_cached(&mut self, width: u16) -> &Vec<Vec<SpanKind>> {
        self.ui.get_prewrapped_span_metadata_cached(width)
    }

    pub fn invalidate_prewrap_cache(&mut self) {
        self.ui.invalidate_prewrap_cache();
    }

    // Used by Criterion benches in `benches/`.
    #[cfg(any(test, feature = "bench"))]
    pub fn new_bench(theme: Theme, markdown_enabled: bool, syntax_enabled: bool) -> Self {
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
            startup_env_only: false,
        };

        let ui = UiState::new_basic(theme, markdown_enabled, syntax_enabled, None);

        App {
            session,
            ui,
            picker: PickerController::new(),
        }
    }

    pub fn calculate_wrapped_line_count(&mut self, terminal_width: u16) -> u16 {
        self.ui.calculate_wrapped_line_count(terminal_width)
    }

    pub fn calculate_max_scroll_offset(
        &mut self,
        available_height: u16,
        terminal_width: u16,
    ) -> u16 {
        self.ui
            .calculate_max_scroll_offset(available_height, terminal_width)
    }

    /// Scroll to the very top of the output area and disable auto-scroll.
    pub fn scroll_to_top(&mut self) {
        self.ui.auto_scroll = false;
        self.ui.scroll_offset = 0;
    }

    /// Scroll to the very bottom of the output area and enable auto-scroll.
    pub fn scroll_to_bottom_view(&mut self, available_height: u16, terminal_width: u16) {
        let max_scroll = self.calculate_max_scroll_offset(available_height, terminal_width);
        self.ui.scroll_offset = max_scroll;
        self.ui.auto_scroll = true;
    }

    /// Page up by one full output area (minus one line overlap). Disables auto-scroll.
    pub fn page_up(&mut self, available_height: u16) {
        self.ui.auto_scroll = false;
        let step = available_height.saturating_sub(1);
        self.ui.scroll_offset = self.ui.scroll_offset.saturating_sub(step);
    }

    /// Page down by one full output area (minus one line overlap). Disables auto-scroll.
    pub fn page_down(&mut self, available_height: u16, terminal_width: u16) {
        self.ui.auto_scroll = false;
        let step = available_height.saturating_sub(1);
        let max_scroll = self.calculate_max_scroll_offset(available_height, terminal_width);
        self.ui.scroll_offset = (self.ui.scroll_offset.saturating_add(step)).min(max_scroll);
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
        // Clear any ephemeral status when the user sends a message
        self.clear_status();

        let user_message = Message {
            role: "user".to_string(),
            content: content.clone(),
        };

        // Log the user message if logging is active
        if let Err(e) = self.session.logging.log_message(&format!("You: {content}")) {
            eprintln!("Failed to log message: {e}");
        }

        self.ui.messages.push_back(user_message);

        // Start assistant message
        let assistant_message = Message {
            role: "assistant".to_string(),
            content: String::new(),
        };
        self.ui.messages.push_back(assistant_message);
        self.ui.current_response.clear();

        // Clear retry state since we're starting a new conversation
        self.session.retrying_message_index = None;

        // Prepare messages for API (excluding the empty assistant message we just added and system messages)
        let mut api_messages = Vec::new();
        for msg in self.ui.messages.iter().take(self.ui.messages.len() - 1) {
            // Only include user and assistant messages in API calls, exclude system messages
            if msg.role == "user" || msg.role == "assistant" {
                api_messages.push(crate::api::ChatMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }
        api_messages
    }

    pub fn set_status<S: Into<String>>(&mut self, s: S) {
        self.ui.status = Some(s.into());
        self.ui.status_set_at = Some(Instant::now());
    }

    pub fn clear_status(&mut self) {
        self.ui.status = None;
        self.ui.status_set_at = None;
    }

    pub fn start_file_prompt_dump(&mut self, filename: String) {
        self.set_mode(UiMode::FilePrompt(FilePrompt {
            kind: FilePromptKind::Dump,
            content: None,
        }));
        self.set_input_text(filename);
    }

    pub fn start_file_prompt_save_block(&mut self, filename: String, content: String) {
        self.set_mode(UiMode::FilePrompt(FilePrompt {
            kind: FilePromptKind::SaveCodeBlock,
            content: Some(content),
        }));
        self.set_input_text(filename);
    }

    pub fn cancel_file_prompt(&mut self) {
        if let UiMode::FilePrompt(_) = self.ui.mode {
            self.set_mode(UiMode::Typing);
        }
        self.clear_input();
    }

    pub fn append_to_response(
        &mut self,
        content: &str,
        available_height: u16,
        terminal_width: u16,
    ) {
        self.ui.current_response.push_str(content);

        // Update the message being retried, or the last message if not retrying
        if let Some(retry_index) = self.session.retrying_message_index {
            if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                if msg.role == "assistant" {
                    msg.content.push_str(content);
                }
            }
        } else if let Some(last_msg) = self.ui.messages.back_mut() {
            if last_msg.role == "assistant" {
                last_msg.content.push_str(content);
            }
        }

        let total_wrapped_lines = {
            let lines = self.get_prewrapped_lines_cached(terminal_width);
            lines.len() as u16
        };

        // Auto-scroll to bottom when new content arrives, but only if auto_scroll is enabled
        if self.ui.auto_scroll {
            if total_wrapped_lines > available_height {
                self.ui.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.ui.scroll_offset = 0;
            }
        }
    }

    pub fn add_system_message(&mut self, content: String) {
        let system_message = Message {
            role: "system".to_string(),
            content,
        };
        self.ui.messages.push_back(system_message);
    }

    pub fn update_scroll_position(&mut self, available_height: u16, terminal_width: u16) {
        // Auto-scroll to bottom when new content is added, but only if auto_scroll is enabled
        if self.ui.auto_scroll {
            let total_wrapped_lines = self.calculate_wrapped_line_count(terminal_width);
            if total_wrapped_lines > available_height {
                self.ui.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.ui.scroll_offset = 0;
            }
        }
    }

    pub fn get_logging_status(&self) -> String {
        self.session.logging.get_status_string()
    }

    pub fn can_retry(&self) -> bool {
        // Can retry if there's at least one assistant message (even if currently streaming)
        self.ui
            .messages
            .iter()
            .any(|msg| msg.role == "assistant" && !msg.content.is_empty())
    }

    pub fn cancel_current_stream(&mut self) {
        if let Some(token) = &self.session.stream_cancel_token {
            token.cancel();
        }
        self.session.stream_cancel_token = None;
        self.ui.is_streaming = false;
        self.ui.stream_interrupted = true;
    }

    pub fn start_new_stream(&mut self) -> (CancellationToken, u64) {
        // Cancel any existing stream first
        self.cancel_current_stream();

        // Increment stream ID to distinguish this stream from previous ones
        self.session.current_stream_id += 1;

        // Create new cancellation token
        let token = CancellationToken::new();
        self.session.stream_cancel_token = Some(token.clone());
        self.ui.is_streaming = true;
        self.ui.stream_interrupted = false;
        self.ui.pulse_start = Instant::now();

        (token, self.session.current_stream_id)
    }

    pub fn calculate_scroll_to_message(
        &self,
        message_index: usize,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        ScrollCalculator::calculate_scroll_to_message_with_flags(
            &self.ui.messages,
            &self.ui.theme,
            self.ui.markdown_enabled,
            self.ui.syntax_enabled,
            message_index,
            terminal_width,
            available_height,
        )
    }

    pub fn finalize_response(&mut self) {
        // Log the complete assistant response if logging is active
        if !self.ui.current_response.is_empty() {
            if let Err(e) = self.session.logging.log_message(&self.ui.current_response) {
                eprintln!("Failed to log response: {e}");
            }
        }

        // Clear retry state since response is now complete
        self.session.retrying_message_index = None;
    }

    /// Scroll so that the given message index is visible.
    pub fn scroll_index_into_view(&mut self, index: usize, term_width: u16, term_height: u16) {
        let input_area_height = self.calculate_input_area_height(term_width);
        let available_height = self.calculate_available_height(term_height, input_area_height);
        self.ui.scroll_offset =
            self.calculate_scroll_to_message(index, term_width, available_height);
    }

    /// Find the last user-authored message index
    pub fn last_user_message_index(&self) -> Option<usize> {
        self.ui
            .messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Find previous user message index before `from_index` (exclusive)
    pub fn prev_user_message_index(&self, from_index: usize) -> Option<usize> {
        if from_index == 0 {
            return None;
        }
        self.ui
            .messages
            .iter()
            .enumerate()
            .take(from_index)
            .rev()
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Find next user message index after `from_index` (exclusive)
    pub fn next_user_message_index(&self, from_index: usize) -> Option<usize> {
        self.ui
            .messages
            .iter()
            .enumerate()
            .skip(from_index + 1)
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Find the first user-authored message index
    pub fn first_user_message_index(&self) -> Option<usize> {
        self.ui
            .messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Enter edit-select mode: lock input and select most recent user message
    pub fn enter_edit_select_mode(&mut self) {
        if let Some(idx) = self.last_user_message_index() {
            self.set_mode(UiMode::EditSelect {
                selected_index: idx,
            });
        }
    }

    /// Exit edit-select mode
    pub fn exit_edit_select_mode(&mut self) {
        if self.in_edit_select_mode() {
            self.set_mode(UiMode::Typing);
        }
    }

    /// Begin in-place edit of a user message at `index`
    pub fn start_in_place_edit(&mut self, index: usize) {
        self.set_mode(UiMode::InPlaceEdit { index });
    }

    /// Cancel in-place edit (does not modify history)
    pub fn cancel_in_place_edit(&mut self) {
        if matches!(self.ui.mode, UiMode::InPlaceEdit { .. }) {
            self.set_mode(UiMode::Typing);
        }
    }

    /// Enter block select mode: lock input and set selected block index
    pub fn enter_block_select_mode(&mut self, index: usize) {
        self.set_mode(UiMode::BlockSelect { block_index: index });
    }

    /// Exit block select mode and unlock input
    pub fn exit_block_select_mode(&mut self) {
        if self.in_block_select_mode() {
            self.set_mode(UiMode::Typing);
        }
    }

    pub fn prepare_retry(
        &mut self,
        available_height: u16,
        terminal_width: u16,
    ) -> Option<Vec<crate::api::ChatMessage>> {
        if !self.can_retry() {
            return None;
        }

        // Update retry time (debounce is now handled at event level)
        self.session.last_retry_time = Instant::now();

        // Check if we're already retrying a specific message
        if let Some(retry_index) = self.session.retrying_message_index {
            // We're already retrying a specific message - just clear its content
            if retry_index < self.ui.messages.len() {
                if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                    if msg.role == "assistant" {
                        msg.content.clear();
                        self.ui.current_response.clear();
                    }
                }
            }
        } else {
            // Not currently retrying - find the last assistant message to retry
            let mut target_index = None;

            // Find the last assistant message with content
            for (i, msg) in self.ui.messages.iter().enumerate().rev() {
                if msg.role == "assistant" && !msg.content.is_empty() {
                    target_index = Some(i);
                    break;
                }
            }

            if let Some(index) = target_index {
                // Mark this message as being retried
                self.session.retrying_message_index = Some(index);

                // Clear the content of this specific message
                if let Some(msg) = self.ui.messages.get_mut(index) {
                    msg.content.clear();
                    self.ui.current_response.clear();
                }

                // Rewrite the log file to remove the last assistant response
                if let Err(e) = self
                    .session
                    .logging
                    .rewrite_log_without_last_response(&self.ui.messages)
                {
                    eprintln!("Failed to rewrite log file: {e}");
                }
            } else {
                return None;
            }
        }

        // Set scroll position to show the user message that corresponds to the retry
        if let Some(retry_index) = self.session.retrying_message_index {
            // Find the user message that precedes this assistant message
            if retry_index > 0 {
                let user_message_index = retry_index - 1;
                self.ui.scroll_offset = self.calculate_scroll_to_message(
                    user_message_index,
                    terminal_width,
                    available_height,
                );
            } else {
                self.ui.scroll_offset = 0;
            }
        }

        // Re-enable auto-scroll for the new response
        self.ui.auto_scroll = true;

        // Prepare messages for API (excluding the message being retried and system messages)
        let mut api_messages = Vec::new();
        if let Some(retry_index) = self.session.retrying_message_index {
            for (i, msg) in self.ui.messages.iter().enumerate() {
                if i < retry_index {
                    // Only include user and assistant messages in API calls, exclude system messages
                    if msg.role == "user" || msg.role == "assistant" {
                        api_messages.push(crate::api::ChatMessage {
                            role: msg.role.clone(),
                            content: msg.content.clone(),
                        });
                    }
                }
            }
        }

        Some(api_messages)
    }

    // fetch_newest_model was removed in favor of explicit model selection via the TUI picker

    /// Calculate how many lines the input text will wrap to using word wrapping
    #[allow(dead_code)]
    pub fn calculate_input_wrapped_lines(&self, width: u16) -> usize {
        self.ui.calculate_input_wrapped_lines(width)
    }

    /// Calculate the input area height based on content
    pub fn calculate_input_area_height(&self, width: u16) -> u16 {
        self.ui.calculate_input_area_height(width)
    }

    /// Update input scroll offset to keep cursor visible
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn update_input_scroll(&mut self, input_area_height: u16, width: u16) {
        self.ui.update_input_scroll(input_area_height, width);
    }

    /// Recompute input layout after editing: height + scroll
    pub fn recompute_input_layout_after_edit(&mut self, terminal_width: u16) {
        self.ui.recompute_input_layout_after_edit(terminal_width);
    }

    /// Apply a mutation to the textarea, then sync the string state
    pub fn apply_textarea_edit<F>(&mut self, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        self.ui.apply_textarea_edit(f);
    }

    /// Apply a mutation to the textarea, then sync and recompute layout
    pub fn apply_textarea_edit_and_recompute<F>(&mut self, terminal_width: u16, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        self.ui.apply_textarea_edit_and_recompute(terminal_width, f);
    }

    /// Calculate available chat height given terminal height and input area height
    pub fn calculate_available_height(&self, term_height: u16, input_area_height: u16) -> u16 {
        term_height
            .saturating_sub(input_area_height + 2) // Dynamic input area + borders
            .saturating_sub(1) // 1 for title
    }

    /// Clear input and reset cursor
    pub fn clear_input(&mut self) {
        self.ui.clear_input();
    }

    /// Get full input text (textarea is the source of truth; kept in sync with `input`)
    pub fn get_input_text(&self) -> &str {
        self.ui.get_input_text()
    }

    /// Set input text into both the string and the textarea
    pub fn set_input_text(&mut self, text: String) {
        self.ui.set_input_text(text);
    }

    /// Sync `input` from the textarea state. Cursor linear position is best-effort.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sync_input_from_textarea(&mut self) {
        self.ui.sync_input_from_textarea();
    }

    fn configure_textarea_appearance(&mut self) {
        self.ui.configure_textarea();
    }

    /// Open a theme picker modal with built-in and custom themes
    pub fn open_theme_picker(&mut self) {
        self.picker.open_theme_picker(&mut self.ui);
    }

    /// Apply theme by id (custom or built-in) and persist in config
    pub fn apply_theme_by_id(&mut self, id: &str) -> Result<(), String> {
        let cfg = Config::load_test_safe();
        let theme = if let Some(ct) = cfg.get_custom_theme(id) {
            Theme::from_spec(&theme_spec_from_custom(ct))
        } else if let Some(spec) = find_builtin_theme(id) {
            Theme::from_spec(&spec)
        } else {
            return Err(format!("Unknown theme: {}", id));
        };
        // Quantize to terminal color depth
        self.ui.theme = crate::utils::color::quantize_theme_for_current_terminal(theme);
        self.ui.current_theme_id = Some(id.to_string());
        self.configure_textarea_appearance();

        let mut cfg = Config::load_test_safe();
        cfg.theme = Some(id.to_string());
        cfg.save_test_safe().map_err(|e| e.to_string())?;
        if let Some(state) = self.theme_picker_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
        }
        Ok(())
    }

    /// Apply theme temporarily for preview (does not persist config)
    pub fn preview_theme_by_id(&mut self, id: &str) {
        // Try custom then built-in then no-op
        let cfg = Config::load_test_safe();
        if let Some(ct) = cfg.get_custom_theme(id) {
            self.ui.theme = crate::utils::color::quantize_theme_for_current_terminal(
                Theme::from_spec(&theme_spec_from_custom(ct)),
            );
            self.configure_textarea_appearance();
            return;
        }
        if let Some(spec) = find_builtin_theme(id) {
            self.ui.theme =
                crate::utils::color::quantize_theme_for_current_terminal(Theme::from_spec(&spec));
            self.configure_textarea_appearance();
        }
    }

    /// Revert theme to the one before opening picker (on cancel)
    pub fn revert_theme_preview(&mut self) {
        let previous_theme = self
            .theme_picker_state()
            .and_then(|state| state.before_theme.clone());

        if let Some(state) = self.theme_picker_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
            state.search_filter.clear();
            state.all_items.clear();
        }

        if let Some(prev) = previous_theme {
            self.ui.theme = prev;
            self.configure_textarea_appearance();
        }
    }

    /// Open a model picker modal with available models from current provider
    pub async fn open_model_picker(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.picker.open_model_picker(&self.session).await
    }

    /// Filter models based on search term and update picker
    pub fn filter_models(&mut self) {
        self.picker.filter_models();
    }

    /// Filter themes based on search term and update picker
    pub fn filter_themes(&mut self) {
        self.picker.filter_themes();
    }

    /// Filter providers based on search term and update picker
    pub fn filter_providers(&mut self) {
        self.picker.filter_providers();
    }

    /// Sort picker items based on current sort mode
    pub fn sort_picker_items(&mut self) {
        self.picker.sort_items();
    }

    /// Update picker title to show sort mode
    pub fn update_picker_title(&mut self) {
        self.picker.update_title();
    }
    /// Apply model by id and persist in current session (not config)
    pub fn apply_model_by_id(&mut self, model_id: &str) {
        self.session.model = model_id.to_string();
        if let Some(state) = self.model_picker_state_mut() {
            state.before_model = None;
        }
        // Complete provider->model transition if we were in one
        if self.picker.in_provider_model_transition {
            self.complete_provider_model_transition();
        }
    }

    /// Revert model to the one before opening picker (on cancel)
    pub fn revert_model_preview(&mut self) {
        let previous_model = self
            .model_picker_state()
            .and_then(|state| state.before_model.clone());

        if let Some(state) = self.model_picker_state_mut() {
            state.before_model = None;
            state.search_filter.clear();
            state.all_items.clear();
            state.has_dates = false;
        }

        if let Some(prev) = previous_model {
            self.session.model = prev;
        }
        // Check if we're in a provider->model transition and need to revert
        if self.picker.in_provider_model_transition {
            self.revert_provider_model_transition();
        }
    }

    /// Open a provider picker modal with available providers
    pub fn open_provider_picker(&mut self) {
        if let Err(message) = self.picker.open_provider_picker(&self.session) {
            self.set_status(message);
        }
    }

    /// Apply provider by id and update auth configuration (session-only)
    ///
    /// Returns a tuple with (Result, bool), where:
    /// - Result<(), String> indicates success or failure of the provider change
    /// - bool indicates whether a model picker should be opened after changing provider
    pub fn apply_provider_by_id(&mut self, provider_id: &str) -> (Result<(), String>, bool) {
        let auth_manager = AuthManager::new();
        let config = Config::load_test_safe();

        // Use the shared authentication resolution function to get provider info
        match auth_manager.resolve_authentication(Some(provider_id), &config) {
            Ok((api_key, base_url, provider_name, provider_display_name)) => {
                // Check if there's a default model for this provider
                let open_model_picker =
                    if let Some(default_model) = config.get_default_model(&provider_name) {
                        // Apply the default model immediately
                        self.session.api_key = api_key;
                        self.session.base_url = base_url;
                        self.session.provider_name = provider_name.clone();
                        self.session.provider_display_name = provider_display_name;
                        self.session.model = default_model.clone();
                        false // No need to open model picker
                    } else {
                        // No default model found, need to save state before changing provider
                        // so we can revert if model picker is cancelled
                        self.picker.in_provider_model_transition = true;
                        self.picker.provider_model_transition_state = Some((
                            self.session.provider_name.clone(),
                            self.session.provider_display_name.clone(),
                            self.session.model.clone(),
                            self.session.api_key.clone(),
                            self.session.base_url.clone(),
                        ));

                        // Apply the new provider
                        self.session.api_key = api_key;
                        self.session.base_url = base_url;
                        self.session.provider_name = provider_name.clone();
                        self.session.provider_display_name = provider_display_name;
                        true // Need to open model picker
                    };

                if let Some(state) = self.provider_picker_state_mut() {
                    state.before_provider = None;
                }

                (Ok(()), open_model_picker)
            }
            Err(e) => (Err(e.to_string()), false),
        }
    }

    /// Apply provider by id and persist as default provider in config
    ///
    /// Returns a tuple with (Result, bool), where:
    /// - Result<(), String> indicates success or failure of the provider change
    /// - bool indicates whether a model picker should be opened after changing provider
    pub fn apply_provider_by_id_persistent(
        &mut self,
        provider_id: &str,
    ) -> (Result<(), String>, bool) {
        // First apply the provider change
        let (result, open_model_picker) = self.apply_provider_by_id(provider_id);
        if let Err(e) = result {
            return (Err(e), false);
        }

        // Then persist to config
        let mut config = Config::load_test_safe();
        config.default_provider = Some(provider_id.to_string());
        match config.save_test_safe() {
            Ok(_) => (Ok(()), open_model_picker),
            Err(e) => (Err(e.to_string()), false),
        }
    }

    /// Revert provider to the one before opening picker (on cancel)
    pub fn revert_provider_preview(&mut self) {
        let previous_provider = self
            .provider_picker_state()
            .and_then(|state| state.before_provider.clone());

        if let Some(state) = self.provider_picker_state_mut() {
            state.before_provider = None;
            state.search_filter.clear();
            state.all_items.clear();
        }

        if let Some((prev_name, prev_display)) = previous_provider {
            self.session.provider_name = prev_name;
            self.session.provider_display_name = prev_display;
            // Note: We don't revert api_key and base_url as they should stay consistent with provider
        }
    }

    /// Revert provider and model to previous state during provider->model transition cancellation
    pub fn revert_provider_model_transition(&mut self) {
        if let Some((
            prev_provider_name,
            prev_provider_display,
            prev_model,
            prev_api_key,
            prev_base_url,
        )) = self.picker.provider_model_transition_state.take()
        {
            self.session.provider_name = prev_provider_name;
            self.session.provider_display_name = prev_provider_display;
            self.session.model = prev_model;
            self.session.api_key = prev_api_key;
            self.session.base_url = prev_base_url;
        }
        self.picker.in_provider_model_transition = false;
        self.picker.provider_model_transition_state = None;
    }

    /// Clear provider->model transition state when model is successfully selected
    pub fn complete_provider_model_transition(&mut self) {
        self.picker.in_provider_model_transition = false;
        self.picker.provider_model_transition_state = None;
    }

    /// Apply model by id and persist as default model for current provider in config
    pub fn apply_model_by_id_persistent(&mut self, model_id: &str) -> Result<(), String> {
        // First apply the model change (this will complete the transition)
        self.apply_model_by_id(model_id);

        // Then persist to config
        let mut config = Config::load_test_safe();
        config.set_default_model(self.session.provider_name.clone(), model_id.to_string());
        config.save_test_safe().map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Apply theme by id for session only (does not persist to config)
    pub fn apply_theme_by_id_session_only(&mut self, id: &str) -> Result<(), String> {
        let cfg = Config::load_test_safe();
        let theme = if let Some(ct) = cfg.get_custom_theme(id) {
            Theme::from_spec(&theme_spec_from_custom(ct))
        } else if let Some(spec) = find_builtin_theme(id) {
            Theme::from_spec(&spec)
        } else {
            return Err(format!("Unknown theme: {}", id));
        };
        // Quantize to terminal color depth
        self.ui.theme = crate::utils::color::quantize_theme_for_current_terminal(theme);
        self.ui.current_theme_id = Some(id.to_string());
        self.configure_textarea_appearance();
        if let Some(state) = self.theme_picker_state_mut() {
            state.before_theme = None;
            state.before_theme_id = None;
        }
        Ok(())
    }

    /// Unset the default model for a specific provider
    pub fn unset_default_model(&mut self, provider: &str) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.unset_default_model(provider);
        config.save_test_safe().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Unset the default theme
    pub fn unset_default_theme(&mut self) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.theme = None;
        config.save_test_safe().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Unset the default provider
    pub fn unset_default_provider(&mut self) -> Result<(), String> {
        let mut config = Config::load_test_safe();
        config.default_provider = None;
        config.save_test_safe().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(all(feature = "bench", not(test)))]
const _: fn(Theme, bool, bool) -> App = App::new_bench;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::text_wrapping::{TextWrapper, WrapConfig};
    use crate::ui::picker::PickerItem;
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use tui_textarea::{CursorMove, Input, Key};

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
        let expected_theme =
            crate::utils::color::quantize_theme_for_current_terminal(Theme::light());
        assert_eq!(
            resolved_theme.background_color,
            expected_theme.background_color
        );
    }

    #[test]
    fn theme_picker_highlights_active_theme_over_default() {
        let mut app = create_test_app();
        // Simulate active theme is light, while default (config) remains None in tests
        app.ui.current_theme_id = Some("light".to_string());

        // Open the theme picker
        app.open_theme_picker();

        // After sorting and selection alignment, ensure selected item has id "light"
        if let Some(picker) = app.picker_state() {
            let idx = picker.selected;
            let selected_id = &picker.items[idx].id;
            assert_eq!(selected_id, "light");
        } else {
            panic!("picker not opened");
        }
    }

    #[test]
    fn model_picker_title_uses_az_when_no_dates() {
        let mut app = create_test_app();
        // Build a model picker with no sort_key (no dates)
        let items = vec![
            PickerItem {
                id: "a-model".into(),
                label: "a-model".into(),
                metadata: None,
                sort_key: None,
            },
            PickerItem {
                id: "z-model".into(),
                label: "z-model".into(),
                metadata: None,
                sort_key: None,
            },
        ];
        let mut picker_state = PickerState::new("Pick Model", items.clone(), 0);
        picker_state.sort_mode = crate::ui::picker::SortMode::Name;
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Model,
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: String::new(),
                all_items: items,
                before_model: None,
                has_dates: false,
            }),
        });
        app.update_picker_title();
        let picker = app.picker_state().unwrap();
        assert!(picker.title.contains("Sort by: A-Z"));
    }

    #[test]
    fn provider_model_cancel_reverts_base_url_and_state() {
        let mut app = create_test_app();
        // Set current state to some new provider context
        app.session.provider_name = "newprov".into();
        app.session.provider_display_name = "NewProv".into();
        app.session.model = "new-model".into();
        app.session.api_key = "new-key".into();
        app.session.base_url = "https://api.newprov.test/v1".into();

        // Simulate saved previous state for transition
        app.picker.in_provider_model_transition = true;
        app.picker.provider_model_transition_state = Some((
            "oldprov".into(),
            "OldProv".into(),
            "old-model".into(),
            "old-key".into(),
            "https://api.oldprov.test/v1".into(),
        ));

        // Cancelling model picker should revert provider/model/api_key/base_url
        app.revert_model_preview();

        assert_eq!(app.session.provider_name, "oldprov");
        assert_eq!(app.session.provider_display_name, "OldProv");
        assert_eq!(app.session.model, "old-model");
        assert_eq!(app.session.api_key, "old-key");
        assert_eq!(app.session.base_url, "https://api.oldprov.test/v1");
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
    }

    #[test]
    fn calculate_available_height_matches_expected_layout_rules() {
        let app = create_test_app();

        let cases = [
            (30, 5, 22), // 30 - (5 + 2) - 1
            (10, 8, 0),  // Saturating at zero when chat area would be negative
            (5, 0, 2),   // Just borders and title removed
        ];

        for (term_height, input_height, expected) in cases {
            assert_eq!(
                app.calculate_available_height(term_height, input_height),
                expected
            );
        }
    }

    #[test]
    fn default_sort_mode_helper_behaviour() {
        let mut app = create_test_app();
        // Theme picker prefers alphabetical → Name
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Theme,
            state: PickerState::new("Pick Theme", vec![], 0),
            data: PickerData::Theme(ThemePickerState {
                search_filter: String::new(),
                all_items: Vec::new(),
                before_theme: None,
                before_theme_id: None,
            }),
        });
        assert!(matches!(
            app.picker_session().unwrap().default_sort_mode(),
            crate::ui::picker::SortMode::Name
        ));
        // Provider picker prefers alphabetical → Name
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Provider,
            state: PickerState::new("Pick Provider", vec![], 0),
            data: PickerData::Provider(ProviderPickerState {
                search_filter: String::new(),
                all_items: Vec::new(),
                before_provider: None,
            }),
        });
        assert!(matches!(
            app.picker_session().unwrap().default_sort_mode(),
            crate::ui::picker::SortMode::Name
        ));
        // Model picker with dates → Date
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Model,
            state: PickerState::new("Pick Model", vec![], 0),
            data: PickerData::Model(ModelPickerState {
                search_filter: String::new(),
                all_items: Vec::new(),
                before_model: None,
                has_dates: true,
            }),
        });
        assert!(matches!(
            app.picker_session().unwrap().default_sort_mode(),
            crate::ui::picker::SortMode::Date
        ));
        // Model picker without dates → Name
        if let Some(PickerSession {
            data: PickerData::Model(state),
            ..
        }) = app.picker_session_mut()
        {
            state.has_dates = false;
        }
        assert!(matches!(
            app.picker_session().unwrap().default_sort_mode(),
            crate::ui::picker::SortMode::Name
        ));
    }

    #[test]
    fn prewrap_cache_reuse_no_changes() {
        let mut app = create_test_app();
        for i in 0..50 {
            app.ui.messages.push_back(Message {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                content: "lorem ipsum dolor sit amet consectetur adipiscing elit".into(),
            });
        }
        let w = 100u16;
        let ptr1 = {
            let p1 = app.get_prewrapped_lines_cached(w);
            assert!(!p1.is_empty());
            p1.as_ptr()
        };
        let ptr2 = {
            let p2 = app.get_prewrapped_lines_cached(w);
            p2.as_ptr()
        };
        assert_eq!(ptr1, ptr2, "cache should be reused when nothing changed");
    }

    #[test]
    fn prewrap_cache_invalidates_on_width_change() {
        let mut app = create_test_app();
        app.ui.messages.push_back(Message {
            role: "user".into(),
            content: "hello world".into(),
        });
        let ptr1 = {
            let p1 = app.get_prewrapped_lines_cached(80);
            p1.as_ptr()
        };
        let ptr2 = {
            let p2 = app.get_prewrapped_lines_cached(120);
            p2.as_ptr()
        };
        assert_ne!(ptr1, ptr2, "cache should invalidate on width change");
    }

    #[test]
    fn prewrap_cache_updates_metadata_for_markdown_last_message() {
        let mut app = create_test_app();
        app.ui
            .messages
            .push_back(create_test_message("user", "This is the opening line."));
        app.ui.messages.push_back(create_test_message(
            "assistant",
            "Initial response that will be replaced.",
        ));

        let width = 72;
        let initial_lines = app.get_prewrapped_lines_cached(width).clone();
        let initial_meta = app.get_prewrapped_span_metadata_cached(width).clone();
        assert_eq!(initial_lines.len(), initial_meta.len());

        if let Some(last) = app.ui.messages.back_mut() {
            last.content = "Here's an updated reply with a [link](https://example.com).".into();
        }

        let updated_lines = app.get_prewrapped_lines_cached(width).clone();
        let updated_meta = app.get_prewrapped_span_metadata_cached(width).clone();
        assert_eq!(updated_lines.len(), updated_meta.len());
        assert!(updated_meta
            .iter()
            .any(|kinds| kinds.iter().any(|kind| kind.is_link())));
    }

    #[test]
    fn prewrap_cache_updates_metadata_for_plain_text_last_message() {
        let mut app = create_test_app();
        app.ui.markdown_enabled = false;
        app.ui.syntax_enabled = false;
        app.ui
            .messages
            .push_back(create_test_message("user", "Plain intro from the user."));
        app.ui.messages.push_back(create_test_message(
            "assistant",
            "A short reply that will expand into a much longer paragraph after the update.",
        ));

        let width = 40;
        let initial_lines = app.get_prewrapped_lines_cached(width).clone();
        let initial_meta = app.get_prewrapped_span_metadata_cached(width).clone();
        assert_eq!(initial_lines.len(), initial_meta.len());

        if let Some(last) = app.ui.messages.back_mut() {
            last.content = "Now the assistant responds with a deliberately long piece of plain text that should wrap across multiple terminal lines once re-rendered.".into();
        }

        let updated_lines = app.get_prewrapped_lines_cached(width).clone();
        let updated_meta = app.get_prewrapped_span_metadata_cached(width).clone();
        assert_eq!(updated_lines.len(), updated_meta.len());
        assert!(updated_meta
            .iter()
            .flat_map(|kinds| kinds.iter())
            .all(|kind| kind.is_text()));
    }

    #[test]
    fn test_system_messages_excluded_from_api() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add a user message
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        // Add a system message (like from /help command)
        app.add_system_message(
            "This is a system message that should not be sent to API".to_string(),
        );

        // Add an assistant message
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        // Add another system message
        app.add_system_message("Another system message".to_string());

        // Test add_user_message - should exclude system messages
        let api_messages = app.add_user_message("How are you?".to_string());

        // Should include: first user message, assistant message, and the new user message
        // (the new empty assistant message gets excluded by take())
        assert_eq!(api_messages.len(), 3);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Hello");
        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[1].content, "Hi there!");
        assert_eq!(api_messages[2].role, "user");
        assert_eq!(api_messages[2].content, "How are you?");

        // Verify system messages are not included
        for msg in &api_messages {
            assert_ne!(msg.role, "system");
        }
    }

    #[test]
    fn test_prepare_retry_excludes_system_messages() {
        let mut app = create_test_app();

        // Add messages in order: user, system, assistant
        app.ui.messages.push_back(Message {
            role: "user".to_string(),
            content: "Test question".to_string(),
        });

        app.add_system_message("System message between user and assistant".to_string());

        app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: "Test response".to_string(),
        });

        // Set up retry state
        app.session.retrying_message_index = Some(2); // Retry the assistant message at index 2

        // Test prepare_retry
        let api_messages = app.prepare_retry(10, 80).unwrap();

        // Should only include the user message, excluding the system message
        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test question");

        // Verify no system messages are included
        for msg in &api_messages {
            assert_ne!(msg.role, "system");
        }
    }

    #[test]
    fn prewrap_cache_plain_text_last_message_wrapping() {
        // Reproduce the fast-path tail update and ensure plain-text wrapping is preserved
        let mut app = crate::utils::test_utils::create_test_app();
        app.ui.markdown_enabled = false;
        let theme = app.ui.theme.clone();

        // Start with two assistant messages
        app.ui.messages.push_back(Message {
            role: "assistant".into(),
            content: "Short".into(),
        });
        app.ui.messages.push_back(Message {
            role: "assistant".into(),
            content: "This is a very long plain text line that should wrap when width is small"
                .into(),
        });

        let width = 20u16;
        app.get_prewrapped_lines_cached(width);

        // Update only the last message content to trigger the fast path
        if let Some(last) = app.ui.messages.back_mut() {
            last.content.push_str(" and now it changed");
        }
        let second = app.get_prewrapped_lines_cached(width).clone();
        // Convert to strings and check for wrapping (no line exceeds width)
        let rendered: Vec<String> = second.iter().map(|l| l.to_string()).collect();
        let content_lines: Vec<&String> = rendered.iter().filter(|s| !s.is_empty()).collect();
        assert!(
            content_lines.len() > 2,
            "Expected multiple wrapped content lines"
        );
        for (i, s) in content_lines.iter().enumerate() {
            assert!(
                s.chars().count() <= width as usize,
                "Line {} exceeds width: '{}' (len={})",
                i,
                s,
                s.len()
            );
        }

        // Silence unused warning
        let _ = theme;
    }

    #[test]
    fn test_sync_cursor_mapping_single_and_multi_line() {
        let mut app = create_test_app();

        // Single line: move to end
        app.set_input_text("hello world".to_string());
        app.ui.textarea.move_cursor(CursorMove::End);
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "hello world");
        assert_eq!(app.ui.input_cursor_position, 11);

        // Multi-line: jump to (row=1, col=3) => after "wor" on second line
        app.set_input_text("hello\nworld".to_string());
        app.ui.textarea.move_cursor(CursorMove::Jump(1, 3));
        app.sync_input_from_textarea();
        // 5 (hello) + 1 (\n) + 3 = 9
        assert_eq!(app.ui.input_cursor_position, 9);
    }

    #[test]
    fn test_backspace_at_start_noop() {
        let mut app = create_test_app();
        app.set_input_text("abc".to_string());
        // Move to head of line
        app.ui.textarea.move_cursor(CursorMove::Head);
        // Simulate backspace (always single-char via input_without_shortcuts)
        app.ui.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: false,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "abc");
        assert_eq!(app.ui.input_cursor_position, 0);
    }

    #[test]
    fn test_backspace_at_line_start_joins_lines() {
        let mut app = create_test_app();
        app.set_input_text("hello\nworld".to_string());
        // Move to start of second line
        app.ui.textarea.move_cursor(CursorMove::Jump(1, 0));
        // Backspace should join lines; use input_without_shortcuts to ensure single-char delete
        app.ui.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: false,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "helloworld");
        // Cursor should be at end of former first line (index 5)
        assert_eq!(app.ui.input_cursor_position, 5);
    }

    #[test]
    fn test_backspace_with_alt_modifier_deletes_single_char() {
        let mut app = create_test_app();
        app.set_input_text("hello world".to_string());
        app.ui.textarea.move_cursor(CursorMove::End);
        // Simulate Alt+Backspace; with input_without_shortcuts it should still delete one char
        app.ui.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: true,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "hello worl");
        assert_eq!(app.ui.input_cursor_position, "hello worl".chars().count());
    }

    #[test]
    fn test_update_input_scroll_keeps_cursor_visible() {
        let mut app = create_test_app();
        // Long line that wraps at width 10 into multiple lines
        let text = "one two three four five six seven eight nine ten";
        app.set_input_text(text.to_string());
        // Simulate small input area: width=20 total => inner available width accounts in method
        let width: u16 = 10; // small terminal width to force wrapping (inner ~4)
        let input_area_height: u16 = 2; // only 2 lines visible
                                        // Place cursor near end
        app.ui.input_cursor_position = text.chars().count().saturating_sub(1);
        app.update_input_scroll(input_area_height, width);
        // With cursor near end, scroll offset should be > 0 to bring cursor into view
        assert!(app.ui.input_scroll_offset > 0);
    }

    #[test]
    fn test_shift_like_up_down_moves_one_line_on_many_newlines() {
        let mut app = create_test_app();
        // Build text with many blank lines
        let text = "top\n\n\n\n\n\n\n\n\n\nbottom";
        app.set_input_text(text.to_string());
        // Jump to bottom line, col=3 (after 'bot')
        let bottom_row_usize = app.ui.textarea.lines().len().saturating_sub(1);
        let bottom_row = bottom_row_usize as u16;
        app.ui.textarea.move_cursor(CursorMove::Jump(bottom_row, 3));
        app.sync_input_from_textarea();
        let (row_before, col_before) = app.ui.textarea.cursor();
        assert_eq!(row_before, bottom_row as usize);
        assert!(col_before <= app.ui.textarea.lines()[bottom_row_usize].chars().count());

        // Move up exactly one line
        app.ui.textarea.move_cursor(CursorMove::Up);
        app.sync_input_from_textarea();
        let (row_after_up, col_after_up) = app.ui.textarea.cursor();
        assert_eq!(row_after_up, bottom_row_usize.saturating_sub(1));
        // Column should clamp reasonably; we just assert it's within line bounds
        assert!(col_after_up <= app.ui.textarea.lines()[8].chars().count());

        // Move down exactly one line
        app.ui.textarea.move_cursor(CursorMove::Down);
        app.sync_input_from_textarea();
        let (row_after_down, _col_after_down) = app.ui.textarea.cursor();
        assert_eq!(row_after_down, bottom_row_usize);
    }

    #[test]
    fn test_shift_like_left_right_moves_one_char() {
        let mut app = create_test_app();
        app.set_input_text("hello".to_string());
        // Move to end, then back by one, then forward by one
        app.ui.textarea.move_cursor(CursorMove::End);
        app.sync_input_from_textarea();
        let end_pos = app.ui.input_cursor_position;
        app.ui.textarea.move_cursor(CursorMove::Back);
        app.sync_input_from_textarea();
        let back_pos = app.ui.input_cursor_position;
        assert_eq!(back_pos, end_pos.saturating_sub(1));
        app.ui.textarea.move_cursor(CursorMove::Forward);
        app.sync_input_from_textarea();
        let forward_pos = app.ui.input_cursor_position;
        assert_eq!(forward_pos, end_pos);
    }

    #[test]
    fn test_cursor_mapping_blankline_insert_no_desync() {
        let mut app = create_test_app();
        let text = "asdf\n\nasdf\n\nasdf";
        app.set_input_text(text.to_string());
        // Jump to blank line 2 (0-based row 3), column 0
        app.ui.textarea.move_cursor(CursorMove::Jump(3, 0));
        app.sync_input_from_textarea();
        // Insert a character on the blank line
        app.ui.textarea.insert_str("x");
        app.sync_input_from_textarea();

        // Compute wrapped position using same wrapper logic (no wrapping with wide width)
        let config = WrapConfig::new(120);
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
            app.get_input_text(),
            app.ui.input_cursor_position,
            &config,
        );
        // Compare to textarea's cursor row/col
        let (row, c) = app.ui.textarea.cursor();
        assert_eq!(line, row);
        assert_eq!(col, c);
    }

    #[test]
    fn test_recompute_input_layout_after_edit_updates_scroll() {
        let mut app = create_test_app();
        // Make text long enough to wrap
        let text = "one two three four five six seven eight nine ten";
        app.set_input_text(text.to_string());
        // Place cursor near end
        app.ui.input_cursor_position = text.chars().count().saturating_sub(1);
        // Very small terminal width to force heavy wrapping; method accounts for borders and margin
        let width: u16 = 6;
        app.recompute_input_layout_after_edit(width);
        // With cursor near end on a heavily wrapped input, expect some scroll
        assert!(app.ui.input_scroll_offset > 0);
        // Changing cursor position to start should reduce or reset scroll
        app.ui.input_cursor_position = 0;
        app.recompute_input_layout_after_edit(width);
        assert_eq!(app.ui.input_scroll_offset, 0);
    }

    #[test]
    fn test_last_and_first_user_message_index() {
        let mut app = create_test_app();
        // No messages
        assert_eq!(app.last_user_message_index(), None);
        assert_eq!(app.first_user_message_index(), None);

        // Add messages: user, assistant, user
        app.ui.messages.push_back(create_test_message("user", "u1"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "a1"));
        app.ui.messages.push_back(create_test_message("user", "u2"));

        assert_eq!(app.first_user_message_index(), Some(0));
        assert_eq!(app.last_user_message_index(), Some(2));
    }

    #[test]
    fn prewrap_height_matches_renderer_with_tables() {
        // Test that scroll height calculations match renderer height when tables are involved
        let mut app = create_test_app();

        // Add a message with a large table that will trigger width-dependent wrapping
        let table_content = r#"| Government System | Definition | Key Properties |
|-------------------|------------|----------------|
| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections, Protection of civil liberties |
| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |
| Monarchy | A form of government in which a single person, known as a monarch, rules until death or abdication. | Hereditary succession, Often ceremonial with limited political power |
"#;

        app.ui.messages.push_back(Message {
            role: "assistant".into(),
            content: table_content.to_string(),
        });

        let width = 80u16;

        // Get the height that the renderer will actually use (prewrapped with width)
        let renderer_height = {
            let lines = app.get_prewrapped_lines_cached(width);
            lines.len() as u16
        };

        // Get the height that scroll calculations currently use
        let scroll_height = app.calculate_wrapped_line_count(width);

        // These should match - if they don't, scroll targeting will be off
        assert_eq!(
            renderer_height, scroll_height,
            "Renderer height ({}) should match scroll calculation height ({})",
            renderer_height, scroll_height
        );
    }

    #[test]
    fn streaming_table_autoscroll_stays_consistent() {
        // Test that autoscroll stays at bottom when streaming table content
        let mut app = create_test_app();

        // Start with a user message
        app.add_user_message("Generate a table".to_string());

        let width = 80u16;
        let available_height = 20u16;

        // Start streaming a table in chunks
        let table_start = "Here's a government systems table:\n\n";
        app.append_to_response(table_start, available_height, width);

        let table_header = "| Government System | Definition | Key Properties |\n|-------------------|------------|----------------|\n";
        app.append_to_response(table_header, available_height, width);

        // Add table rows that will cause wrapping and potentially height changes
        let row1 = "| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections |\n";
        app.append_to_response(row1, available_height, width);

        let row2 = "| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |\n";
        app.append_to_response(row2, available_height, width);

        // After each append, if we're auto-scrolling, we should be at the bottom
        if app.ui.auto_scroll {
            let expected_max_scroll = app.calculate_max_scroll_offset(available_height, width);
            assert_eq!(
                app.ui.scroll_offset, expected_max_scroll,
                "Auto-scroll should keep us at bottom. Current offset: {}, Expected max: {}",
                app.ui.scroll_offset, expected_max_scroll
            );
        }
    }

    #[test]
    fn block_selection_offset_matches_renderer_with_tables() {
        // Test that block selection scroll calculations match renderer when tables are involved
        let mut app = create_test_app();

        // Add content with a table followed by a code block
        let content_with_table_and_code = r#"Here's a table:

| Government System | Definition | Key Properties |
|-------------------|------------|----------------|
| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections |
| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |

And here's some code:

```rust
fn main() {
    println!("Hello, world!");
}
```"#;

        app.ui.messages.push_back(Message {
            role: "assistant".into(),
            content: content_with_table_and_code.to_string(),
        });

        let width = 80u16;
        let available_height = 20u16;

        // Get codeblock ranges (these are computed from widthless lines)
        let ranges = crate::ui::markdown::compute_codeblock_ranges(&app.ui.messages, &app.ui.theme);
        assert!(!ranges.is_empty(), "Should have at least one code block");

        let (code_block_start, _len, _content) = &ranges[0];

        // Test that block selection navigation uses the same width-aware approach as the renderer
        // Both should now use width-aware line building for consistent results

        // The key insight: Both block navigation and rendering should use the same cached approach
        // for consistency. In production, block navigation would also use get_prewrapped_lines_cached.
        let lines = app.get_prewrapped_lines_cached(width);

        let _block_nav_offset = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
            lines,
            width,
            available_height,
            *code_block_start,
        );

        // Since both use the same method, heights are identical
        let block_nav_height = lines.len();
        let renderer_height = lines.len();

        // This should always pass now since they're the same method
        assert_eq!(
            block_nav_height, renderer_height,
            "Block navigation height ({}) should match renderer height ({}) for accurate block selection",
            block_nav_height, renderer_height
        );

        // Legacy widthless path is deprecated under the unified layout engine.
        // We no longer assert differences against that path because width-aware layout
        // is the single source of truth for visual line counts.
    }

    #[test]
    fn narrow_terminal_exposes_scroll_height_mismatch() {
        // Test with very narrow terminal that forces significant table wrapping differences
        let mut app = create_test_app();

        // Add a wide table that will need significant rebalancing in narrow terminals
        let wide_table = r#"| Very Long Government System Name | Very Detailed Definition That Goes On And On | Extremely Detailed Key Properties That Include Many Words |
|-----------------------------------|-----------------------------------------------|------------------------------------------------------------|
| Constitutional Democratic Republic | A complex system where power is distributed among elected representatives who operate within a constitutional framework with checks and balances | Multi-party elections, separation of powers, constitutional limits, judicial review, civil liberties protection |
| Authoritarian Single-Party State | A centralized system where one political party maintains exclusive control over government institutions and suppresses opposition | Centralized control, restricted freedoms, state propaganda, limited political participation, strict social control |

Some additional text after the table."#;

        app.ui.messages.push_back(Message {
            role: "assistant".into(),
            content: wide_table.to_string(),
        });

        // Use very narrow width that will force aggressive table column rebalancing
        let width = 40u16;

        // Get the height that the renderer will actually use (prewrapped with narrow width)
        let renderer_height = {
            let lines = app.get_prewrapped_lines_cached(width);
            lines.len() as u16
        };

        // Get the height that scroll calculations currently use (widthless, then scroll heuristic)
        let scroll_height = app.calculate_wrapped_line_count(width);

        // This should expose the mismatch if it exists
        assert_eq!(
            renderer_height, scroll_height,
            "Narrow terminal: Renderer height ({}) should match scroll calculation height ({})",
            renderer_height, scroll_height
        );
    }

    #[test]
    fn streaming_table_with_cache_invalidation_consistency() {
        // Test the exact scenario: streaming table generation with cache invalidation
        let mut app = create_test_app();

        let width = 80u16;
        let available_height = 20u16;

        // Start with user message and empty assistant response
        app.add_user_message("Generate a large comparison table".to_string());

        // Simulate streaming a large table piece by piece, with cache invalidation
        let table_chunks = vec![
            "Here's a detailed comparison table:\n\n",
            "| Feature | Option A | Option B | Option C |\n",
            "|---------|----------|----------|----------|\n",
            "| Performance | Very fast execution with optimized algorithms | Moderate speed with good balance | Slower but more flexible |
",
            "| Memory Usage | Low memory footprint, efficient data structures | Medium usage with some overhead | Higher memory requirements |
",
            "| Ease of Use | Complex setup but powerful once configured | User-friendly with good documentation | Simple and intuitive interface |
",
            "| Cost | Enterprise pricing with volume discounts available | Reasonable pricing for small to medium teams | Free with optional premium features |
",
        ];

        for chunk in table_chunks {
            // Before append: get current scroll state
            let _scroll_before = app.ui.scroll_offset;
            let _max_scroll_before = app.calculate_max_scroll_offset(available_height, width);

            // Append content (this invalidates prewrap cache)
            app.append_to_response(chunk, available_height, width);

            // After append: check scroll consistency
            let scroll_after = app.ui.scroll_offset;
            let max_scroll_after = app.calculate_max_scroll_offset(available_height, width);

            // During streaming with auto_scroll=true, we should always be at bottom
            if app.ui.auto_scroll {
                assert_eq!(
                    scroll_after, max_scroll_after,
                    "Auto-scroll should keep us at bottom after streaming chunk"
                );
            }

            // The key test: prewrap cache and scroll calculation should give same height
            let prewrap_height = app.get_prewrapped_lines_cached(width).len() as u16;
            let scroll_calc_height = app.calculate_wrapped_line_count(width);

            assert_eq!(
                prewrap_height, scroll_calc_height,
                "After streaming chunk, prewrap height ({}) should match scroll calc height ({})",
                prewrap_height, scroll_calc_height
            );
        }
    }

    #[test]
    fn test_page_up_down_and_home_end_behavior() {
        let mut app = create_test_app();
        // Create enough messages to require scrolling
        for _ in 0..50 {
            app.ui
                .messages
                .push_back(create_test_message("assistant", "line content"));
        }

        let width: u16 = 80;
        let input_area_height = 3u16; // pretend a small input area
        let term_height = 24u16;
        let available_height = app.calculate_available_height(term_height, input_area_height);

        // Sanity: have some scrollable height
        let max_scroll = app.calculate_max_scroll_offset(available_height, width);
        assert!(max_scroll > 0);

        // Start in the middle
        let step = available_height.saturating_sub(1);
        app.ui.scroll_offset = (step * 2).min(max_scroll);

        // Page up reduces by step, not below 0
        let before = app.ui.scroll_offset;
        app.page_up(available_height);
        let after_up = app.ui.scroll_offset;
        assert_eq!(after_up, before.saturating_sub(step));
        assert!(!app.ui.auto_scroll);

        // Page down increases by step, clamped to max
        app.page_down(available_height, width);
        let after_down = app.ui.scroll_offset;
        assert!(after_down >= after_up);
        assert!(after_down <= max_scroll);
        assert!(!app.ui.auto_scroll);

        // Home goes to top and disables auto-scroll
        app.scroll_to_top();
        assert_eq!(app.ui.scroll_offset, 0);
        assert!(!app.ui.auto_scroll);

        // End goes to bottom and enables auto-scroll
        app.scroll_to_bottom_view(available_height, width);
        assert_eq!(app.ui.scroll_offset, max_scroll);
        assert!(app.ui.auto_scroll);
    }

    #[test]
    fn test_prev_next_user_message_index_navigation() {
        let mut app = create_test_app();
        // indices: 0 user, 1 assistant, 2 system, 3 user
        app.ui.messages.push_back(create_test_message("user", "u1"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "a1"));
        app.ui
            .messages
            .push_back(create_test_message("system", "s1"));
        app.ui.messages.push_back(create_test_message("user", "u2"));

        // From index 3 (user) prev should be 0 (skipping non-user)
        assert_eq!(app.prev_user_message_index(3), Some(0));
        // From index 0 next should be 3 (skipping non-user)
        assert_eq!(app.next_user_message_index(0), Some(3));
        // From index 1 prev should be 0
        assert_eq!(app.prev_user_message_index(1), Some(0));
        // From index 1 next should be 3
        assert_eq!(app.next_user_message_index(1), Some(3));
    }

    #[test]
    fn test_set_input_text_places_cursor_at_end() {
        let mut app = create_test_app();
        let text = String::from("line1\nline2");
        app.set_input_text(text.clone());
        // Linear cursor at end
        assert_eq!(app.ui.input_cursor_position, text.chars().count());
        // Textarea cursor at end (last row/col)
        let (row, col) = app.ui.textarea.cursor();
        let lines = app.ui.textarea.lines();
        assert_eq!(row, lines.len() - 1);
        assert_eq!(col, lines.last().unwrap().chars().count());
    }
}
