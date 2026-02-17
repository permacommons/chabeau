use super::code::{
    flush_code_block_buffer, language_hint_from_codeblock_kind, push_codeblock_text,
};
use super::lists::{ListKind, MAX_LIST_HANGING_INDENT_WIDTH};
use super::metadata::RenderedMessageDetails;
use super::parser::find_items_needing_blank_lines;
use super::table::TableRenderer;
use super::wrap::wrap_spans_to_width_generic_shared;
use crate::core::message::{self, AppMessageKind, Message, TranscriptRole};
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
#[cfg(test)]
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;

type RenderedLinesWithMetadata = (Vec<Line<'static>>, Vec<Vec<SpanKind>>);

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
    ToolCall,
    ToolResult,
}

impl RoleKind {
    fn from_message(msg: &Message) -> Self {
        if msg.role == TranscriptRole::User {
            RoleKind::User
        } else if msg.role == TranscriptRole::Assistant {
            RoleKind::Assistant
        } else if msg.role == message::TranscriptRole::ToolCall {
            RoleKind::ToolCall
        } else if msg.role == message::TranscriptRole::ToolResult {
            RoleKind::ToolResult
        } else if message::is_app_message_role(msg.role) {
            RoleKind::App(message::app_message_kind_from_role(msg.role))
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
        RoleKind::ToolCall | RoleKind::ToolResult => {
            theme.app_message_style(AppMessageKind::Info).text_style
        }
    }
}

fn tool_prefix(role: RoleKind) -> Option<&'static str> {
    match role {
        RoleKind::ToolCall => Some("Tool call: "),
        RoleKind::ToolResult => Some("Tool result: "),
        _ => None,
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
pub(super) struct MarkdownRendererConfig {
    pub(super) collect_span_metadata: bool,
    pub(super) syntax_highlighting: bool,
    pub(super) width: Option<MarkdownWidthConfig>,
    pub(super) user_display_name: Option<String>,
}

/// Width-aware configuration for optional wrapping and table layout.
#[derive(Clone, Copy, Debug)]
pub(super) struct MarkdownWidthConfig {
    pub(super) terminal_width: Option<usize>,
    pub(super) table_policy: crate::ui::layout::TableOverflowPolicy,
}

pub fn render_message_with_config(
    msg: &Message,
    theme: &Theme,
    config: MessageRenderConfig,
) -> RenderedMessageDetails {
    let role = RoleKind::from_message(msg);
    let use_markdown = config.markdown && !matches!(role, RoleKind::ToolCall);
    let (mut lines, mut metadata) = if use_markdown {
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
    if !use_markdown {
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

pub(super) struct MarkdownRenderer<'a> {
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
    pub(super) fn new(
        role: RoleKind,
        content: &'a str,
        theme: &'a Theme,
        config: MarkdownRendererConfig,
    ) -> Self {
        let app_prefix_indent = match role {
            RoleKind::App(kind) => {
                let prefix = theme.app_message_style(kind).prefix.clone();
                let width = prefix.width().max(1);
                Some(" ".repeat(width))
            }
            RoleKind::ToolCall | RoleKind::ToolResult => tool_prefix(role).map(|prefix| {
                let width = prefix.width().max(1);
                " ".repeat(width)
            }),
            _ => None,
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
            items_needing_blank_lines_before: find_items_needing_blank_lines(content),
            current_item_index: 0,
            in_code_block: None,
            code_block_lines: Vec::new(),
            code_block_count: 0,
            table_renderer: None,
            did_prefix: !matches!(
                role,
                RoleKind::User | RoleKind::App(_) | RoleKind::ToolCall | RoleKind::ToolResult
            ),
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
            RoleKind::ToolCall | RoleKind::ToolResult => {
                let style = self.theme.app_message_style(AppMessageKind::Info);
                if !self.did_prefix {
                    if let Some(prefix) = tool_prefix(self.role) {
                        self.push_span(
                            Span::styled(prefix.to_string(), style.prefix_style),
                            SpanKind::AppPrefix,
                        );
                    }
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
            RoleKind::ToolCall | RoleKind::ToolResult => {
                if !self.did_prefix {
                    let style = self.theme.app_message_style(AppMessageKind::Info);
                    if let Some(prefix) = tool_prefix(self.role) {
                        self.push_span(
                            Span::styled(prefix.to_string(), style.prefix_style),
                            SpanKind::AppPrefix,
                        );
                    }
                    self.did_prefix = true;
                }
            }
            RoleKind::Assistant => {}
        }
    }

    pub(super) fn render(mut self) -> RenderedLinesWithMetadata {
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
                        if matches!(
                            self.role,
                            RoleKind::User
                                | RoleKind::App(_)
                                | RoleKind::ToolCall
                                | RoleKind::ToolResult
                        ) {
                            self.ensure_role_prefix_or_indent();
                        }
                        if self.pending_list_indent.is_none() && !self.list_stack.is_empty() {
                            self.pending_list_indent = Some(self.current_list_indent_width());
                        }
                    }
                    Tag::Heading { level, .. } => {
                        self.flush_current_spans(true);
                        let style = self.theme.md_heading_style(level as u8);
                        if matches!(
                            self.role,
                            RoleKind::User
                                | RoleKind::App(_)
                                | RoleKind::ToolCall
                                | RoleKind::ToolResult
                        ) {
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
                        if matches!(
                            self.role,
                            RoleKind::User
                                | RoleKind::App(_)
                                | RoleKind::ToolCall
                                | RoleKind::ToolResult
                        ) {
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
                        // Don't add blank line here - all inner elements (paragraph, heading,
                        // code block, list, nested blockquote) already add their own trailing
                        // blank lines, so adding another here would double-space.
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
                    if matches!(
                        self.role,
                        RoleKind::User
                            | RoleKind::App(_)
                            | RoleKind::ToolCall
                            | RoleKind::ToolResult
                    ) && self.did_prefix
                    {
                        match self.role {
                            RoleKind::User => {
                                self.push_span(Span::raw(USER_CONTINUATION_INDENT), SpanKind::Text);
                            }
                            RoleKind::App(_) => {
                                if let Some(indent) = self.app_prefix_indent.clone() {
                                    self.push_span(Span::raw(indent), SpanKind::Text);
                                }
                            }
                            RoleKind::ToolCall | RoleKind::ToolResult => {
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
                let indent_wrapped_user_lines = indent_user_wraps
                    && matches!(
                        self.role,
                        RoleKind::User
                            | RoleKind::App(_)
                            | RoleKind::ToolCall
                            | RoleKind::ToolResult
                    );
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
                            RoleKind::ToolCall | RoleKind::ToolResult => Span::raw(
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
            RoleKind::ToolCall | RoleKind::ToolResult => self
                .app_prefix_indent
                .as_deref()
                .map(UnicodeWidthStr::width)
                .unwrap_or(1),
            RoleKind::Assistant => 0,
        }
    }
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
        RoleKind::ToolCall | RoleKind::ToolResult => {
            let style = theme.app_message_style(AppMessageKind::Info);
            let prefix = tool_prefix(role).unwrap_or("Tool: ");
            let indent_width = prefix.width().max(1);
            let indent = " ".repeat(indent_width);
            for (idx, line) in content.lines().enumerate() {
                let mut spans = Vec::new();
                let mut kinds = Vec::new();
                if idx == 0 {
                    spans.push(Span::styled(prefix.to_string(), style.prefix_style));
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

const USER_CONTINUATION_INDENT: &str = "     ";
