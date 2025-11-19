#![allow(clippy::items_after_test_module)]
use crate::core::message::{self, AppMessageKind, Message, ROLE_ASSISTANT, ROLE_USER};
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;
#[path = "markdown_wrap.rs"]
mod wrap;
use wrap::wrap_spans_to_width_generic_shared;

mod table;
use table::TableRenderer;

#[cfg(test)]
pub mod test_fixtures;

#[derive(Clone, Debug)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

/// Description of a rendered message (line-based), used by the TUI renderer.
pub struct RenderedMessage {
    pub lines: Vec<Line<'static>>,
}

/// Extended render metadata used by the layout engine when downstream consumers
/// need per-message spans.
pub struct RenderedMessageDetails {
    pub lines: Vec<Line<'static>>,
    pub span_metadata: Option<Vec<Vec<SpanKind>>>,
}

type RenderedLinesWithMetadata = (Vec<Line<'static>>, Vec<Vec<SpanKind>>);

const MAX_LIST_HANGING_INDENT_WIDTH: usize = 32;

impl RenderedMessageDetails {
    pub fn into_rendered(self) -> RenderedMessage {
        RenderedMessage { lines: self.lines }
    }
}

pub fn render_message_markdown_details_with_policy_and_user_name(
    msg: &Message,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
    policy: crate::ui::layout::TableOverflowPolicy,
    user_display_name: Option<&str>,
) -> RenderedMessageDetails {
    let cfg = MessageRenderConfig::markdown(true, syntax_enabled)
        .with_span_metadata()
        .with_terminal_width(terminal_width, policy)
        .with_user_display_name(user_display_name.map(|s| s.to_string()));
    render_message_with_config(msg, theme, cfg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    User,
    Assistant,
    App(AppMessageKind),
}

impl RoleKind {
    fn from_message(msg: &Message) -> Self {
        if msg.role == ROLE_USER {
            RoleKind::User
        } else if msg.role == ROLE_ASSISTANT {
            RoleKind::Assistant
        } else if message::is_app_message_role(&msg.role) {
            RoleKind::App(message::app_message_kind_from_role(&msg.role))
        } else {
            RoleKind::Assistant
        }
    }
}

fn base_text_style(role: RoleKind, theme: &Theme) -> Style {
    match role {
        RoleKind::User => theme.user_text_style,
        RoleKind::Assistant => theme.md_paragraph_style(),
        RoleKind::App(kind) => theme.app_message_style(kind).text_style,
    }
}

/// Configuration for the higher level message renderer abstraction.
#[derive(Clone, Debug)]
pub struct MessageRenderConfig {
    pub markdown: bool,
    pub collect_span_metadata: bool,
    pub syntax_highlighting: bool,
    pub terminal_width: Option<usize>,
    pub table_policy: crate::ui::layout::TableOverflowPolicy,
    pub user_display_name: Option<String>,
}

impl MessageRenderConfig {
    pub fn markdown(markdown_enabled: bool, syntax_highlighting: bool) -> Self {
        if markdown_enabled {
            Self {
                markdown: true,
                collect_span_metadata: false,
                syntax_highlighting,
                terminal_width: None,
                table_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
                user_display_name: None,
            }
        } else {
            Self {
                markdown: false,
                collect_span_metadata: false,
                syntax_highlighting: false,
                terminal_width: None,
                table_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
                user_display_name: None,
            }
        }
    }

    pub fn with_span_metadata(mut self) -> Self {
        self.collect_span_metadata = true;
        self
    }

    pub fn with_terminal_width(
        mut self,
        width: Option<usize>,
        policy: crate::ui::layout::TableOverflowPolicy,
    ) -> Self {
        self.terminal_width = width;
        self.table_policy = policy;
        self
    }

    pub fn with_user_display_name(mut self, display_name: Option<String>) -> Self {
        self.user_display_name = display_name;
        self
    }
}

/// Configuration options for the markdown renderer abstraction.
#[derive(Clone, Debug, Default)]
struct MarkdownRendererConfig {
    collect_span_metadata: bool,
    syntax_highlighting: bool,
    width: Option<MarkdownWidthConfig>,
    user_display_name: Option<String>,
}

/// Width-aware configuration for optional wrapping and table layout.
#[derive(Clone, Copy, Debug)]
struct MarkdownWidthConfig {
    terminal_width: Option<usize>,
    table_policy: crate::ui::layout::TableOverflowPolicy,
}

pub fn render_message_with_config(
    msg: &Message,
    theme: &Theme,
    config: MessageRenderConfig,
) -> RenderedMessageDetails {
    let role = RoleKind::from_message(msg);
    let (mut lines, mut metadata) = if config.markdown {
        let renderer_config = MarkdownRendererConfig {
            collect_span_metadata: config.collect_span_metadata,
            syntax_highlighting: config.syntax_highlighting,
            width: Some(MarkdownWidthConfig {
                terminal_width: config.terminal_width,
                table_policy: config.table_policy,
            }),
            user_display_name: config.user_display_name.clone(),
        };
        MarkdownRenderer::new(role, &msg.content, theme, renderer_config).render()
    } else {
        render_plain_message(
            role,
            &msg.content,
            theme,
            config.collect_span_metadata,
            config.user_display_name.as_deref(),
        )
    };
    if !config.markdown {
        if let Some(width) = config.terminal_width {
            let width = width.min(u16::MAX as usize) as u16;
            let (wrapped_lines, wrapped_metadata) =
                crate::utils::scroll::ScrollCalculator::prewrap_lines_with_metadata(
                    &lines,
                    if config.collect_span_metadata {
                        Some(&metadata)
                    } else {
                        None
                    },
                    width,
                );
            lines = wrapped_lines;
            if config.collect_span_metadata {
                metadata = wrapped_metadata;
            } else {
                metadata = Vec::new();
            }
        }
    }
    RenderedMessageDetails {
        lines,
        span_metadata: if config.collect_span_metadata {
            Some(metadata)
        } else {
            None
        },
    }
}

struct MarkdownRenderer<'a> {
    role: RoleKind,
    content: &'a str,
    theme: &'a Theme,
    config: MarkdownRendererConfig,
    lines: Vec<Line<'static>>,
    span_metadata: Vec<Vec<SpanKind>>,
    current_spans: Vec<Span<'static>>,
    current_span_kinds: Vec<SpanKind>,
    style_stack: Vec<Style>,
    kind_stack: Vec<SpanKind>,
    list_stack: Vec<ListKind>,
    list_indent_stack: Vec<usize>,
    pending_list_indent: Option<usize>,
    /// Track which list items (at any nesting level) should have blank lines before them,
    /// indexed by their absolute position in the document (0-based)
    items_needing_blank_lines_before: std::collections::HashSet<usize>,
    /// Current item index across all nesting levels (increments for every list item encountered)
    current_item_index: usize,
    in_code_block: Option<String>,
    code_block_lines: Vec<String>,
    code_block_count: usize,
    table_renderer: Option<TableRenderer>,
    did_prefix: bool,
    app_prefix_indent: Option<String>,
}

impl<'a> MarkdownRenderer<'a> {
    /// Check if there's a blank line immediately before the given position in the content.
    fn has_blank_line_before(content: &str, pos: usize) -> bool {
        // Find the start of the line containing this position
        let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        if line_start <= 1 {
            return false; // No room for a previous line
        }

        // Get the content before the current line's newline
        let before_newline = line_start - 1;
        let prev_content = &content[..before_newline];

        // Find the previous line
        if let Some(prev_line_start) = prev_content.rfind('\n') {
            let prev_line = &prev_content[prev_line_start + 1..];
            prev_line.trim().is_empty()
        } else {
            // Only one line exists before current line; check if it's blank
            prev_content.trim().is_empty()
        }
    }

    /// Use pulldown-cmark's parser to find list items preceded by blank lines.
    /// Returns a set of item indices (0-based, in document order) that should have blank lines before them.
    fn find_items_needing_blank_lines(content: &str) -> std::collections::HashSet<usize> {
        let mut result = std::collections::HashSet::new();
        let parser = Parser::new_ext(content, Options::all()).into_offset_iter();
        let mut item_index = 0;

        for (event, range) in parser {
            if let Event::Start(Tag::Item) = event {
                if item_index > 0 && Self::has_blank_line_before(content, range.start) {
                    result.insert(item_index);
                }
                item_index += 1;
            }
        }

        result
    }

    fn new(
        role: RoleKind,
        content: &'a str,
        theme: &'a Theme,
        config: MarkdownRendererConfig,
    ) -> Self {
        let app_prefix_indent = if let RoleKind::App(kind) = role {
            let prefix = theme.app_message_style(kind).prefix.clone();
            let width = prefix.width().max(1);
            Some(" ".repeat(width))
        } else {
            None
        };
        Self {
            role,
            content,
            theme,
            config,
            lines: Vec::new(),
            span_metadata: Vec::new(),
            current_spans: Vec::new(),
            current_span_kinds: Vec::new(),
            style_stack: vec![base_text_style(role, theme)],
            kind_stack: vec![SpanKind::Text],
            list_stack: Vec::new(),
            list_indent_stack: Vec::new(),
            pending_list_indent: None,
            items_needing_blank_lines_before: Self::find_items_needing_blank_lines(content),
            current_item_index: 0,
            in_code_block: None,
            code_block_lines: Vec::new(),
            code_block_count: 0,
            table_renderer: None,
            did_prefix: !matches!(role, RoleKind::User | RoleKind::App(_)),
            app_prefix_indent,
        }
    }

    fn get_user_prefix(&self) -> String {
        match &self.config.user_display_name {
            Some(name) => format!("{}: ", name),
            None => "You: ".to_string(),
        }
    }

    fn ensure_role_prefix_or_indent(&mut self) {
        match self.role {
            RoleKind::User => {
                if !self.did_prefix {
                    let user_prefix = self.get_user_prefix();
                    self.push_span(
                        Span::styled(user_prefix, self.theme.user_prefix_style),
                        SpanKind::UserPrefix,
                    );
                    self.did_prefix = true;
                } else {
                    self.push_span(Span::raw(USER_CONTINUATION_INDENT), SpanKind::Text);
                }
            }
            RoleKind::App(kind) => {
                let style = self.theme.app_message_style(kind);
                if !self.did_prefix {
                    self.push_span(
                        Span::styled(style.prefix.clone(), style.prefix_style),
                        SpanKind::AppPrefix,
                    );
                    self.did_prefix = true;
                } else if let Some(indent) = self.app_prefix_indent.clone() {
                    self.push_span(Span::raw(indent), SpanKind::Text);
                }
            }
            RoleKind::Assistant => {}
        }
    }

    fn ensure_role_prefix_once(&mut self) {
        match self.role {
            RoleKind::User => {
                if !self.did_prefix {
                    let user_prefix = self.get_user_prefix();
                    self.push_span(
                        Span::styled(user_prefix, self.theme.user_prefix_style),
                        SpanKind::UserPrefix,
                    );
                    self.did_prefix = true;
                }
            }
            RoleKind::App(kind) => {
                if !self.did_prefix {
                    let style = self.theme.app_message_style(kind);
                    self.push_span(
                        Span::styled(style.prefix.clone(), style.prefix_style),
                        SpanKind::AppPrefix,
                    );
                    self.did_prefix = true;
                }
            }
            RoleKind::Assistant => {}
        }
    }

    fn render(mut self) -> RenderedLinesWithMetadata {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_MATH);
        options.insert(Options::ENABLE_GFM);
        options.insert(Options::ENABLE_SUPERSCRIPT);
        options.insert(Options::ENABLE_SUBSCRIPT);
        let parser = Parser::new_ext(self.content, options);
        let mut parser = parser.peekable();

        while let Some(event) = parser.next() {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {
                        if matches!(self.role, RoleKind::User | RoleKind::App(_)) {
                            self.ensure_role_prefix_or_indent();
                        }
                        if self.pending_list_indent.is_none() && !self.list_stack.is_empty() {
                            self.pending_list_indent = Some(self.current_list_indent_width());
                        }
                    }
                    Tag::Heading { level, .. } => {
                        self.flush_current_spans(true);
                        let style = self.theme.md_heading_style(level as u8);
                        if matches!(self.role, RoleKind::User | RoleKind::App(_)) {
                            self.ensure_role_prefix_once();
                        }
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::BlockQuote(_) => {
                        self.style_stack.push(self.theme.md_blockquote_style());
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::List(start) => {
                        self.list_stack.push(match start {
                            Some(n) => ListKind::Ordered(n),
                            None => ListKind::Unordered,
                        });
                        self.list_indent_stack.push(0);
                        self.pending_list_indent = None;
                    }
                    Tag::Item => {
                        // Check if this item (at any nesting level) needs a blank line before it
                        if self
                            .items_needing_blank_lines_before
                            .contains(&self.current_item_index)
                        {
                            self.push_empty_line();
                        }
                        self.current_item_index += 1;

                        self.flush_current_spans(true);
                        let marker = match self
                            .list_stack
                            .last()
                            .cloned()
                            .unwrap_or(ListKind::Unordered)
                        {
                            ListKind::Unordered => "- ".to_string(),
                            ListKind::Ordered(_) => {
                                if let Some(ListKind::Ordered(ref mut k)) =
                                    self.list_stack.last_mut()
                                {
                                    let cur = *k;
                                    *k += 1;
                                    format!("{}. ", cur)
                                } else {
                                    "1. ".to_string()
                                }
                            }
                        };
                        // Calculate indent from parent levels before updating current level
                        let parent_indent: usize = self
                            .list_indent_stack
                            .iter()
                            .take(self.list_indent_stack.len().saturating_sub(1))
                            .sum();
                        if let Some(indent) = self.list_indent_stack.last_mut() {
                            *indent = marker.width();
                        }
                        if matches!(self.role, RoleKind::User | RoleKind::App(_)) {
                            self.ensure_role_prefix_once();
                        }
                        self.pending_list_indent = Some(parent_indent);
                        self.push_span(
                            Span::styled(marker, self.theme.md_list_marker_style()),
                            SpanKind::Text,
                        );
                    }
                    Tag::CodeBlock(kind) => {
                        self.flush_current_spans(true);
                        self.in_code_block = Some(language_hint_from_codeblock_kind(kind));
                        self.code_block_lines.clear();
                    }
                    Tag::Emphasis => {
                        let style = self
                            .style_stack
                            .last()
                            .copied()
                            .unwrap_or_default()
                            .add_modifier(Modifier::ITALIC);
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::Strong => {
                        let style = self
                            .style_stack
                            .last()
                            .copied()
                            .unwrap_or_default()
                            .add_modifier(Modifier::BOLD);
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::Strikethrough => {
                        let style = self
                            .style_stack
                            .last()
                            .copied()
                            .unwrap_or_default()
                            .add_modifier(Modifier::DIM);
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::Superscript | Tag::Subscript => {
                        let style = self.style_stack.last().copied().unwrap_or_default();
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::Link { dest_url, .. } => {
                        self.style_stack.push(self.theme.md_link_style());
                        self.kind_stack.push(SpanKind::link(dest_url.as_ref()));
                    }
                    Tag::Image { dest_url, .. } => {
                        self.style_stack.push(self.theme.md_link_style());
                        self.kind_stack.push(SpanKind::link(dest_url.as_ref()));
                    }
                    Tag::Table(_) => {
                        self.flush_current_spans(true);
                        if self.config.width.is_some() {
                            self.table_renderer = Some(TableRenderer::new());
                        }
                    }
                    Tag::TableHead => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.start_header();
                        }
                    }
                    Tag::TableRow => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.start_row();
                        }
                    }
                    Tag::TableCell => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.start_cell();
                        }
                    }
                    _ => {}
                },
                Event::End(tag_end) => match tag_end {
                    TagEnd::Paragraph => {
                        self.flush_current_spans(true);
                        if self.list_stack.is_empty() {
                            // Outside lists, always add blank line after paragraph
                            self.push_empty_line();
                        } else {
                            // Inside a list - peek ahead to preserve blank lines before block elements
                            // Note: We DON'T add blank before Tag::List because the Tag::Item inside
                            // that nested list will add the blank line via our preprocessing.
                            // We only add blanks before blocks that don't have Tag::Item events.
                            let next_is_block = matches!(
                                parser.peek(),
                                Some(Event::Start(
                                    Tag::Paragraph
                                        | Tag::CodeBlock(_)
                                        | Tag::BlockQuote(_)
                                        | Tag::Heading { .. }
                                ))
                            );
                            if next_is_block {
                                self.push_empty_line();
                            }
                            // Otherwise suppress blank line (our preprocessing handles item-level spacing)
                        }
                    }
                    TagEnd::Heading(_) => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::BlockQuote(_) => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::List(_) => {
                        self.flush_current_spans(true);

                        // Add blank line when outermost list ends
                        if self.list_stack.len() == 1 {
                            self.push_empty_line();
                        }

                        self.list_stack.pop();
                        self.list_indent_stack.pop();
                        self.pending_list_indent = None;
                    }
                    TagEnd::Item => {
                        self.flush_current_spans(true);
                        self.pending_list_indent = None;
                    }
                    TagEnd::CodeBlock => {
                        self.finalize_code_block();
                    }
                    TagEnd::Emphasis
                    | TagEnd::Strong
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Image
                    | TagEnd::Superscript
                    | TagEnd::Subscript => {
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::Table => {
                        if let Some(table) = self.table_renderer.take() {
                            if let Some(width_cfg) = self.config.width {
                                let table_lines = table.finalize(
                                    self.theme,
                                    width_cfg.terminal_width,
                                    width_cfg.table_policy,
                                );
                                for (line, kinds) in table_lines {
                                    self.push_line_direct(line, kinds);
                                }
                                self.push_empty_line();
                            }
                        }
                    }
                    TagEnd::TableHead => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.end_header();
                        }
                    }
                    TagEnd::TableRow => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.end_row();
                        }
                    }
                    TagEnd::TableCell => {
                        if let Some(ref mut table) = self.table_renderer {
                            table.end_cell();
                        }
                    }
                    _ => {}
                },
                Event::Text(text) => {
                    if self.in_code_block.is_some() {
                        push_codeblock_text(&mut self.code_block_lines, &text);
                    } else {
                        let span = Span::styled(
                            detab(&text),
                            *self
                                .style_stack
                                .last()
                                .unwrap_or(&base_text_style(self.role, self.theme)),
                        );
                        let kind = self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        if let Some(ref mut table) = self.table_renderer {
                            table.add_span(span, kind);
                        } else {
                            self.push_span(span, kind);
                        }
                    }
                }
                Event::Code(code) => {
                    let span = Span::styled(detab(&code), self.theme.md_inline_code_style());
                    let kind = self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                    if let Some(ref mut table) = self.table_renderer {
                        table.add_span(span, kind);
                    } else {
                        self.push_span(span, kind);
                    }
                }
                Event::InlineMath(math) | Event::DisplayMath(math) => {
                    let span = Span::styled(detab(&math), self.theme.md_inline_code_style());
                    let kind = self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                    if let Some(ref mut table) = self.table_renderer {
                        table.add_span(span, kind);
                    } else {
                        self.push_span(span, kind);
                    }
                }
                Event::SoftBreak => {
                    self.flush_current_spans(true);
                    if !self.list_stack.is_empty() {
                        self.pending_list_indent = Some(self.current_list_indent_width());
                    }
                    if matches!(self.role, RoleKind::User | RoleKind::App(_)) && self.did_prefix {
                        match self.role {
                            RoleKind::User => {
                                self.push_span(Span::raw(USER_CONTINUATION_INDENT), SpanKind::Text);
                            }
                            RoleKind::App(_) => {
                                if let Some(indent) = self.app_prefix_indent.clone() {
                                    self.push_span(Span::raw(indent), SpanKind::Text);
                                }
                            }
                            RoleKind::Assistant => {}
                        }
                    }
                }
                Event::HardBreak => {
                    self.flush_current_spans(true);
                    if !self.list_stack.is_empty() {
                        self.pending_list_indent = Some(self.current_list_indent_width());
                    }
                }
                Event::Rule => {
                    self.flush_current_spans(true);
                    self.push_horizontal_rule();
                    self.push_empty_line();
                }
                Event::TaskListMarker(_checked) => {
                    self.push_span(
                        Span::styled("[ ] ", self.theme.md_list_marker_style()),
                        SpanKind::Text,
                    );
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    if let Some(ref mut table) = self.table_renderer {
                        let trimmed = html.trim();
                        if trimmed == "<br>" || trimmed == "<br/>" {
                            table.new_line_in_cell();
                        }
                    }
                }
                Event::FootnoteReference(_) => {}
            }
        }

        self.flush_current_spans(true);
        if !self.lines.is_empty()
            && self
                .lines
                .last()
                .map(|l| !l.to_string().is_empty())
                .unwrap_or(false)
        {
            self.push_empty_line();
        }

        let metadata = if self.config.collect_span_metadata {
            self.span_metadata
        } else {
            Vec::new()
        };

        (self.lines, metadata)
    }

    fn push_span(&mut self, span: Span<'static>, kind: SpanKind) {
        if self.current_spans.is_empty() {
            if let Some(indent) = self.pending_list_indent.take() {
                if indent > 0 {
                    self.current_spans.push(Span::raw(" ".repeat(indent)));
                    self.current_span_kinds.push(SpanKind::Text);
                }
            }
        }
        self.current_spans.push(span);
        self.current_span_kinds.push(kind);
    }

    fn flush_current_spans(&mut self, indent_user_wraps: bool) {
        if self.current_spans.is_empty() {
            return;
        }

        if let Some(width_cfg) = self.config.width {
            if let Some(width) = width_cfg.terminal_width {
                let zipped: Vec<(Span<'static>, SpanKind)> = self
                    .current_spans
                    .iter()
                    .cloned()
                    .zip(self.current_span_kinds.iter().cloned())
                    .collect();
                let hanging_indent = if self.list_stack.is_empty() {
                    0
                } else {
                    self.current_list_indent_width()
                        .min(MAX_LIST_HANGING_INDENT_WIDTH)
                };
                let indent_wrapped_user_lines =
                    indent_user_wraps && matches!(self.role, RoleKind::User | RoleKind::App(_));
                let continuation_indent_width = if indent_wrapped_user_lines {
                    hanging_indent + self.role_continuation_indent_width()
                } else {
                    hanging_indent
                };
                let wrapped =
                    wrap_spans_to_width_generic_shared(&zipped, width, continuation_indent_width);
                for (idx, segs) in wrapped.into_iter().enumerate() {
                    let (mut spans_only, mut kinds_only): (Vec<_>, Vec<_>) =
                        segs.into_iter().unzip();
                    if idx > 0 && hanging_indent > 0 {
                        spans_only.insert(0, Span::raw(" ".repeat(hanging_indent)));
                        kinds_only.insert(0, SpanKind::Text);
                    }
                    if idx > 0 && indent_wrapped_user_lines {
                        let indent_span = match self.role {
                            RoleKind::User => Span::raw(USER_CONTINUATION_INDENT),
                            RoleKind::App(_) => Span::raw(
                                self.app_prefix_indent.clone().unwrap_or_else(|| " ".into()),
                            ),
                            RoleKind::Assistant => Span::raw(""),
                        };
                        spans_only.insert(0, indent_span);
                        kinds_only.insert(0, SpanKind::Text);
                    }
                    self.push_line(spans_only, kinds_only);
                }
                self.current_spans.clear();
                self.current_span_kinds.clear();
                return;
            }
        }

        let spans = std::mem::take(&mut self.current_spans);
        let kinds = std::mem::take(&mut self.current_span_kinds);
        self.push_line(spans, kinds);
    }

    fn push_line(&mut self, spans: Vec<Span<'static>>, kinds: Vec<SpanKind>) {
        let line = Line::from(spans);
        self.push_line_direct(line, kinds);
    }

    fn push_line_direct(&mut self, line: Line<'static>, kinds: Vec<SpanKind>) {
        if self.config.collect_span_metadata {
            self.span_metadata.push(kinds);
        }
        self.lines.push(line);
    }

    fn push_empty_line(&mut self) {
        self.push_line(Vec::new(), Vec::new());
    }

    fn push_horizontal_rule(&mut self) {
        let available_width = self
            .config
            .width
            .and_then(|cfg| cfg.terminal_width)
            .unwrap_or(80)
            .max(1);
        let target_width = ((available_width as f32) * 0.8).round() as usize;
        let rule_width = target_width.clamp(1, available_width);
        let padding = available_width.saturating_sub(rule_width);
        let left_padding = padding / 2;
        let right_padding = padding.saturating_sub(left_padding);

        let mut spans = Vec::new();
        let mut kinds = Vec::new();

        if left_padding > 0 {
            spans.push(Span::raw(" ".repeat(left_padding)));
            kinds.push(SpanKind::Text);
        }

        spans.push(Span::styled(
            "─".repeat(rule_width),
            self.theme.md_rule_style(),
        ));
        kinds.push(SpanKind::Text);

        if right_padding > 0 {
            spans.push(Span::raw(" ".repeat(right_padding)));
            kinds.push(SpanKind::Text);
        }

        self.push_line(spans, kinds);
    }

    fn finalize_code_block(&mut self) {
        let list_indent = self.current_list_indent_width();
        let metadata = if self.config.collect_span_metadata {
            Some(&mut self.span_metadata)
        } else {
            None
        };
        flush_code_block_buffer(
            &mut self.code_block_lines,
            self.config.syntax_highlighting,
            self.in_code_block.as_deref(),
            self.theme,
            &mut self.lines,
            metadata,
            list_indent,
            self.code_block_count,
        );
        self.code_block_count += 1;
        self.push_empty_line();
        self.in_code_block = None;
        self.pending_list_indent = (list_indent > 0).then_some(list_indent);
    }

    fn current_list_indent_width(&self) -> usize {
        self.list_indent_stack.iter().sum()
    }

    fn role_continuation_indent_width(&self) -> usize {
        match self.role {
            RoleKind::User => USER_CONTINUATION_INDENT.width(),
            RoleKind::App(_) => self
                .app_prefix_indent
                .as_deref()
                .map(UnicodeWidthStr::width)
                .unwrap_or(1),
            RoleKind::Assistant => 0,
        }
    }
}

