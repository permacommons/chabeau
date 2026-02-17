use super::{session::PendingToolCall, session::SessionContext, ui_state::UiState};
use crate::character::card::CharacterCard;
use crate::core::message::{AppMessageKind, Message, TranscriptRole};
use crate::utils::scroll::ScrollCalculator;
use serde_json::Value;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

pub struct ConversationController<'a> {
    session: &'a mut SessionContext,
    ui: &'a mut UiState,
    persona_manager: &'a crate::core::persona::PersonaManager,
    preset_manager: &'a crate::core::preset::PresetManager,
}

impl<'a> ConversationController<'a> {
    pub fn new(
        session: &'a mut SessionContext,
        ui: &'a mut UiState,
        persona_manager: &'a crate::core::persona::PersonaManager,
        preset_manager: &'a crate::core::preset::PresetManager,
    ) -> Self {
        Self {
            session,
            ui,
            persona_manager,
            preset_manager,
        }
    }

    /// Apply persona modifications to a system prompt
    /// Returns the modified prompt if a persona is active, otherwise returns the original
    fn apply_persona_to_system_prompt(&self, base_prompt: &str, char_name: Option<&str>) -> String {
        self.persona_manager
            .get_modified_system_prompt(base_prompt, char_name)
    }

    fn apply_preset_to_messages(&self, messages: &mut Vec<crate::api::ChatMessage>) {
        let char_name = self
            .session
            .get_character()
            .map(|character| character.data.name.as_str());
        self.preset_manager
            .apply_to_messages(messages, self.persona_manager, char_name);
    }

    fn character_greeting_text(&self) -> Option<String> {
        let character = self.session.get_character()?;
        let user_name = self
            .persona_manager
            .get_active_persona()
            .map(|p| p.display_name.as_str());
        let char_name = Some(character.data.name.as_str());
        let greeting = character.get_greeting_with_substitutions(user_name, char_name);

        if greeting.trim().is_empty() {
            None
        } else {
            Some(greeting)
        }
    }

    /// Display character greeting if not yet shown
    pub fn show_character_greeting_if_needed(&mut self) {
        if self.session.should_show_greeting() {
            if let Some(greeting) = self.character_greeting_text() {
                let greeting_message = Message::new(TranscriptRole::Assistant, greeting);
                self.ui.messages.push_back(greeting_message);
                self.session.mark_greeting_shown();
            }
        }
    }

    pub fn clear_transcript(&mut self) {
        self.ui.messages.clear();
        self.ui.current_response.clear();
        self.ui.invalidate_prewrap_cache();

        self.session.retrying_message_index = None;
        self.session.is_refining = false;
        self.session.original_refining_content = None;
        self.session.last_refine_prompt = None;
        self.session.has_received_assistant_message = false;
        self.session.character_greeting_shown = false;
        self.session.tool_pipeline.reset();
    }

    pub fn remove_trailing_empty_assistant_messages(&mut self) {
        let mut removed = false;

        while let Some(last_message) = self.ui.messages.back() {
            if last_message.is_assistant() && last_message.content.trim().is_empty() {
                self.ui.messages.pop_back();
                removed = true;
            } else {
                break;
            }
        }

        if removed {
            if let Some(index) = self.session.retrying_message_index {
                if index >= self.ui.messages.len() {
                    self.session.retrying_message_index = None;
                }
            }
        }
    }

