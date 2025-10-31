use super::App;
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

    pub fn complete_in_place_edit(&mut self, index: usize, new_text: String) {
        let Some(actual_index) = self.ui.take_in_place_edit_index() else {
            return;
        };

        if actual_index != index {
            return;
        }

        if actual_index >= self.ui.messages.len() || self.ui.messages[actual_index].role != "user" {
            return;
        }

        self.ui.messages[actual_index].content = new_text;
        self.invalidate_prewrap_cache();
        let user_display_name = self.persona_manager.get_display_name();
        let _ = self
            .session
            .logging
            .rewrite_log_without_last_response(&self.ui.messages, &user_display_name);
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
