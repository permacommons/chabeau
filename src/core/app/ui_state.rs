use crate::core::config::data::Config;
use crate::core::message::{AppMessageKind, Message, ROLE_ASSISTANT, ROLE_USER};
use crate::core::text_wrapping::{TextWrapper, WrapConfig, WrappedCursorLayout};
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use ratatui::prelude::Size;
use ratatui::text::Line;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tui_textarea::{CursorMove, TextArea};

/// Background activity being performed in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// Streaming a chat response from the API.
    ChatStream,

    /// Fetching the list of available models from the provider.
    ModelRequest,

    /// Refreshing MCP tools/resources/prompts.
    McpRefresh,
}

/// Type of file operation prompt being displayed.
#[derive(Debug, Clone)]
pub enum FilePromptKind {
    /// Dumping the full conversation to a file.
    Dump,

    /// Saving a specific code block to a file.
    SaveCodeBlock,
}

#[derive(Debug, Clone)]
pub struct FilePrompt {
    pub kind: FilePromptKind,
    pub content: Option<String>,
}

/// Tool permission prompt metadata.
#[derive(Debug, Clone)]
pub struct ToolPrompt {
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub args_summary: String,
}

#[derive(Debug, Clone)]
pub struct McpPromptArgument {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct McpPromptInput {
    pub server_id: String,
    pub server_name: String,
    pub prompt_name: String,
    pub prompt_title: Option<String>,
    pub pending_args: Vec<McpPromptArgument>,
    pub collected: std::collections::HashMap<String, String>,
    pub next_index: usize,
}

/// Target message type for edit-select operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSelectTarget {
    /// Select a user message for editing.
    User,

    /// Select an assistant message for editing.
    Assistant,
}

/// Current UI interaction mode.
#[derive(Debug, Clone)]
pub enum UiMode {
    /// Default typing mode for composing new messages.
    Typing,

    /// Selecting a message to edit (user or assistant).
    EditSelect {
        /// Index of the currently selected message.
        selected_index: usize,
        /// Whether selecting user or assistant messages.
        target: EditSelectTarget,
    },

    /// Selecting a code block to save.
    BlockSelect {
        /// Index of the selected code block.
        block_index: usize,
    },

    /// Editing a message in place within the transcript.
    InPlaceEdit {
        /// Index of the message being edited.
        index: usize,
    },

    /// Prompting for a file path (save or dump operation).
    FilePrompt(FilePrompt),

    /// Prompting for tool permission approval.
    ToolPrompt(ToolPrompt),

    /// Prompting for MCP prompt arguments.
    McpPromptInput(McpPromptInput),
}

/// Which UI pane currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFocus {
    /// Transcript area has focus (for scrolling).
    Transcript,

    /// Input area has focus (for typing).
    Input,
}

#[derive(Debug, Clone)]
struct InputLayoutCache {
    width: usize,
    revision: u64,
    layout: Arc<WrappedCursorLayout>,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub messages: VecDeque<Message>,
    input: String,
    input_cursor_position: usize,
    pub mode: UiMode,
    pub current_response: String,
    pub scroll_offset: u16,
    pub horizontal_scroll_offset: u16,
    pub auto_scroll: bool,
    pub is_streaming: bool,
    pub activity_indicator: Option<ActivityKind>,
    pub pulse_start: Instant,
    pub stream_interrupted: bool,
    pub input_scroll_offset: u16,
    textarea: TextArea<'static>,
    pub theme: Theme,
    pub current_theme_id: Option<String>,
    pub markdown_enabled: bool,
    pub syntax_enabled: bool,
    pub(crate) prewrap_cache: Option<PrewrapCache>,
    pub status: Option<String>,
    pub status_set_at: Option<Instant>,
    pub user_display_name: String,
    pub exit_requested: bool,
    pub print_transcript_on_exit: bool,
    pub compose_mode: bool,
    pub last_term_size: Size,
    pub focus: UiFocus,
    pub input_cursor_preferred_column: Option<usize>,
    editing_assistant_message: bool,
    input_layout_cache: Option<InputLayoutCache>,
    input_revision: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum VerticalCursorDirection {
    Up,
    Down,
}

impl UiState {
    pub fn is_input_active(&self) -> bool {
        matches!(
            self.mode,
            UiMode::Typing
                | UiMode::InPlaceEdit { .. }
                | UiMode::FilePrompt(_)
                | UiMode::ToolPrompt(_)
                | UiMode::McpPromptInput(_)
        )
    }

    pub fn focus_transcript(&mut self) {
        self.focus = UiFocus::Transcript;
    }

    pub fn focus_input(&mut self) {
        self.focus = UiFocus::Input;
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            UiFocus::Transcript => UiFocus::Input,
            UiFocus::Input => UiFocus::Transcript,
        };
    }

    pub fn is_input_focused(&self) -> bool {
        self.focus == UiFocus::Input
    }

    pub fn is_transcript_focused(&self) -> bool {
        self.focus == UiFocus::Transcript
    }

