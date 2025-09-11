use crate::api::models::{fetch_models, sort_models};
use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::core::message::Message;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::ui::appearance::{detect_preferred_appearance, Appearance};
use crate::ui::builtin_themes::{find_builtin_theme, theme_spec_from_custom};
use crate::ui::picker::{PickerItem, PickerState};
use crate::ui::theme::Theme;
use crate::utils::logging::LoggingState;
use crate::utils::scroll::ScrollCalculator;
use crate::utils::url::construct_api_url;
use chrono::Utc;
use ratatui::text::Line;
use reqwest::Client;
use std::{collections::VecDeque, time::Instant};
use tokio_util::sync::CancellationToken;
use tui_textarea::TextArea;

pub struct App {
    pub messages: VecDeque<Message>,
    pub input: String,
    pub input_cursor_position: usize,
    pub input_mode: bool,
    // Edit/select modes
    pub edit_select_mode: bool,
    pub selected_user_message_index: Option<usize>,
    pub in_place_edit_index: Option<usize>,
    pub current_response: String,
    pub client: Client,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub provider_name: String,
    pub provider_display_name: String,
    pub scroll_offset: u16,
    pub horizontal_scroll_offset: u16,
    pub auto_scroll: bool,
    pub is_streaming: bool,
    pub pulse_start: Instant,
    pub stream_interrupted: bool,
    pub logging: LoggingState,
    pub stream_cancel_token: Option<CancellationToken>,
    pub current_stream_id: u64,
    pub last_retry_time: Instant,
    pub retrying_message_index: Option<usize>,
    pub input_scroll_offset: u16,
    pub textarea: TextArea<'static>,
    pub theme: Theme,
    pub picker: Option<PickerState>,
    pub picker_mode: Option<PickerMode>,
    // Block select mode (inline, like Ctrl+P for user messages)
    pub block_select_mode: bool,
    pub selected_block_index: Option<usize>,
    pub theme_before_picker: Option<Theme>,
    pub theme_id_before_picker: Option<String>,
    // Rendering toggles
    pub markdown_enabled: bool,
    pub syntax_enabled: bool,
    // Cached prewrapped chat lines for fast redraws in normal mode
    pub(crate) prewrap_cache: Option<PrewrapCache>,
    // One-line ephemeral status message (shown in input border)
    pub status: Option<String>,
    pub status_set_at: Option<Instant>,
    // When present, the input area is used to prompt for a filename
    pub file_prompt: Option<FilePrompt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerMode {
    Theme,
}

impl App {
    pub async fn new_with_auth(
        model: String,
        log_file: Option<String>,
        provider: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let auth_manager = AuthManager::new();
        let config = Config::load()?;

        // Use the shared authentication resolution function
        let (api_key, base_url, provider_name, provider_display_name) =
            auth_manager.resolve_authentication(provider.as_deref(), &config)?;

        // Determine the model to use:
        // 1. If a specific model was requested (not "default"), use that
        // 2. If a default model is set for this provider in config, use that
        // 3. Otherwise, fetch and use the newest available model
        let final_model = if model != "default" {
            model
        } else if let Some(default_model) = config.get_default_model(&provider_name) {
            default_model.clone()
        } else {
            // Try to fetch the newest model directly since we're already in an async context
            let temp_client = Client::new();
            let temp_app = App {
                messages: VecDeque::new(),
                input: String::new(),
                input_cursor_position: 0,
                input_mode: true,
                edit_select_mode: false,
                selected_user_message_index: None,
                in_place_edit_index: None,
                current_response: String::new(),
                client: temp_client.clone(),
                model: model.clone(),
                api_key: api_key.clone(),
                base_url: base_url.clone(),
                provider_name: provider_name.to_string(),
                provider_display_name: provider_display_name.clone(),
                scroll_offset: 0,
                horizontal_scroll_offset: 0,
                auto_scroll: true,
                is_streaming: false,
                pulse_start: Instant::now(),
                stream_interrupted: false,
                logging: LoggingState::new(None)?,
                stream_cancel_token: None,
                current_stream_id: 0,
                last_retry_time: Instant::now(),
                retrying_message_index: None,
                input_scroll_offset: 0,
                textarea: TextArea::default(),
                theme: match &config.theme {
                    Some(name) => Theme::from_name(name),
                    None => Theme::dark_default(),
                },
                picker: None,
                picker_mode: None,
                block_select_mode: false,
                selected_block_index: None,
                theme_before_picker: None,
                theme_id_before_picker: None,
                markdown_enabled: config.markdown.unwrap_or(true),
                syntax_enabled: config.syntax.unwrap_or(true),
                prewrap_cache: None,
                status: None,
                status_set_at: None,
                file_prompt: None,
            };

            // Try to fetch the newest model
            match temp_app.fetch_newest_model().await {
                Ok(Some(newest_model)) => {
                    eprintln!("🔄 Using newest available model: {newest_model}");
                    newest_model
                }
                Ok(None) => {
                    return Err(
                        "No models found for this provider. Please specify a model explicitly."
                            .into(),
                    );
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to fetch models from API: {e}. Please specify a model explicitly."
                    )
                    .into());
                }
            }
        };

        // Print configuration info
        eprintln!("🚀 Starting Chabeau - Terminal Chat Interface");
        eprintln!("🔐 Provider: {provider_name}");
        eprintln!("📡 Using model: {final_model}");

