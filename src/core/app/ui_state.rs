use crate::core::config::Config;
use crate::core::message::Message;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use ratatui::text::Line;
use std::collections::VecDeque;
use std::time::Instant;
use tui_textarea::TextArea;

#[derive(Debug, Clone)]
pub enum FilePromptKind {
    Dump,
    SaveCodeBlock,
}

#[derive(Debug, Clone)]
pub struct FilePrompt {
    pub kind: FilePromptKind,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub enum UiMode {
    Typing,
    EditSelect { selected_index: usize },
    BlockSelect { block_index: usize },
    InPlaceEdit { index: usize },
    FilePrompt(FilePrompt),
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub messages: VecDeque<Message>,
    pub input: String,
    pub input_cursor_position: usize,
    pub mode: UiMode,
    pub current_response: String,
    pub scroll_offset: u16,
    pub horizontal_scroll_offset: u16,
    pub auto_scroll: bool,
    pub is_streaming: bool,
    pub pulse_start: Instant,
    pub stream_interrupted: bool,
    pub input_scroll_offset: u16,
    pub textarea: TextArea<'static>,
    pub theme: Theme,
    pub current_theme_id: Option<String>,
    pub markdown_enabled: bool,
    pub syntax_enabled: bool,
    pub(crate) prewrap_cache: Option<PrewrapCache>,
    pub status: Option<String>,
    pub status_set_at: Option<Instant>,
    pub exit_requested: bool,
    pub compose_mode: bool,
}

impl UiState {
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
            exit_requested: false,
            compose_mode: false,
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

    pub fn set_input_text(&mut self, text: String) {
        self.input = text;
        let lines: Vec<String> = if self.input.is_empty() {
            Vec::new()
        } else {
            self.input.split('\n').map(|s| s.to_string()).collect()
        };
        self.textarea = TextArea::from(lines);
        self.input_cursor_position = self.input.chars().count();
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
    }

    pub fn clear_input(&mut self) {
        self.set_input_text(String::new());
    }

    pub fn sync_input_from_textarea(&mut self) {
        let lines = self.textarea.lines();
        self.input = lines.join("\n");
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
        if row >= lines.len() {
            pos = self.input.chars().count();
        }
        self.input_cursor_position = pos;
    }

    pub fn calculate_input_wrapped_lines(&self, width: u16) -> usize {
        if self.get_input_text().is_empty() {
            return 1;
        }

        let config = WrapConfig::new(width as usize);
        TextWrapper::count_wrapped_lines(self.get_input_text(), &config)
    }

    pub fn calculate_input_area_height(&self, width: u16) -> u16 {
        if self.get_input_text().is_empty() {
            return 1;
        }

        let available_width = width.saturating_sub(3);
        let wrapped_lines = self.calculate_input_wrapped_lines(available_width);

        if wrapped_lines <= 1 && !self.get_input_text().contains('\n') {
            1
        } else {
            (wrapped_lines as u16).clamp(2, 6)
        }
    }

    fn calculate_cursor_line_position(&self, available_width: usize) -> u16 {
        let config = WrapConfig::new(available_width);
        TextWrapper::calculate_cursor_line(
            self.get_input_text(),
            self.input_cursor_position,
            &config,
        ) as u16
    }

    pub fn update_input_scroll(&mut self, input_area_height: u16, width: u16) {
        let available_width = width.saturating_sub(3);
        let total_input_lines = self.calculate_input_wrapped_lines(available_width) as u16;

        if total_input_lines <= input_area_height {
            self.input_scroll_offset = 0;
        } else {
            let cursor_line = self.calculate_cursor_line_position(available_width as usize);

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
    }

    pub fn apply_textarea_edit_and_recompute<F>(&mut self, terminal_width: u16, f: F)
    where
        F: FnOnce(&mut TextArea<'static>),
    {
        self.apply_textarea_edit(f);
        self.recompute_input_layout_after_edit(terminal_width);
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
}

#[derive(Debug, Clone)]
pub(crate) struct PrewrapCache {
    width: u16,
    markdown_enabled: bool,
    syntax_enabled: bool,
    theme_sig: u64,
    messages_len: usize,
    last_msg_hash: u64,
    lines: Vec<Line<'static>>,
    span_metadata: Vec<Vec<SpanKind>>,
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

    let mut new_meta: Vec<Vec<SpanKind>> = Vec::with_capacity(start + layout.span_metadata.len());
    new_meta.extend_from_slice(&cache.span_metadata[..start]);
    new_meta.extend_from_slice(&layout.span_metadata);
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
    h.finish()
}