    fn bump_input_revision(&mut self) {
        self.input_revision = self.input_revision.wrapping_add(1);
        self.input_layout_cache = None;
    }

    pub fn in_edit_select_mode(&self) -> bool {
        matches!(self.mode, UiMode::EditSelect { .. })
    }

    pub fn edit_select_target(&self) -> Option<EditSelectTarget> {
        if let UiMode::EditSelect { target, .. } = self.mode {
            Some(target)
        } else {
            None
        }
    }

    pub fn selected_edit_message_index(&self) -> Option<usize> {
        if let UiMode::EditSelect { selected_index, .. } = self.mode {
            Some(selected_index)
        } else {
            None
        }
    }

    pub fn set_selected_edit_message_index(&mut self, index: usize) {
        if let UiMode::EditSelect { selected_index, .. } = &mut self.mode {
            *selected_index = index;
        }
    }

    pub fn selected_user_message_index(&self) -> Option<usize> {
        match self.mode {
            UiMode::EditSelect {
                selected_index,
                target: EditSelectTarget::User,
            } => Some(selected_index),
            _ => None,
        }
    }

    pub fn set_selected_user_message_index(&mut self, index: usize) {
        if let UiMode::EditSelect {
            selected_index,
            target: EditSelectTarget::User,
        } = &mut self.mode
        {
            *selected_index = index;
        }
    }

    pub fn selected_assistant_message_index(&self) -> Option<usize> {
        match self.mode {
            UiMode::EditSelect {
                selected_index,
                target: EditSelectTarget::Assistant,
            } => Some(selected_index),
            _ => None,
        }
    }

    pub fn set_selected_assistant_message_index(&mut self, index: usize) {
        if let UiMode::EditSelect {
            selected_index,
            target: EditSelectTarget::Assistant,
        } = &mut self.mode
        {
            *selected_index = index;
        }
    }

    pub fn in_block_select_mode(&self) -> bool {
        matches!(self.mode, UiMode::BlockSelect { .. })
    }

    pub fn selected_block_index(&self) -> Option<usize> {
        if let UiMode::BlockSelect { block_index } = self.mode {
            Some(block_index)
        } else {
            None
        }
    }

    pub fn set_selected_block_index(&mut self, index: usize) {
        if let UiMode::BlockSelect { block_index } = &mut self.mode {
            *block_index = index;
        }
    }