/// Provides only content and optional language hint for each code block, in order of appearance.
///
/// # Deprecated
///
/// Use [`crate::ui::span::extract_code_block_content`] with cached metadata instead.
/// This function re-parses all markdown on every call, which is inefficient.
#[deprecated(
    since = "0.6.1",
    note = "Use crate::ui::span::extract_code_block_content with cached metadata instead"
)]
pub fn compute_codeblock_contents_with_lang(
    messages: &VecDeque<crate::core::message::Message>,
) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for msg in messages {
        if message::is_app_message_role(&msg.role) {
            continue;
        }
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_MATH);
        options.insert(Options::ENABLE_GFM);
        options.insert(Options::ENABLE_SUPERSCRIPT);
        options.insert(Options::ENABLE_SUBSCRIPT);
        let parser = Parser::new_ext(&msg.content, options);
        let mut in_code_block: Option<String> = None;
        let mut buf: Vec<String> = Vec::new();
        for ev in parser {
            match ev {
                Event::Start(Tag::CodeBlock(kind)) => {
                    in_code_block = Some(language_hint_from_codeblock_kind(kind));
                    buf.clear();
                }
                Event::End(TagEnd::CodeBlock) => {
                    let content = buf.join("\n");
                    let lang = in_code_block.as_ref().and_then(|s| {
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.clone())
                        }
                    });
                    out.push((content, lang));
                    in_code_block = None;
                }
                Event::Text(text) => {
                    if in_code_block.is_some() {
                        push_codeblock_text(&mut buf, &text);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::{
        render_message_markdown_details_with_policy_and_user_name, render_message_with_config,
        table::TableRenderer, MarkdownRenderer, MarkdownRendererConfig, MarkdownWidthConfig,
        MessageRenderConfig, RoleKind,
    };
    use crate::core::message::Message;
    use crate::ui::span::SpanKind;
    use crate::utils::test_utils::SAMPLE_HYPERTEXT_PARAGRAPH;
    use pulldown_cmark::{Options, Parser};
    use ratatui::style::Modifier;
    use ratatui::text::Span;
    use std::collections::VecDeque;
    use unicode_width::UnicodeWidthStr;

    fn render_markdown_for_test(
        message: &Message,
        theme: &crate::ui::theme::Theme,
        syntax_enabled: bool,
        width: Option<usize>,
    ) -> super::RenderedMessage {
        let cfg = MessageRenderConfig::markdown(true, syntax_enabled)
            .with_terminal_width(width, crate::ui::layout::TableOverflowPolicy::WrapCells);
        render_message_with_config(message, theme, cfg).into_rendered()
    }

    #[test]
    fn markdown_details_metadata_matches_lines_and_tags() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "Testing metadata with a [link](https://example.com) inside.".into(),
        };

        let details = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.as_ref().expect("metadata present");
        assert_eq!(metadata.len(), details.lines.len());
        let mut saw_link = false;
        for (line, kinds) in details.lines.iter().zip(metadata.iter()) {
            assert_eq!(line.spans.len(), kinds.len());
            for kind in kinds {
                if let Some(href) = kind.link_href() {
                    saw_link = true;
                    assert_eq!(href, "https://example.com");
                }
            }
        }
        assert!(saw_link, "expected link metadata to be captured");

        let width = Some(24usize);
        let details_with_width = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            width,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata_wrapped = details_with_width
            .span_metadata
            .as_ref()
            .expect("metadata present for width-aware render");
        assert_eq!(metadata_wrapped.len(), details_with_width.lines.len());
        for (line, kinds) in details_with_width.lines.iter().zip(metadata_wrapped.iter()) {
            assert_eq!(line.spans.len(), kinds.len());
        }
    }

    #[test]
    fn markdown_images_emit_clickable_links() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content:
                "Look at this sketch: ![diagram](https://example.com/diagram.png) neat, right?"
                    .into(),
        };

        let cfg = MessageRenderConfig::markdown(true, false).with_span_metadata();
        let details = render_message_with_config(&message, &theme, cfg);
        let metadata = details.span_metadata.expect("metadata present");
        let mut saw_image_link = false;
        for kinds in metadata {
            for kind in kinds {
                if let Some(meta) = kind.link_meta() {
                    if meta.href() == "https://example.com/diagram.png" {
                        saw_image_link = true;
                    }
                }
            }
        }

        assert!(
            saw_image_link,
            "expected image alt text to emit a hyperlink"
        );

        let rendered_text = details
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(rendered_text.contains("diagram"));
    }

    #[test]
    fn horizontal_rules_render_as_centered_lines() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "Above\n\n---\n\nBelow".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, Some(50));
        let hr_line = rendered
            .lines
            .iter()
            .find(|line| line.to_string().contains('─'))
            .expect("horizontal rule should render");

        let hr_text = hr_line.to_string();
        assert_eq!(UnicodeWidthStr::width(hr_text.as_str()), 50);

        let hr_chars: Vec<char> = hr_text.chars().collect();
        let first_rule_idx = hr_chars
            .iter()
            .position(|c| *c == '─')
            .expect("rule characters present");
        let rule_len = hr_chars[first_rule_idx..]
            .iter()
            .take_while(|c| **c == '─')
            .count();
        let right_padding = hr_chars.len().saturating_sub(first_rule_idx + rule_len);

        assert_eq!(first_rule_idx, 5);
        assert_eq!(rule_len, 40);
        assert_eq!(right_padding, 5);

        let rule_span = hr_line
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains('─'))
            .expect("rule span present");
        assert_eq!(rule_span.style, theme.md_rule_style());
    }

    #[test]
    fn wrapped_list_items_align_under_text() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- Parent item that wraps within the width budget and keeps alignment.\n  - Child item that wraps nicely under its parent alignment requirement.\n    - Grandchild entry that wraps and keeps deeper indentation consistent.".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, Some(28));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let parent_idx = lines
            .iter()
            .position(|l| l.starts_with("- Parent item"))
            .expect("parent line present");
        let parent_continuation = &lines[parent_idx + 1];
        assert!(
            !parent_continuation.trim().is_empty()
                && !parent_continuation.trim_start().starts_with('-')
        );
        assert_eq!(
            parent_continuation
                .chars()
                .take_while(|c| c.is_whitespace())
                .count(),
            2
        );

        let child_idx = lines
            .iter()
            .position(|l| l.contains("Child item that wraps"))
            .expect("child line present");
        let child_continuation = &lines[child_idx + 1];
        assert!(
            !child_continuation.trim().is_empty()
                && !child_continuation.trim_start().starts_with('-')
        );
        assert_eq!(
            child_continuation
                .chars()
                .take_while(|c| c.is_whitespace())
                .count(),
            4
        );

        let grandchild_idx = lines
            .iter()
            .position(|l| l.contains("Grandchild entry"))
            .expect("grandchild line present");
        let grandchild_continuation = &lines[grandchild_idx + 1];
        assert!(
            !grandchild_continuation.trim().is_empty()
                && !grandchild_continuation.trim_start().starts_with('-')
        );
        assert_eq!(
            grandchild_continuation
                .chars()
                .take_while(|c| c.is_whitespace())
                .count(),
            6
        );
    }

    #[test]
    fn superscript_and_subscript_render_without_markers() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "Subscripts: ~abc~ alongside superscripts: ^def^.".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        assert!(
            lines.len() >= 2,
            "expected rendered output to include paragraph and trailing blank line"
        );
        assert_eq!(lines[0], "Subscripts: abc alongside superscripts: def.");
        assert!(
            lines[1].is_empty(),
            "renderer should emit blank line after paragraph"
        );
    }

    #[test]
    fn gfm_callout_blockquotes_render_content() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "> [!NOTE]\n> Always document parser upgrades.".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        assert!(
            lines.len() >= 3,
            "expected callout blockquote to render with trailing spacing"
        );
        assert_eq!(lines[0], "Always document parser upgrades.");
        assert!(
            lines[1].is_empty(),
            "blockquote rendering should emit a separating blank line"
        );
    }

    #[test]
    fn metadata_marks_user_prefix() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "user".into(),
            content: "Hello world".into(),
        };

        let details = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );

        let metadata = details.span_metadata.expect("metadata present");
        assert!(!metadata.is_empty());
        let first_line = &metadata[0];
        assert!(!first_line.is_empty());
        assert!(matches!(first_line[0], SpanKind::UserPrefix));
        assert!(first_line.iter().skip(1).all(|k| k.is_text()));
    }

    #[test]
    fn metadata_marks_app_prefix() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message::app_info("Heads up");

        let details = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );

        let metadata = details.span_metadata.expect("metadata present");
        assert!(!metadata.is_empty());
        let first_line = &metadata[0];
        assert!(!first_line.is_empty());
        assert!(matches!(first_line[0], SpanKind::AppPrefix));
        assert!(first_line.iter().skip(1).all(|k| k.is_text()));
    }

    #[test]
    fn metadata_marks_table_links() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r"| Label | Value |
