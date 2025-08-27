use crate::api::models::{fetch_models, sort_models};
use crate::auth::AuthManager;
use crate::core::config::Config;
use crate::core::constants::INDICATOR_SPACE;
use crate::core::message::Message;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::utils::logging::LoggingState;
use crate::utils::scroll::ScrollCalculator;
use crate::utils::url::construct_api_url;
use chrono::Utc;
use ratatui::text::Line;
use reqwest::Client;
use std::{collections::VecDeque, time::Instant};
use tokio_util::sync::CancellationToken;

pub struct App {
    pub messages: VecDeque<Message>,
    pub input: String,
    pub input_cursor_position: usize,
    pub input_mode: bool,
    pub current_response: String,
    pub client: Client,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub provider_name: String,
    pub provider_display_name: String,
    pub scroll_offset: u16,
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
}

impl App {
    pub async fn new_with_auth(
        model: String,
        log_file: Option<String>,
        provider: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let auth_manager = AuthManager::new();
        let config = Config::load()?;

        let (api_key, base_url, provider_name, provider_display_name) = if let Some(
            ref provider_name,
        ) = provider
        {
            if provider_name.is_empty() {
                // User specified -p without a value, use config default if available
                if let Some(ref default_provider) = config.default_provider {
                    if let Some((base_url, api_key)) =
                        auth_manager.get_auth_for_provider(default_provider)?
                    {
                        let display_name = auth_manager
                            .find_provider_by_name(default_provider)
                            .map(|p| p.display_name.clone())
                            .unwrap_or_else(|| default_provider.clone());
                        (
                            api_key,
                            base_url,
                            default_provider.to_lowercase(),
                            display_name,
                        )
                    } else {
                        return Err(format!("No authentication found for default provider '{default_provider}'. Run 'chabeau auth' to set up authentication.").into());
                    }
                } else {
                    // Try to find any available authentication
                    if let Some((provider, api_key)) = auth_manager.find_first_available_auth() {
                        (
                            api_key,
                            provider.base_url,
                            provider.name.to_lowercase(),
                            provider.display_name,
                        )
                    } else {
                        // Fall back to environment variables
                        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                            "❌ No authentication configured and OPENAI_API_KEY environment variable not set

Please either:
1. Run 'chabeau auth' to set up authentication, or
2. Set environment variables:
   export OPENAI_API_KEY=\"your-api-key-here\"
   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
                        })?;

                        let base_url = std::env::var("OPENAI_BASE_URL")
                            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

                        (
                            api_key,
                            base_url,
                            "openai".to_string(),
                            "OpenAI".to_string(),
                        )
                    }
                }
            } else {
                // User specified a provider - normalize to lowercase for consistent config lookup
                let normalized_provider_name = provider_name.to_lowercase();
                if let Some((base_url, api_key)) =
                    auth_manager.get_auth_for_provider(provider_name)?
                {
                    let display_name = auth_manager
                        .find_provider_by_name(provider_name)
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| provider_name.clone());
                    (api_key, base_url, normalized_provider_name, display_name)
                } else {
                    return Err(format!("No authentication found for provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into());
                }
            }
        } else if let Some(ref provider_name) = config.default_provider {
            // Config specifies a default provider
            if let Some((base_url, api_key)) = auth_manager.get_auth_for_provider(provider_name)? {
                let display_name = auth_manager
                    .find_provider_by_name(provider_name)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| provider_name.clone());
                (
                    api_key,
                    base_url,
                    provider_name.to_lowercase(),
                    display_name,
                )
            } else {
                return Err(format!("No authentication found for default provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into());
            }
        } else {
            // Try to find any available authentication
            if let Some((provider, api_key)) = auth_manager.find_first_available_auth() {
                (
                    api_key,
                    provider.base_url,
                    provider.name.to_lowercase(),
                    provider.display_name,
                )
            } else {
                // Fall back to environment variables
                let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    "❌ No authentication configured and OPENAI_API_KEY environment variable not set

Please either:
1. Run 'chabeau auth' to set up authentication, or
2. Set environment variables:
   export OPENAI_API_KEY=\"your-api-key-here\"
   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
                })?;

                let base_url = std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

                (
                    api_key,
                    base_url,
                    "openai".to_string(),
                    "OpenAI".to_string(),
                )
            }
        };

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
                current_response: String::new(),
                client: temp_client.clone(),
                model: model.clone(),
                api_key: api_key.clone(),
                base_url: base_url.clone(),
                provider_name: provider_name.to_string(),
                provider_display_name: provider_display_name.clone(),
                scroll_offset: 0,
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

        Ok(App {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            input_mode: true,
            current_response: String::new(),
            client: Client::new(),
            model: final_model,
            api_key,
            base_url,
            provider_name: provider_name.to_string(),
            provider_display_name,
            scroll_offset: 0,
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
        })
    }

    pub fn build_display_lines(&self) -> Vec<Line<'_>> {
        ScrollCalculator::build_display_lines(&self.messages)
    }

    pub fn calculate_wrapped_line_count(&self, terminal_width: u16) -> u16 {
        let lines = self.build_display_lines();
        ScrollCalculator::calculate_wrapped_line_count(&lines, terminal_width)
    }

    pub fn calculate_max_scroll_offset(&self, available_height: u16, terminal_width: u16) -> u16 {
        ScrollCalculator::calculate_max_scroll_offset(
            &self.messages,
            terminal_width,
            available_height,
        )
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
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
        ScrollCalculator::calculate_scroll_to_message(
            &self.messages,
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
        if self.input.is_empty() {
            return 1; // At least one line for the cursor
        }

        let config = WrapConfig::new(width as usize);
        TextWrapper::count_wrapped_lines(&self.input, &config)
    }

    /// Calculate the input area height based on content
    pub fn calculate_input_area_height(&self, width: u16) -> u16 {
        if self.input.is_empty() {
            return 1; // Default to 1 line when empty
        }

        let available_width = width.saturating_sub(2 + INDICATOR_SPACE); // Account for borders + indicator space
        let wrapped_lines = self.calculate_input_wrapped_lines(available_width);

        // Start at 1 line, expand to 2 when we have content that wraps or newlines
        // Then expand up to maximum of 6 lines
        if wrapped_lines <= 1 && !self.input.contains('\n') {
            1 // Single line, no wrapping, no newlines
        } else {
            (wrapped_lines as u16).clamp(2, 6) // Expand to 2-6 lines
        }
    }

    /// Update input scroll offset to keep cursor visible
    pub fn update_input_scroll(&mut self, input_area_height: u16, width: u16) {
        let available_width = width.saturating_sub(2 + INDICATOR_SPACE); // Account for borders + indicator space
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

    /// Calculate which line the cursor is on, accounting for word wrapping
    fn calculate_cursor_line_position(&self, available_width: usize) -> u16 {
        let config = WrapConfig::new(available_width);
        TextWrapper::calculate_cursor_line(&self.input, self.input_cursor_position, &config) as u16
    }

    // Input cursor movement methods

    /// Move cursor to the beginning of the input (Ctrl+A)
    pub fn move_cursor_to_beginning(&mut self) {
        self.input_cursor_position = 0;
    }

    /// Move cursor to the end of the input (Ctrl+E)
    pub fn move_cursor_to_end(&mut self) {
        self.input_cursor_position = self.input.chars().count();
    }

    /// Move cursor one position to the left (Left Arrow)
    pub fn move_cursor_left(&mut self) {
        if self.input_cursor_position > 0 {
            self.input_cursor_position -= 1;
        }
    }

    /// Move cursor one position to the right (Right Arrow)
    pub fn move_cursor_right(&mut self) {
        let max_position = self.input.chars().count();
        if self.input_cursor_position < max_position {
            self.input_cursor_position += 1;
        }
    }

    /// Move cursor up one line in multi-line input (Alt+Up)
    pub fn move_cursor_up_line(&mut self, available_width: usize) {
        let config = WrapConfig::new(available_width);
        let (current_line, current_col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
            &self.input,
            self.input_cursor_position,
            &config,
        );

        if current_line > 0 {
            // Move to the previous line at the same column position (or end of line if shorter)
            let target_line = current_line - 1;
            let target_position = self.find_position_at_line_col(target_line, current_col, &config);
            self.input_cursor_position = target_position;
        }
    }

    /// Move cursor down one line in multi-line input (Alt+Down)
    pub fn move_cursor_down_line(&mut self, available_width: usize) {
        let config = WrapConfig::new(available_width);
        let (current_line, current_col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
            &self.input,
            self.input_cursor_position,
            &config,
        );

        let total_lines = TextWrapper::count_wrapped_lines(&self.input, &config);
        if current_line < total_lines - 1 {
            // Move to the next line at the same column position (or end of line if shorter)
            let target_line = current_line + 1;
            let target_position = self.find_position_at_line_col(target_line, current_col, &config);
            self.input_cursor_position = target_position;
        }
    }

    /// Helper function to find cursor position at a specific line and column in wrapped text
    fn find_position_at_line_col(
        &self,
        target_line: usize,
        target_col: usize,
        config: &WrapConfig,
    ) -> usize {
        let wrapped_text = TextWrapper::wrap_text(&self.input, config);
        let lines: Vec<&str> = wrapped_text.split('\n').collect();

        if target_line >= lines.len() {
            return self.input.chars().count(); // End of input
        }

        // Count characters up to the target line
        let mut char_count = 0;
        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx == target_line {
                // We're at the target line, add the column position (clamped to line length)
                let line_len = line.chars().count();
                char_count += target_col.min(line_len);
                break;
            } else {
                // Add all characters from this line plus the newline (if it was an original newline)
                char_count += line.chars().count();

                // Check if this line break corresponds to an original newline in the input
                // by comparing with the original text structure
                if line_idx < lines.len() - 1 {
                    // Find the corresponding position in original text
                    let original_chars: Vec<char> = self.input.chars().collect();
                    if char_count < original_chars.len() && original_chars[char_count] == '\n' {
                        char_count += 1; // This was an original newline
                    }
                    // If it wasn't an original newline, it was a wrap point, so don't add 1
                }
            }
        }

        char_count.min(self.input.chars().count())
    }

    // Input text manipulation methods

    /// Insert character at cursor position
    pub fn insert_char_at_cursor(&mut self, c: char) {
        let char_indices: Vec<_> = self.input.char_indices().collect();

        if self.input_cursor_position >= char_indices.len() {
            // Cursor is at the end, just append
            self.input.push(c);
        } else {
            // Insert at the cursor position
            let byte_index = char_indices[self.input_cursor_position].0;
            self.input.insert(byte_index, c);
        }

        self.input_cursor_position += 1;
    }

    /// Insert string at cursor position
    pub fn insert_str_at_cursor(&mut self, s: &str) {
        let char_indices: Vec<_> = self.input.char_indices().collect();

        if self.input_cursor_position >= char_indices.len() {
            // Cursor is at the end, just append
            self.input.push_str(s);
        } else {
            // Insert at the cursor position
            let byte_index = char_indices[self.input_cursor_position].0;
            self.input.insert_str(byte_index, s);
        }

        self.input_cursor_position += s.chars().count();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before_cursor(&mut self) -> bool {
        if self.input_cursor_position == 0 {
            return false; // Nothing to delete
        }

        let char_indices: Vec<_> = self.input.char_indices().collect();

        if self.input_cursor_position <= char_indices.len() {
            let char_to_remove_index = self.input_cursor_position - 1;

            if char_to_remove_index < char_indices.len() {
                let byte_start = char_indices[char_to_remove_index].0;
                let byte_end = if char_to_remove_index + 1 < char_indices.len() {
                    char_indices[char_to_remove_index + 1].0
                } else {
                    self.input.len()
                };

                self.input.drain(byte_start..byte_end);
                self.input_cursor_position -= 1;
                return true;
            }
        }

        false
    }

    /// Clear input and reset cursor
    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor_position = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{create_test_app, create_test_message};

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
}
