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
        if let Some(last_msg) = self.messages.back_mut() {
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
        // Can retry if there's at least one assistant message and we're not currently streaming
        !self.is_streaming &&
        self.messages.iter().any(|msg| msg.role == "assistant" && !msg.content.is_empty())
    }

    pub fn prepare_retry(&mut self, available_height: u16) -> Option<Vec<crate::api::ChatMessage>> {
        if !self.can_retry() {
            return None;
        }

        // Find the last assistant message and remove it
        let mut found_assistant = false;
        let mut messages_to_keep = Vec::new();

        for msg in self.messages.iter().rev() {
            if msg.role == "assistant" && !msg.content.is_empty() && !found_assistant {
                found_assistant = true;
                // Skip this message (don't add to messages_to_keep)
                continue;
            }
            messages_to_keep.push(msg.clone());
        }

        if !found_assistant {
            return None;
        }

        // Reverse back to original order
        messages_to_keep.reverse();
        self.messages = messages_to_keep.into();

        // Clear current response
        self.current_response.clear();

        // Calculate scroll position as if the response was deleted
        let total_lines = self.build_display_lines().len() as u16;
        if total_lines > available_height {
            self.scroll_offset = total_lines.saturating_sub(available_height);
        } else {
            self.scroll_offset = 0;
        }

        // Re-enable auto-scroll for the new response
        self.auto_scroll = true;

        // Rewrite the log file to remove the last assistant response
        if let Err(e) = self.logging.rewrite_log_without_last_response(&self.messages) {
            eprintln!("Failed to rewrite log file: {}", e);
        }

        // Add a new empty assistant message for the retry
        let assistant_message = Message {
            role: "assistant".to_string(),
            content: String::new(),
        };
        self.messages.push_back(assistant_message);

        // Prepare messages for API (excluding the empty assistant message we just added)
        let mut api_messages = Vec::new();
        for msg in self.messages.iter().take(self.messages.len() - 1) {
            api_messages.push(crate::api::ChatMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        Some(api_messages)
    }
}