    fn last_message_index_with_role(&self, role: &str) -> Option<usize> {
        self.messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| m.role == role)
            .map(|(i, _)| i)
    }

    fn prev_message_index_with_role(&self, role: &str, from_index: usize) -> Option<usize> {
        if from_index == 0 {
            return None;
        }

        self.messages
            .iter()
            .enumerate()
            .take(from_index)
            .rev()
            .find(|(_, m)| m.role == role)
            .map(|(i, _)| i)
    }

    fn next_message_index_with_role(&self, role: &str, from_index: usize) -> Option<usize> {
        self.messages
            .iter()
            .enumerate()
            .skip(from_index + 1)
            .find(|(_, m)| m.role == role)
            .map(|(i, _)| i)
    }

    fn first_message_index_with_role(&self, role: &str) -> Option<usize> {
        self.messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.role == role)
            .map(|(i, _)| i)
    }

    pub fn last_user_message_index(&self) -> Option<usize> {
        self.last_message_index_with_role(ROLE_USER)
    }

    pub fn prev_user_message_index(&self, from_index: usize) -> Option<usize> {
        self.prev_message_index_with_role(ROLE_USER, from_index)
    }

    pub fn next_user_message_index(&self, from_index: usize) -> Option<usize> {
        self.next_message_index_with_role(ROLE_USER, from_index)
    }

    pub fn first_user_message_index(&self) -> Option<usize> {
        self.first_message_index_with_role(ROLE_USER)
    }

    pub fn last_assistant_message_index(&self) -> Option<usize> {
        self.last_message_index_with_role(ROLE_ASSISTANT)
    }

    pub fn prev_assistant_message_index(&self, from_index: usize) -> Option<usize> {
        self.prev_message_index_with_role(ROLE_ASSISTANT, from_index)
    }

    pub fn next_assistant_message_index(&self, from_index: usize) -> Option<usize> {
        self.next_message_index_with_role(ROLE_ASSISTANT, from_index)
    }

    pub fn first_assistant_message_index(&self) -> Option<usize> {
        self.first_message_index_with_role(ROLE_ASSISTANT)
    }

    pub fn enter_edit_select_mode(&mut self, target: EditSelectTarget) {
        self.clear_assistant_editing();
        let start_index = match target {
            EditSelectTarget::User => self.last_user_message_index(),
            EditSelectTarget::Assistant => self.last_assistant_message_index(),
        };

        if let Some(idx) = start_index {
            self.focus_transcript();
            self.set_mode(UiMode::EditSelect {
                selected_index: idx,
                target,
            });
        }
    }

    pub fn exit_edit_select_mode(&mut self) {
        if self.in_edit_select_mode() {
            self.set_mode(UiMode::Typing);
        }
    }

    pub fn start_in_place_edit(&mut self, index: usize) {
        self.focus_input();
        self.clear_assistant_editing();
        self.set_mode(UiMode::InPlaceEdit { index });
    }

    pub fn cancel_in_place_edit(&mut self) {
        if self.in_place_edit_index().is_some() {
            self.set_mode(UiMode::Typing);
            self.clear_assistant_editing();
        }
    }

    pub fn enter_block_select_mode(&mut self, index: usize) {
        self.focus_transcript();
        self.set_mode(UiMode::BlockSelect { block_index: index });
    }

    pub fn exit_block_select_mode(&mut self) {
        if self.in_block_select_mode() {
            self.set_mode(UiMode::Typing);
        }
    }

    pub fn in_place_edit_index(&self) -> Option<usize> {
        if let UiMode::InPlaceEdit { index } = self.mode {
            Some(index)
        } else {
            None
        }
    }

    pub fn take_in_place_edit_index(&mut self) -> Option<usize> {
        if let UiMode::InPlaceEdit { index } = self.mode {
            self.set_mode(UiMode::Typing);
            Some(index)
        } else {
            None
        }
    }

    pub fn toggle_compose_mode(&mut self) {
        self.compose_mode = !self.compose_mode;

        if self.last_term_size.width > 0 {
            let width = self.last_term_size.width;
            self.recompute_input_layout_after_edit(width);
        }
    }

    pub fn begin_activity(&mut self, kind: ActivityKind) {
        self.activity_indicator = Some(kind);
        self.pulse_start = Instant::now();
    }

    pub fn end_activity(&mut self, kind: ActivityKind) {
        if self.activity_indicator == Some(kind) {
            self.activity_indicator = None;
        }
    }

    pub fn is_activity_indicator_visible(&self) -> bool {
        self.activity_indicator.is_some()
    }

    pub fn begin_streaming(&mut self) {
        self.is_streaming = true;
        self.stream_interrupted = false;
        self.begin_activity(ActivityKind::ChatStream);
        self.focus_transcript();
    }

    pub fn end_streaming(&mut self) {
        self.is_streaming = false;
        if matches!(self.activity_indicator, Some(ActivityKind::ChatStream)) {
            self.activity_indicator = None;
        }
    }

    pub(crate) fn new_basic(
        theme: Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
        current_theme_id: Option<String>,
    ) -> Self {
        Self {
            messages: VecDeque::new(),
            input: String::new(),
            input_cursor_position: 0,
            mode: UiMode::Typing,
            current_response: String::new(),
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            activity_indicator: None,
            pulse_start: Instant::now(),
            stream_interrupted: false,
            input_scroll_offset: 0,
            textarea: TextArea::default(),
            theme,
            current_theme_id,
            markdown_enabled,
            syntax_enabled,
            prewrap_cache: None,
            status: None,
            status_set_at: None,
            user_display_name: "You".to_string(),
            exit_requested: false,
            print_transcript_on_exit: false,
            compose_mode: false,
            last_term_size: Size::default(),
            focus: UiFocus::Transcript,
            input_cursor_preferred_column: None,
            editing_assistant_message: false,
            input_layout_cache: None,
            input_revision: 0,
        }
    }

    pub(crate) fn from_config(theme: Theme, config: &Config) -> Self {
        Self::new_basic(
            theme,
            config.markdown.unwrap_or(true),
            config.syntax.unwrap_or(true),
            config.theme.clone(),
        )
    }

    pub(crate) fn configure_textarea(&mut self) {
        let textarea_style = self
            .theme
            .input_text_style
            .patch(ratatui::style::Style::default().bg(self.theme.background_color));
        self.textarea.set_style(textarea_style);
        self.textarea
            .set_cursor_style(self.theme.input_cursor_style);
        self.textarea
            .set_cursor_line_style(self.theme.input_cursor_line_style);
    }

    pub fn get_input_text(&self) -> &str {
        &self.input
    }

    pub fn get_input_cursor_position(&self) -> usize {
        self.input_cursor_position
    }

    pub fn get_textarea_cursor(&self) -> (usize, usize) {
        self.textarea.cursor()
    }

    pub fn get_textarea_line_count(&self) -> usize {
        self.textarea.lines().len()
    }

    pub fn get_textarea_line_len(&self, row: usize) -> usize {
        self.textarea
            .lines()
            .get(row)
            .map(|l| l.chars().count())
            .unwrap_or(0)
    }

    pub fn set_cursor_position(&mut self, pos: usize) {
        self.jump_cursor_to_position(pos);
        self.input_cursor_preferred_column = None;
    }

    pub fn set_input_text(&mut self, text: String) {
        self.editing_assistant_message = false;
        self.input = text;
        let lines: Vec<String> = if self.input.is_empty() {
            Vec::new()
        } else {
            self.input.split('\n').map(|s| s.to_string()).collect()
        };
        self.textarea = TextArea::from(lines);
        self.input_cursor_position = self.input.chars().count();
        self.input_cursor_preferred_column = None;
        if !self.input.is_empty() {
            let last_row = self.textarea.lines().len().saturating_sub(1) as u16;
            let last_col = self
                .textarea
                .lines()
                .last()
                .map(|l| l.chars().count() as u16)
                .unwrap_or(0);
            self.textarea
                .move_cursor(tui_textarea::CursorMove::Jump(last_row, last_col));
        }
        self.configure_textarea();
        self.bump_input_revision();
    }

    pub fn set_input_text_with_cursor(&mut self, text: String, cursor_pos: usize) {
        self.set_input_text(text);
        self.jump_cursor_to_position(cursor_pos);
        self.input_cursor_preferred_column = None;
    }

    fn jump_cursor_to_position(&mut self, cursor_pos: usize) {
        let line_lengths: Vec<usize> = self
            .textarea
            .lines()
            .iter()
            .map(|line| line.chars().count())
            .collect();

        if line_lengths.is_empty() {
            self.textarea.move_cursor(CursorMove::Jump(0, 0));
            self.sync_input_from_textarea();
            return;
        }

        let total_chars = self.input.chars().count();
        let clamped = cursor_pos.min(total_chars);

        let mut consumed = 0usize;
        let mut target_row = line_lengths.len().saturating_sub(1);
        let mut target_col = *line_lengths.last().unwrap_or(&0);

        for (index, len) in line_lengths.iter().enumerate() {
            if clamped <= consumed + len {
                target_row = index;
                target_col = clamped.saturating_sub(consumed);
                break;
            }
            consumed += len + 1;
        }

        self.textarea
            .move_cursor(CursorMove::Jump(target_row as u16, target_col as u16));
        self.sync_input_from_textarea();
    }

    pub fn clear_input(&mut self) {
        self.set_input_text(String::new());
    }

    pub fn set_input_text_for_assistant_edit(&mut self, text: String) {
        self.set_input_text(text);
        self.editing_assistant_message = true;
    }

    pub fn is_editing_assistant_message(&self) -> bool {
        self.editing_assistant_message
    }

    pub fn clear_assistant_editing(&mut self) {
        self.editing_assistant_message = false;
    }

    pub fn sync_input_from_textarea(&mut self) {
        let lines = self.textarea.lines();
        let new_input = lines.join("\n");
        let (row, col) = self.textarea.cursor();
        let mut pos = 0usize;
        for (i, line) in lines.iter().enumerate() {
            if i < row {
                pos += line.chars().count();
                pos += 1;
            } else if i == row {
                let line_len = line.chars().count();
                pos += col.min(line_len);
                break;
            }
        }
        let total_chars = new_input.chars().count();
        if row >= lines.len() {
            pos = total_chars;
        }
        let text_changed = new_input != self.input;
        self.input = new_input;
        self.input_cursor_position = pos;
        if text_changed {
            self.bump_input_revision();
        }
    }

    pub fn calculate_input_wrapped_lines(&self, width: u16) -> usize {
        if self.get_input_text().is_empty() {
            return 1;
        }

        let config = WrapConfig::new(width as usize);
        TextWrapper::cursor_layout(self.get_input_text(), &config).line_count()
    }

    pub fn calculate_input_area_height(&self, width: u16) -> u16 {
        if self.get_input_text().is_empty() {
            return 1;
        }

        let available_width = width.saturating_sub(5);
        let wrapped_lines = self.calculate_input_wrapped_lines(available_width);

        if wrapped_lines <= 1 && !self.get_input_text().contains('\n') {
            1
        } else {
            let max_height = if self.compose_mode {
                let half_height = self.last_term_size.height / 2;
                half_height.saturating_sub(2).max(2)
            } else {
                6
            };

            (wrapped_lines as u16).clamp(2, max_height)
        }
    }

    pub fn update_input_scroll(&mut self, input_area_height: u16, width: u16) {
        let available_width = width.saturating_sub(5) as usize;
        let layout = if let Some(layout) = self.ensure_wrapped_cursor_layout(width) {
            layout
        } else {
            Arc::new(TextWrapper::cursor_layout(
                self.get_input_text(),
                &WrapConfig::new(available_width),
            ))
        };
        let total_input_lines = layout.line_count() as u16;

        if total_input_lines <= input_area_height {
            self.input_scroll_offset = 0;
        } else {
            let cursor_line = layout.coordinates_for_index(self.input_cursor_position).0 as u16;

            if cursor_line < self.input_scroll_offset {
                self.input_scroll_offset = cursor_line;
            } else if cursor_line >= self.input_scroll_offset + input_area_height {
                self.input_scroll_offset = cursor_line.saturating_sub(input_area_height - 1);
            }

            let max_scroll = total_input_lines.saturating_sub(input_area_height);
            self.input_scroll_offset = self.input_scroll_offset.min(max_scroll);
        }
    }

    pub fn recompute_input_layout_after_edit(&mut self, terminal_width: u16) {
        let input_area_height = self.calculate_input_area_height(terminal_width);
        self.update_input_scroll(input_area_height, terminal_width);
    }

    pub fn apply_textarea_edit<F>(&mut self, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        f(&mut self.textarea);
        self.sync_input_from_textarea();
        self.input_cursor_preferred_column = None;
    }

    pub fn apply_textarea_edit_and_recompute<F>(&mut self, terminal_width: u16, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        self.apply_textarea_edit(f);
        self.recompute_input_layout_after_edit(terminal_width);
    }

    fn input_wrap_width(&self, terminal_width: u16) -> Option<usize> {
        let available_width = terminal_width.saturating_sub(5);
        if available_width == 0 {
            None
        } else {
            Some(available_width as usize)
        }
    }

    fn ensure_wrapped_cursor_layout(
        &mut self,
        terminal_width: u16,
    ) -> Option<Arc<WrappedCursorLayout>> {
        let wrap_width = self.input_wrap_width(terminal_width)?;

        let needs_refresh = match &self.input_layout_cache {
            Some(cache) => cache.width != wrap_width || cache.revision != self.input_revision,
            None => true,
        };

        if needs_refresh {
            let layout =
                TextWrapper::cursor_layout(self.get_input_text(), &WrapConfig::new(wrap_width));
            self.input_layout_cache = Some(InputLayoutCache {
                width: wrap_width,
                revision: self.input_revision,
                layout: Arc::new(layout),
            });
        }

        self.input_layout_cache
            .as_ref()
            .map(|cache| Arc::clone(&cache.layout))
    }

    fn wrapped_cursor_context(
        &mut self,
        terminal_width: u16,
    ) -> Option<(Arc<WrappedCursorLayout>, usize, usize, usize, usize)> {
        let char_count = self.get_input_text().chars().count();
        let current_position = self.input_cursor_position;
        let layout = self.ensure_wrapped_cursor_layout(terminal_width)?;
        let position_map = layout.position_map();
        if position_map.is_empty() {
            return Some((layout, 0, 0, 0, char_count));
        }

        let current_index = current_position.min(position_map.len().saturating_sub(1));
        let (current_line, current_col) = position_map[current_index];

        Some((layout, current_index, current_line, current_col, char_count))
    }

    pub fn move_cursor_in_wrapped_input(
        &mut self,
        terminal_width: u16,
        direction: VerticalCursorDirection,
    ) -> bool {
        if self.get_input_text().is_empty() {
            self.input_cursor_preferred_column = None;
            return false;
        }

        let Some((layout, current_index, current_line, current_col, char_count)) =
            self.wrapped_cursor_context(terminal_width)
        else {
            let before = self.textarea.cursor();
            match direction {
                VerticalCursorDirection::Up => self.textarea.move_cursor(CursorMove::Up),
                VerticalCursorDirection::Down => self.textarea.move_cursor(CursorMove::Down),
            }
            let after = self.textarea.cursor();
            self.sync_input_from_textarea();
            self.input_cursor_preferred_column = None;
            return before != after;
        };

        let desired_col = self.input_cursor_preferred_column.unwrap_or(current_col);
        let max_line = layout.line_count().saturating_sub(1);

        let target_line = match direction {
            VerticalCursorDirection::Up => {
                if current_line == 0 {
                    self.input_cursor_preferred_column = Some(desired_col);
                    return false;
                }
                current_line.saturating_sub(1)
            }
            VerticalCursorDirection::Down => {
                if current_line >= max_line {
                    self.input_cursor_preferred_column = Some(desired_col);
                    return false;
                }
                current_line.saturating_add(1)
            }
        };

        let Some(new_index) = layout.find_index_on_line(target_line, desired_col) else {
            self.input_cursor_preferred_column = Some(desired_col);
            return false;
        };

        let target_index = new_index.min(char_count);
        if target_index == current_index {
            self.input_cursor_preferred_column = Some(desired_col);
            return false;
        }

        self.jump_cursor_to_position(target_index);
        self.input_cursor_preferred_column = Some(desired_col);
        true
    }

    pub fn move_cursor_page_in_wrapped_input(
        &mut self,
        terminal_width: u16,
        direction: VerticalCursorDirection,
        steps: usize,
    ) -> bool {
        if steps == 0 || self.get_input_text().is_empty() {
            return false;
        }

        let mut moved = false;
        for _ in 0..steps {
            if self.move_cursor_in_wrapped_input(terminal_width, direction) {
                moved = true;
            } else {
                break;
            }
        }

        moved
    }

    pub fn move_cursor_to_visual_line_start(&mut self, terminal_width: u16) -> bool {
        if self.get_input_text().is_empty() {
            self.input_cursor_preferred_column = Some(0);
            return false;
        }

        let Some((layout, current_index, current_line, _, char_count)) =
            self.wrapped_cursor_context(terminal_width)
        else {
            let before = self.textarea.cursor();
            self.textarea.move_cursor(CursorMove::Head);
            let after = self.textarea.cursor();
            self.sync_input_from_textarea();
            self.input_cursor_preferred_column = Some(after.1);
            return before != after;
        };

        let Some((start, _)) = layout.line_bounds(current_line) else {
            self.input_cursor_preferred_column = Some(0);
            return false;
        };

        let target_index = start.min(char_count);
        if target_index == current_index {
            self.input_cursor_preferred_column = Some(0);
            return false;
        }

        self.jump_cursor_to_position(target_index);
        self.input_cursor_preferred_column = Some(0);
        true
    }

    pub fn move_cursor_to_visual_line_end(&mut self, terminal_width: u16) -> bool {
        if self.get_input_text().is_empty() {
            self.input_cursor_preferred_column = Some(0);
            return false;
        }

        let Some((layout, current_index, current_line, _, char_count)) =
            self.wrapped_cursor_context(terminal_width)
        else {
            let before = self.textarea.cursor();
            self.textarea.move_cursor(CursorMove::End);
            let after = self.textarea.cursor();
            self.sync_input_from_textarea();
            self.input_cursor_preferred_column = Some(after.1);
            return before != after;
        };

        let Some((_, end)) = layout.line_bounds(current_line) else {
            self.input_cursor_preferred_column = Some(0);
            return false;
        };

        let target_index = end.min(char_count);
        if target_index == current_index {
            let (_, col) = layout.coordinates_for_index(current_index);
            self.input_cursor_preferred_column = Some(col);
            return false;
        }

        let (_, col) = layout.coordinates_for_index(target_index);
        self.jump_cursor_to_position(target_index);
        self.input_cursor_preferred_column = Some(col);
        true
    }

    pub fn file_prompt(&self) -> Option<&FilePrompt> {
        if let UiMode::FilePrompt(ref prompt) = self.mode {
            Some(prompt)
        } else {
            None
        }
    }

    pub fn tool_prompt(&self) -> Option<&ToolPrompt> {
        if let UiMode::ToolPrompt(ref prompt) = self.mode {
            Some(prompt)
        } else {
            None
        }
    }

    pub fn mcp_prompt_input(&self) -> Option<&McpPromptInput> {
        if let UiMode::McpPromptInput(ref prompt) = self.mode {
            Some(prompt)
        } else {
            None
        }
    }

    pub fn start_file_prompt_dump(&mut self, filename: String) {
        self.focus_input();
        self.set_mode(UiMode::FilePrompt(FilePrompt {
            kind: FilePromptKind::Dump,
            content: None,
        }));
        self.set_input_text(filename);
    }

    pub fn start_file_prompt_save_block(&mut self, filename: String, content: String) {
        self.focus_input();
        self.set_mode(UiMode::FilePrompt(FilePrompt {
            kind: FilePromptKind::SaveCodeBlock,
            content: Some(content),
        }));
        self.set_input_text(filename);
    }

    pub fn cancel_file_prompt(&mut self) {
        if let UiMode::FilePrompt(_) = self.mode {
            self.set_mode(UiMode::Typing);
        }
        self.clear_input();
    }

    pub fn start_tool_prompt(
        &mut self,
        server_id: String,
        server_name: String,
        tool_name: String,
        args_summary: String,
    ) {
        self.focus_transcript();
        self.pulse_start = Instant::now();
        self.set_mode(UiMode::ToolPrompt(ToolPrompt {
            server_id,
            server_name,
            tool_name,
            args_summary,
        }));
    }

    pub fn start_mcp_prompt_input(&mut self, prompt: McpPromptInput) {
        self.focus_input();
        self.set_mode(UiMode::McpPromptInput(prompt));
        self.clear_input();
    }

    pub fn cancel_tool_prompt(&mut self) {
        if let UiMode::ToolPrompt(_) = self.mode {
            self.set_mode(UiMode::Typing);
        }
        self.focus_input();
    }

    pub fn cancel_mcp_prompt_input(&mut self) {
        if let UiMode::McpPromptInput(_) = self.mode {
            self.set_mode(UiMode::Typing);
        }
        self.clear_input();
    }

    /// Scroll to the very top of the output area and disable auto-scroll.
    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        self.scroll_offset = 0;
    }

    /// Scroll to the very bottom of the output area and enable auto-scroll.
    pub fn scroll_to_bottom_view(&mut self, available_height: u16, terminal_width: u16) {
        let max_scroll = self.calculate_max_scroll_offset(available_height, terminal_width);
        self.scroll_offset = max_scroll;
        self.auto_scroll = true;
    }

    /// Page up by one full output area (minus one line overlap). Disables auto-scroll.
    pub fn page_up(&mut self, available_height: u16) {
        self.auto_scroll = false;
        let step = available_height.saturating_sub(1);
        self.scroll_offset = self.scroll_offset.saturating_sub(step);
    }

    /// Page down by one full output area (minus one line overlap). Disables auto-scroll.
    pub fn page_down(&mut self, available_height: u16, terminal_width: u16) {
        self.auto_scroll = false;
        let step = available_height.saturating_sub(1);
        let max_scroll = self.calculate_max_scroll_offset(available_height, terminal_width);
        self.scroll_offset = (self.scroll_offset.saturating_add(step)).min(max_scroll);
    }

    pub fn calculate_wrapped_line_count(&mut self, terminal_width: u16) -> u16 {
        let lines = self.get_prewrapped_lines_cached(terminal_width);
        lines.len() as u16
    }

    pub fn calculate_max_scroll_offset(
        &mut self,
        available_height: u16,
        terminal_width: u16,
    ) -> u16 {
        let total = self.calculate_wrapped_line_count(terminal_width);
        if total > available_height {
            total.saturating_sub(available_height)
        } else {
            0
        }
    }

    pub fn get_prewrapped_lines_cached(&mut self, width: u16) -> &Vec<Line<'static>> {
        let theme_sig = compute_theme_signature(&self.theme);
        let markdown = self.markdown_enabled;
        let syntax = self.syntax_enabled;
        let msg_len = self.messages.len();
        let last_hash = hash_last_message(&self.messages);

        let mut can_reuse = false;
        let mut only_last_changed = false;
        if let Some(c) = &self.prewrap_cache {
            if c.width == width
                && c.markdown_enabled == markdown
                && c.syntax_enabled == syntax
                && c.theme_sig == theme_sig
                && c.messages_len == msg_len
            {
                if c.last_msg_hash == last_hash {
                    can_reuse = true;
                } else {
                    only_last_changed = true;
                }
            }
        }

        let layout_cfg = crate::ui::layout::LayoutConfig {
            width: Some(width as usize),
            markdown_enabled: markdown,
            syntax_enabled: syntax,
            table_overflow_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
            user_display_name: Some(self.user_display_name.clone()),
        };

        if can_reuse {
            // Up-to-date
        } else if only_last_changed {
            if let (Some(c), Some(last_msg)) = (self.prewrap_cache.as_mut(), self.messages.back()) {
                let mut last_only = VecDeque::with_capacity(1);
                last_only.push_back(last_msg.clone());
                let layout = crate::ui::layout::LayoutEngine::layout_messages(
                    &last_only,
                    &self.theme,
                    &layout_cfg,
                );
                splice_last_message_layout(c, layout, last_hash);
            } else {
                only_last_changed = false;
            }
        }

        if self.prewrap_cache.is_none() || (!can_reuse && !only_last_changed) {
            let layout = crate::ui::layout::LayoutEngine::layout_messages(
                &self.messages,
                &self.theme,
                &layout_cfg,
            );
            let last_span = layout.message_spans.last().cloned();
            let (last_start, last_len) = last_span
                .map(|span| (span.start, span.len))
                .unwrap_or((0, 0));
            let lines = layout.lines;
            let span_metadata = layout.span_metadata;
            self.prewrap_cache = Some(PrewrapCache {
                width,
                markdown_enabled: markdown,
                syntax_enabled: syntax,
                theme_sig,
                messages_len: msg_len,
                last_msg_hash: last_hash,
                lines,
                span_metadata,
                last_start,
                last_len,
            });
        }

        &self.prewrap_cache.as_ref().unwrap().lines
    }

    pub fn get_prewrapped_span_metadata_cached(&mut self, width: u16) -> &Vec<Vec<SpanKind>> {
        self.get_prewrapped_lines_cached(width);
        &self.prewrap_cache.as_ref().unwrap().span_metadata
    }

    pub fn invalidate_prewrap_cache(&mut self) {
        self.prewrap_cache = None;
    }

    pub fn update_user_display_name(&mut self, display_name: String) {
        if self.user_display_name != display_name {
            self.user_display_name = display_name;
            // Invalidate cache since user display name affects rendering
            self.invalidate_prewrap_cache();
        }
    }

    pub(crate) fn set_mode(&mut self, mode: UiMode) {
        self.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::{EditSelectTarget, UiFocus, UiMode, UiState};
    use crate::ui::theme::Theme;
    use crate::utils::test_utils::create_test_message;

    #[test]
    fn default_focus_is_transcript() {
        let ui = UiState::new_basic(Theme::dark_default(), true, true, None);
        assert_eq!(ui.focus, UiFocus::Transcript);
    }

    #[test]
    fn focus_transitions_round_trip() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);
        ui.focus_input();
        assert!(ui.is_input_focused());
        ui.focus_transcript();
        assert!(ui.is_transcript_focused());
        ui.toggle_focus();
        assert!(ui.is_input_focused());
    }

    #[test]
    fn begin_streaming_forces_transcript_focus() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);
        ui.focus_input();
        ui.begin_streaming();
        assert!(ui.is_transcript_focused());
    }

    #[test]
    fn enter_edit_select_mode_focuses_last_user_message() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);
        ui.messages
            .push_back(create_test_message("assistant", "ignore"));
        ui.messages.push_back(create_test_message("user", "first"));
        ui.messages
            .push_back(create_test_message("assistant", "still ignore"));
        ui.messages.push_back(create_test_message("user", "last"));

        ui.enter_edit_select_mode(EditSelectTarget::User);

        match ui.mode {
            UiMode::EditSelect {
                selected_index,
                target: EditSelectTarget::User,
            } => assert_eq!(selected_index, 3),
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn exit_edit_select_mode_returns_to_typing() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);
        ui.set_mode(UiMode::EditSelect {
            selected_index: 0,
            target: EditSelectTarget::User,
        });

        ui.exit_edit_select_mode();

        assert!(matches!(ui.mode, UiMode::Typing));
    }

    #[test]
    fn block_select_mode_transitions_round_trip() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);

        ui.enter_block_select_mode(2);
        match ui.mode {
            UiMode::BlockSelect { block_index } => assert_eq!(block_index, 2),
            other => panic!("expected block select mode, got {other:?}"),
        }

        ui.exit_block_select_mode();
        assert!(matches!(ui.mode, UiMode::Typing));
    }

    #[test]
    fn cancel_in_place_edit_returns_to_typing() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);

        ui.start_in_place_edit(1);
        assert!(matches!(ui.mode, UiMode::InPlaceEdit { index: 1 }));

        ui.cancel_in_place_edit();
        assert!(matches!(ui.mode, UiMode::Typing));
    }

    #[test]
    fn assistant_edit_flag_tracks_input_usage() {
        let mut ui = UiState::new_basic(Theme::dark_default(), true, true, None);

        assert!(!ui.is_editing_assistant_message());
        ui.set_input_text_for_assistant_edit("revise".into());
        assert!(ui.is_editing_assistant_message());

        ui.clear_input();
        assert!(!ui.is_editing_assistant_message());
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrewrapCache {
    width: u16,
    markdown_enabled: bool,
    syntax_enabled: bool,
    theme_sig: u64,
    messages_len: usize,
    last_msg_hash: u64,
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) span_metadata: Vec<Vec<SpanKind>>,
    last_start: usize,
    last_len: usize,
}