|-------|-------|
| Mixed | plain text and [Example](https://example.com) with trailing words |
"
            .into(),
        };

        let details = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            Some(50),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata present");

        let mut saw_link = false;
        let mut saw_plain = false;
        for (line, kinds) in details.lines.iter().zip(metadata.iter()) {
            let mut line_has_link = false;
            let mut line_has_plain = false;
            for (span, kind) in line.spans.iter().zip(kinds.iter()) {
                let content = span.content.as_ref();
                if matches!(kind, SpanKind::Link(_)) && content.contains("Example") {
                    saw_link = true;
                    line_has_link = true;
                    if let Some(href) = kind.link_href() {
                        assert_eq!(href, "https://example.com");
                    }
                }
                if kind.is_text() && content.chars().any(|ch| ch.is_alphanumeric()) {
                    saw_plain = true;
                    line_has_plain = true;
                }
            }
            if line_has_link {
                assert!(
                    line_has_plain,
                    "expected plain text metadata to accompany link within the same table line",
                );
            }
        }
        assert!(
            saw_link,
            "expected to observe link metadata within table cell"
        );
        assert!(
            saw_plain,
            "expected to observe non-link text metadata within table cell"
        );
    }

    #[test]
    fn shared_renderer_with_metadata_matches_details_wrapper() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content:
                "A [link](https://example.com) and a code block.\n\n```rust\nfn main() {}\n```"
                    .into(),
        };

        let expected = render_message_markdown_details_with_policy_and_user_name(
            &message,
            &theme,
            true,
            Some(48),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );

        let (lines, metadata) = MarkdownRenderer::new(
            RoleKind::Assistant,
            &message.content,
            &theme,
            MarkdownRendererConfig {
                collect_span_metadata: true,
                syntax_highlighting: true,
                width: Some(MarkdownWidthConfig {
                    terminal_width: Some(48),
                    table_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
                }),
                user_display_name: None,
            },
        )
        .render();

        assert_eq!(expected.lines, lines);
        let expected_metadata = expected
            .span_metadata
            .expect("details wrapper should provide metadata");
        assert_eq!(expected_metadata, metadata);
    }

    #[test]
    fn ordered_list_item_code_block_is_indented_under_marker() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "1. Intro text\n\n   ```\n   fn greet() {}\n   ```\n\n   Follow up text"
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let bullet_line = rendered
            .lines
            .iter()
            .find(|line| line.to_string().contains("Intro text"))
            .expect("bullet line present");
        let bullet_marker_width = bullet_line
            .spans
            .first()
            .map(|span| span.content.as_ref().width())
            .expect("marker span present");

        let code_line = rendered
            .lines
            .iter()
            .find(|line| line.to_string().contains("fn greet() {}"))
            .expect("code block line present");
        let indent_span = code_line.spans.first().expect("indent span present");

        assert!(indent_span.content.as_ref().chars().all(|ch| ch == ' '));
        assert_eq!(indent_span.content.as_ref().width(), bullet_marker_width);

        let follow_up_line = rendered
            .lines
            .iter()
            .find(|line| line.to_string().contains("Follow up text"))
            .expect("follow up text present");
        let follow_up_indent = follow_up_line
            .spans
            .first()
            .expect("indent span present")
            .content
            .as_ref();
        assert!(follow_up_indent.chars().all(|ch| ch == ' '));
        assert_eq!(follow_up_indent.width(), bullet_marker_width);
    }

    #[test]
    fn multi_item_ordered_list_keeps_code_block_with_correct_item() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "1. **Open a new terminal** on your local machine (keeping your SSH session open) and run `scp` as above.\n2. **Use `scp` in reverse** from the remote side *to* your local machine (if remote can reach your local machine and SSH is accessible), e.g.:\n   ```bash\n   scp /path/to/file you@your_local_IP:/path/to/local/destination/\n   ```\n   But this only works if your local machine is running an SSH server and is network-reachable — rarely the case.\n3. **Use `rsync` over SSH** similarly to `scp`."
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let bullet_two_index = rendered
            .lines
            .iter()
            .position(|line| line.to_string().starts_with("2. "))
            .expect("bullet two present");

        let bullet_two_indent = rendered.lines[bullet_two_index]
            .spans
            .first()
            .map(|span| span.content.as_ref().width())
            .expect("bullet two span");

        let code_block_index = rendered
            .lines
            .iter()
            .enumerate()
            .find_map(|(idx, line)| {
                if line.to_string().contains("scp /path/to/file") {
                    Some(idx)
                } else {
                    None
                }
            })
            .expect("code block line present");

        assert!(bullet_two_index < code_block_index);

        let code_line = &rendered.lines[code_block_index];
        let code_indent_span = code_line.spans.first().expect("indent span present");
        assert!(code_indent_span
            .content
            .as_ref()
            .chars()
            .all(|ch| ch == ' '));
        assert_eq!(code_indent_span.content.as_ref().width(), bullet_two_indent);

        let follow_up_index = rendered
            .lines
            .iter()
            .position(|line| line.to_string().contains("But this only works"))
            .expect("follow up text present");
        assert!(code_block_index < follow_up_index);

        let follow_up_indent = rendered.lines[follow_up_index]
            .spans
            .first()
            .expect("follow up indent span present");
        assert!(follow_up_indent
            .content
            .as_ref()
            .chars()
            .all(|ch| ch == ' '));
        assert_eq!(follow_up_indent.content.as_ref().width(), bullet_two_indent);
    }

    #[test]
    fn markdown_links_wrap_at_word_boundaries_with_width() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "abcd efgh [hypertext dreams](https://docs.hypertext.org) and more text"
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, Some(10));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();
        let combined = lines.join("\n");

        assert!(
            combined.contains("hypertext"),
            "combined output should include the link text: {:?}",
            combined
        );
        assert!(
            !combined.contains("hype\nrtext"),
            "link text should wrap at the space boundary, not mid-word: {:?}",
            combined
        );

        let wider = render_markdown_for_test(&message, &theme, true, Some(15));
        let wider_text = wider
            .lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !wider_text.contains("hype\nrtext"),
            "link text should stay intact even when more columns are available: {:?}",
            wider_text
        );
    }

    #[test]
    fn markdown_links_wrap_in_long_paragraph_without_mid_word_break() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: SAMPLE_HYPERTEXT_PARAGRAPH.to_string(),
        };

        let rendered = render_markdown_for_test(&message, &theme, true, Some(158));
        let combined = rendered
            .lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !combined.contains("hype\nrtext"),
            "wide layout still broke link mid-word: {:?}",
            combined
        );
        assert!(
            combined.contains("hypertext dreams"),
            "link text missing from output: {:?}",
            combined
        );
    }

    #[test]
    fn debug_table_events() {
        let markdown = r###"| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |"###;

        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(markdown, options);

        for event in parser {
            println!("{:?}", event);
        }
    }

    #[test]
    fn table_rendering_works() {
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r###"Here's a table:

| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |

End of table."###
                .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();
        let rendered = render_markdown_for_test(&messages[0], &theme, true, None);

        // Check that we have table lines with borders
        let lines_str: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();
        println!("Rendered lines:");
        for (i, line) in lines_str.iter().enumerate() {
            println!("{}: {}", i, line);
        }

        // Should contain box drawing characters
        let has_table_borders = lines_str
            .iter()
            .any(|line| line.contains("┌") || line.contains("├") || line.contains("└"));
        assert!(
            has_table_borders,
            "Table should contain box drawing characters"
        );

        // Should contain table content
        let has_table_content = lines_str
            .iter()
            .any(|line| line.contains("Header 1") && line.contains("Header 2"));
        assert!(has_table_content, "Table should contain header content");
    }

    #[test]
    fn table_renders_emoji_and_br_correctly() {
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| Header | Data |
|---|---|
| Abc | 123 |
| Def | 456 |
| Emoji | 🚀<br/>Hi |
"
            .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();
        let rendered = render_markdown_for_test(&messages[0], &theme, true, None);
        let lines_str: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Extract table lines
        let mut rendered_table_lines: Vec<String> = Vec::new();
        let mut in_table = false;
        for line in lines_str {
            if line.contains("┌") {
                in_table = true;
            }
            if in_table {
                rendered_table_lines.push(line.to_string());
                if line.contains("└") {
                    break;
                }
            }
        }

        // Verify the key functionality: emoji and <br> rendering
        // Instead of hardcoding exact spacing, check for structural correctness
        assert!(
            rendered_table_lines.len() >= 7,
            "Should have at least 7 table lines (top, header, sep, 3 data rows, bottom)"
        );

        // Check that table has proper structure
        assert!(
            rendered_table_lines[0].starts_with("┌"),
            "Should start with top border"
        );
        assert!(
            rendered_table_lines.last().unwrap().starts_with("└"),
            "Should end with bottom border"
        );

        // Check header content
        let header_line = &rendered_table_lines[1];
        assert!(
            header_line.contains("Header") && header_line.contains("Data"),
            "Header should contain expected text"
        );

        // Check data content including emoji and <br> handling
        let all_table_content = rendered_table_lines.join(" ");
        assert!(
            all_table_content.contains("Abc") && all_table_content.contains("123"),
            "Should contain first row data"
        );
        assert!(
            all_table_content.contains("Def") && all_table_content.contains("456"),
            "Should contain second row data"
        );
        assert!(
            all_table_content.contains("Emoji") && all_table_content.contains("🚀"),
            "Should contain emoji"
        );
        assert!(
            all_table_content.contains("Hi"),
            "Should contain <br>-separated text on new line"
        );

        // Key test: emoji should appear on one line and "Hi" should appear on the next line
        let emoji_line_idx = rendered_table_lines
            .iter()
            .position(|line| line.contains("🚀"))
            .expect("Should find emoji line");
        let hi_line_idx = rendered_table_lines
            .iter()
            .position(|line| line.contains("Hi"))
            .expect("Should find Hi line");
        assert_eq!(
            hi_line_idx,
            emoji_line_idx + 1,
            "<br> should create new line: 🚀 and Hi should be on consecutive lines"
        );
    }

    #[test]
    fn test_table_balancing_with_terminal_width() {
        // Manually create a table for testing
        let mut test_table = TableRenderer::new();

        // Add a header row with long headers
        test_table.start_header();
        test_table.start_cell();
        test_table.add_span(Span::raw("Very Long Header Name"), SpanKind::Text);
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Short"), SpanKind::Text);
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Another Very Long Header Name"), SpanKind::Text);
        test_table.end_cell();
        test_table.end_header();

        // Add a data row
        test_table.start_row();
        test_table.start_cell();
        test_table.add_span(Span::raw("Short"), SpanKind::Text);
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(
            Span::raw("VeryLongContentThatShouldBeHandled"),
            SpanKind::Text,
        );
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Data"), SpanKind::Text);
        test_table.end_cell();
        test_table.end_row();

        let theme = crate::ui::theme::Theme::dark_default();

        // Test with narrow terminal (50 chars)
        let narrow_lines = test_table.render_table_with_width(&theme, Some(50));
        let narrow_strings: Vec<String> = narrow_lines
            .iter()
            .map(|(line, _)| line.to_string())
            .collect();

        // With content preservation approach, we prioritize readability over strict width limits
        // Verify table is rendered (has content) but may exceed width to preserve content
        assert!(
            !narrow_strings.is_empty(),
            "Table should render even in narrow terminal"
        );

        // Verify no content is truncated with ellipsis
        for line in &narrow_strings {
            assert!(
                !line.contains("…"),
                "Should not truncate content with ellipsis: '{}'",
                line
            );
        }

        // Test with wide terminal (100 chars) - should use ideal widths
        let wide_lines = test_table.render_table_with_width(&theme, Some(100));
        let wide_strings: Vec<String> = wide_lines
            .iter()
            .map(|(line, _)| line.to_string())
            .collect();

        // With the current algorithm, both tables might end up with similar widths if
        // content preservation is prioritized. Check that at least they're reasonable.
        if let (Some(narrow_border), Some(wide_border)) =
            (narrow_strings.first(), wide_strings.first())
        {
            let narrow_width = UnicodeWidthStr::width(narrow_border.as_str());
            let wide_width = UnicodeWidthStr::width(wide_border.as_str());
            // Both should be reasonable width tables
            assert!(
                narrow_width > 30,
                "Narrow table should still be reasonable width: {}",
                narrow_width
            );
            assert!(
                wide_width > 30,
                "Wide table should still be reasonable width: {}",
                wide_width
            );
            // Wide should be at least as wide as narrow (allow equal for content preservation)
            assert!(
                wide_width >= narrow_width,
                "Wide table should be at least as wide as narrow: narrow={}, wide={}",
                narrow_width,
                wide_width
            );
        }
    }

    #[test]
    fn test_table_column_width_balancing() {
        // Property-based assertions for the column width balancer
        // MIN_COL_WIDTH in balancer
        const MIN_COL_WIDTH: usize = 8;

        // Case 1: Ideal widths fit comfortably — must return exactly the ideals (no need to fill extra space)
        let ts = TableRenderer::new();
        let ideal_fit = vec![10, 10, 10];
        let term_width = 80; // plenty of space
        let out = ts.balance_column_widths(
            &ideal_fit,
            Some(term_width),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        assert_eq!(out, ideal_fit, "When ideals fit, use ideals exactly");
        assert!(out.iter().all(|&w| w >= MIN_COL_WIDTH));
        // Sum does not need to equal available; only constraint is it must not exceed available when ideals fit
        let overhead = ideal_fit.len() * 2 + (ideal_fit.len() + 1);
        let available = term_width - overhead;
        assert!(out.iter().sum::<usize>() <= available);

        // Build a table with content to exercise longest-unbreakable-word minimums
        let mut ts2 = TableRenderer::new();
        // Header
        ts2.start_header();
        ts2.start_cell();
        ts2.add_span(Span::raw("H1"), SpanKind::Text);
        ts2.end_cell();
        ts2.start_cell();
        ts2.add_span(Span::raw("H2"), SpanKind::Text);
        ts2.end_cell();
        ts2.start_cell();
        ts2.add_span(Span::raw("H3"), SpanKind::Text);
        ts2.end_cell();
        ts2.end_header();
        // Data row with unbreakable words: 8, 10, 12 chars respectively
        ts2.start_row();
        ts2.start_cell();
        ts2.add_span(Span::raw("aaaaaaaa"), SpanKind::Text);
        ts2.end_cell(); // 8
        ts2.start_cell();
        ts2.add_span(Span::raw("bbbbbbbbbb"), SpanKind::Text);
        ts2.end_cell(); // 10
        ts2.start_cell();
        ts2.add_span(Span::raw("cccccccccccc"), SpanKind::Text);
        ts2.end_cell(); // 12
        ts2.end_row();

        // Case 2: Some extra space, but not enough to reach all ideals
        let ideals = vec![20, 15, 30]; // each >= its column's longest word and >= MIN_COL_WIDTH
        let cols = ideals.len();
        let term_width = 50; // overhead for 3 cols = 3*2 + 4 = 10 -> available = 40
        let overhead = cols * 2 + (cols + 1);
        let available = term_width - overhead; // 40
        let out2 = ts2.balance_column_widths(
            &ideals,
            Some(term_width),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        // Property checks
        // - Each width respects per-column minimums (longest word and MIN_COL_WIDTH)
        let minima = [8usize, 10, 12];
        for (i, &w) in out2.iter().enumerate() {
            assert!(w >= MIN_COL_WIDTH, "col {} below MIN_COL_WIDTH: {}", i, w);
            assert!(
                w >= minima[i],
                "col {} below longest-word minimum: {} < {}",
                i,
                w,
                minima[i]
            );
            assert!(
                w <= ideals[i],
                "col {} exceeded ideal width: {} > {}",
                i,
                w,
                ideals[i]
            );
        }
        // - Total cannot exceed available when minima fit within available
        assert!(minima.iter().sum::<usize>() <= available);
        assert_eq!(
            out2.iter().sum::<usize>(),
            available,
            "Should fully utilize available space toward ideals when possible"
        );

        // Case 3: Extremely narrow terminal — available smaller than sum of minima.
        // Expect widths to equal the per-column minima (overflow allowed, borders intact).
        let term_width_narrow = 25; // overhead is still 10 -> available = 15 < sum(minima)=30
        let out3 = ts2.balance_column_widths(
            &ideals,
            Some(term_width_narrow),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        assert_eq!(
            out3, minima,
            "When available < sum(minima), return minima to avoid mid-word breaks"
        );

        // Case 4: No terminal width provided — return ideals (subject to MIN_COL_WIDTH which already holds)
        let out4 = ts.balance_column_widths(
            &[8, 10, 12],
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        assert_eq!(out4, vec![8, 10, 12]);
    }

    #[test]
    fn test_table_balancing_performance() {
        // Test performance with large table
        let table_state = TableRenderer::new();
        let ideal_widths: Vec<usize> = (0..50).map(|i| i * 2 + 5).collect();

        let start = std::time::Instant::now();
        let _balanced = table_state.balance_column_widths(
            &ideal_widths,
            Some(200),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        let duration = start.elapsed();

        // Should complete very quickly (under 1ms for reasonable table sizes)
        assert!(
            duration.as_millis() < 10,
            "Table balancing should be fast, took {:?}",
            duration
        );
    }

    #[test]
    fn test_table_no_content_truncation_wide_terminal() {
        // This test defines our goal: no content should ever be truncated with ellipsis
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| Short | Medium Content Here | Very Long Column With Lots Of Text That Should Not Be Truncated |
|-------|---------------------|------------------------------------------------------------------|
| A     | Some content here   | This is a very long piece of text that contains important information that the user needs to see in full without any truncation or ellipsis |
| B     | More content        | Another long piece of text with technical details and specifications that must remain fully visible to be useful |
"
                .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Wide terminal - should fit everything without wrapping or truncation
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(150));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find content lines (not borders)
        let content_lines: Vec<&String> = lines
            .iter()
            .filter(|line| {
                line.contains("A")
                    || line.contains("B")
                    || line.contains("important information")
                    || line.contains("technical details")
            })
            .collect();

        // NO content line should contain ellipsis - this is our fundamental requirement
        for line in &content_lines {
            assert!(
                !line.contains("…"),
                "Found ellipsis truncation in line: '{}'",
                line
            );
        }

        // All important text should be present somewhere in the table
        let all_content = lines.join(" ");
        assert!(
            all_content.contains("important information"),
            "Long text was truncated"
        );
        assert!(
            all_content.contains("technical details"),
            "Long text was truncated"
        );
        assert!(
            all_content.contains("specifications"),
            "Long text was truncated"
        );
    }

    #[test]
    fn test_table_content_wrapping_medium_terminal() {
        // Test that content wraps within cells when terminal is narrower
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| Name | Description |
|------|-------------|
| API  | This is a detailed description of how the API works with multiple parameters and return values |
| SDK  | Software Development Kit with comprehensive documentation and examples for developers |
"
                .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Medium terminal width - should wrap content within cells
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // No ellipsis should be present
        for line in &lines {
            assert!(
                !line.contains("…"),
                "Found ellipsis truncation in line: '{}'",
                line
            );
        }

        // All content should be present even if wrapped
        let all_content = lines.join(" ");

        assert!(all_content.contains("detailed description"));
        assert!(all_content.contains("multiple parameters"));
        assert!(all_content.contains("Software Development Kit"));
        // Check for words that may be wrapped across lines
        assert!(all_content.contains("comprehensive"));
        assert!(all_content.contains("documentation"));

        // Check table structure
        let table_lines: Vec<&String> = lines
            .iter()
            .filter(|line| {
                line.contains("│") || line.contains("┌") || line.contains("├") || line.contains("└")
            })
            .collect();

        // With the improved column balancing, we may have less wrapping than before
        // The key is that content is preserved without ellipsis
        assert!(
            table_lines.len() >= 5,
            "Should have at least basic table structure (header + data + borders), got {}",
            table_lines.len()
        );
    }

    #[test]
    fn test_logical_row_continuation() {
        // Test that empty first cells continue the previous logical row
        let mut test_table = TableRenderer::new();

        // Add header
        test_table.start_header();
        test_table.start_cell();
        test_table.add_span(Span::raw("Command"), SpanKind::Text);
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Description"), SpanKind::Text);
        test_table.end_cell();
        test_table.end_header();

        // Add first data row
        test_table.start_row();
        test_table.start_cell();
        test_table.add_span(Span::raw("git commit"), SpanKind::Text);
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(
            Span::raw("Creates a new commit with staged changes"),
            SpanKind::Text,
        );
        test_table.end_cell();
        test_table.end_row();

        // Add continuation row (empty first cell)
        test_table.start_row();
        test_table.start_cell();
        // Empty first cell - should continue previous row
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("and includes a commit message"), SpanKind::Text);
        test_table.end_cell();
        test_table.end_row();

        let theme = crate::ui::theme::Theme::dark_default();
        let lines = test_table.render_table_with_width(&theme, Some(60));
        let line_strings: Vec<String> = lines.iter().map(|(line, _)| line.to_string()).collect();

        // Should not truncate any content
        for line in &line_strings {
            assert!(!line.contains("…"), "Found ellipsis in line: '{}'", line);
        }

        // Both parts of the description should be present
        let all_content = line_strings.join(" ");
        assert!(all_content.contains("Creates a new commit"));
        assert!(all_content.contains("and includes a commit message"));

        // The continuation should appear in the same logical row as the command
        // This means we should see both parts of the description in cells adjacent to "git commit"
        let content_section = line_strings
            .iter()
            .skip_while(|line| !line.contains("git commit"))
            .take_while(|line| !line.contains("└"))
            .cloned()
            .collect::<Vec<String>>()
            .join(" ");

        assert!(content_section.contains("Creates a new commit"));
        assert!(content_section.contains("and includes a commit message"));
    }

    #[test]
    fn test_table_should_not_wrap_borders() {
        // This test reproduces the real-world issue where table borders get wrapped
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r#"| System of Government | Definition | Key Features | Examples |
|---------------------|------------|--------------|----------|
| Democracy | Government by the people, either directly or through elected representatives. | Universal suffrage, free elections, protection of civil liberties. | United States, India, Germany |
| Republic | A form of government in which power resides with the citizens, who elect representatives to govern on their behalf. | Elected officials, separation of powers, rule of law. | France, Brazil, South Africa |
| Dictatorship | A form of government in which a single person or a small group holds absolute power. | Lack of free elections, suppression of opposition, centralized control. | North Korea, Cuba, Syria |"#.into(),
        });

        let theme = crate::ui::theme::Theme::dark_default();

        // Test the CORRECT semantic approach: render with width constraints from the start
        let terminal_width = 120u16;
        let lines = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages, &theme, true, true, Some(terminal_width as usize)
        );
        let line_strings: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

        println!("=== PROPERLY RENDERED TABLE ===");
        for (i, line) in line_strings.iter().enumerate() {
            println!("{:2}: {}", i, line);
        }

        // Key test: When using the semantic approach, table borders should be complete
        for line in &line_strings {
            if line.contains("┌") || line.contains("├") || line.contains("└") {
                // Border lines should be complete
                assert!(
                    line.contains("┐") || line.contains("┤") || line.contains("┘"),
                    "Border line should be complete: '{}'",
                    line
                );
            }
        }

        // The key success: borders are not wrapped (no double-wrapping issue)
        // Note: Table might be wide, but that's better than broken borders
        println!("Success! Table borders are intact and not wrapped.");

        // Verify table structure is intact
        let table_content = line_strings.join("\n");
        assert!(
            table_content.contains("Democracy") && table_content.contains("Dictatorship"),
            "Table content should be preserved"
        );
    }

    #[test]
    fn test_styled_words_wrap_at_boundaries_in_table() {
        // Focused regression: styled words in table cells should wrap at word
        // boundaries (including hyphen breaks), not inside the styled words.
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r#"| Feature | Benefits |
|---------|----------|
| X | **Dramatically** _improved_ decision-making capabilities with ***real-time*** analytics |
"#
            .into(),
        });

        let theme = crate::ui::theme::Theme::dark_default();

        // Use a modest width to force wrapping within the Benefits cell
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Collect only table content lines (skip borders/separators)
        let content_lines: Vec<&String> = lines
            .iter()
            .filter(|line| {
                line.contains("│")
                    && !line.contains("┌")
                    && !line.contains("├")
                    && !line.contains("└")
                    && !line.contains("─")
            })
            .collect();

        // Join for simpler substring checks
        let all_content = content_lines
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>()
            .join(" ");

        // 1) Space between styled words must be preserved across spans
        assert!(
            all_content.contains("Dramatically improved"),
            "Space around styled words should be preserved: {}",
            all_content
        );

        // 2) Hyphenated word may wrap after the hyphen, but not mid-segment
        // Accept either kept together or split at the hyphen with a space inserted by wrapping
        let hyphen_ok =
            all_content.contains("decision-making") || all_content.contains("decision- making");
        assert!(
            hyphen_ok,
            "Hyphen should be a soft break point: {}",
            all_content
        );

        // 3) No truncation
        for line in &lines {
            assert!(!line.contains("…"), "No truncation expected: '{}'", line);
        }
    }

    #[test]
    fn cell_wraps_at_space_across_spans() {
        // Ensure wrapping prefers spaces even when they occur across styled spans
        let theme = crate::ui::theme::Theme::dark_default();
        let ts = TableRenderer::new();

        let bold = theme.md_paragraph_style().add_modifier(Modifier::BOLD);
        let spans = vec![
            (Span::styled("foo", bold), SpanKind::Text),
            (Span::raw(" "), SpanKind::Text),
            (Span::styled("bar", bold), SpanKind::Text),
        ];

        // Width fits "foo" exactly; space + "bar" should go to next line
        let lines =
            ts.wrap_spans_to_width(&spans, 3, crate::ui::layout::TableOverflowPolicy::WrapCells);
        let rendered: Vec<String> = lines
            .iter()
            .map(|spans| {
                spans
                    .iter()
                    .map(|(s, _)| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "foo");
        assert_eq!(rendered[1], "bar");
    }

    #[test]
    fn cell_wraps_after_hyphen() {
        // Ensure hyphen is treated as a soft break opportunity
        let theme = crate::ui::theme::Theme::dark_default();
        let ts = TableRenderer::new();
        let style = theme.md_paragraph_style();
        let spans = vec![(Span::styled("decision-making", style), SpanKind::Text)];

        // Allow exactly "decision-" on first line
        let lines = ts.wrap_spans_to_width(
            &spans,
            10,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        let rendered: Vec<String> = lines
            .iter()
            .map(|spans| {
                spans
                    .iter()
                    .map(|(s, _)| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "decision-");
        assert_eq!(rendered[1], "making");
    }

    #[test]
    fn test_table_wrapping_with_mixed_content() {
        // Test wrapping behavior with mixed short and long content
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| ID | Status | Details |
|----|--------|----------|
| 1  | OK     | Everything is working perfectly and all systems are operational |
| 2  | ERROR  | A critical error occurred during processing and requires immediate attention |
| 3  | WARN   | Warning: deprecated function usage detected |
"
            .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Narrow terminal that requires wrapping
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(45));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Verify no truncation
        for line in &lines {
            assert!(!line.contains("…"), "Found ellipsis in: '{}'", line);
        }

        // All content must be preserved
        let all_content = lines.join(" ");
        // Check for key words that may be wrapped across lines
        assert!(all_content.contains("working") && all_content.contains("perfectly"));
        assert!(all_content.contains("systems") && all_content.contains("operational"));
        assert!(
            all_content.contains("critical")
                && all_content.contains("error")
                && all_content.contains("occurred")
        );
        assert!(all_content.contains("immediate") && all_content.contains("attention"));
        assert!(
            all_content.contains("deprecated")
                && all_content.contains("function")
                && all_content.contains("usage")
        );

        // Should create a reasonable number of table lines (not excessive)
        let table_lines: Vec<&String> = lines.iter().filter(|line| line.contains("│")).collect();

        // We should have content lines but not an excessive number
        assert!(
            table_lines.len() >= 3,
            "Should have at least header + data rows"
        );
        assert!(
            table_lines.len() <= 15,
            "Should not create excessive wrapped lines"
        );
    }

    #[test]
    fn test_extremely_narrow_terminal_no_truncation() {
        // Test that even extremely narrow terminals never truncate content
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| A | B |
|---|---|
| VeryLongUnbreakableWord | AnotherLongWord |
"
            .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Extremely narrow terminal (20 chars)
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(20));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Critical: NO truncation even in extreme cases
        for line in &lines {
            assert!(
                !line.contains("…"),
                "Found ellipsis even in extreme narrow case: '{}'",
                line
            );
        }

        // Content must be preserved - either wrapped or allow horizontal scroll
        let all_content = lines.join(" ");
        // With short unbreakable words (<= 30 chars), they should be preserved by expanding the column
        // But if the terminal is very narrow, the word might still get broken as a last resort
        // The key is NO ellipsis truncation

        // The word "VeryLongUnbreakableWord" should have its parts preserved even when broken
        assert!(
            all_content.contains("VeryLong")
                && (all_content.contains("Unbreaka") || all_content.contains("bleWord")),
            "Word parts should be preserved"
        );
        assert!(
            all_content.contains("Another") && all_content.contains("Word"),
            "Second word should be preserved"
        );

        // In extreme cases, we accept horizontal scrolling over truncation
        // So some lines might exceed the 20 char limit
        println!("Narrow terminal output:");
        for (i, line) in lines.iter().enumerate() {
            println!("{}: '{}' ({})", i, line, line.len());
        }
    }

    #[test]
    fn test_table_with_emoji_and_unicode_no_truncation() {
        // Test that emoji and Unicode characters are handled without truncation
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r"| Status | Message | Details |
|--------|---------|----------|
| ✅     | Success | Operation completed successfully with all parameters validated |
| ❌     | Error   | An error occurred while processing the request with Unicode chars: résumé, naïve, café |
| 🚀     | Launch  | System is ready for deployment with full internationalization support |
"
                .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Medium width terminal
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(70));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // No truncation of Unicode content
        for line in &lines {
            assert!(
                !line.contains("…"),
                "Found ellipsis with Unicode content: '{}'",
                line
            );
        }

        // All Unicode content must be preserved
        let all_content = lines.join(" ");
        assert!(all_content.contains("✅"));
        assert!(all_content.contains("❌"));
        assert!(all_content.contains("🚀"));
        assert!(all_content.contains("résumé"));
        assert!(all_content.contains("naïve"));
        assert!(all_content.contains("café"));
        assert!(all_content.contains("internationalization"));
    }

    #[test]
    fn table_preserves_words_with_available_space() {
        // Test that words like "Dictatorship" don't get split mid-word when
        // terminal has adequate width, while keeping columns balanced
        // Use a table that has more content to force the column balancing issue
        let markdown = r#"
| Government System | Definition | Key Properties |
|-------------------|------------|----------------|
| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections, Protection of civil liberties |
| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |
"#;

        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: markdown.to_string(),
        });

        let theme = crate::ui::theme::Theme::dark_default();
        // Force a narrower width to trigger the column balancing that causes word splits
        let rendered = render_markdown_for_test(messages.front().unwrap(), &theme, true, Some(80));
        let lines_str: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Extract table content
        let table_content = lines_str.join("\n");

        // Key assertion: "Dictatorship" should appear intact on a single line
        // (not split as "Dictator" + "ship" or similar)
        assert!(
            table_content.contains("Dictatorship"),
            "Table should contain the complete word 'Dictatorship'"
        );

        // Ensure it's not split across lines
        let has_partial_dictator =
            table_content.contains("Dictator") && !table_content.contains("Dictatorship");
        assert!(
            !has_partial_dictator,
            "Word 'Dictatorship' should not be split mid-word when space is available"
        );

        // Verify table structure is maintained
        assert!(
            table_content.contains("┌") && table_content.contains("└"),
            "Table should have proper borders"
        );
    }

    #[test]
    fn test_government_systems_table_from_testcase() {
        // This test captures the exact content from testcase.txt to verify:
        // 1. Styled words don't swallow whitespace
        // 2. Vertical borders remain aligned
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r#"| Government Type | Description | Key Characteristics | Examples |
|-----------------|-------------|--------------------|---------|
| **Democracy** | A system where power is vested in the people, who rule either *directly* or through elected representatives. | - Free and fair elections<br/>- Protection of individual rights and freedoms<br/>- Rule of law and separation of powers | - *United States*, *India*, *Germany* |
| **Republic** | A form of government where the country is considered a "public matter" (*res publica*), with power held by the people and their elected representatives. | - Elected officials represent the citizens<br/>- Written constitution and rule of law<br/>- Protection of minority rights | - *France*, *Italy*, *Brazil* |
| **Monarchy** | A system where a single person, known as a monarch, rules until death or abdication. | - Hereditary succession of the ruler<br/>- Can be constitutional or absolute<br/>- Often combined with other forms of government | - *United Kingdom* (constitutional), *Saudi Arabia* (absolute) |
| **Dictatorship** | A system where power is concentrated in the hands of a single person or a small group, often with no meaningful opposition. | - Single-party rule or military rule<br/>- Suppression of political opposition and civil liberties<br/>- Often characterized by censorship and propaganda | - *North Korea*, *Cuba*, *Syria* |
| **Theocracy** | A system where government is *the rule of God* or a divine being, with religious leaders holding political power. | - Religious law (e.g., Sharia) as the basis for governance<br/>- Religious leaders hold political authority<br/>- Often limited civil liberties for non-believers or dissenters | - *Iran*, *Vatican City* |
| **Communism** | A system where the means of production are owned and controlled by the state, aiming for a classless society. | - Central planning and state ownership of industry<br/>- Single-party rule and suppression of political opposition<br/>- Emphasis on collective ownership and equality | - *China*, *Cuba*, *North Korea* |"#.into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();

        // Test with a medium terminal width to force wrapping
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(120));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        println!("=== Government Systems Table Output ===");
        for (i, line) in lines.iter().enumerate() {
            println!("{:3}: {}", i, line);
        }

        // Check for the two main bugs:

        // Bug 1: Styled words should not swallow whitespace (FIXED!)
        let all_content = lines.join(" ");
        // The key test: spaces around styled text should be preserved
        assert!(
            all_content.contains("either") && all_content.contains("directly"),
            "Words should be separated"
        );
        assert!(
            all_content.contains("- United States, India"),
            "Country names should have proper spacing"
        );
        assert!(
            all_content.contains("rule") && all_content.contains("God"),
            "Key words should be present"
        );

        // Most importantly, we should NOT see the old concatenated words bug
        assert!(
            !all_content.contains("eitherdirectlyor"),
            "✓ Words are no longer concatenated!"
        );
        assert!(
            !all_content.contains("-UnitedStates,India"),
            "✓ Spaces are preserved around styled text!"
        );

        // Bug 2: Vertical borders should be aligned
        // All table content lines should have their │ characters at consistent positions
        let table_lines: Vec<&String> = lines
            .iter()
            .filter(|line| {
                line.contains("│")
                    && !line.contains("┌")
                    && !line.contains("├")
                    && !line.contains("└")
            })
            .collect();

        if table_lines.len() >= 2 {
            // Get positions of all │ characters in the first content line
            let first_line = table_lines[0];
            let first_border_positions: Vec<usize> = first_line
                .char_indices()
                .filter_map(|(i, c)| if c == '│' { Some(i) } else { None })
                .collect();

            // Verify all other content lines have │ at the same positions
            for (line_idx, line) in table_lines.iter().enumerate().skip(1) {
                let border_positions: Vec<usize> = line
                    .char_indices()
                    .filter_map(|(i, c)| if c == '│' { Some(i) } else { None })
                    .collect();

                assert_eq!(
                    first_border_positions, border_positions,
                    "Border positions should be aligned. Line {}: expected {:?}, got {:?}\nFirst line: '{}'\nThis line:  '{}'",
                    line_idx, first_border_positions, border_positions, first_line, line
                );
            }
        }

        // Verify no content is truncated
        for line in &lines {
            assert!(
                !line.contains("…"),
                "No content should be truncated: '{}'",
                line
            );
        }
    }

    #[test]
    fn test_table_cell_word_wrapping_regression() {
        // Reproduce the table wrapping issue - test that words wrap within table cells
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: r###"Here's a table with long content that should wrap:

| Column A | Column B | Column C |
|----------|----------|----------|
| This is a very long sentence that should definitely wrap within the cell when the terminal is narrow | Short | Another moderately long piece of content |
| Short content | This is another extremely long sentence that contains many words and should wrap properly within the table cell boundaries | More content here |
"###.to_string(),
        });

        let theme = crate::ui::theme::Theme::dark_default();

        // Test with narrow terminal width (60 chars) to force wrapping
        let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        println!("\nRendered table with width 60:");
        for (i, line) in lines.iter().enumerate() {
            println!("{:2}: {}", i, line);
        }

        // Look for table content
        let table_start = lines
            .iter()
            .position(|line| line.contains("┌"))
            .expect("Should find table start");
        let table_end = lines
            .iter()
            .position(|line| line.contains("└"))
            .expect("Should find table end");

        let table_lines = &lines[table_start..=table_end];

        // Find the rows with long content
        let content_rows: Vec<&String> = table_lines
            .iter()
            .filter(|line| {
                line.contains("│")
                    && !line.contains("┌")
                    && !line.contains("├")
                    && !line.contains("└")
                    && !line.contains("─")
            })
            .collect();

        println!("\nContent rows ({} total):", content_rows.len());
        for (i, row) in content_rows.iter().enumerate() {
            let width = UnicodeWidthStr::width(row.as_str());
            println!("{:2}: {} (width: {})", i, row, width);
        }

        // The key test: if wrapping is working, we should see multiple rows for the same logical table row
        // Each long sentence should be broken across multiple lines
        assert!(
            content_rows.len() > 3,
            "Should have more than 3 content rows due to wrapping. Found: {} rows",
            content_rows.len()
        );

        // Check that long text appears to be wrapped (partial text in multiple rows)
        let all_content = content_rows
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        // The long sentences should be present in the content (may be split across lines)
        assert!(
            all_content.contains("very")
                && all_content.contains("long")
                && all_content.contains("sentence"),
            "Should contain parts of first long sentence"
        );
        assert!(
            all_content.contains("extremely")
                && all_content.contains("long")
                && all_content.contains("sentence"),
            "Should contain parts of second long sentence"
        );

        // But no single row should contain the complete long sentence (it should be wrapped)
        let has_complete_first_sentence = content_rows.iter().any(|row|
            row.contains("This is a very long sentence that should definitely wrap within the cell when the terminal is narrow")
        );
        let has_complete_second_sentence = content_rows.iter().any(|row|
            row.contains("This is another extremely long sentence that contains many words and should wrap properly within the table cell boundaries")
        );

        assert!(
            !has_complete_first_sentence,
            "First long sentence should be wrapped, not appear complete in one row"
        );
        assert!(
            !has_complete_second_sentence,
            "Second long sentence should be wrapped, not appear complete in one row"
        );

        // Verify no row is excessively wide due to lack of wrapping
        for (i, row) in content_rows.iter().enumerate() {
            let row_width = UnicodeWidthStr::width(row.as_str());
            assert!(row_width <= 100, "Row {} should not be excessively wide due to proper wrapping: width={}, content: '{}'", i, row_width, row);
        }
    }

    // Phase 0 tests: Code block span metadata (currently failing, will pass in Phase 1)

    #[test]

    fn code_block_spans_have_metadata() {
        use super::test_fixtures;
        let msg = test_fixtures::single_block();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        // Find spans that should be code blocks
        let code_spans: Vec<_> = metadata
            .iter()
            .flat_map(|line| line.iter())
            .filter(|kind| kind.is_code_block())
            .collect();

        assert!(
            !code_spans.is_empty(),
            "Code block should have CodeBlock metadata"
        );

        // Verify metadata contains language and block index
        if let Some(meta) = code_spans[0].code_block_meta() {
            assert_eq!(meta.language(), Some("rust"));
            assert_eq!(meta.block_index(), 0);
        } else {
            panic!("Expected CodeBlock metadata");
        }
    }

    #[test]

    fn multiple_code_blocks_have_unique_indices() {
        use super::test_fixtures;
        let msg = test_fixtures::multiple_blocks();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        // Extract unique block indices
        let mut indices = std::collections::HashSet::new();
        for line_meta in metadata.iter() {
            for kind in line_meta.iter() {
                if let Some(meta) = kind.code_block_meta() {
                    indices.insert(meta.block_index());
                }
            }
        }

        assert_eq!(indices.len(), 3, "Should have 3 unique code block indices");
        assert!(indices.contains(&0));
        assert!(indices.contains(&1));
        assert!(indices.contains(&2));
    }

    #[test]

    fn empty_code_block_has_metadata() {
        use super::test_fixtures;
        let msg = test_fixtures::empty_block();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        // Empty blocks produce no span metadata and are not navigable.
        // This is correct behavior - there's no content to select or extract.
        let has_code_meta = metadata
            .iter()
            .flat_map(|line| line.iter())
            .any(|k| k.is_code_block());

        assert!(
            !has_code_meta,
            "Empty blocks should not create code block metadata"
        );
    }

    #[test]

    fn wrapped_code_preserves_metadata_across_lines() {
        use super::test_fixtures;
        let msg = test_fixtures::wrapped_code();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            Some(40), // Narrow width to force wrapping
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        // All code spans should have block_index 0
        let block_indices: Vec<usize> = metadata
            .iter()
            .flat_map(|line| line.iter())
            .filter_map(|k| k.code_block_meta().map(|m| m.block_index()))
            .collect();

        assert!(!block_indices.is_empty(), "Should have code block metadata");
        assert!(
            block_indices.iter().all(|&idx| idx == 0),
            "All wrapped lines should have same block_index"
        );
    }

    #[test]

    fn code_block_without_language_has_metadata() {
        use super::test_fixtures;
        let msg = test_fixtures::no_language_tag();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        let code_metas: Vec<_> = metadata
            .iter()
            .flat_map(|line| line.iter())
            .filter_map(|k| k.code_block_meta())
            .collect();

        assert!(!code_metas.is_empty(), "Should have code block metadata");
        assert_eq!(
            code_metas[0].language(),
            None,
            "Block without language should have None language"
        );
    }

    #[test]

    fn nested_code_blocks_have_metadata() {
        use super::test_fixtures;
        let msg = test_fixtures::nested_in_list();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        // Should have two code blocks (indices 0 and 1)
        let mut indices = std::collections::HashSet::new();
        for line_meta in metadata.iter() {
            for kind in line_meta.iter() {
                if let Some(meta) = kind.code_block_meta() {
                    indices.insert(meta.block_index());
                }
            }
        }

        assert_eq!(indices.len(), 2, "Should have 2 code blocks in list");
    }

    #[test]

    fn user_message_code_blocks_have_metadata() {
        use super::test_fixtures;
        let msg = test_fixtures::user_message_with_code();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            Some("User"),
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        let has_code_blocks = metadata
            .iter()
            .flat_map(|line| line.iter())
            .any(|k| k.is_code_block());

        assert!(
            has_code_blocks,
            "User messages should have code block metadata"
        );
    }

    #[test]

    fn code_and_link_metadata_coexist() {
        use super::test_fixtures;
        let msg = test_fixtures::code_and_links();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        let has_code_blocks = metadata
            .iter()
            .flat_map(|line| line.iter())
            .any(|k| k.is_code_block());

        let has_links = metadata
            .iter()
            .flat_map(|line| line.iter())
            .any(|k| k.is_link());

        assert!(has_code_blocks, "Should have code block metadata");
        assert!(has_links, "Should have link metadata");
    }

    #[test]

    fn various_language_tags_preserved() {
        use super::test_fixtures;
        let msg = test_fixtures::various_languages();
        let theme = crate::ui::theme::Theme::dark_default();

        let details = render_message_markdown_details_with_policy_and_user_name(
            &msg,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            None,
        );
        let metadata = details.span_metadata.expect("metadata should be present");

        let languages: Vec<Option<&str>> = metadata
            .iter()
            .flat_map(|line| line.iter())
            .filter_map(|k| k.code_block_meta())
            .map(|m| m.language())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Should find bash, javascript, json, txt
        assert!(
            languages.len() >= 4,
            "Should preserve different language tags"
        );
    }

    #[test]
    fn nested_bullet_lists_render_with_indentation() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content:
                "* Item 1\n    * Sub-item 1.1\n    * Sub-item 1.2\n        * Sub-sub-item 1.2.1"
                    .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Verify that nested items have leading spaces
        // Line 0 should be "- Item 1" (no indent)
        // Line 1 should be "  - Sub-item 1.1" (2 space indent from parent "- " marker)
        // Line 2 should be "  - Sub-item 1.2" (2 space indent)
        // Line 3 should be "    - Sub-sub-item 1.2.1" (4 space indent: 2 from first level + 2 from second level)

        assert!(
            lines.len() >= 4,
            "Should have at least 4 lines, got {}",
            lines.len()
        );
        assert!(
            lines[0].starts_with("- "),
            "First item should start with '- ', got: '{}'",
            lines[0]
        );
        assert!(
            lines[1].starts_with("  - "),
            "Sub-item should have 2-space indent, got: '{}'",
            lines[1]
        );
        assert!(
            lines[2].starts_with("  - "),
            "Sub-item should have 2-space indent, got: '{}'",
            lines[2]
        );
        assert!(
            lines[3].starts_with("    - "),
            "Sub-sub-item should have 4-space indent, got: '{}'",
            lines[3]
        );
    }

    #[test]
    fn nested_lists_dont_add_blank_lines_between_same_level_items() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- Budget tree, branch one\n  - Emergency fund\n    - Sub-sticky note\n  - Groceries"
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the indices of key items
        let emergency_idx = lines.iter().position(|l| l.contains("Emergency")).unwrap();
        let sub_sticky_idx = lines.iter().position(|l| l.contains("Sub-sticky")).unwrap();
        let groceries_idx = lines.iter().position(|l| l.contains("Groceries")).unwrap();

        // After "Sub-sticky note" ends its nested list, "Groceries" should immediately follow
        // without any blank lines, since they're both at the same level (level 2)
        assert_eq!(
            groceries_idx,
            sub_sticky_idx + 1,
            "Groceries should come immediately after Sub-sticky note without blank lines. Lines: {:#?}",
            lines
        );

        // Verify the structure is correct
        assert!(
            emergency_idx < sub_sticky_idx,
            "Emergency should come before Sub-sticky"
        );
        assert!(
            sub_sticky_idx < groceries_idx,
            "Sub-sticky should come before Groceries"
        );
    }

    #[test]
    fn list_with_source_blank_lines_preserves_spacing_between_top_level_items() {
        // When the markdown source has blank lines between top-level list items,
        // those should be preserved to provide visual breathing room
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- Strategic Foundations\n  - Long-Horizon Thinking\n    - Scenario Branches\n\n- Implementation Patterns\n  - Knowledge Architecture\n    - Modular repositories\n\n- Resilience\n  - Stressors".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find indices
        let implementation_idx = lines
            .iter()
            .position(|l| l.contains("Implementation"))
            .unwrap();
        let resilience_idx = lines.iter().position(|l| l.contains("Resilience")).unwrap();

        // Check if there's a blank line between Strategic section and Implementation section
        let has_blank_before_implementation = lines[implementation_idx - 1].trim().is_empty();

        // Check if there's a blank line between Implementation section and Resilience section
        let has_blank_before_resilience = lines[resilience_idx - 1].trim().is_empty();

        assert!(
            has_blank_before_implementation,
            "Should have blank line before 'Implementation Patterns' (source has blank line). Lines: {:#?}",
            lines
        );
        assert!(
            has_blank_before_resilience,
            "Should have blank line before 'Resilience' (source has blank line). Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn list_without_source_blank_lines_has_no_spacing_between_top_level_items() {
        // When the markdown source has NO blank lines between top-level list items,
        // they should render consecutively without extra spacing
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- First section\n  - Nested item\n- Second section\n  - Another nested"
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find index
        let second_idx = lines
            .iter()
            .position(|l| l.contains("Second section"))
            .unwrap();

        // Second section should come relatively soon after First section ends
        // There should be no blank line between them since source has none
        // We need to account for the nested item, so check the line before Second section
        let line_before_second = &lines[second_idx - 1];

        assert!(
            !line_before_second.trim().is_empty(),
            "Should NOT have blank line before 'Second section' (source has no blank line). Line before: '{}'. All lines: {:#?}",
            line_before_second,
            lines
        );
    }

    #[test]
    fn list_preceded_by_paragraph_has_blank_line_before() {
        // A list preceded by a paragraph should have a blank line separating them
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "Here is some introductory text.\n\n- First item\n- Second item".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let intro_idx = lines
            .iter()
            .position(|l| l.contains("introductory"))
            .unwrap();
        let first_item_idx = lines.iter().position(|l| l.contains("First item")).unwrap();

        // There should be a blank line between the paragraph and the list
        assert!(
            first_item_idx > intro_idx + 1,
            "Should have blank line between paragraph and list. Lines: {:#?}",
            lines
        );
        assert!(
            lines[intro_idx + 1].trim().is_empty(),
            "Line after paragraph should be blank. Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn list_followed_by_paragraph_has_blank_line_after() {
        // A list followed by a paragraph should have a blank line separating them
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- First item\n- Second item\n\nThis is concluding text.".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let second_item_idx = lines
            .iter()
            .position(|l| l.contains("Second item"))
            .unwrap();
        let concluding_idx = lines.iter().position(|l| l.contains("concluding")).unwrap();

        // There should be a blank line between the list and the paragraph
        assert!(
            concluding_idx > second_item_idx + 1,
            "Should have blank line between list and paragraph. Lines: {:#?}",
            lines
        );
        assert!(
            lines[second_item_idx + 1].trim().is_empty(),
            "Line after list should be blank. Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn list_preceded_by_heading_has_blank_line_before() {
        // A list preceded by a heading should have a blank line separating them
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "## My Section\n\n- First item\n- Second item".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let heading_idx = lines.iter().position(|l| l.contains("My Section")).unwrap();
        let first_item_idx = lines.iter().position(|l| l.contains("First item")).unwrap();

        // There should be a blank line between the heading and the list
        assert!(
            first_item_idx > heading_idx + 1,
            "Should have blank line between heading and list. Lines: {:#?}",
            lines
        );
        assert!(
            lines[heading_idx + 1].trim().is_empty(),
            "Line after heading should be blank. Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn list_followed_by_heading_has_blank_line_after() {
        // A list followed by a heading should have a blank line separating them
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "- First item\n- Second item\n\n## Next Section".into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let second_item_idx = lines
            .iter()
            .position(|l| l.contains("Second item"))
            .unwrap();
        let heading_idx = lines
            .iter()
            .position(|l| l.contains("Next Section"))
            .unwrap();

        // There should be a blank line between the list and the heading
        assert!(
            heading_idx > second_item_idx + 1,
            "Should have blank line between list and heading. Lines: {:#?}",
            lines
        );
        assert!(
            lines[second_item_idx + 1].trim().is_empty(),
            "Line after list should be blank. Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn complex_nested_lists_with_long_text_preserve_blank_lines() {
        // Test complex nested markdown with multiple levels, long wrapping text,
        // and blank lines at various nesting depths
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"### Architecture Overview

1. **Primary Concept: The Architecture of a Modern Knowledge System**
   In designing a contemporary knowledge system, several foundational components must be conceptualized, integrated, and optimized for scalability. The architecture should balance information retrieval efficiency, semantic accuracy, and human-centered accessibility.
   Below is a structured decomposition of its design hierarchy:

   - **Layer One: Data Acquisition and Normalization**
     Collecting heterogeneous data streams across structured and unstructured sources forms the backbone of long-term informational reliability.
     Examples include web-scraped data, curated research papers, user-generated content, and transaction logs.

     - **Sub-layer A: Source Validation**
       - Ensure authenticity through cryptographic checksums.
       - Implement redundancy detection using fuzzy hashing.
       - Maintain timestamp precision to establish causal consistency.

     - **Sub-layer B: Normalization Pipeline**
       - Convert text encodings to UTF-8 for cross-platform compatibility.
       - Apply tokenization with semantic segmentation to retain linguistic intent.

   - **Layer Two: Knowledge Representation**
     Once normalized, data should be molded into adaptive knowledge graphs or relational mappings.
     These structures serve to bridge connections across domains, entities, and abstract relationships.

     - **Sub-layer A: Ontological Framework**
       - Define entities, attributes, and relations using logical formalisms.
       - Incorporate context-sensitive nodes for ambiguous linguistic references.
"#.into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, Some(80));
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Debug: print all lines
        eprintln!("\n=== RENDERED OUTPUT ===");
        for (i, line) in lines.iter().enumerate() {
            eprintln!("{:3}: '{}'", i, line);
        }
        eprintln!("=== END OUTPUT ===\n");

        // Find key elements
        let heading_idx = lines
            .iter()
            .position(|l| l.contains("Architecture Overview"))
            .unwrap();
        let primary_idx = lines
            .iter()
            .position(|l| l.contains("Primary Concept"))
            .unwrap();
        let layer_one_idx = lines.iter().position(|l| l.contains("Layer One")).unwrap();
        let sublayer_a_idx = lines
            .iter()
            .position(|l| l.contains("Sub-layer A"))
            .unwrap();
        let ensure_auth_idx = lines
            .iter()
            .position(|l| l.contains("Ensure authenticity"))
            .unwrap();
        let implement_red_idx = lines
            .iter()
            .position(|l| l.contains("Implement redundancy"))
            .unwrap();
        let sublayer_b_idx = lines
            .iter()
            .position(|l| l.contains("Sub-layer B"))
            .unwrap();
        let layer_two_idx = lines.iter().position(|l| l.contains("Layer Two")).unwrap();

        // === EXPECTATION 1: Blank line after heading ===
        assert!(
            lines[heading_idx + 1].trim().is_empty(),
            "Should have blank line after heading. Line {}: '{}'",
            heading_idx + 1,
            lines[heading_idx + 1]
        );

        // === EXPECTATION 2: Ordered list item starts after blank line ===
        assert_eq!(
            heading_idx + 2,
            primary_idx,
            "Ordered list should start right after blank line following heading"
        );

        // === EXPECTATION 3: Blank line before Layer One (nested bullet in ordered list) ===
        // Source has blank line after "Below is a structured decomposition..." and before Layer One
        let line_before_layer_one = &lines[layer_one_idx - 1];
        assert!(
            line_before_layer_one.trim().is_empty(),
            "Should have blank line before 'Layer One' (source has blank line). Line {}: '{}'",
            layer_one_idx - 1,
            line_before_layer_one
        );

        // === EXPECTATION 4: Blank line before Sub-layer A ===
        // Source has blank line after "Examples include web-scraped data..." and before Sub-layer A
        let line_before_sublayer_a = &lines[sublayer_a_idx - 1];
        assert!(
            line_before_sublayer_a.trim().is_empty(),
            "Should have blank line before 'Sub-layer A' (source has blank line). Line {}: '{}'",
            sublayer_a_idx - 1,
            line_before_sublayer_a
        );

        // === EXPECTATION 5: No blank lines within Sub-layer A's nested items ===
        // The three items under Sub-layer A should be consecutive (no blank lines in source)
        let line_after_ensure = &lines[ensure_auth_idx + 1];
        assert!(
            !line_after_ensure.trim().is_empty(),
            "Should NOT have blank line after 'Ensure authenticity' (no blank line in source). Line {}: '{}'",
            ensure_auth_idx + 1,
            line_after_ensure
        );

        let line_after_implement = &lines[implement_red_idx + 1];
        assert!(
            !line_after_implement.trim().is_empty(),
            "Should NOT have blank line after 'Implement redundancy' (no blank line in source). Line {}: '{}'",
            implement_red_idx + 1,
            line_after_implement
        );

        // === EXPECTATION 6: Blank line before Sub-layer B ===
        // Source has blank line after last item of Sub-layer A and before Sub-layer B
        let line_before_sublayer_b = &lines[sublayer_b_idx - 1];
        assert!(
            line_before_sublayer_b.trim().is_empty(),
            "Should have blank line before 'Sub-layer B' (source has blank line). Line {}: '{}'",
            sublayer_b_idx - 1,
            line_before_sublayer_b
        );

        // === EXPECTATION 7: Blank line before Layer Two ===
        // Source has blank line after last item of Sub-layer B and before Layer Two
        let line_before_layer_two = &lines[layer_two_idx - 1];
        assert!(
            line_before_layer_two.trim().is_empty(),
            "Should have blank line before 'Layer Two' (source has blank line). Line {}: '{}'",
            layer_two_idx - 1,
            line_before_layer_two
        );
    }

    #[test]
    fn blank_line_before_paragraph_doesnt_cause_blank_before_later_list_item() {
        // Regression test: prev_was_blank persisting across paragraph text would cause
        // a later list item to incorrectly get a blank line before it.
        // Scenario: item, paragraph, blank, paragraph, item - the second item should NOT
        // get a blank line because there's no blank immediately before it.
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- First item
Paragraph text after first item.

More paragraph text.
- Second item (should have NO blank before it)"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the items
        let first_idx = lines.iter().position(|l| l.contains("First item")).unwrap();
        let second_idx = lines
            .iter()
            .position(|l| l.contains("Second item"))
            .unwrap();

        // The second item should NOT have a blank line before it from our preprocessing
        // (it may have one from TagEnd::Paragraph, but that's a separate rendering decision)
        // What we're testing is that the blank line before "More paragraph text" doesn't
        // cause our preprocessing to mark the second item as needing a blank line.

        // If the bug exists, find_items_needing_blank_lines would return {1} (second item)
        // With the fix, it should return {} (no items need blank lines from preprocessing)
        // We can't directly test the set, but we can verify the item doesn't get EXTRA spacing

        // Actually, let me verify by checking there's only one blank line before second item
        // (from TagEnd::Paragraph), not two (from both TagEnd::Paragraph and our preprocessing)
        let mut blank_count = 0;
        for i in (first_idx + 1)..second_idx {
            if lines[i].trim().is_empty() {
                blank_count += 1;
            }
        }

        // Should have at most 2 blank lines (one after "Paragraph text" paragraph end,
        // one for the explicit blank line in source before "More paragraph text")
        // If our preprocessing bug existed, there'd be an additional blank before the second item
        assert!(
            blank_count <= 2,
            "Should have at most 2 blank lines between items (not from preprocessing bug). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
    }

    #[test]
    fn lists_with_numeric_text_before_them_dont_shift_indices() {
        // Regression test: lines starting with digits like "2024 roadmap" should not be
        // counted as list items, which would shift indices and cause blank lines
        // to appear at wrong positions
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"2024 roadmap includes several initiatives.

1. First initiative
2. Second initiative

3. Third initiative (after blank line)"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the items
        let first_idx = lines
            .iter()
            .position(|l| l.contains("First initiative"))
            .unwrap();
        let second_idx = lines
            .iter()
            .position(|l| l.contains("Second initiative"))
            .unwrap();
        let third_idx = lines
            .iter()
            .position(|l| l.contains("Third initiative"))
            .unwrap();

        // No blank line between first and second (they're consecutive in source)
        assert_eq!(
            second_idx,
            first_idx + 1,
            "Second item should immediately follow first. Lines: {:#?}",
            lines
        );

        // Blank line before third (source has blank line)
        let line_before_third = &lines[third_idx - 1];
        assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );
    }

    #[test]
    fn lists_with_plus_markers_preserve_blank_lines() {
        // Regression test: + markers should be recognized as list items and preserve
        // blank lines from source, just like - and * markers
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"+ First item
+ Second item

+ Third item (after blank line)
+ Fourth item"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the items
        let first_idx = lines.iter().position(|l| l.contains("First item")).unwrap();
        let second_idx = lines
            .iter()
            .position(|l| l.contains("Second item"))
            .unwrap();
        let third_idx = lines.iter().position(|l| l.contains("Third item")).unwrap();
        let fourth_idx = lines
            .iter()
            .position(|l| l.contains("Fourth item"))
            .unwrap();

        // No blank line between first and second (consecutive in source)
        assert_eq!(
            second_idx,
            first_idx + 1,
            "Second item should immediately follow first. Lines: {:#?}",
            lines
        );

        // Blank line before third (source has blank line)
        let line_before_third = &lines[third_idx - 1];
        assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );

        // No blank line between third and fourth (consecutive in source)
        assert_eq!(
            fourth_idx,
            third_idx + 1,
            "Fourth item should immediately follow third. Lines: {:#?}",
            lines
        );
    }

    #[test]
    fn code_blocks_dont_shift_list_item_indices() {
        // Regression test: lines inside fenced code blocks that look like list items
        // should not increment item_index, which would shift indices and cause
        // blank lines to appear at wrong positions
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"Example code:
```
- not a real item
- also not real
```

- First real item
- Second real item

- Third real item (should have blank before)"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the real items (not the ones in the code block)
        let first_idx = lines
            .iter()
            .position(|l| l.contains("First real item"))
            .unwrap();
        let second_idx = lines
            .iter()
            .position(|l| l.contains("Second real item"))
            .unwrap();
        let third_idx = lines
            .iter()
            .position(|l| l.contains("Third real item"))
            .unwrap();

        // No blank line between first and second (consecutive in source)
        assert_eq!(
            second_idx,
            first_idx + 1,
            "Second item should immediately follow first. Lines: {:#?}",
            lines
        );

        // Blank line before third (source has blank line)
        let line_before_third = &lines[third_idx - 1];
        assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );
    }

    #[test]
    fn list_items_with_multiple_paragraphs_preserve_blank_lines() {
        // Regression test: blank lines between paragraphs within a single list item
        // should be preserved, not suppressed by the "skip blank after paragraph in list" logic
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- First paragraph in item

  Second paragraph in same item

- Next item"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Find the paragraphs
        let first_para_idx = lines
            .iter()
            .position(|l| l.contains("First paragraph"))
            .unwrap();
        let second_para_idx = lines
            .iter()
            .position(|l| l.contains("Second paragraph"))
            .unwrap();
        let next_item_idx = lines.iter().position(|l| l.contains("Next item")).unwrap();

        // Should have a blank line between the two paragraphs within the same item
        assert!(
            second_para_idx > first_para_idx + 1,
            "Second paragraph should have blank line before it. First at {}, Second at {}. Lines: {:#?}",
            first_para_idx,
            second_para_idx,
            lines
        );

        // Verify there's actually a blank line
        let line_between = &lines[first_para_idx + 1];
        assert!(
            line_between.trim().is_empty(),
            "Should have blank line between paragraphs in same item. Line {}: '{}'. Lines: {:#?}",
            first_para_idx + 1,
            line_between,
            lines
        );

        // Should also have blank line before next item
        let line_before_next = &lines[next_item_idx - 1];
        assert!(
            line_before_next.trim().is_empty(),
            "Should have blank line before next item. Line {}: '{}'. Lines: {:#?}",
            next_item_idx - 1,
            line_before_next,
            lines
        );
    }

    #[test]
    fn list_items_preserve_blank_lines_before_all_block_elements() {
        // Regression test: blank lines before code blocks, nested lists, and blockquotes
        // within list items should be preserved, not just blank lines before paragraphs
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- Introduction paragraph

  ```python
  code_example()
  ```

- Main point about something

  - Nested item one
  - Nested item two

- Context paragraph

  > Important quote here"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        // Test 1: Blank line before code block
        let intro_idx = lines
            .iter()
            .position(|l| l.contains("Introduction"))
            .unwrap();
        let code_idx = lines
            .iter()
            .position(|l| l.contains("code_example"))
            .unwrap();
        assert!(
            code_idx > intro_idx + 1,
            "Code block should have blank line before it. Intro at {}, Code at {}. Lines: {:#?}",
            intro_idx,
            code_idx,
            lines
        );

        // Test 2: Blank line before nested list
        let main_point_idx = lines.iter().position(|l| l.contains("Main point")).unwrap();
        let nested_one_idx = lines
            .iter()
            .position(|l| l.contains("Nested item one"))
            .unwrap();
        assert!(
            nested_one_idx > main_point_idx + 1,
            "Nested list should have blank line before it. Main at {}, Nested at {}. Lines: {:#?}",
            main_point_idx,
            nested_one_idx,
            lines
        );

        // Test 3: Blank line before blockquote
        let context_idx = lines
            .iter()
            .position(|l| l.contains("Context paragraph"))
            .unwrap();
        let quote_idx = lines
            .iter()
            .position(|l| l.contains("Important quote"))
            .unwrap();
        assert!(
            quote_idx > context_idx + 1,
            "Blockquote should have blank line before it. Context at {}, Quote at {}. Lines: {:#?}",
            context_idx,
            quote_idx,
            lines
        );
    }

    #[test]
    fn list_paragraphs_keep_indent_after_blank_lines() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- **Primary Concept**
  In designing a contemporary knowledge system, several foundational components must be conceptualized.

  Once normalized, data should be molded into adaptive knowledge graphs or relational mappings.
- Next item"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let detail_idx = lines
            .iter()
            .position(|l| l.contains("In designing a contemporary"))
            .unwrap();
        let followup_idx = lines
            .iter()
            .position(|l| l.contains("Once normalized"))
            .unwrap();

        assert!(
            lines[detail_idx].starts_with("  In designing"),
            "Detail paragraph should be indented under the list marker. Line: '{}'",
            lines[detail_idx]
        );
        assert!(
            lines[followup_idx].starts_with("  Once normalized"),
            "Follow-up paragraph should reuse list indent after blank line. Line: '{}'",
            lines[followup_idx]
        );
    }

    #[test]
    fn list_paragraphs_with_soft_breaks_keep_indent() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- **Primary Concept: The Architecture of a Modern Knowledge System**
  In designing a contemporary knowledge system, several foundational components must be conceptualized, integrated, and optimized for scalability.
  The architecture should balance **information retrieval efficiency**, **semantic accuracy**, and **human-centered accessibility**.
  Below is a structured decomposition of its design hierarchy:"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let designing_idx = lines
            .iter()
            .position(|l| l.contains("In designing a contemporary"))
            .unwrap();
        let architecture_idx = lines
            .iter()
            .position(|l| l.contains("The architecture should balance"))
            .unwrap();
        let below_idx = lines
            .iter()
            .position(|l| l.contains("Below is a structured decomposition"))
            .unwrap();

        assert!(
            lines[designing_idx].starts_with("  In designing"),
            "First soft-wrapped paragraph line should be indented under marker. Line: '{}'",
            lines[designing_idx]
        );
        assert!(
            lines[architecture_idx].starts_with("  The architecture"),
            "Second soft-wrapped paragraph line should keep list indent. Line: '{}'",
            lines[architecture_idx]
        );
        assert!(
            lines[below_idx].starts_with("  Below is"),
            "Third soft-wrapped paragraph line should keep list indent. Line: '{}'",
            lines[below_idx]
        );
    }

    #[test]
    fn nested_lists_with_single_blank_line_dont_double_space() {
        // Regression test: when a list item contains a nested list with a single blank
        // line before it, we should render ONE blank line, not two (one from TagEnd::Paragraph
        // peeking ahead and seeing Tag::List, and another from Tag::Item preprocessing)
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"- parent

  - child one
  - child two"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let parent_idx = lines.iter().position(|l| l.contains("parent")).unwrap();
        let child_idx = lines.iter().position(|l| l.contains("child one")).unwrap();

        // Count blank lines between parent and child
        let mut blank_count = 0;
        for i in (parent_idx + 1)..child_idx {
            if lines[i].trim().is_empty() {
                blank_count += 1;
            }
        }

        // Should have exactly 1 blank line, not 2
        assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between parent and nested child (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
    }

    #[test]
    fn blockquote_followed_by_list_has_single_blank_line() {
        // Regression test: when a blockquote is followed by a list with a single blank
        // line between them, we should render ONE blank line, not two (one from TagEnd::Paragraph
        // inside the blockquote, and another from TagEnd::BlockQuote)
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"> "Relax," it squeals, "we're diversified in hope and overdue library fines."

- **Merit:** it funds the dream of four walls and a window box."#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let quote_idx = lines.iter().position(|l| l.contains("Relax")).unwrap();
        let list_idx = lines.iter().position(|l| l.contains("Merit")).unwrap();

        // Count blank lines between blockquote and list
        let mut blank_count = 0;
        for i in (quote_idx + 1)..list_idx {
            if lines[i].trim().is_empty() {
                blank_count += 1;
            }
        }

        // Should have exactly 1 blank line, not 2
        assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and list (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
    }

    #[test]
    fn blockquote_followed_by_paragraph_has_single_blank_line() {
        // Similar issue: blockquote followed by a paragraph should preserve
        // the single blank line from the source, not double it
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"> Important quote here.

This is a paragraph after the quote."#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let quote_idx = lines.iter().position(|l| l.contains("Important quote")).unwrap();
        let para_idx = lines.iter().position(|l| l.contains("This is a paragraph")).unwrap();

        // Count blank lines between blockquote and paragraph
        let mut blank_count = 0;
        for i in (quote_idx + 1)..para_idx {
            if lines[i].trim().is_empty() {
                blank_count += 1;
            }
        }

        // Should have exactly 1 blank line, not 2
        assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and paragraph (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
    }

    #[test]
    fn blockquote_followed_by_heading_has_single_blank_line() {
        // Same issue with headings after blockquotes
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: r#"> Important quote here.

## Next Section"#
                .into(),
        };

        let rendered = render_markdown_for_test(&message, &theme, false, None);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();

        let quote_idx = lines.iter().position(|l| l.contains("Important quote")).unwrap();
        let heading_idx = lines.iter().position(|l| l.contains("Next Section")).unwrap();

        // Count blank lines between blockquote and heading
        let mut blank_count = 0;
        for i in (quote_idx + 1)..heading_idx {
            if lines[i].trim().is_empty() {
                blank_count += 1;
            }
        }

        // Should have exactly 1 blank line, not 2
        assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and heading (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
    }
}

const USER_CONTINUATION_INDENT: &str = "     ";

fn language_hint_from_codeblock_kind(kind: CodeBlockKind) -> String {
    match kind {
        CodeBlockKind::Fenced(lang) => lang.to_string(),
        _ => String::new(),
    }
}

fn push_codeblock_text(code_block_lines: &mut Vec<String>, text: &str) {
    for l in text.lines() {
        code_block_lines.push(detab(l));
    }
}

fn plain_codeblock_lines(code_block_lines: &[String], theme: &Theme) -> Vec<Line<'static>> {
    let mut style = theme.md_codeblock_text_style();
    if let Some(bg) = theme.md_codeblock_bg_color() {
        style = style.bg(bg);
    }
    code_block_lines
        .iter()
        .map(|line| Line::from(vec![Span::styled(line.clone(), style)]))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn flush_code_block_buffer(
    code_block_lines: &mut Vec<String>,
    syntax_enabled: bool,
    language_hint: Option<&str>,
    theme: &Theme,
    lines: &mut Vec<Line<'static>>,
    span_metadata: Option<&mut Vec<Vec<SpanKind>>>,
    list_indent: usize,
    block_index: usize,
) {
    if code_block_lines.is_empty() {
        return;
    }

    let produced_lines = if syntax_enabled {
        let joined = code_block_lines.join("\n");
        crate::utils::syntax::highlight_code_block(language_hint.unwrap_or(""), &joined, theme)
            .unwrap_or_else(|| plain_codeblock_lines(code_block_lines, theme))
    } else {
        plain_codeblock_lines(code_block_lines, theme)
    };

    let indent = if list_indent > 0 {
        Some(" ".repeat(list_indent))
    } else {
        None
    };

    if let Some(metadata) = span_metadata {
        for mut line in produced_lines {
            let has_indent = if let Some(indent) = indent.as_ref() {
                line.spans.insert(0, Span::raw(indent.clone()));
                true
            } else {
                false
            };

            // Convert empty language string to None
            let lang = language_hint.and_then(|s| if s.is_empty() { None } else { Some(s) });
            let code_block_kind = SpanKind::code_block(lang, block_index);

            // Build metadata: indent span (if any) is Text, code spans are CodeBlock
            let mut line_metadata = Vec::with_capacity(line.spans.len());
            for (i, _) in line.spans.iter().enumerate() {
                if i == 0 && has_indent {
                    // First span is the indent added for list nesting, not part of code
                    line_metadata.push(SpanKind::Text);
                } else {
                    line_metadata.push(code_block_kind.clone());
                }
            }

            metadata.push(line_metadata);
            lines.push(line);
        }
    } else {
        for mut line in produced_lines {
            if let Some(indent) = indent.as_ref() {
                line.spans.insert(0, Span::raw(indent.clone()));
            }
            lines.push(line);
        }
    }

    code_block_lines.clear();
}

fn detab(s: &str) -> String {
    // Simple, predictable detab: replace tabs with 4 spaces
    s.replace('\t', "    ")
}

fn render_plain_message(
    role: RoleKind,
    content: &str,
    theme: &Theme,
    collect_span_metadata: bool,
    user_display_name: Option<&str>,
) -> RenderedLinesWithMetadata {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut metadata: Vec<Vec<SpanKind>> = Vec::new();

    match role {
        RoleKind::User => {
            for (idx, line) in content.lines().enumerate() {
                let mut spans = Vec::new();
                let mut kinds = Vec::new();
                if idx == 0 {
                    let user_prefix = format!("{}: ", user_display_name.unwrap_or("You"));
                    spans.push(Span::styled(user_prefix, theme.user_prefix_style));
                    kinds.push(SpanKind::UserPrefix);
                } else {
                    spans.push(Span::raw(USER_CONTINUATION_INDENT));
                    kinds.push(SpanKind::Text);
                }
                spans.push(Span::styled(detab(line), theme.user_text_style));
                kinds.push(SpanKind::Text);
                if collect_span_metadata {
                    metadata.push(kinds);
                }
                lines.push(Line::from(spans));
            }
        }
        RoleKind::Assistant => {
            let style = base_text_style(role, theme);
            for line in content.lines() {
                let span = Span::styled(detab(line), style);
                if collect_span_metadata {
                    metadata.push(vec![SpanKind::Text]);
                }
                lines.push(Line::from(span));
            }
        }
        RoleKind::App(kind) => {
            let style = theme.app_message_style(kind);
            let indent_width = style.prefix.width().max(1);
            let indent = " ".repeat(indent_width);
            for (idx, line) in content.lines().enumerate() {
                let mut spans = Vec::new();
                let mut kinds = Vec::new();
                if idx == 0 {
                    spans.push(Span::styled(style.prefix.clone(), style.prefix_style));
                    kinds.push(SpanKind::AppPrefix);
                } else {
                    spans.push(Span::raw(indent.clone()));
                    kinds.push(SpanKind::Text);
                }
                spans.push(Span::styled(detab(line), style.text_style));
                kinds.push(SpanKind::Text);
                if collect_span_metadata {
                    metadata.push(kinds);
                }
                lines.push(Line::from(spans));
            }
        }
    }

    if !content.is_empty() {
        if collect_span_metadata {
            metadata.push(vec![SpanKind::Text]);
        }
        lines.push(Line::from(""));
    }

    (lines, metadata)
}

/// Build display lines for all messages using markdown rendering
#[cfg(test)]
pub fn build_markdown_display_lines(
    messages: &VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        let rendered =
            render_message_with_config(msg, theme, MessageRenderConfig::markdown(true, true))
                .into_rendered();
        lines.extend(rendered.lines);
    }
    lines
}
