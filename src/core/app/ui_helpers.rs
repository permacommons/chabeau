use super::App;
use crate::commands::matching_commands;
use crate::core::message::{Message, ROLE_ASSISTANT, ROLE_USER};
use crate::ui::span::SpanKind;
use ratatui::text::Line;

impl App {
    pub fn clear_status(&mut self) {
        self.conversation().clear_status();
    }

    pub fn toggle_compose_mode(&mut self) {
        self.ui.toggle_compose_mode();
    }

    pub fn cancel_file_prompt(&mut self) {
        self.ui.cancel_file_prompt();
    }

    pub fn has_in_place_edit(&self) -> bool {
        self.ui.in_place_edit_index().is_some()
    }

    pub fn cancel_in_place_edit(&mut self) {
        self.ui.cancel_in_place_edit();
    }

    pub fn clear_input(&mut self) {
        self.ui.clear_input();
    }

    pub fn recompute_input_layout_after_edit(&mut self, width: u16) {
        self.ui.recompute_input_layout_after_edit(width);
    }

    pub fn insert_into_input(&mut self, text: &str, width: u16) {
        self.ui.focus_input();
        self.ui.apply_textarea_edit_and_recompute(width, |ta| {
            ta.insert_str(text);
        });
    }

    pub fn complete_slash_command(&mut self, term_width: u16) -> bool {
        if !self.ui.is_input_active() {
            return false;
        }

        self.ui.focus_input();

        let input = self.ui.get_input_text().to_string();
        if !input.starts_with('/') {
            return false;
        }

        let chars: Vec<char> = input.chars().collect();
        let cursor = self.ui.input_cursor_position.min(chars.len());

        if cursor == 0 {
            return false;
        }

        if chars[..cursor].contains(&'\n') {
            return false;
        }

        let mut command_end = 1;
        while command_end < chars.len() && !chars[command_end].is_whitespace() {
            command_end += 1;
        }

        if cursor > command_end {
            return false;
        }

        let typed: String = chars[1..cursor].iter().collect();
        let matches = matching_commands(&typed);

        if matches.is_empty() {
            if !typed.is_empty() {
                self.conversation()
                    .set_status(format!("No command matches '/{}'", typed));
                return true;
            }
            return false;
        }

        let remainder: String = chars[command_end..].iter().collect();
        let command_names: Vec<&str> = matches.iter().map(|command| command.name).collect();

        if command_names.len() == 1 {
            apply_command_completion(&mut self.ui, command_names[0], &remainder, true, term_width);
            return true;
        }

        let prefix = longest_common_prefix(&command_names);
        if prefix.len() > typed.len() {
            apply_command_completion(&mut self.ui, &prefix, &remainder, false, term_width);
            return true;
        }

        let suggestions = format_command_suggestions(&command_names);
        self.conversation()
            .set_status(format!("Commands: {}", suggestions));
        true
    }

    pub fn complete_in_place_edit(&mut self, index: usize, new_text: String) {
        let Some(actual_index) = self.ui.take_in_place_edit_index() else {
            return;
        };

        if actual_index != index {
            return;
        }

        if actual_index >= self.ui.messages.len() {
            return;
        }

        let role = self.ui.messages[actual_index].role.as_str();
        if role != ROLE_USER && role != ROLE_ASSISTANT {
            return;
        }

        self.ui.messages[actual_index].content = new_text;
        self.invalidate_prewrap_cache();
        let user_display_name = self.persona_manager.get_display_name();
        let _ = self
            .session
            .logging
            .rewrite_log_without_last_response(&self.ui.messages, &user_display_name);
        self.ui.clear_assistant_editing();
    }

    pub fn complete_assistant_edit(&mut self, new_text: String) {
        if !self.ui.is_editing_assistant_message() {
            return;
        }

        self.ui.messages.push_back(Message {
            role: ROLE_ASSISTANT.to_string(),
            content: new_text,
        });
        self.invalidate_prewrap_cache();
        let user_display_name = self.persona_manager.get_display_name();
        let _ = self
            .session
            .logging
            .rewrite_log_without_last_response(&self.ui.messages, &user_display_name);
        self.ui.clear_assistant_editing();
    }

    pub fn request_exit(&mut self) {
        self.ui.exit_requested = true;
    }

    pub fn update_user_display_name(&mut self, name: String) {
        self.ui.update_user_display_name(name);
    }

    pub fn input_area_height(&self, width: u16) -> u16 {
        self.ui.calculate_input_area_height(width)
    }

    pub fn get_prewrapped_lines_cached(&mut self, width: u16) -> &Vec<Line<'static>> {
        self.ui.get_prewrapped_lines_cached(width)
    }

    pub fn get_prewrapped_span_metadata_cached(&mut self, width: u16) -> &Vec<Vec<SpanKind>> {
        self.ui.get_prewrapped_span_metadata_cached(width)
    }

    pub fn invalidate_prewrap_cache(&mut self) {
        self.ui.invalidate_prewrap_cache();
    }

    pub(crate) fn configure_textarea_appearance(&mut self) {
        self.ui.configure_textarea();
    }

    pub fn get_logging_status(&self) -> String {
        self.session.logging.get_status_string()
    }
}

fn apply_command_completion(
    ui: &mut crate::core::app::ui_state::UiState,
    completion: &str,
    remainder: &str,
    add_space: bool,
    term_width: u16,
) {
    let mut new_input = String::new();
    new_input.push('/');
    new_input.push_str(completion);

    let mut cursor_chars = 1 + completion.chars().count();

    match remainder.chars().next() {
        Some(ch) if ch.is_whitespace() => {
            new_input.push_str(remainder);
        }
        Some(_) => {
            if add_space {
                new_input.push(' ');
                cursor_chars += 1;
            }
            new_input.push_str(remainder);
        }
        None => {
            if add_space {
                new_input.push(' ');
                cursor_chars += 1;
            }
        }
    }

    ui.set_input_text_with_cursor(new_input, cursor_chars);
    ui.recompute_input_layout_after_edit(term_width);
}

fn longest_common_prefix(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }

    let mut prefix: Vec<char> = names[0].chars().collect();

    for name in &names[1..] {
        let mut new_len = 0;
        for (a, b) in prefix.iter().copied().zip(name.chars()) {
            if a == b {
                new_len += 1;
            } else {
                break;
            }
        }
        prefix.truncate(new_len);
        if prefix.is_empty() {
            break;
        }
    }

    prefix.into_iter().collect()
}

fn format_command_suggestions(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }

    let max_display = 6;
    let mut pieces: Vec<String> = names
        .iter()
        .take(max_display)
        .map(|name| format!("/{}", name))
        .collect();
    if names.len() > max_display {
        pieces.push("…".to_string());
    }
    pieces.join(", ")
}