fn splice_last_message_layout(
    cache: &mut PrewrapCache,
    layout: crate::ui::layout::Layout,
    last_msg_hash: u64,
) {
    let start = cache.last_start;
    let mut new_lines: Vec<Line<'static>> = Vec::with_capacity(start + layout.lines.len());
    new_lines.extend_from_slice(&cache.lines[..start]);
    new_lines.extend_from_slice(&layout.lines);
    cache.lines = new_lines;

    // Find the maximum existing block index in the cache
    let mut max_existing_block_index = None;
    for line_meta in &cache.span_metadata[..start] {
        for kind in line_meta {
            if let Some(meta) = kind.code_block_meta() {
                max_existing_block_index = Some(
                    max_existing_block_index
                        .map(|max: usize| max.max(meta.block_index()))
                        .unwrap_or(meta.block_index()),
                );
            }
        }
    }

    // Renumber block indices in the new message to be globally unique
    let mut new_message_metadata = layout.span_metadata;
    if let Some(max_idx) = max_existing_block_index {
        let offset = max_idx + 1;

        for line_meta in &mut new_message_metadata {
            for kind in line_meta {
                if let SpanKind::CodeBlock(ref mut meta) = kind {
                    *meta = crate::ui::span::CodeBlockMeta::new(
                        meta.language().map(String::from),
                        meta.block_index() + offset,
                    );
                }
            }
        }
    }

    let mut new_meta: Vec<Vec<SpanKind>> = Vec::with_capacity(start + new_message_metadata.len());
    new_meta.extend_from_slice(&cache.span_metadata[..start]);
    new_meta.extend_from_slice(&new_message_metadata);
    cache.span_metadata = new_meta;

    let last_span = layout.message_spans.last().cloned().unwrap_or_default();
    cache.last_start = start;
    cache.last_len = last_span.len;
    cache.last_msg_hash = last_msg_hash;
}

fn hash_last_message(messages: &VecDeque<Message>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    if let Some(m) = messages.back() {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    h.finish()
}

fn compute_theme_signature(theme: &crate::ui::theme::Theme) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    format!("{:?}", theme.background_color).hash(&mut h);
    format!("{:?}", theme.md_codeblock_bg_color()).hash(&mut h);
    format!("{:?}", theme.user_text_style).hash(&mut h);
    format!("{:?}", theme.assistant_text_style).hash(&mut h);
    format!("{:?}", theme.app_message_style(AppMessageKind::Info)).hash(&mut h);
    format!("{:?}", theme.app_message_style(AppMessageKind::Warning)).hash(&mut h);
    format!("{:?}", theme.app_message_style(AppMessageKind::Error)).hash(&mut h);
    format!("{:?}", theme.app_message_style(AppMessageKind::Log)).hash(&mut h);
    h.finish()
}
