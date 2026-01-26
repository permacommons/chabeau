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
        let cursor = self.ui.get_input_cursor_position().min(chars.len());

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
            return self.complete_mcp_server_argument(term_width, &chars, cursor, command_end);
        }

        let typed: String = chars[1..cursor].iter().collect();
        let mut command_names: Vec<String> = matching_commands(&typed)
            .iter()
            .map(|command| command.name.to_string())
            .collect();
        let prompt_commands = self.matching_mcp_prompt_commands(&typed);
        command_names.extend(prompt_commands);

        if command_names.is_empty() {
            if !typed.is_empty() {
                self.conversation()
                    .set_status(format!("No command matches '/{}'", typed));
                return true;
            }
            return false;
        }

        let remainder: String = chars[command_end..].iter().collect();
        command_names.sort();
        command_names.dedup();
        let command_names_ref: Vec<&str> = command_names.iter().map(String::as_str).collect();

        if command_names_ref.len() == 1 {
            apply_command_completion(
                &mut self.ui,
                command_names_ref[0],
                &remainder,
                true,
                term_width,
            );
            return true;
        }

        let prefix = longest_common_prefix(&command_names_ref);
        if prefix.len() > typed.len() {
            apply_command_completion(&mut self.ui, &prefix, &remainder, false, term_width);
            return true;
        }

        let suggestions = format_command_suggestions(&command_names_ref);
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

    fn complete_mcp_server_argument(
        &mut self,
        term_width: u16,
        chars: &[char],
        cursor: usize,
        command_end: usize,
    ) -> bool {
        let command: String = chars[1..command_end].iter().collect();
        if !command.eq_ignore_ascii_case("mcp") {
            return false;
        }

        let mut arg_start = command_end;
        while arg_start < chars.len() && chars[arg_start].is_whitespace() {
            arg_start += 1;
        }

        if arg_start == chars.len() || cursor < arg_start {
            arg_start = cursor;
        }

        let mut arg_end = arg_start;
        while arg_end < chars.len() && !chars[arg_end].is_whitespace() {
            arg_end += 1;
        }

        if cursor > arg_end {
            return false;
        }

        let prefix: String = chars[arg_start..cursor].iter().collect();

        let mut server_ids: Vec<String> = self
            .mcp
            .servers()
            .map(|server| server.config.id.clone())
            .collect();
        server_ids.sort();
        server_ids.dedup();

        if server_ids.is_empty() {
            self.conversation()
                .set_status("No MCP servers configured.".to_string());
            return true;
        }

        let mut matches: Vec<&str> = server_ids
            .iter()
            .map(String::as_str)
            .filter(|name| name.starts_with(&prefix))
            .collect();
        matches.sort();

        if matches.is_empty() {
            if !prefix.is_empty() {
                self.conversation()
                    .set_status(format!("No MCP server matches '{}'", prefix));
                return true;
            }
            return false;
        }

        let before_arg: String = chars[..arg_start].iter().collect();
        let remainder: String = chars[arg_end..].iter().collect();

        if matches.len() == 1 {
            apply_argument_completion(
                &mut self.ui,
                &before_arg,
                matches[0],
                &remainder,
                true,
                term_width,
            );
            return true;
        }

        let common = longest_common_prefix(&matches);
        if common.len() > prefix.len() {
            apply_argument_completion(
                &mut self.ui,
                &before_arg,
                &common,
                &remainder,
                false,
                term_width,
            );
            return true;
        }

        let suggestions = format_mcp_server_suggestions(&matches);
        self.conversation()
            .set_status(format!("MCP servers: {}", suggestions));
        true
    }

    fn matching_mcp_prompt_commands(&self, prefix: &str) -> Vec<String> {
        let mut commands = Vec::new();
        for server in self.mcp.servers() {
            let Some(prompts) = &server.cached_prompts else {
                continue;
            };
            for prompt in &prompts.prompts {
                let command = format!("{}:{}", server.config.id, prompt.name);
                if command.starts_with(prefix) {
                    commands.push(command);
                }
            }
        }
        commands
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

fn apply_argument_completion(
    ui: &mut crate::core::app::ui_state::UiState,
    prefix: &str,
    completion: &str,
    remainder: &str,
    add_space: bool,
    term_width: u16,
) {
    let mut new_input = String::new();
    new_input.push_str(prefix);
    new_input.push_str(completion);

    let mut cursor_chars = prefix.chars().count() + completion.chars().count();

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

fn format_mcp_server_suggestions(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }

    let max_display = 6;
    let mut pieces: Vec<String> = names
        .iter()
        .take(max_display)
        .map(|name| name.to_string())
        .collect();
    if names.len() > max_display {
        pieces.push("…".to_string());
    }
    pieces.join(", ")
}