    fn assemble_api_messages<'m, I>(
        &self,
        history: I,
        additional_system_prompt: Option<String>,
    ) -> Vec<crate::api::ChatMessage>
    where
        I: Iterator<Item = &'m Message>,
    {
        let mut api_messages = Vec::new();

        let character = self.session.get_character();

        let base_system_prompt = if let Some(character) = character {
            let user_name = self
                .persona_manager
                .get_active_persona()
                .map(|p| p.display_name.as_str());
            let char_name = Some(character.data.name.as_str());
            character.build_system_prompt_with_substitutions(user_name, char_name)
        } else {
            "".to_string()
        };

        let char_name = character.map(|c| c.data.name.as_str());
        let modified_system_prompt =
            self.apply_persona_to_system_prompt(&base_system_prompt, char_name);

        let mut final_system_prompt = modified_system_prompt;
        if let Some(additional) = additional_system_prompt {
            if !final_system_prompt.is_empty() {
                final_system_prompt.push_str("\n\n");
            }
            final_system_prompt.push_str(&additional);
        }

        if !final_system_prompt.is_empty() {
            api_messages.push(crate::api::ChatMessage {
                role: "system".to_string(),
                content: final_system_prompt,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }

        for msg in history {
            if msg.is_assistant() && msg.content.trim().is_empty() {
                continue;
            }

            if msg.is_user() || msg.is_assistant() {
                api_messages.push(crate::api::ChatMessage {
                    role: msg.role.as_str().to_string(),
                    content: msg.content.clone(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
        }

        if let Some(character) = character {
            if let Some(message) = self.post_history_system_message(character) {
                api_messages.push(message);
            }
        }

        self.apply_preset_to_messages(&mut api_messages);

        api_messages
    }

    pub fn api_messages_from_history(&self) -> Vec<crate::api::ChatMessage> {
        self.assemble_api_messages(self.ui.messages.iter(), None)
    }

    pub fn add_message(&mut self, message: Message) {
        self.ui.messages.push_back(message);
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
        self.clear_status();

        self.remove_trailing_empty_assistant_messages();

        let user_message = Message::new(TranscriptRole::User, content.clone());

        let user_display_name = self.persona_manager.get_display_name();
        if let Err(e) = self
            .session
            .logging
            .log_message(&format!("{user_display_name}: {content}"))
        {
            self.add_app_message(
                AppMessageKind::Warning,
                format!(
                    "Logging error: {}. Conversation will continue but may not be saved.",
                    e
                ),
            );
        }

        self.ui.messages.push_back(user_message);

        let assistant_message = Message::new(TranscriptRole::Assistant, String::new());
        self.ui.messages.push_back(assistant_message);
        self.ui.current_response.clear();
        self.session.active_assistant_message_index =
            Some(self.ui.messages.len().saturating_sub(1));

        self.session.retrying_message_index = None;
        self.session.original_refining_content = None;

        let history_len = self.ui.messages.len().saturating_sub(1);
        self.assemble_api_messages(self.ui.messages.iter().take(history_len), None)
    }

    pub fn add_assistant_placeholder(&mut self) {
        let assistant_message = Message::new(TranscriptRole::Assistant, String::new());
        self.ui.messages.push_back(assistant_message);
        self.ui.current_response.clear();
        self.session.active_assistant_message_index =
            Some(self.ui.messages.len().saturating_sub(1));

        self.session.retrying_message_index = None;
        self.session.original_refining_content = None;
        self.session.is_refining = false;
        self.session.has_received_assistant_message = false;
    }

    pub fn add_app_message(&mut self, kind: AppMessageKind, content: String) {
        // Log app/log messages to the file
        if kind == AppMessageKind::Log {
            if let Err(e) = self.session.logging.log_message(&format!("## {}", content)) {
                // Can't call add_app_message recursively for Log type, so add warning directly
                let warning = Message::app(
                    AppMessageKind::Warning,
                    format!("Logging error: {}. Log file may be incomplete.", e),
                );
                self.ui.messages.push_back(warning);
            }
        }

        let message = Message::app(kind, content);
        self.ui.messages.push_back(message);
    }

    pub fn take_pending_tool_calls(&mut self) -> Vec<(u32, PendingToolCall)> {
        if self.session.tool_pipeline.pending_tool_calls.is_empty() {
            return Vec::new();
        }

        let has_assistant_content = !self.ui.current_response.trim().is_empty();
        if !has_assistant_content {
            self.remove_trailing_empty_assistant_messages();
        }

        let pending_map = std::mem::take(&mut self.session.tool_pipeline.pending_tool_calls);
        let pending: Vec<(u32, PendingToolCall)> = pending_map.into_iter().collect();

        for (_, tool_call) in pending.iter() {
            if tool_call
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(crate::mcp::MCP_INSTANT_RECALL_TOOL))
            {
                continue;
            }
            let arguments = tool_call.arguments.trim();
            let summary = summarize_tool_call_arguments(arguments);
            let content = match (tool_call.name.as_deref(), summary.as_deref()) {
                (Some(name), Some(summary)) => format!("{name} | Arguments: {summary}"),
                (Some(name), None) => name.to_string(),
                (None, Some(summary)) => format!("Arguments: {summary}"),
                (None, None) => "Unknown tool call".to_string(),
            };
            self.ui.messages.push_back(Message::tool_call(content));
        }

        pending
    }

    pub fn clear_pending_tool_calls(&mut self) {
        self.session.tool_pipeline.pending_tool_calls.clear();
    }

    pub fn add_tool_result_message(&mut self, content: String) {
        self.ui.messages.push_back(Message::tool_result(content));
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
        let is_first_refine_chunk = self.session.is_refining;
        if is_first_refine_chunk {
            self.session.is_refining = false; // consume the flag
        }

        if let Some(retry_index) = self.session.retrying_message_index {
            if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                if msg.is_assistant() {
                    if is_first_refine_chunk {
                        msg.content.clear();
                    }
                    msg.content.push_str(content);
                }
            }
        } else if let Some(last_msg) = self.ui.messages.back_mut() {
            if last_msg.is_assistant() {
                last_msg.content.push_str(content);
            }
        }

        self.ui.current_response.push_str(content);

        if !content.is_empty() {
            self.session.has_received_assistant_message = true;
        }

        // Delegate scroll math to the centralized helper
        self.update_scroll_position(available_height, terminal_width);
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
            // If this was a retry/refine, rewrite log excluding the message being retried
            // This happens lazily (only after successful API response) to avoid data loss
            if let Some(retry_index) = self.session.retrying_message_index {
                let user_display_name = self.persona_manager.get_display_name();
                if let Err(e) = self.session.logging.rewrite_log_skip_index(
                    &self.ui.messages,
                    &user_display_name,
                    Some(retry_index),
                ) {
                    self.add_app_message(
                        AppMessageKind::Warning,
                        format!("Logging error: {}. Log file may be incomplete.", e),
                    );
                }
            }

            // Then log the new response
            if let Err(e) = self.session.logging.log_message(&self.ui.current_response) {
                self.add_app_message(
                    AppMessageKind::Warning,
                    format!("Logging error: {}. Response may not be saved to log.", e),
                );
            }
        }

        if self.session.original_refining_content.is_none() {
            self.session.retrying_message_index = None;
        }
        self.session.is_refining = false;
        self.ui.current_response.clear();
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
        self.session.tool_pipeline.reset();
        self.session.mcp_tools_enabled = false;

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

        if self.session.original_refining_content.is_some() {
            if let Some(last_prompt) = self.session.last_refine_prompt.clone() {
                return self.prepare_refine(last_prompt, available_height, terminal_width, true);
            }
        }

        self.session.last_retry_time = Instant::now();

        if let Some(retry_index) = self.session.retrying_message_index {
            if retry_index < self.ui.messages.len() {
                self.session.active_assistant_message_index = Some(retry_index);
                self.session
                    .tool_pipeline
                    .prune_for_assistant_index(retry_index);
                if !self.session.has_received_assistant_message {
                    if let Some(greeting) = self.character_greeting_text() {
                        if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                            if msg.is_assistant() {
                                msg.content = greeting;
                                self.ui.current_response.clear();
                                self.session.retrying_message_index = None;
                                return None;
                            }
                        }
                    }
                }

                if let Some(msg) = self.ui.messages.get_mut(retry_index) {
                    if msg.is_assistant() {
                        msg.content.clear();
                        self.ui.current_response.clear();
                    }
                }
            }
        } else {
            let mut target_index = None;

            for (i, msg) in self.ui.messages.iter().enumerate().rev() {
                if msg.is_assistant() && !msg.content.is_empty() {
                    target_index = Some(i);
                    break;
                }
            }

            if let Some(index) = target_index {
                if !self.session.has_received_assistant_message {
                    if let Some(greeting) = self.character_greeting_text() {
                        if let Some(msg) = self.ui.messages.get_mut(index) {
                            if msg.is_assistant() {
                                msg.content = greeting;
                                self.ui.current_response.clear();
                                self.session.retrying_message_index = None;
                                return None;
                            }
                        }
                    }
                }

                self.session.retrying_message_index = Some(index);
                self.session.active_assistant_message_index = Some(index);
                self.session.tool_pipeline.prune_for_assistant_index(index);

                if let Some(msg) = self.ui.messages.get_mut(index) {
                    msg.content.clear();
                    self.ui.current_response.clear();
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

        let retry_index = self.session.retrying_message_index?;

        let api_messages =
            self.assemble_api_messages(self.ui.messages.iter().take(retry_index), None);

        Some(api_messages)
    }

    fn post_history_system_message(
        &self,
        character: &CharacterCard,
    ) -> Option<crate::api::ChatMessage> {
        // Apply persona substitutions to post-history instructions
        let user_name = self
            .persona_manager
            .get_active_persona()
            .map(|p| p.display_name.as_str());
        let char_name = Some(character.data.name.as_str());

        character
            .get_post_history_instructions_with_substitutions(user_name, char_name)
            .and_then(|instructions| {
                let trimmed = instructions.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(crate::api::ChatMessage {
                        role: "system".to_string(),
                        content: instructions,
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    })
                }
            })
    }

    pub fn can_retry(&self) -> bool {
        self.ui
            .messages
            .iter()
            .any(|msg| msg.is_assistant() && !msg.content.is_empty())
    }

    pub fn stream_parameters(
        &mut self,
        messages: Vec<Message>,
        additional_system_prompt: Option<String>,
    ) -> crate::core::chat_stream::StreamParams {
        let (cancel_token, stream_id) = self.start_new_stream_headless();
        let api_messages = self.assemble_api_messages(messages.iter(), additional_system_prompt);
        crate::core::chat_stream::StreamParams {
            api_messages,
            client: self.session.client.clone(),
            model: self.session.model.clone(),
            api_key: self.session.api_key.clone(),
            base_url: self.session.base_url.clone(),
            provider_name: self.session.provider_name.clone(),
            tools: None,
            cancel_token,
            stream_id,
        }
    }

    fn cancel_current_stream_headless(&mut self) {
        if let Some(token) = &self.session.stream_cancel_token {
            token.cancel();
        }
        self.session.stream_cancel_token = None;
    }

    fn start_new_stream_headless(&mut self) -> (CancellationToken, u64) {
        self.cancel_current_stream_headless();
        self.clear_pending_tool_calls();
        self.session.current_stream_id += 1;
        let token = CancellationToken::new();
        self.session.stream_cancel_token = Some(token.clone());
        (token, self.session.current_stream_id)
    }

    pub fn prepare_refine(
        &mut self,
        prompt: String,
        available_height: u16,
        terminal_width: u16,
        use_original: bool,
    ) -> Option<Vec<crate::api::ChatMessage>> {
        if !self.can_retry() {
            return None;
        }

        self.session.last_retry_time = Instant::now();
        self.session.is_refining = true;

        if self.session.retrying_message_index.is_none() {
            let mut target_index = None;
            for (i, msg) in self.ui.messages.iter().enumerate().rev() {
                if msg.is_assistant() && !msg.content.is_empty() {
                    target_index = Some(i);
                    break;
                }
            }
            self.session.retrying_message_index = target_index;
        }

        if !use_original {
            self.session.original_refining_content = None;
            self.session.last_refine_prompt = Some(prompt.clone());
        }

        if self.session.original_refining_content.is_none() {
            if let Some(index) = self.session.retrying_message_index {
                if let Some(msg) = self.ui.messages.get(index) {
                    self.session.original_refining_content = Some(msg.content.clone());
                }
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
            self.ui.auto_scroll = true;

            let mut history_for_api: Vec<Message> = self
                .ui
                .messages
                .iter()
                .take(retry_index + 1)
                .cloned()
                .collect();

            if use_original {
                if let Some(original_content) = &self.session.original_refining_content {
                    if let Some(last_msg) = history_for_api.last_mut() {
                        if last_msg.is_assistant() {
                            last_msg.content = original_content.clone();
                        }
                    }
                }
            }

            let instructions = self.session.refine_instructions.clone();
            let prefix = self.session.refine_prefix.as_str();
            let mut api_messages =
                self.assemble_api_messages(history_for_api.iter(), Some(instructions));
            api_messages.push(crate::api::ChatMessage {
                role: "user".to_string(),
                content: format!("{} {}", prefix, prompt),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
            Some(api_messages)
        } else {
            None
        }
    }
}

fn summarize_tool_call_arguments(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }

    let summary = match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => {
            let mut parts = Vec::new();
            for (key, value) in map.iter() {
                let value_summary = summarize_tool_call_value(value);
                parts.push(format!("{key}={value_summary}"));
            }
            parts.join(", ")
        }
        Ok(Value::Array(items)) => abbreviate_tool_call_value(
            &serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()),
        ),
        Ok(Value::String(value)) => {
            abbreviate_tool_call_value(&serde_json::to_string(&value).unwrap_or(value))
        }
        Ok(Value::Number(value)) => value.to_string(),
        Ok(Value::Bool(value)) => value.to_string(),
        Ok(Value::Null) => "null".to_string(),
        Err(_) => abbreviate_tool_call_value(&collapse_whitespace(raw)),
    };

    let summary = collapse_whitespace(&summary);
    if summary.is_empty() {
        None
    } else {
        Some(summary)
    }
}

fn summarize_tool_call_value(value: &Value) -> String {
    let summary = match value {
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| value.clone()),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
        Value::Array(items) => serde_json::to_string(items).unwrap_or_else(|_| "[]".into()),
        Value::Object(map) => serde_json::to_string(map).unwrap_or_else(|_| "{}".into()),
    };
    abbreviate_tool_call_value(&summary)
}

fn abbreviate_tool_call_value(value: &str) -> String {
    const TOOL_CALL_VALUE_LIMIT: usize = 100;
    if value.chars().count() <= TOOL_CALL_VALUE_LIMIT {
        return value.to_string();
    }

    let mut shortened = String::with_capacity(TOOL_CALL_VALUE_LIMIT + 1);
    for (idx, ch) in value.chars().enumerate() {
        if idx >= TOOL_CALL_VALUE_LIMIT {
            break;
        }
        shortened.push(ch);
    }
    shortened.push('…');
    shortened
}

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if last_was_space {
                continue;
            }
            last_was_space = true;
            out.push(' ');
            continue;
        }
        last_was_space = false;
        out.push(ch);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::data::{Config, Persona};
    use crate::core::message::{self, Message, TranscriptRole};
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::{
        create_test_app, create_test_message, create_test_message_with_role,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_app_messages_excluded_from_api() {
        let mut app = create_test_app();

        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_app_message(
                AppMessageKind::Info,
                "This is an app message that should not be sent to API".to_string(),
            );
        }

        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation
                .add_app_message(AppMessageKind::Warning, "Another app message".to_string());
        }

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
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
            assert!(!message::is_app_message_role(&msg.role));
        }
    }

    #[test]
    fn test_tool_call_argument_values_truncate_long_strings() {
        let long_value = "a".repeat(120);
        let raw = format!(r#"{{"q":"{long_value}","n":1}}"#);

        let summary = summarize_tool_call_arguments(&raw).expect("summary");
        let expected_q =
            abbreviate_tool_call_value(&serde_json::to_string(&long_value).expect("json"));

        assert!(summary.contains(&format!("q={expected_q}")));
        assert!(summary.contains("n=1"));
    }

    #[test]
    fn add_user_message_omits_trailing_empty_assistant_turns() {
        let mut app = create_test_app();

        app.ui.messages.push_back(create_test_message_with_role(
            TranscriptRole::User,
            "First attempt",
        ));
        app.ui
            .messages
            .push_back(create_test_message_with_role(TranscriptRole::Assistant, ""));

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Try again?".to_string())
        };

        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "First attempt");
        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "Try again?");
        assert!(api_messages
            .iter()
            .all(|msg| msg.role != "assistant" || !msg.content.trim().is_empty()));

        let mut iter = app.ui.messages.iter().rev();
        let last = iter.next().expect("missing assistant placeholder");
        assert_eq!(last.role, TranscriptRole::Assistant);
        assert!(last.content.is_empty());

        let second_last = iter.next().expect("missing user retry message");
        assert_eq!(second_last.role, TranscriptRole::User);
        assert_eq!(second_last.content, "Try again?");

        assert_eq!(
            app.ui
                .messages
                .iter()
                .filter(|msg| msg.role == TranscriptRole::Assistant && msg.content.is_empty())
                .count(),
            1
        );
    }

    #[test]
    fn test_prepare_retry_excludes_system_messages() {
        let mut app = create_test_app();

        app.ui.messages.push_back(Message {
            role: TranscriptRole::User,
            content: "Test question".to_string(),
        });

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_app_message(
                AppMessageKind::Info,
                "App message between user and assistant".to_string(),
            );
        }

        app.ui.messages.push_back(Message {
            role: TranscriptRole::Assistant,
            content: "Test response".to_string(),
        });

        app.session.retrying_message_index = Some(2);
        app.session.has_received_assistant_message = true;

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.prepare_retry(10, 80).unwrap()
        };

        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test question");

        for msg in &api_messages {
            assert!(!message::is_app_message_role(&msg.role));
        }
    }

    #[test]
    fn test_add_user_message_with_character_active() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character with system prompt and post-history instructions
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful and friendly".to_string(),
                scenario: "Testing scenario".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "Example dialogue".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: Some("Always be polite.".to_string()),
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Add a previous message
        app.ui
            .messages
            .push_back(create_test_message("user", "Previous message"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Previous response"));

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("New message".to_string())
        };

        // Should have: system prompt, previous user, previous assistant, new user, post-history
        assert_eq!(api_messages.len(), 5);

        // First message should be character system prompt
        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("You are TestBot."));
        assert!(api_messages[0].content.contains("Character: TestBot"));

        // Middle messages should be conversation history
        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "Previous message");
        assert_eq!(api_messages[2].role, "assistant");
        assert_eq!(api_messages[2].content, "Previous response");
        assert_eq!(api_messages[3].role, "user");
        assert_eq!(api_messages[3].content, "New message");

        // Last message should be post-history instructions
        assert_eq!(api_messages[4].role, "system");
        assert_eq!(api_messages[4].content, "Always be polite.");
    }

    #[test]
    fn test_persona_bio_char_placeholder_with_active_character() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        let config = Config {
            personas: vec![Persona {
                id: "mentor".to_string(),
                display_name: "Mentor".to_string(),
                bio: Some("Guide {{char}} with wisdom.".to_string()),
            }],
            ..Default::default()
        };

        app.persona_manager = PersonaManager::load_personas(&config).unwrap();
        app.persona_manager
            .set_active_persona("mentor")
            .expect("Failed to activate persona");

        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Aria".to_string(),
                description: "A skilled musician".to_string(),
                personality: "Creative and calm".to_string(),
                scenario: "Guiding apprentices".to_string(),
                first_mes: "Welcome.".to_string(),
                mes_example: "Example.".to_string(),
                creator_notes: None,
                system_prompt: Some("Stay supportive.".to_string()),
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Hello".to_string())
        };

        assert!(!api_messages.is_empty());
        assert_eq!(api_messages[0].role, "system");
        assert!(
            api_messages[0].content.contains("Guide Aria with wisdom."),
            "System prompt should include character name substitution: {}",
            api_messages[0].content
        );
    }

    #[test]
    fn add_user_message_logs_persona_display_name() {
        let mut app = create_test_app();

        let config = Config {
            personas: vec![Persona {
                id: "captain".to_string(),
                display_name: "Captain".to_string(),
                bio: None,
            }],
            ..Default::default()
        };

        app.persona_manager = PersonaManager::load_personas(&config).unwrap();
        app.persona_manager
            .set_active_persona("captain")
            .expect("Failed to activate persona");

        let temp_dir = tempdir().expect("failed to create temp dir for log");
        let log_path = temp_dir.path().join("conversation.log");
        let log_path_string = log_path.to_string_lossy().into_owned();
        app.session
            .logging
            .set_log_file(log_path_string)
            .expect("failed to enable logging");

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Hello there".to_string());
        }

        let contents = fs::read_to_string(&log_path).expect("failed to read log file");
        assert!(
            contents.contains("Captain: Hello there"),
            "Log should include persona display name, contents: {contents}"
        );
    }

    #[test]
    fn log_rewrite_excludes_app_messages() {
        let mut app = create_test_app();

        let temp_dir = tempdir().expect("failed to create temp dir for log");
        let log_path = temp_dir.path().join("conversation.log");
        let log_path_string = log_path.to_string_lossy().into_owned();
        app.session
            .logging
            .set_log_file(log_path_string)
            .expect("failed to enable logging");

        // Add messages including an app message
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Hello".to_string());
        }

        app.ui.messages.push_back(create_test_message_with_role(
            TranscriptRole::Assistant,
            "Hi there!",
        ));

        // Add an app message
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_app_message(
                AppMessageKind::Warning,
                "This is a warning message".to_string(),
            );
        }

        // Trigger a log rewrite (simulating an edit/retry)
        let user_display_name = app.persona_manager.get_display_name();
        app.session
            .logging
            .rewrite_log_without_last_response(&app.ui.messages, &user_display_name)
            .expect("failed to rewrite log");

        let contents = fs::read_to_string(&log_path).expect("failed to read log file");

        // Verify conversation messages are in the log
        assert!(
            contents.contains("You: Hello"),
            "Log should contain user message, contents: {contents}"
        );
        assert!(
            contents.contains("Hi there!"),
            "Log should contain assistant message, contents: {contents}"
        );

        // Verify app message is NOT in the log
        assert!(
            !contents.contains("This is a warning message"),
            "Log should NOT contain app messages, contents: {contents}"
        );
    }

    #[test]
    fn test_add_user_message_with_character_no_post_history() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character without post-history instructions
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hi".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Test message".to_string())
        };

        // Should have: system prompt, user message (no post-history)
        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("Character: TestBot"));
        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "Test message");
    }

    #[test]
    fn test_add_user_message_without_character() {
        let mut app = create_test_app();

        // No character set
        assert!(app.session.get_character().is_none());

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Test message".to_string())
        };

        // Should only have the user message, no system messages
        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test message");
    }

    #[test]
    fn test_prepare_retry_with_character_active() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: Some("Be concise.".to_string()),
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "First question"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "First response"));
        app.ui
            .messages
            .push_back(create_test_message("user", "Second question"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Second response to retry"));

        // Set retry index to the last assistant message
        app.session.retrying_message_index = Some(3);
        app.session.has_received_assistant_message = true;

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.prepare_retry(10, 80).unwrap()
        };

        // Should have: system prompt, first user, first assistant, second user, post-history
        assert_eq!(api_messages.len(), 5);

        // First should be character system prompt
        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("You are TestBot."));

        // Middle should be conversation history up to retry point
        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "First question");
        assert_eq!(api_messages[2].role, "assistant");
        assert_eq!(api_messages[2].content, "First response");
        assert_eq!(api_messages[3].role, "user");
        assert_eq!(api_messages[3].content, "Second question");

        // Last should be post-history instructions
        assert_eq!(api_messages[4].role, "system");
        assert_eq!(api_messages[4].content, "Be concise.");
    }

    #[test]
    fn test_prepare_retry_without_character() {
        let mut app = create_test_app();

        // No character set
        assert!(app.session.get_character().is_none());

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "Question"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Response to retry"));

        app.session.retrying_message_index = Some(1);
        app.session.has_received_assistant_message = true;

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.prepare_retry(10, 80).unwrap()
        };

        // Should only have the user message, no system messages
        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Question");
    }

    #[test]
    fn test_retry_character_greeting_reinserts_locally() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello! I'm TestBot.".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        assert_eq!(app.ui.messages.len(), 1);
        assert_eq!(app.ui.messages[0].role, "assistant");
        assert!(!app.session.has_received_assistant_message);

        app.session.retrying_message_index = Some(0);

        let result = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.prepare_retry(10, 80)
        };

        assert!(result.is_none());
        assert_eq!(app.ui.messages[0].content, "Hello! I'm TestBot.");
        assert!(app.session.retrying_message_index.is_none());
        assert!(!app.session.has_received_assistant_message);
    }

    #[test]
    fn test_retry_character_greeting_updates_after_persona_change() {
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::core::config::data::{Config, Persona};
        use crate::core::persona::PersonaManager;

        let mut app = create_test_app();

        let config = Config {
            personas: vec![
                Persona {
                    id: "first".to_string(),
                    display_name: "First".to_string(),
                    bio: None,
                },
                Persona {
                    id: "second".to_string(),
                    display_name: "Second".to_string(),
                    bio: None,
                },
            ],
            ..Default::default()
        };
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");
        app.persona_manager
            .set_active_persona("first")
            .expect("Failed to activate persona");

        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hi {{user}}! I'm {{char}}.".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        assert_eq!(app.ui.messages.len(), 1);
        assert_eq!(app.ui.messages[0].role, "assistant");
        assert_eq!(app.ui.messages[0].content, "Hi First! I'm TestBot.");
        assert!(!app.session.has_received_assistant_message);

        app.persona_manager
            .set_active_persona("second")
            .expect("Failed to activate second persona");

        let result = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.prepare_retry(10, 80)
        };

        assert!(result.is_none());
        assert_eq!(app.ui.messages[0].content, "Hi Second! I'm TestBot.");
        assert!(app.session.retrying_message_index.is_none());
        assert!(!app.session.has_received_assistant_message);
    }

    #[test]
    fn test_character_messages_with_transcript_system_messages() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Add user message
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        // Add transcript app message (should be excluded from API)
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_app_message(
                AppMessageKind::Info,
                "Help text displayed in UI".to_string(),
            );
        }

        // Add assistant response
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("How are you?".to_string())
        };

        // Should have: character system prompt, first user, first assistant, new user
        // Transcript app message should be excluded
        assert_eq!(api_messages.len(), 4);

        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("You are TestBot."));

        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "Hello");

        assert_eq!(api_messages[2].role, "assistant");
        assert_eq!(api_messages[2].content, "Hi there!");

        assert_eq!(api_messages[3].role, "user");
        assert_eq!(api_messages[3].content, "How are you?");

        // Verify transcript system message is not in API messages
        for msg in &api_messages {
            assert_ne!(msg.content, "Help text displayed in UI");
        }
    }

    #[test]
    fn test_show_character_greeting_if_needed() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character with a greeting
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello! I'm TestBot.".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Initially no messages
        assert_eq!(app.ui.messages.len(), 0);

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Should have added greeting as assistant message
        assert_eq!(app.ui.messages.len(), 1);
        let greeting_msg = app.ui.messages.front().unwrap();
        assert_eq!(greeting_msg.role, "assistant");
        assert_eq!(greeting_msg.content, "Hello! I'm TestBot.");

        // Greeting should be marked as shown
        assert!(app.session.character_greeting_shown);

        // Calling again should not add another greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }
        assert_eq!(app.ui.messages.len(), 1);
    }

    #[test]
    fn test_show_character_greeting_empty_greeting() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character with empty greeting
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "   ".to_string(), // Empty/whitespace greeting
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Should not have added any messages (empty greeting)
        assert_eq!(app.ui.messages.len(), 0);
        assert!(!app.session.character_greeting_shown);
    }

    #[test]
    fn test_show_character_greeting_no_character() {
        let mut app = create_test_app();

        // No character set
        assert!(app.session.get_character().is_none());

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Should not have added any messages
        assert_eq!(app.ui.messages.len(), 0);
        assert!(!app.session.character_greeting_shown);
    }

    #[test]
    fn test_character_greeting_with_persona_substitutions() {
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::core::config::data::{Config, Persona};
        use crate::core::persona::PersonaManager;

        let mut app = create_test_app();

        // Set up persona
        let config = Config {
            personas: vec![Persona {
                id: "alice-dev".to_string(),
                display_name: "Alice".to_string(),
                bio: Some("You are talking to {{user}}, a senior developer.".to_string()),
            }],
            ..Default::default()
        };
        app.persona_manager =
            PersonaManager::load_personas(&config).expect("Failed to load personas");
        app.persona_manager
            .set_active_persona("alice-dev")
            .expect("Failed to activate persona");

        // Set up character with substitution placeholders in greeting
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello {{user}}! I'm {{char}}, ready to help!".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Verify greeting was added with substitutions applied
        assert_eq!(app.ui.messages.len(), 1);
        let greeting_msg = &app.ui.messages[0];
        assert_eq!(greeting_msg.role, "assistant");
        assert_eq!(
            greeting_msg.content,
            "Hello Alice! I'm TestBot, ready to help!"
        );
        assert!(app.session.character_greeting_shown);
    }

    #[test]
    fn test_persona_system_prompt_integration_with_character() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character with a system prompt
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Test message".to_string())
        };

        // Should have character system prompt (potentially modified by persona)
        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "system");
        // The content should contain the character system prompt
        assert!(api_messages[0].content.contains("You are TestBot."));
        assert_eq!(api_messages[1].role, "user");
        assert_eq!(api_messages[1].content, "Test message");
    }

    #[test]
    fn test_persona_system_prompt_integration_without_character() {
        let mut app = create_test_app();

        // No character set
        assert!(app.session.get_character().is_none());

        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Test message".to_string())
        };

        // Should only have the user message since no persona is active
        // (PersonaManager is created fresh each time with no active persona)
        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test message");
    }

    #[test]
    fn test_character_greeting_included_in_api_messages() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();

        // Set up a character with a greeting
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: "A test character".to_string(),
                personality: "Helpful".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Greetings!".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: Some("You are TestBot.".to_string()),
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);

        // Show greeting
        {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.show_character_greeting_if_needed();
        }

        // Add user message
        let api_messages = {
            let mut conversation = ConversationController::new(
                &mut app.session,
                &mut app.ui,
                &app.persona_manager,
                &app.preset_manager,
            );
            conversation.add_user_message("Hello".to_string())
        };

        // Should have: system prompt, greeting (assistant), user message
        assert_eq!(api_messages.len(), 3);

        assert_eq!(api_messages[0].role, "system");
        assert!(api_messages[0].content.contains("You are TestBot."));

        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[1].content, "Greetings!");

        assert_eq!(api_messages[2].role, "user");
        assert_eq!(api_messages[2].content, "Hello");
    }

    #[test]
    fn test_persona_with_blank_bio_does_not_add_system_message() {
        let cases = [
            ("empty", "Empty", ""),
            ("whitespace", "Whitespace", "   \n\t"),
        ];

        for (persona_id, display_name, bio) in cases {
            let config = Config {
                personas: vec![Persona {
                    id: persona_id.to_string(),
                    display_name: display_name.to_string(),
                    bio: Some(bio.to_string()),
                }],
                ..Default::default()
            };

            let persona_manager = PersonaManager::load_personas(&config).unwrap();

            let mut app = create_test_app();
            app.persona_manager = persona_manager;
            app.persona_manager
                .set_active_persona(persona_id)
                .expect("persona activation");

            let api_messages = {
                let mut conversation = ConversationController::new(
                    &mut app.session,
                    &mut app.ui,
                    &app.persona_manager,
                    &app.preset_manager,
                );
                conversation.add_user_message("Hello".to_string())
            };

            assert!(
                api_messages.iter().all(|msg| msg.role != "system"),
                "system message injected for persona {persona_id}"
            );
        }
    }
}
