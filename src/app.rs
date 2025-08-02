use crate::logging::LoggingState;
use crate::message::Message;
use reqwest::Client;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::{
    collections::VecDeque,
    time::Instant,
};
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
    pub fn new(model: String, log_file: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            "❌ Error: OPENAI_API_KEY environment variable not set

Please set your OpenAI API key:
export OPENAI_API_KEY=\"your-api-key-here\"

Optionally, you can also set a custom base URL:
export OPENAI_BASE_URL=\"https://api.openai.com/v1\""
        })?;

        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

        // Print configuration info
        eprintln!("🚀 Starting Chabeau - Terminal Chat Interface");
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
        let mut lines = Vec::new();

        for msg in &self.messages {
            if msg.role == "user" {
                // User messages: cyan with "You:" prefix and indentation
                lines.push(Line::from(vec![
                    Span::styled("You: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&msg.content, Style::default().fg(Color::Cyan)),
                ]));
                lines.push(Line::from(""));  // Empty line for spacing
            } else if msg.role == "system" {
                // System messages: gray/dim color
                lines.push(Line::from(Span::styled(&msg.content, Style::default().fg(Color::DarkGray))));
                lines.push(Line::from(""));  // Empty line for spacing
            } else if !msg.content.is_empty() {
                // Assistant messages: no prefix, just content in white/default color
                // Split content into lines for proper wrapping
                for content_line in msg.content.lines() {
                    if content_line.trim().is_empty() {
                        lines.push(Line::from(""));
                    } else {
                        lines.push(Line::from(Span::styled(content_line, Style::default().fg(Color::White))));
                    }
                }
                lines.push(Line::from(""));  // Empty line for spacing
            }
        }

        lines
    }

    pub fn calculate_max_scroll_offset(&self, available_height: u16) -> u16 {
        let total_lines = self.build_display_lines().len() as u16;
        if total_lines > available_height {
            total_lines.saturating_sub(available_height)
        } else {
            0
        }
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

    pub fn append_to_response(&mut self, content: &str, available_height: u16) {
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
            // Calculate the scroll offset needed to show the bottom
            let total_lines = self.build_display_lines().len() as u16;
            if total_lines > available_height {
                self.scroll_offset = total_lines.saturating_sub(available_height);
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

    pub fn get_logging_status(&self) -> String {
        self.logging.get_status_string()
    }

    pub fn can_retry(&self) -> bool {
        // Can retry if there's at least one assistant message (even if currently streaming)
        self.messages.iter().any(|msg| msg.role == "assistant" && !msg.content.is_empty())
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

    pub fn prepare_retry(&mut self, available_height: u16) -> Option<Vec<crate::api::ChatMessage>> {
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
                if let Err(e) = self.logging.rewrite_log_without_last_response(&self.messages) {
                    eprintln!("Failed to rewrite log file: {}", e);
                }
            } else {
                return None;
            }
        }

        // Calculate scroll position
        let total_lines = self.build_display_lines().len() as u16;
        if total_lines > available_height {
            self.scroll_offset = total_lines.saturating_sub(available_height);
        } else {
            self.scroll_offset = 0;
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