        // Note: We use the OpenAI API format for all providers including Anthropic
        // This is known to work well with Anthropic's models
        let api_endpoint = construct_api_url(&base_url, "chat/completions");
        eprintln!("🌐 API endpoint: {api_endpoint}");

        if let Some(ref log_path) = log_file {
            eprintln!("📝 Logging to: {log_path}");
        }
        eprintln!("💡 Press Ctrl+C to quit, Enter to send messages");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let logging = LoggingState::new(log_file.clone())?;
        // If logging was enabled via command line, log the start timestamp
        if let Some(_log_path) = log_file {
            let timestamp = Utc::now().to_rfc3339();
            if let Err(e) = logging.log_message(&format!("## Logging started at {}", timestamp)) {
                eprintln!("Warning: Failed to write initial log timestamp: {}", e);
            }
        }

        // Resolve theme: prefer explicit config (custom or built-in). If unset, try
        // detecting preferred appearance via OS hint and choose a suitable default.
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
            None => match detect_preferred_appearance() {
                Some(Appearance::Light) => Theme::light(),
                Some(Appearance::Dark) => Theme::dark_default(),
                None => Theme::dark_default(),
            },
        };

        let mut app = App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            edit_select_mode: false,
            selected_user_message_index: None,
            in_place_edit_index: None,
            current_response: String::new(),
            client: Client::new(),
            model: final_model,
            api_key,
            base_url,
            provider_name: provider_name.to_string(),
            provider_display_name,
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: Instant::now(),
            stream_interrupted: false,
            logging,
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
            textarea: TextArea::default(),
            theme: resolved_theme,
            picker: None,
            picker_mode: None,
            block_select_mode: false,
            selected_block_index: None,
            theme_before_picker: None,
            theme_id_before_picker: None,
            markdown_enabled: config.markdown.unwrap_or(true),
            syntax_enabled: config.syntax.unwrap_or(true),
            prewrap_cache: None,
            status: None,
            status_set_at: None,
            file_prompt: None,
        };

        // Keep textarea state in sync with the string input initially
        app.set_input_text(String::new());
        app.configure_textarea_appearance();

        Ok(app)
    }

    /// Return prewrapped chat lines for current state, caching across frames when safe.
    /// Cache is used only in normal mode (no selection/highlight) and invalidated when
    /// width, flags, theme signature, message count, or last message content hash changes.
    pub fn get_prewrapped_lines_cached(&mut self, width: u16) -> &Vec<Line<'static>> {
        let theme_sig = compute_theme_signature(&self.theme);
        let markdown = self.markdown_enabled;
        let syntax = self.syntax_enabled;
        let msg_len = self.messages.len();
        let last_hash = hash_last_message(&self.messages);

        let mut can_reuse = false;
        let mut only_last_changed = false;
        if let Some(c) = &self.prewrap_cache {
            if c.width == width
                && c.markdown_enabled == markdown
                && c.syntax_enabled == syntax
                && c.theme_sig == theme_sig
                && c.messages_len == msg_len
            {
                if c.last_msg_hash == last_hash {
                    can_reuse = true;
                } else {
                    only_last_changed = true;
                }
            }
        }

        if can_reuse {
            // Up-to-date
        } else if only_last_changed {
            // Fast path: only last message content changed; update tail of cache
            if let (Some(c), Some(last_msg)) = (self.prewrap_cache.as_mut(), self.messages.back()) {
                let last_lines = if markdown {
                    crate::ui::markdown::render_message_markdown_opts(last_msg, &self.theme, syntax)
                        .lines
                } else {
                    crate::ui::markdown::build_plain_display_lines(
                        &VecDeque::from([last_msg.clone()]),
                        &self.theme,
                    )
                };
                let last_pre =
                    crate::utils::scroll::ScrollCalculator::prewrap_lines(&last_lines, width);
                let start = c.last_start;
                let old_len = c.last_len;
                let mut new_buf: Vec<Line<'static>> =
                    Vec::with_capacity(c.lines.len() - old_len + last_pre.len());
                new_buf.extend_from_slice(&c.lines[..start]);
                new_buf.extend_from_slice(&last_pre);
                c.lines = new_buf;
                c.last_len = last_pre.len();
                c.last_msg_hash = last_hash;
            } else {
                // Fallback to full rebuild if something unexpected
                only_last_changed = false;
            }
        }

        if self.prewrap_cache.is_none() || (!can_reuse && !only_last_changed) {
            let lines =
                crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags(
                    &self.messages,
                    &self.theme,
                    markdown,
                    syntax,
                );
            let pre = crate::utils::scroll::ScrollCalculator::prewrap_lines(&lines, width);
            // Compute last message prewrapped length to allow fast tail updates
            let (last_start, last_len) = if let Some(last_msg) = self.messages.back() {
                let last_lines = if markdown {
                    crate::ui::markdown::render_message_markdown_opts(last_msg, &self.theme, syntax)
                        .lines
                } else {
                    crate::ui::markdown::build_plain_display_lines(
                        &VecDeque::from([last_msg.clone()]),
                        &self.theme,
                    )
                };
                let last_pre =
                    crate::utils::scroll::ScrollCalculator::prewrap_lines(&last_lines, width);
                let len = last_pre.len();
                let start = pre.len().saturating_sub(len);
                (start, len)
            } else {
                (0, 0)
            };
            self.prewrap_cache = Some(PrewrapCache {
                width,
                markdown_enabled: markdown,
                syntax_enabled: syntax,
                theme_sig,
                messages_len: msg_len,
                last_msg_hash: last_hash,
                last_start,
                last_len,
                lines: pre,
            });
        }

        // Safe unwrap since we just populated if missing
        &self.prewrap_cache.as_ref().unwrap().lines
    }

    pub fn invalidate_prewrap_cache(&mut self) {
        self.prewrap_cache = None;
    }

    // Used by Criterion benches in `benches/`.
    #[allow(dead_code)]
    pub fn new_bench(theme: Theme, markdown_enabled: bool, syntax_enabled: bool) -> Self {
        Self {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            edit_select_mode: false,
            selected_user_message_index: None,
            in_place_edit_index: None,
            current_response: String::new(),
            client: reqwest::Client::new(),
            model: "bench".into(),
            api_key: String::new(),
            base_url: String::new(),
            provider_name: "bench".into(),
            provider_display_name: "Bench".into(),
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
            stream_interrupted: false,
            logging: crate::utils::logging::LoggingState::new(None).unwrap(),
            stream_cancel_token: None,
            current_stream_id: 0,
            last_retry_time: std::time::Instant::now(),
            retrying_message_index: None,
            input_scroll_offset: 0,
            textarea: tui_textarea::TextArea::default(),
            theme,
            picker: None,
            picker_mode: None,
            block_select_mode: false,
            selected_block_index: None,
            theme_before_picker: None,
            theme_id_before_picker: None,
            markdown_enabled,
            syntax_enabled,
            prewrap_cache: None,
            status: None,
            status_set_at: None,
            file_prompt: None,
        }
    }

    pub fn build_display_lines(&self) -> Vec<Line<'static>> {
        // Default display lines without selection/highlight
        ScrollCalculator::build_display_lines_with_theme_and_flags(
            &self.messages,
            &self.theme,
            self.markdown_enabled,
            self.syntax_enabled,
        )
    }

    pub fn calculate_wrapped_line_count(&self, terminal_width: u16) -> u16 {
        let lines = self.build_display_lines();
        ScrollCalculator::calculate_wrapped_line_count(&lines, terminal_width)
    }

    pub fn calculate_max_scroll_offset(&self, available_height: u16, terminal_width: u16) -> u16 {
        let lines = self.build_display_lines();
        let total = ScrollCalculator::calculate_wrapped_line_count(&lines, terminal_width);
        if total > available_height {
            total.saturating_sub(available_height)
        } else {
            0
        }
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
        // Clear any ephemeral status when the user sends a message
        self.clear_status();

        let user_message = Message {
            role: "user".to_string(),
            content: content.clone(),
        };

        // Log the user message if logging is active
        if let Err(e) = self.logging.log_message(&format!("You: {content}")) {
            eprintln!("Failed to log message: {e}");
        }

        self.messages.push_back(user_message);

        // Start assistant message
        let assistant_message = Message {
            role: "assistant".to_string(),
            content: String::new(),
        };
        self.messages.push_back(assistant_message);
        self.current_response.clear();

        // Clear retry state since we're starting a new conversation
        self.retrying_message_index = None;

        // Prepare messages for API (excluding the empty assistant message we just added and system messages)
        let mut api_messages = Vec::new();
        for msg in self.messages.iter().take(self.messages.len() - 1) {
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
        self.status = Some(s.into());
        self.status_set_at = Some(Instant::now());
    }

    pub fn clear_status(&mut self) {
        self.status = None;
        self.status_set_at = None;
    }

    pub fn start_file_prompt_dump(&mut self, filename: String) {
        self.file_prompt = Some(FilePrompt {
            kind: FilePromptKind::Dump,
            content: None,
        });
        self.set_input_text(filename);
        self.input_mode = true;
        self.in_place_edit_index = None;
        self.edit_select_mode = false;
        self.block_select_mode = false;
    }

    pub fn start_file_prompt_save_block(&mut self, filename: String, content: String) {
        self.file_prompt = Some(FilePrompt {
            kind: FilePromptKind::SaveCodeBlock,
            content: Some(content),
        });
        self.set_input_text(filename);
        self.input_mode = true;
        self.in_place_edit_index = None;
        self.edit_select_mode = false;
        self.block_select_mode = false;
    }

    pub fn cancel_file_prompt(&mut self) {
        self.file_prompt = None;
        self.clear_input();
    }

    pub fn append_to_response(
        &mut self,
        content: &str,
        available_height: u16,
        terminal_width: u16,
    ) {
        self.current_response.push_str(content);

        // Update the message being retried, or the last message if not retrying
        if let Some(retry_index) = self.retrying_message_index {
            if let Some(msg) = self.messages.get_mut(retry_index) {
                if msg.role == "assistant" {
                    msg.content = self.current_response.clone();
                }
            }
        } else if let Some(last_msg) = self.messages.back_mut() {
            if last_msg.role == "assistant" {
                last_msg.content = self.current_response.clone();
            }
        }

        // Auto-scroll to bottom when new content arrives, but only if auto_scroll is enabled
        if self.auto_scroll {
            // Calculate the scroll offset needed to show the bottom using wrapped line count
            let total_wrapped_lines = self.calculate_wrapped_line_count(terminal_width);
            if total_wrapped_lines > available_height {
                self.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.scroll_offset = 0;
            }
        }
    }

    pub fn add_system_message(&mut self, content: String) {
        let system_message = Message {
            role: "system".to_string(),
            content,
        };
        self.messages.push_back(system_message);
    }

    pub fn update_scroll_position(&mut self, available_height: u16, terminal_width: u16) {
        // Auto-scroll to bottom when new content is added, but only if auto_scroll is enabled
        if self.auto_scroll {
            // Calculate the scroll offset needed to show the bottom using wrapped line count
            let total_wrapped_lines = self.calculate_wrapped_line_count(terminal_width);
            if total_wrapped_lines > available_height {
                self.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.scroll_offset = 0;
            }
        }
    }

    pub fn get_logging_status(&self) -> String {
        self.logging.get_status_string()
    }

    pub fn can_retry(&self) -> bool {
        // Can retry if there's at least one assistant message (even if currently streaming)
        self.messages
            .iter()
            .any(|msg| msg.role == "assistant" && !msg.content.is_empty())
    }

    pub fn cancel_current_stream(&mut self) {
        if let Some(token) = &self.stream_cancel_token {
            token.cancel();
        }
        self.stream_cancel_token = None;
        self.is_streaming = false;
        self.stream_interrupted = true;
    }

    pub fn start_new_stream(&mut self) -> (CancellationToken, u64) {
        // Cancel any existing stream first
        self.cancel_current_stream();

        // Increment stream ID to distinguish this stream from previous ones
        self.current_stream_id += 1;

        // Create new cancellation token
        let token = CancellationToken::new();
        self.stream_cancel_token = Some(token.clone());
        self.is_streaming = true;
        self.stream_interrupted = false;
        self.pulse_start = Instant::now();

        (token, self.current_stream_id)
    }

    pub fn calculate_scroll_to_message(
        &self,
        message_index: usize,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        ScrollCalculator::calculate_scroll_to_message_with_flags(
            &self.messages,
            &self.theme,
            self.markdown_enabled,
            self.syntax_enabled,
            message_index,
            terminal_width,
            available_height,
        )
    }

    pub fn finalize_response(&mut self) {
        // Log the complete assistant response if logging is active
        if !self.current_response.is_empty() {
            if let Err(e) = self.logging.log_message(&self.current_response) {
                eprintln!("Failed to log response: {e}");
            }
        }

        // Clear retry state since response is now complete
        self.retrying_message_index = None;
    }

    /// Scroll so that the given message index is visible.
    pub fn scroll_index_into_view(&mut self, index: usize, term_width: u16, term_height: u16) {
        let input_area_height = self.calculate_input_area_height(term_width);
        let available_height = self.calculate_available_height(term_height, input_area_height);
        self.scroll_offset = self.calculate_scroll_to_message(index, term_width, available_height);
    }

    /// Find the last user-authored message index
    pub fn last_user_message_index(&self) -> Option<usize> {
        self.messages
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
        self.messages
            .iter()
            .enumerate()
            .take(from_index)
            .rev()
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Find next user message index after `from_index` (exclusive)
    pub fn next_user_message_index(&self, from_index: usize) -> Option<usize> {
        self.messages
            .iter()
            .enumerate()
            .skip(from_index + 1)
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Find the first user-authored message index
    pub fn first_user_message_index(&self) -> Option<usize> {
        self.messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
    }

    /// Enter edit-select mode: lock input and select most recent user message
    pub fn enter_edit_select_mode(&mut self) {
        self.edit_select_mode = true;
        self.input_mode = false; // lock input area
        self.selected_user_message_index = self.last_user_message_index();
    }

    /// Exit edit-select mode
    pub fn exit_edit_select_mode(&mut self) {
        self.edit_select_mode = false;
        self.input_mode = true; // unlock input area
    }

    /// Begin in-place edit of a user message at `index`
    pub fn start_in_place_edit(&mut self, index: usize) {
        self.in_place_edit_index = Some(index);
        self.input_mode = true;
    }

    /// Cancel in-place edit (does not modify history)
    pub fn cancel_in_place_edit(&mut self) {
        self.in_place_edit_index = None;
    }

    /// Enter block select mode: lock input and set selected block index
    pub fn enter_block_select_mode(&mut self, index: usize) {
        self.block_select_mode = true;
        self.selected_block_index = Some(index);
        self.input_mode = false; // lock input area
    }

    /// Exit block select mode and unlock input
    pub fn exit_block_select_mode(&mut self) {
        self.block_select_mode = false;
        self.selected_block_index = None;
        self.input_mode = true; // unlock input area
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
        self.last_retry_time = Instant::now();

        // Check if we're already retrying a specific message
        if let Some(retry_index) = self.retrying_message_index {
            // We're already retrying a specific message - just clear its content
            if retry_index < self.messages.len() {
                if let Some(msg) = self.messages.get_mut(retry_index) {
                    if msg.role == "assistant" {
                        msg.content.clear();
                        self.current_response.clear();
                    }
                }
            }
        } else {
            // Not currently retrying - find the last assistant message to retry
            let mut target_index = None;

            // Find the last assistant message with content
            for (i, msg) in self.messages.iter().enumerate().rev() {
                if msg.role == "assistant" && !msg.content.is_empty() {
                    target_index = Some(i);
                    break;
                }
            }

            if let Some(index) = target_index {
                // Mark this message as being retried
                self.retrying_message_index = Some(index);

                // Clear the content of this specific message
                if let Some(msg) = self.messages.get_mut(index) {
                    msg.content.clear();
                    self.current_response.clear();
                }

                // Rewrite the log file to remove the last assistant response
                if let Err(e) = self
                    .logging
                    .rewrite_log_without_last_response(&self.messages)
                {
                    eprintln!("Failed to rewrite log file: {e}");
                }
            } else {
                return None;
            }
        }

        // Set scroll position to show the user message that corresponds to the retry
        if let Some(retry_index) = self.retrying_message_index {
            // Find the user message that precedes this assistant message
            if retry_index > 0 {
                let user_message_index = retry_index - 1;
                self.scroll_offset = self.calculate_scroll_to_message(
                    user_message_index,
                    terminal_width,
                    available_height,
                );
            } else {
                self.scroll_offset = 0;
            }
        }

        // Re-enable auto-scroll for the new response
        self.auto_scroll = true;

        // Prepare messages for API (excluding the message being retried and system messages)
        let mut api_messages = Vec::new();
        if let Some(retry_index) = self.retrying_message_index {
            for (i, msg) in self.messages.iter().enumerate() {
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

    pub async fn fetch_newest_model(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        // We need to create a new client here because we're in a different context
        let client = reqwest::Client::new();

        // Use the shared function to fetch models
        let models_response =
            fetch_models(&client, &self.base_url, &self.api_key, &self.provider_name).await?;

        if models_response.data.is_empty() {
            return Ok(None);
        }

        // Sort models using the shared function
        let mut models = models_response.data;
        sort_models(&mut models);

        // Return the ID of the first (newest) model
        Ok(models.first().map(|m| m.id.clone()))
    }

    /// Calculate how many lines the input text will wrap to using word wrapping
    pub fn calculate_input_wrapped_lines(&self, width: u16) -> usize {
        if self.get_input_text().is_empty() {
            return 1; // At least one line for the cursor
        }

        let config = WrapConfig::new(width as usize);
        TextWrapper::count_wrapped_lines(self.get_input_text(), &config)
    }

    /// Calculate the input area height based on content
    pub fn calculate_input_area_height(&self, width: u16) -> u16 {
        if self.get_input_text().is_empty() {
            return 1; // Default to 1 line when empty
        }

        // Account for borders and keep a one-column right margin
        // Wrap one character earlier to avoid cursor touching the border
        let available_width = width.saturating_sub(3);
        let wrapped_lines = self.calculate_input_wrapped_lines(available_width);

        // Start at 1 line, expand to 2 when we have content that wraps or newlines
        // Then expand up to maximum of 6 lines
        if wrapped_lines <= 1 && !self.get_input_text().contains('\n') {
            1 // Single line, no wrapping, no newlines
        } else {
            (wrapped_lines as u16).clamp(2, 6) // Expand to 2-6 lines
        }
    }

    /// Update input scroll offset to keep cursor visible
    pub fn update_input_scroll(&mut self, input_area_height: u16, width: u16) {
        // Account for borders and keep a one-column right margin
        // Wrap one character earlier to avoid cursor touching the border
        let available_width = width.saturating_sub(3);
        let total_input_lines = self.calculate_input_wrapped_lines(available_width) as u16;

        if total_input_lines <= input_area_height {
            // All content fits, no scrolling needed
            self.input_scroll_offset = 0;
        } else {
            // Calculate cursor line position accounting for text wrapping
            let cursor_line = self.calculate_cursor_line_position(available_width as usize);

            // Ensure cursor is visible within the input area
            if cursor_line < self.input_scroll_offset {
                // Cursor is above visible area, scroll up
                self.input_scroll_offset = cursor_line;
            } else if cursor_line >= self.input_scroll_offset + input_area_height {
                // Cursor is below visible area, scroll down
                self.input_scroll_offset = cursor_line.saturating_sub(input_area_height - 1);
            }

            // Ensure scroll offset doesn't exceed bounds
            let max_scroll = total_input_lines.saturating_sub(input_area_height);
            self.input_scroll_offset = self.input_scroll_offset.min(max_scroll);
        }
    }

    /// Recompute input layout after editing: height + scroll
    pub fn recompute_input_layout_after_edit(&mut self, terminal_width: u16) {
        let input_area_height = self.calculate_input_area_height(terminal_width);
        self.update_input_scroll(input_area_height, terminal_width);
    }

    /// Apply a mutation to the textarea, then sync the string state
    pub fn apply_textarea_edit<F>(&mut self, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        f(&mut self.textarea);
        self.sync_input_from_textarea();
    }

    /// Apply a mutation to the textarea, then sync and recompute layout
    pub fn apply_textarea_edit_and_recompute<F>(&mut self, terminal_width: u16, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        self.apply_textarea_edit(f);
        self.recompute_input_layout_after_edit(terminal_width);
    }

    /// Calculate available chat height given terminal height and input area height
    pub fn calculate_available_height(&self, term_height: u16, input_area_height: u16) -> u16 {
        term_height
            .saturating_sub(input_area_height + 2) // Dynamic input area + borders
            .saturating_sub(1) // 1 for title
    }

    /// Calculate which line the cursor is on, accounting for word wrapping
    fn calculate_cursor_line_position(&self, available_width: usize) -> u16 {
        let config = WrapConfig::new(available_width);
        TextWrapper::calculate_cursor_line(
            self.get_input_text(),
            self.input_cursor_position,
            &config,
        ) as u16
    }

    /// Clear input and reset cursor
    pub fn clear_input(&mut self) {
        self.set_input_text(String::new());
    }

    /// Get full input text (textarea is the source of truth; kept in sync with `input`)
    pub fn get_input_text(&self) -> &str {
        &self.input
    }

    /// Set input text into both the string and the textarea
    pub fn set_input_text(&mut self, text: String) {
        self.input = text;
        let lines: Vec<String> = if self.input.is_empty() {
            Vec::new()
        } else {
            self.input.split('\n').map(|s| s.to_string()).collect()
        };
        self.textarea = TextArea::from(lines);
        // Place both our linear cursor and the textarea cursor at the end of text
        self.input_cursor_position = self.input.chars().count();
        if !self.input.is_empty() {
            let last_row = self.textarea.lines().len().saturating_sub(1) as u16;
            let last_col = self
                .textarea
                .lines()
                .last()
                .map(|l| l.chars().count() as u16)
                .unwrap_or(0);
            self.textarea
                .move_cursor(tui_textarea::CursorMove::Jump(last_row, last_col));
        }
        self.configure_textarea_appearance();
    }

    /// Sync `input` from the textarea state. Cursor linear position is best-effort.
    pub fn sync_input_from_textarea(&mut self) {
        let lines = self.textarea.lines();
        self.input = lines.join("\n");
        // Compute linear cursor position from (row, col) in textarea
        let (row, col) = self.textarea.cursor();
        let mut pos = 0usize;
        for (i, line) in lines.iter().enumerate() {
            if i < row {
                pos += line.chars().count();
                // account for newline separator between lines
                pos += 1;
            } else if i == row {
                let line_len = line.chars().count();
                pos += col.min(line_len);
                break;
            }
        }
        // If cursor row is beyond current lines (shouldn't happen), clamp to end
        if row >= lines.len() {
            pos = self.input.chars().count();
        }
        self.input_cursor_position = pos;
    }

    fn configure_textarea_appearance(&mut self) {
        // Apply theme styles to textarea, including background for visibility
        let textarea_style = self
            .theme
            .input_text_style
            .patch(ratatui::style::Style::default().bg(self.theme.background_color));
        self.textarea.set_style(textarea_style);
        self.textarea
            .set_cursor_style(self.theme.input_cursor_style);
        self.textarea
            .set_cursor_line_style(self.theme.input_cursor_line_style);
    }

    /// Open a theme picker modal with built-in and custom themes
    pub fn open_theme_picker(&mut self) {
        self.picker_mode = Some(PickerMode::Theme);
        // Save current theme and configured id for cancel
        self.theme_before_picker = Some(self.theme.clone());
        if let Ok(cfg) = Config::load() {
            self.theme_id_before_picker = cfg.theme.clone();
        }
        let mut items: Vec<PickerItem> = Vec::new();
        // Built-ins
        for t in crate::ui::builtin_themes::load_builtin_themes() {
            items.push(PickerItem {
                id: t.id.clone(),
                label: t.display_name.clone(),
            });
        }
        // Custom
        if let Ok(cfg) = Config::load() {
            for ct in cfg.list_custom_themes() {
                items.push(PickerItem {
                    id: ct.id.clone(),
                    label: format!("{} (custom)", ct.display_name),
                });
            }
        }
        // Select current theme if present
        let mut selected = 0usize;
        if let Ok(cfg) = Config::load() {
            if let Some(ref id) = cfg.theme {
                if let Some((idx, _)) = items
                    .iter()
                    .enumerate()
                    .find(|(_, it)| it.id.eq_ignore_ascii_case(id))
                {
                    selected = idx;
                }
            }
        }
        self.picker = Some(PickerState::new("Pick Theme", items, selected));
    }

    /// Apply theme by id (custom or built-in) and persist in config
    pub fn apply_theme_by_id(&mut self, id: &str) -> Result<(), String> {
        let cfg = Config::load().map_err(|e| e.to_string())?;
        let theme = if let Some(ct) = cfg.get_custom_theme(id) {
            Theme::from_spec(&theme_spec_from_custom(ct))
        } else if let Some(spec) = find_builtin_theme(id) {
            Theme::from_spec(&spec)
        } else {
            return Err(format!("Unknown theme: {}", id));
        };

        self.theme = theme;
        self.configure_textarea_appearance();

        let mut cfg = Config::load().map_err(|e| e.to_string())?;
        cfg.theme = Some(id.to_string());
        cfg.save().map_err(|e| e.to_string())?;
        // Clear preview snapshot once committed
        self.theme_before_picker = None;
        self.theme_id_before_picker = None;
        Ok(())
    }

    /// Apply theme temporarily for preview (does not persist config)
    pub fn preview_theme_by_id(&mut self, id: &str) {
        // Try custom then built-in then no-op
        if let Ok(cfg) = Config::load() {
            if let Some(ct) = cfg.get_custom_theme(id) {
                self.theme = Theme::from_spec(&theme_spec_from_custom(ct));
                self.configure_textarea_appearance();
                return;
            }
        }
        if let Some(spec) = find_builtin_theme(id) {
            self.theme = Theme::from_spec(&spec);
            self.configure_textarea_appearance();
        }
    }

    /// Revert theme to the one before opening picker (on cancel)
    pub fn revert_theme_preview(&mut self) {
        if let Some(prev) = self.theme_before_picker.clone() {
            self.theme = prev;
            self.configure_textarea_appearance();
        }
        // Do not modify config
        self.theme_before_picker = None;
        self.theme_id_before_picker = None;
    }
}

// Simple cache holder for prewrapped lines
pub(crate) struct PrewrapCache {
    width: u16,
    markdown_enabled: bool,
    syntax_enabled: bool,
    theme_sig: u64,
    messages_len: usize,
    last_msg_hash: u64,
    last_start: usize,
    last_len: usize,
    lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone)]
pub enum FilePromptKind {
    Dump,
    SaveCodeBlock,
}

#[derive(Debug, Clone)]
pub struct FilePrompt {
    pub kind: FilePromptKind,
    pub content: Option<String>,
}

fn hash_last_message(messages: &VecDeque<Message>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    if let Some(m) = messages.back() {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    h.finish()
}

fn compute_theme_signature(theme: &crate::ui::theme::Theme) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    // Background and codeblock bg are strong signals
    format!("{:?}", theme.background_color).hash(&mut h);
    format!("{:?}", theme.md_codeblock_bg_color()).hash(&mut h);
    // Include a couple of primary styles (fg colors)
    format!("{:?}", theme.user_text_style).hash(&mut h);
    format!("{:?}", theme.assistant_text_style).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use tui_textarea::{CursorMove, Input, Key};

    #[test]
    fn prewrap_cache_reuse_no_changes() {
        let mut app = create_test_app();
        for i in 0..50 {
            app.messages.push_back(Message {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                content: "lorem ipsum dolor sit amet consectetur adipiscing elit".into(),
            });
        }
        let w = 100u16;
        let p1 = app.get_prewrapped_lines_cached(w);
        assert!(!p1.is_empty());
        let ptr1 = p1.as_ptr();
        let p2 = app.get_prewrapped_lines_cached(w);
        let ptr2 = p2.as_ptr();
        assert_eq!(ptr1, ptr2, "cache should be reused when nothing changed");
    }

    #[test]
    fn prewrap_cache_invalidates_on_width_change() {
        let mut app = create_test_app();
        app.messages.push_back(Message {
            role: "user".into(),
            content: "hello world".into(),
        });
        let p1 = app.get_prewrapped_lines_cached(80);
        let ptr1 = p1.as_ptr();
        let p2 = app.get_prewrapped_lines_cached(120);
        let ptr2 = p2.as_ptr();
        assert_ne!(ptr1, ptr2, "cache should invalidate on width change");
    }

    #[test]
    fn test_system_messages_excluded_from_api() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add a user message
        app.messages.push_back(create_test_message("user", "Hello"));

        // Add a system message (like from /help command)
        app.add_system_message(
            "This is a system message that should not be sent to API".to_string(),
        );

        // Add an assistant message
        app.messages
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
        app.messages.push_back(Message {
            role: "user".to_string(),
            content: "Test question".to_string(),
        });

        app.add_system_message("System message between user and assistant".to_string());

        app.messages.push_back(Message {
            role: "assistant".to_string(),
            content: "Test response".to_string(),
        });

        // Set up retry state
        app.retrying_message_index = Some(2); // Retry the assistant message at index 2

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
    fn test_sync_cursor_mapping_single_and_multi_line() {
        let mut app = create_test_app();

        // Single line: move to end
        app.set_input_text("hello world".to_string());
        app.textarea.move_cursor(CursorMove::End);
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "hello world");
        assert_eq!(app.input_cursor_position, 11);

        // Multi-line: jump to (row=1, col=3) => after "wor" on second line
        app.set_input_text("hello\nworld".to_string());
        app.textarea.move_cursor(CursorMove::Jump(1, 3));
        app.sync_input_from_textarea();
        // 5 (hello) + 1 (\n) + 3 = 9
        assert_eq!(app.input_cursor_position, 9);
    }

    #[test]
    fn test_backspace_at_start_noop() {
        let mut app = create_test_app();
        app.set_input_text("abc".to_string());
        // Move to head of line
        app.textarea.move_cursor(CursorMove::Head);
        // Simulate backspace (always single-char via input_without_shortcuts)
        app.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: false,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "abc");
        assert_eq!(app.input_cursor_position, 0);
    }

    #[test]
    fn test_backspace_at_line_start_joins_lines() {
        let mut app = create_test_app();
        app.set_input_text("hello\nworld".to_string());
        // Move to start of second line
        app.textarea.move_cursor(CursorMove::Jump(1, 0));
        // Backspace should join lines; use input_without_shortcuts to ensure single-char delete
        app.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: false,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "helloworld");
        // Cursor should be at end of former first line (index 5)
        assert_eq!(app.input_cursor_position, 5);
    }

    #[test]
    fn test_backspace_with_alt_modifier_deletes_single_char() {
        let mut app = create_test_app();
        app.set_input_text("hello world".to_string());
        app.textarea.move_cursor(CursorMove::End);
        // Simulate Alt+Backspace; with input_without_shortcuts it should still delete one char
        app.textarea.input_without_shortcuts(Input {
            key: Key::Backspace,
            ctrl: false,
            alt: true,
            shift: false,
        });
        app.sync_input_from_textarea();
        assert_eq!(app.get_input_text(), "hello worl");
        assert_eq!(app.input_cursor_position, "hello worl".chars().count());
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
        app.input_cursor_position = text.chars().count().saturating_sub(1);
        app.update_input_scroll(input_area_height, width);
        // With cursor near end, scroll offset should be > 0 to bring cursor into view
        assert!(app.input_scroll_offset > 0);
    }

    #[test]
    fn test_shift_like_up_down_moves_one_line_on_many_newlines() {
        let mut app = create_test_app();
        // Build text with many blank lines
        let text = "top\n\n\n\n\n\n\n\n\n\nbottom";
        app.set_input_text(text.to_string());
        // Jump to bottom line, col=3 (after 'bot')
        let bottom_row_usize = app.textarea.lines().len().saturating_sub(1);
        let bottom_row = bottom_row_usize as u16;
        app.textarea.move_cursor(CursorMove::Jump(bottom_row, 3));
        app.sync_input_from_textarea();
        let (row_before, col_before) = app.textarea.cursor();
        assert_eq!(row_before, bottom_row as usize);
        assert!(col_before <= app.textarea.lines()[bottom_row_usize].chars().count());

        // Move up exactly one line
        app.textarea.move_cursor(CursorMove::Up);
        app.sync_input_from_textarea();
        let (row_after_up, col_after_up) = app.textarea.cursor();
        assert_eq!(row_after_up, bottom_row_usize.saturating_sub(1));
        // Column should clamp reasonably; we just assert it's within line bounds
        assert!(col_after_up <= app.textarea.lines()[8].chars().count());

        // Move down exactly one line
        app.textarea.move_cursor(CursorMove::Down);
        app.sync_input_from_textarea();
        let (row_after_down, _col_after_down) = app.textarea.cursor();
        assert_eq!(row_after_down, bottom_row_usize);
    }

    #[test]
    fn test_shift_like_left_right_moves_one_char() {
        let mut app = create_test_app();
        app.set_input_text("hello".to_string());
        // Move to end, then back by one, then forward by one
        app.textarea.move_cursor(CursorMove::End);
        app.sync_input_from_textarea();
        let end_pos = app.input_cursor_position;
        app.textarea.move_cursor(CursorMove::Back);
        app.sync_input_from_textarea();
        let back_pos = app.input_cursor_position;
        assert_eq!(back_pos, end_pos.saturating_sub(1));
        app.textarea.move_cursor(CursorMove::Forward);
        app.sync_input_from_textarea();
        let forward_pos = app.input_cursor_position;
        assert_eq!(forward_pos, end_pos);
    }

    #[test]
    fn test_cursor_mapping_blankline_insert_no_desync() {
        let mut app = create_test_app();
        let text = "asdf\n\nasdf\n\nasdf";
        app.set_input_text(text.to_string());
        // Jump to blank line 2 (0-based row 3), column 0
        app.textarea.move_cursor(CursorMove::Jump(3, 0));
        app.sync_input_from_textarea();
        // Insert a character on the blank line
        app.textarea.insert_str("x");
        app.sync_input_from_textarea();

        // Compute wrapped position using same wrapper logic (no wrapping with wide width)
        let config = WrapConfig::new(120);
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
            app.get_input_text(),
            app.input_cursor_position,
            &config,
        );
        // Compare to textarea's cursor row/col
        let (row, c) = app.textarea.cursor();
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
        app.input_cursor_position = text.chars().count().saturating_sub(1);
        // Very small terminal width to force heavy wrapping; method accounts for borders and margin
        let width: u16 = 6;
        app.recompute_input_layout_after_edit(width);
        // With cursor near end on a heavily wrapped input, expect some scroll
        assert!(app.input_scroll_offset > 0);
        // Changing cursor position to start should reduce or reset scroll
        app.input_cursor_position = 0;
        app.recompute_input_layout_after_edit(width);
        assert_eq!(app.input_scroll_offset, 0);
    }

    #[test]
    fn test_last_and_first_user_message_index() {
        let mut app = create_test_app();
        // No messages
        assert_eq!(app.last_user_message_index(), None);
        assert_eq!(app.first_user_message_index(), None);

        // Add messages: user, assistant, user
        app.messages.push_back(create_test_message("user", "u1"));
        app.messages
            .push_back(create_test_message("assistant", "a1"));
        app.messages.push_back(create_test_message("user", "u2"));

        assert_eq!(app.first_user_message_index(), Some(0));
        assert_eq!(app.last_user_message_index(), Some(2));
    }

    #[test]
    fn test_prev_next_user_message_index_navigation() {
        let mut app = create_test_app();
        // indices: 0 user, 1 assistant, 2 system, 3 user
        app.messages.push_back(create_test_message("user", "u1"));
        app.messages
            .push_back(create_test_message("assistant", "a1"));
        app.messages.push_back(create_test_message("system", "s1"));
        app.messages.push_back(create_test_message("user", "u2"));

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
        assert_eq!(app.input_cursor_position, text.chars().count());
        // Textarea cursor at end (last row/col)
        let (row, col) = app.textarea.cursor();
        let lines = app.textarea.lines();
        assert_eq!(row, lines.len() - 1);
        assert_eq!(col, lines.last().unwrap().chars().count());
    }
}
