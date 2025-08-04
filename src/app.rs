use crate::auth::AuthManager;
use crate::logging::LoggingState;
use crate::message::Message;
use crate::scroll::ScrollCalculator;
use ratatui::text::Line;
use reqwest::Client;
use std::{collections::VecDeque, time::Instant};
use tokio_util::sync::CancellationToken;

pub struct App {
    pub messages: VecDeque<Message>,
    pub input: String,
    pub input_mode: bool,
    pub current_response: String,
    pub client: Client,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub provider_name: String,
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
}

impl App {
    pub fn new_with_auth(
        model: String,
        log_file: Option<String>,
        provider: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let auth_manager = AuthManager::new();

        let (api_key, base_url, provider_name) = if let Some(provider_name) = provider {
            // User specified a provider
            if let Some((base_url, api_key)) = auth_manager.get_auth_for_provider(&provider_name)? {
                (api_key, base_url, provider_name)
            } else {
                return Err(format!("No authentication found for provider '{}'. Run 'chabeau auth' to set up authentication.", provider_name).into());
            }
        } else {
            // Try to find any available authentication
            if let Some((provider, api_key)) = auth_manager.find_first_available_auth() {
                (api_key, provider.base_url, provider.display_name)
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

                (api_key, base_url, "Environment Variables".to_string())
            }
        };

        // Print configuration info
        eprintln!("🚀 Starting Chabeau - Terminal Chat Interface");
        eprintln!("🔐 Provider: {}", provider_name);
        eprintln!("📡 Using model: {}", model);
        eprintln!("🌐 API endpoint: {}", base_url);
        if let Some(ref log_path) = log_file {
            eprintln!("📝 Logging to: {}", log_path);
        }
        eprintln!("💡 Press Ctrl+C to quit, Enter to send messages");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let logging = LoggingState::new(log_file)?;

        Ok(App {
            messages: VecDeque::new(),
            input: String::new(),
            input_mode: true,
            current_response: String::new(),
            client: Client::new(),
            model,
            api_key,
            base_url,
            provider_name,
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
        })
    }

    pub fn build_display_lines(&self) -> Vec<Line> {
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
        if let Err(e) = self.logging.log_message(&format!("You: {}", content)) {
            eprintln!("Failed to log message: {}", e);
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

        // Prepare messages for API (excluding the empty assistant message we just added)
        let mut api_messages = Vec::new();
        for msg in self.messages.iter().take(self.messages.len() - 1) {
            api_messages.push(crate::api::ChatMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
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

    pub fn finalize_response(&mut self) {
        // Log the complete assistant response if logging is active
        if !self.current_response.is_empty() {
            if let Err(e) = self.logging.log_message(&self.current_response) {
                eprintln!("Failed to log response: {}", e);
            }
        }

        // Clear retry state since response is now complete
        self.retrying_message_index = None;
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
                    eprintln!("Failed to rewrite log file: {}", e);
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

        // Prepare messages for API (excluding the message being retried)
        let mut api_messages = Vec::new();
        if let Some(retry_index) = self.retrying_message_index {
            for (i, msg) in self.messages.iter().enumerate() {
                if i < retry_index {
                    api_messages.push(crate::api::ChatMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
            }
        }

        Some(api_messages)
    }
}
