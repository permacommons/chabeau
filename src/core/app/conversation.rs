use super::{session::SessionContext, ui_state::UiState};
use crate::character::card::CharacterCard;
use crate::core::message::Message;
use crate::utils::scroll::ScrollCalculator;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

pub struct ConversationController<'a> {
    session: &'a mut SessionContext,
    ui: &'a mut UiState,
    persona_manager: &'a crate::core::persona::PersonaManager,
}

impl<'a> ConversationController<'a> {
    pub fn new(
        session: &'a mut SessionContext,
        ui: &'a mut UiState,
        persona_manager: &'a crate::core::persona::PersonaManager,
    ) -> Self {
        Self {
            session,
            ui,
            persona_manager,
        }
    }

    /// Apply persona modifications to a system prompt
    /// Returns the modified prompt if a persona is active, otherwise returns the original
    fn apply_persona_to_system_prompt(&self, base_prompt: &str, char_name: Option<&str>) -> String {
        self.persona_manager
            .get_modified_system_prompt(base_prompt, char_name)
    }

    /// Display character greeting if not yet shown
    pub fn show_character_greeting_if_needed(&mut self) {
        if self.session.should_show_greeting() {
            if let Some(character) = self.session.get_character() {
                // Apply persona substitutions to the greeting
                let user_name = self
                    .persona_manager
                    .get_active_persona()
                    .map(|p| p.display_name.as_str());
                let char_name = Some(character.data.name.as_str());
                let greeting = character.get_greeting_with_substitutions(user_name, char_name);

                let greeting_message = Message {
                    role: "assistant".to_string(),
                    content: greeting,
                };
                self.ui.messages.push_back(greeting_message);
                self.session.mark_greeting_shown();
            }
        }
    }

    pub fn add_user_message(&mut self, content: String) -> Vec<crate::api::ChatMessage> {
        self.clear_status();

        let user_message = Message {
            role: "user".to_string(),
            content: content.clone(),
        };

        let user_display_name = self.persona_manager.get_display_name();
        if let Err(e) = self
            .session
            .logging
            .log_message(&format!("{user_display_name}: {content}"))
        {
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

        // Inject character instructions as API system message if active
        // Note: This is role="system" for the API, not a transcript system message
        if let Some(character) = self.session.get_character() {
            // Apply persona substitutions to character system prompt
            let user_name = self
                .persona_manager
                .get_active_persona()
                .map(|p| p.display_name.as_str());
            let char_name = Some(character.data.name.as_str());
            let base_system_prompt =
                character.build_system_prompt_with_substitutions(user_name, char_name);
            let modified_system_prompt =
                self.apply_persona_to_system_prompt(&base_system_prompt, char_name);
            api_messages.push(crate::api::ChatMessage {
                role: "system".to_string(),
                content: modified_system_prompt,
            });
        } else {
            // No character active, but check if persona should modify an empty system prompt
            let base_system_prompt = "";
            let modified_system_prompt =
                self.apply_persona_to_system_prompt(base_system_prompt, None);
            if !modified_system_prompt.is_empty() {
                api_messages.push(crate::api::ChatMessage {
                    role: "system".to_string(),
                    content: modified_system_prompt,
                });
            }
        }

        // Add conversation history (including greeting if present)
        // Note: Transcript system messages (help, status) are excluded here
        for msg in self.ui.messages.iter().take(self.ui.messages.len() - 1) {
            if msg.role == "user" || msg.role == "assistant" {
                api_messages.push(crate::api::ChatMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }

        // Add post-history instructions as API system message if present
        if let Some(character) = self.session.get_character() {
            if let Some(message) = self.post_history_system_message(character) {
                api_messages.push(message);
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

                let user_display_name = self.persona_manager.get_display_name();
                if let Err(e) = self
                    .session
                    .logging
                    .rewrite_log_without_last_response(&self.ui.messages, &user_display_name)
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

        // Inject character instructions as API system message if active
        if let Some(character) = self.session.get_character() {
            // Apply persona substitutions to character system prompt
            let user_name = self
                .persona_manager
                .get_active_persona()
                .map(|p| p.display_name.as_str());
            let char_name = Some(character.data.name.as_str());
            let base_system_prompt =
                character.build_system_prompt_with_substitutions(user_name, char_name);
            let modified_system_prompt =
                self.apply_persona_to_system_prompt(&base_system_prompt, char_name);
            api_messages.push(crate::api::ChatMessage {
                role: "system".to_string(),
                content: modified_system_prompt,
            });
        } else {
            // No character active, but check if persona should modify an empty system prompt
            let base_system_prompt = "";
            let modified_system_prompt =
                self.apply_persona_to_system_prompt(base_system_prompt, None);
            if !modified_system_prompt.is_empty() {
                api_messages.push(crate::api::ChatMessage {
                    role: "system".to_string(),
                    content: modified_system_prompt,
                });
            }
        }

        // Add conversation history up to retry point
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

        // Add post-history instructions as API system message if present
        if let Some(character) = self.session.get_character() {
            if let Some(message) = self.post_history_system_message(character) {
                api_messages.push(message);
            }
        }

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
                    })
                }
            })
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
    use crate::core::config::{Config, Persona};
    use crate::core::message::Message;
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_system_messages_excluded_from_api() {
        let mut app = create_test_app();

        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_system_message(
                "This is a system message that should not be sent to API".to_string(),
            );
        }

        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_system_message("Another system message".to_string());
        }

        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation
                .add_system_message("System message between user and assistant".to_string());
        }

        app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: "Test response".to_string(),
        });

        app.session.retrying_message_index = Some(2);

        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.prepare_retry(10, 80).unwrap()
        };

        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Test question");

        for msg in &api_messages {
            assert_ne!(msg.role, "system");
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_user_message("Hello there".to_string());
        }

        let contents = fs::read_to_string(&log_path).expect("failed to read log file");
        assert!(
            contents.contains("Captain: Hello there"),
            "Log should include persona display name, contents: {contents}"
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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

        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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

        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.prepare_retry(10, 80).unwrap()
        };

        // Should only have the user message, no system messages
        assert_eq!(api_messages.len(), 1);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[0].content, "Question");
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

        // Add transcript system message (should be excluded from API)
        {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_system_message("Help text displayed in UI".to_string());
        }

        // Add assistant response
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.add_user_message("How are you?".to_string())
        };

        // Should have: character system prompt, first user, first assistant, new user
        // Transcript system message should be excluded
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.show_character_greeting_if_needed();
        }

        // Should not have added any messages
        assert_eq!(app.ui.messages.len(), 0);
        assert!(!app.session.character_greeting_shown);
    }

    #[test]
    fn test_character_greeting_with_persona_substitutions() {
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::core::config::{Config, Persona};
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
            conversation.show_character_greeting_if_needed();
        }

        // Add user message
        let api_messages = {
            let mut conversation =
                ConversationController::new(&mut app.session, &mut app.ui, &app.persona_manager);
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
}
