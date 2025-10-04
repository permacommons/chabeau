use super::{session::SessionContext, ui_state::UiState};
use crate::core::message::Message;
use crate::utils::scroll::ScrollCalculator;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

pub struct ConversationController<'a> {
    session: &'a mut SessionContext,
    ui: &'a mut UiState,
}

impl<'a> ConversationController<'a> {
    pub fn new(session: &'a mut SessionContext, ui: &'a mut UiState) -> Self {
        Self { session, ui }
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
        self.clear_status();

        let user_message = Message {
            role: "user".to_string(),
            content: content.clone(),
        };

        if let Err(e) = self.session.logging.log_message(&format!("You: {content}")) {
            eprintln!("Failed to log message: {e}");
        }

        self.ui.messages.push_back(user_message);

        let assistant_message = Message {
            role: "assistant".to_string(),
            content: String::new(),
        };
        self.ui.messages.push_back(assistant_message);
        self.ui.current_response.clear();

        self.session.retrying_message_index = None;

        let mut api_messages = Vec::new();
        for msg in self.ui.messages.iter().take(self.ui.messages.len() - 1) {
            if msg.role == "user" || msg.role == "assistant" {
                api_messages.push(crate::api::ChatMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }
        api_messages
    }

    pub fn add_system_message(&mut self, content: String) {
        let system_message = Message {
            role: "system".to_string(),
            content,
        };
        self.ui.messages.push_back(system_message);
    }

    pub fn set_status<S: Into<String>>(&mut self, s: S) {
        self.ui.status = Some(s.into());
        self.ui.status_set_at = Some(Instant::now());
    }

    pub fn clear_status(&mut self) {
        self.ui.status = None;
        self.ui.status_set_at = None;
    }

    pub fn append_to_response(
        &mut self,
        content: &str,
        available_height: u16,
        terminal_width: u16,
    ) {
        self.ui.current_response.push_str(content);

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
            let lines = self.ui.get_prewrapped_lines_cached(terminal_width);
            lines.len() as u16
        };

        if self.ui.auto_scroll {
            if total_wrapped_lines > available_height {
                self.ui.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.ui.scroll_offset = 0;
            }
        }
    }

    pub fn update_scroll_position(&mut self, available_height: u16, terminal_width: u16) {
        if self.ui.auto_scroll {
            let total_wrapped_lines = self.ui.calculate_wrapped_line_count(terminal_width);
            if total_wrapped_lines > available_height {
                self.ui.scroll_offset = total_wrapped_lines.saturating_sub(available_height);
            } else {
                self.ui.scroll_offset = 0;
            }
        }
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

    pub fn scroll_index_into_view(&mut self, index: usize, term_width: u16, term_height: u16) {
        let input_area_height = self.ui.calculate_input_area_height(term_width);
        let available_height = self.calculate_available_height(term_height, input_area_height);
        self.ui.scroll_offset =
            self.calculate_scroll_to_message(index, term_width, available_height);
    }

    pub fn calculate_available_height(&self, term_height: u16, input_area_height: u16) -> u16 {
        term_height
            .saturating_sub(input_area_height + 2)
            .saturating_sub(1)
    }

    pub fn finalize_response(&mut self) {
        if !self.ui.current_response.is_empty() {
            if let Err(e) = self.session.logging.log_message(&self.ui.current_response) {
                eprintln!("Failed to log response: {e}");
            }
        }

        self.session.retrying_message_index = None;
    }

    pub fn cancel_current_stream(&mut self) {
        if let Some(token) = &self.session.stream_cancel_token {
            token.cancel();
        }
        self.session.stream_cancel_token = None;
        self.ui.end_streaming();
        self.ui.stream_interrupted = true;
    }

    pub fn start_new_stream(&mut self) -> (CancellationToken, u64) {
        self.cancel_current_stream();

        self.session.current_stream_id += 1;

        let token = CancellationToken::new();
        self.session.stream_cancel_token = Some(token.clone());
        self.ui.begin_streaming();

        (token, self.session.current_stream_id)
    }

    pub fn prepare_retry(
        &mut self,
        available_height: u16,
        terminal_width: u16,
    ) -> Option<Vec<crate::api::ChatMessage>> {
        if !self.can_retry() {
            return None;
        }

        self.session.last_retry_time = Instant::now();

        if let Some(retry_index) = self.session.retrying_message_index {
            if retry_index < self.ui.messages.len() {
                if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                    if msg.role == "assistant" {
                        msg.content.clear();
                        self.ui.current_response.clear();
                    }
                }
            }
        } else {
            let mut target_index = None;

            for (i, msg) in self.ui.messages.iter().enumerate().rev() {
                if msg.role == "assistant" && !msg.content.is_empty() {
                    target_index = Some(i);
                    break;
                }
            }

            if let Some(index) = target_index {
                self.session.retrying_message_index = Some(index);

                if let Some(msg) = self.ui.messages.get_mut(index) {
                    msg.content.clear();
                    self.ui.current_response.clear();
                }

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

        if let Some(retry_index) = self.session.retrying_message_index {
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

        self.ui.auto_scroll = true;

        let mut api_messages = Vec::new();
        if let Some(retry_index) = self.session.retrying_message_index {
            for (i, msg) in self.ui.messages.iter().enumerate() {
                if i < retry_index && (msg.role == "user" || msg.role == "assistant") {
                    api_messages.push(crate::api::ChatMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
            }
        }

        Some(api_messages)
    }

    pub fn can_retry(&self) -> bool {
        self.ui
            .messages
            .iter()
            .any(|msg| msg.role == "assistant" && !msg.content.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::Message;
    use crate::utils::test_utils::{create_test_app, create_test_message};

    #[test]
    fn test_system_messages_excluded_from_api() {
        let mut app = create_test_app();

        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        {
            let mut conversation = ConversationController::new(&mut app.session, &mut app.ui);
            conversation.add_system_message(
                "This is a system message that should not be sent to API".to_string(),
            );
        }

        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        {
            let mut conversation = ConversationController::new(&mut app.session, &mut app.ui);
            conversation.add_system_message("Another system message".to_string());
        }

        let api_messages = {
            let mut conversation = ConversationController::new(&mut app.session, &mut app.ui);
            conversation.add_user_message("How are you?".to_string())
        };

        assert_eq!(api_messages.len(), 3);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Hello");
        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[1].content, "Hi there!");
        assert_eq!(api_messages[2].role, "user");
        assert_eq!(api_messages[2].content, "How are you?");

        for msg in &api_messages {
            assert_ne!(msg.role, "system");
        }
    }

    #[test]
    fn test_prepare_retry_excludes_system_messages() {
        let mut app = create_test_app();

        app.ui.messages.push_back(Message {
            role: "user".to_string(),
            content: "Test question".to_string(),
        });

        {
            let mut conversation = ConversationController::new(&mut app.session, &mut app.ui);
            conversation
                .add_system_message("System message between user and assistant".to_string());
        }

        app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: "Test response".to_string(),
        });

        app.session.retrying_message_index = Some(2);

        let api_messages = {
            let mut conversation = ConversationController::new(&mut app.session, &mut app.ui);
            conversation.prepare_retry(10, 80).unwrap()
        };

        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test question");

        for msg in &api_messages {
            assert_ne!(msg.role, "system");
        }
    }
}
