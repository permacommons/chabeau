#![allow(clippy::items_after_test_module)]
use crate::core::message::Message;
use crate::ui::layout::MessageLineSpan;
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;
#[path = "markdown_wrap.rs"]
mod wrap;
use wrap::wrap_spans_to_width_generic_shared;

mod table;
pub(crate) use table::TableRenderer;

#[derive(Clone, Debug)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

const USER_CONTINUATION_INDENT: &str = "     ";

/// Description of a rendered message (line-based), used by the TUI renderer.
pub struct RenderedMessage {
    pub lines: Vec<Line<'static>>,
}

/// Extended render metadata used by the layout engine when downstream consumers
/// need code block ranges or per-message spans.
pub struct RenderedMessageDetails {
    pub lines: Vec<Line<'static>>,
    pub codeblock_ranges: Vec<(usize, usize, String)>,
    pub span_metadata: Option<Vec<Vec<SpanKind>>>,
}

type RenderedLinesWithMetadata = (
    Vec<Line<'static>>,
    Vec<(usize, usize, String)>,
    Vec<Vec<SpanKind>>,
);

impl RenderedMessageDetails {
    pub fn into_rendered(self) -> RenderedMessage {
        RenderedMessage { lines: self.lines }
    }
}

/// Render markdown with options to enable/disable syntax highlighting and terminal width for table balancing.
pub fn render_message_markdown_opts_with_width(
    msg: &Message,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
) -> RenderedMessage {
    // Backward-compatible wrapper that uses the default table policy (WrapCells)
    render_message_markdown_details_with_policy(
        msg,
        theme,
        syntax_enabled,
        terminal_width,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
    )
    .into_rendered()
}

pub fn render_message_markdown_details_with_policy(
    msg: &Message,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
    policy: crate::ui::layout::TableOverflowPolicy,
) -> RenderedMessageDetails {
    match msg.role.as_str() {
        "system" => {
            // Render system messages with markdown
            let (lines, ranges, metadata) = render_message_with_ranges_with_width_and_policy(
                RoleKind::Assistant, // Use assistant styling for system messages
                &msg.content,
                theme,
                syntax_enabled,
                terminal_width,
                policy,
            );
            RenderedMessageDetails {
                lines,
                codeblock_ranges: ranges,
                span_metadata: Some(metadata),
            }
        }
        "user" => {
            let (lines, ranges, metadata) = render_message_with_ranges_with_width_and_policy(
                RoleKind::User,
                &msg.content,
                theme,
                syntax_enabled,
                terminal_width,
                policy,
            );
            RenderedMessageDetails {
                lines,
                codeblock_ranges: ranges,
                span_metadata: Some(metadata),
            }
        }
        _ => {
            let (lines, ranges, metadata) = render_message_with_ranges_with_width_and_policy(
                RoleKind::Assistant,
                &msg.content,
                theme,
                syntax_enabled,
                terminal_width,
                policy,
            );
            RenderedMessageDetails {
                lines,
                codeblock_ranges: ranges,
                span_metadata: Some(metadata),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    User,
    Assistant,
}

fn base_text_style(role: RoleKind, theme: &Theme) -> Style {
    match role {
        RoleKind::User => theme.user_text_style,
        RoleKind::Assistant => theme.md_paragraph_style(),
    }
}

/// Configuration options for the markdown renderer abstraction.
#[derive(Clone, Copy, Debug, Default)]
struct MarkdownRendererConfig {
    collect_span_metadata: bool,
    syntax_highlighting: bool,
    width: Option<MarkdownWidthConfig>,
}

/// Width-aware configuration for optional wrapping and table layout.
#[derive(Clone, Copy, Debug)]
struct MarkdownWidthConfig {
    terminal_width: Option<usize>,
    table_policy: crate::ui::layout::TableOverflowPolicy,
}

struct MarkdownRenderer<'a> {
    role: RoleKind,
    content: &'a str,
    theme: &'a Theme,
    config: MarkdownRendererConfig,
    lines: Vec<Line<'static>>,
    span_metadata: Vec<Vec<SpanKind>>,
    ranges: Vec<(usize, usize, String)>,
    current_spans: Vec<Span<'static>>,
    current_span_kinds: Vec<SpanKind>,
    style_stack: Vec<Style>,
    kind_stack: Vec<SpanKind>,
    list_stack: Vec<ListKind>,
    in_code_block: Option<String>,
    code_block_lines: Vec<String>,
    table_state: Option<TableRenderer>,
    did_prefix_user: bool,
}

impl<'a> MarkdownRenderer<'a> {
    fn new(
        role: RoleKind,
        content: &'a str,
        theme: &'a Theme,
        config: MarkdownRendererConfig,
    ) -> Self {
        Self {
            role,
            content,
            theme,
            config,
            lines: Vec::new(),
            span_metadata: Vec::new(),
            ranges: Vec::new(),
            current_spans: Vec::new(),
            current_span_kinds: Vec::new(),
            style_stack: vec![base_text_style(role, theme)],
            kind_stack: vec![SpanKind::Text],
            list_stack: Vec::new(),
            in_code_block: None,
            code_block_lines: Vec::new(),
            table_state: None,
            did_prefix_user: role != RoleKind::User,
        }
    }

    fn render(mut self) -> RenderedLinesWithMetadata {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);
        let parser = Parser::new_ext(self.content, options);

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {
                        if self.role == RoleKind::User {
                            if !self.did_prefix_user {
                                self.push_span(
                                    Span::styled("You: ", self.theme.user_prefix_style),
                                    SpanKind::UserPrefix,
                                );
                                self.did_prefix_user = true;
                            } else {
                                self.push_span(Span::raw(USER_CONTINUATION_INDENT), SpanKind::Text);
                            }
                        }
                    }
                    Tag::Heading { level, .. } => {
                        self.flush_current_spans(true);
                        let style = self.theme.md_heading_style(level as u8);
                        if self.role == RoleKind::User && !self.did_prefix_user {
                            self.push_span(
                                Span::styled("You: ", self.theme.user_prefix_style),
                                SpanKind::UserPrefix,
                            );
                            self.did_prefix_user = true;
                        }
                        self.style_stack.push(style);
                        let current_kind =
                            self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                        self.kind_stack.push(current_kind);
                    }
                    Tag::BlockQuote => {
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
                    }
                    Tag::Item => {
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
                        if self.role == RoleKind::User && !self.did_prefix_user {
                            self.push_span(
                                Span::styled("You: ", self.theme.user_prefix_style),
                                SpanKind::UserPrefix,
                            );
                            self.did_prefix_user = true;
                        }
                        self.push_span(
                            Span::styled(marker, self.theme.md_list_marker_style()),
                            SpanKind::Text,
                        );
                    }
                    Tag::CodeBlock(kind) => {
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
                    Tag::Link { dest_url, .. } => {
                        self.style_stack.push(self.theme.md_link_style());
                        self.kind_stack.push(SpanKind::link(dest_url.as_ref()));
                    }
                    Tag::Table(_) => {
                        self.flush_current_spans(true);
                        if self.config.width.is_some() {
                            self.table_state = Some(TableRenderer::new());
                        }
                    }
                    Tag::TableHead => {
                        if let Some(ref mut table) = self.table_state {
                            table.start_header();
                        }
                    }
                    Tag::TableRow => {
                        if let Some(ref mut table) = self.table_state {
                            table.start_row();
                        }
                    }
                    Tag::TableCell => {
                        if let Some(ref mut table) = self.table_state {
                            table.start_cell();
                        }
                    }
                    _ => {}
                },
                Event::End(tag_end) => match tag_end {
                    TagEnd::Paragraph => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                    }
                    TagEnd::Heading(_) => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::BlockQuote => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::List(_) => {
                        self.flush_current_spans(true);
                        self.push_empty_line();
                        self.list_stack.pop();
                    }
                    TagEnd::Item => {
                        self.flush_current_spans(true);
                    }
                    TagEnd::CodeBlock => {
                        if self.config.collect_span_metadata {
                            flush_code_block_buffer(
                                &mut self.code_block_lines,
                                self.config.syntax_highlighting,
                                self.in_code_block.as_deref(),
                                self.theme,
                                &mut self.lines,
                                Some(&mut self.span_metadata),
                                &mut self.ranges,
                            );
                        } else {
                            flush_code_block_buffer(
                                &mut self.code_block_lines,
                                self.config.syntax_highlighting,
                                self.in_code_block.as_deref(),
                                self.theme,
                                &mut self.lines,
                                None,
                                &mut self.ranges,
                            );
                        }
                        self.push_empty_line();
                        self.in_code_block = None;
                    }
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                        self.style_stack.pop();
                        self.kind_stack.pop();
                    }
                    TagEnd::Table => {
                        if let Some(table) = self.table_state.take() {
                            if let Some(width_cfg) = self.config.width {
                                let table_lines = table.render_table_with_width_policy(
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
                        if let Some(ref mut table) = self.table_state {
                            table.end_header();
                        }
                    }
                    TagEnd::TableRow => {
                        if let Some(ref mut table) = self.table_state {
                            table.end_row();
                        }
                    }
                    TagEnd::TableCell => {
                        if let Some(ref mut table) = self.table_state {
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
                        if let Some(ref mut table) = self.table_state {
                            table.add_span(span, kind);
                        } else {
                            self.push_span(span, kind);
                        }
                    }
                }
                Event::Code(code) => {
                    let span = Span::styled(detab(&code), self.theme.md_inline_code_style());
                    let kind = self.kind_stack.last().cloned().unwrap_or(SpanKind::Text);
                    if let Some(ref mut table) = self.table_state {
                        table.add_span(span, kind);
                    } else {
                        self.push_span(span, kind);
                    }
                }
                Event::SoftBreak => {
                    self.flush_current_spans(true);
                    if self.role == RoleKind::User && self.did_prefix_user {
                        self.push_span(Span::raw(USER_CONTINUATION_INDENT), SpanKind::Text);
                    }
                }
                Event::HardBreak => {
                    self.flush_current_spans(true);
                }
                Event::Rule => {
                    self.flush_current_spans(true);
                    self.push_empty_line();
                }
                Event::TaskListMarker(_checked) => {
                    self.push_span(
                        Span::styled("[ ] ", self.theme.md_list_marker_style()),
                        SpanKind::Text,
                    );
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    if let Some(ref mut table) = self.table_state {
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

        (self.lines, self.ranges, metadata)
    }

    fn push_span(&mut self, span: Span<'static>, kind: SpanKind) {
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
                let wrapped = wrap_spans_to_width_generic_shared(&zipped, width);
                let indent_wrapped_user_lines = indent_user_wraps && self.role == RoleKind::User;
                for (idx, segs) in wrapped.into_iter().enumerate() {
                    let (mut spans_only, mut kinds_only): (Vec<_>, Vec<_>) =
                        segs.into_iter().unzip();
                    if idx == 0 || !indent_wrapped_user_lines {
                        self.push_line(spans_only, kinds_only);
                    } else {
                        let mut spans_with_indent = Vec::with_capacity(spans_only.len() + 1);
                        let mut kinds_with_indent = Vec::with_capacity(kinds_only.len() + 1);
                        spans_with_indent.push(Span::raw(USER_CONTINUATION_INDENT));
                        kinds_with_indent.push(SpanKind::Text);
                        spans_with_indent.append(&mut spans_only);
                        kinds_with_indent.append(&mut kinds_only);
                        self.push_line(spans_with_indent, kinds_with_indent);
                    }
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
}
/// Test-only helper: compute code block ranges across messages using
/// the simplified width-agnostic renderer. Intended for unit tests to
/// validate code block extraction and range mapping without involving
/// full table or width-aware rendering.
#[cfg(test)]
pub fn compute_codeblock_ranges(
    messages: &VecDeque<crate::core::message::Message>,
    theme: &Theme,
) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for msg in messages {
        let is_user = msg.role == "user";
        if msg.role == "system" {
            let (lines, _, _) = render_message_with_ranges_with_width_and_policy(
                RoleKind::Assistant,
                &msg.content,
                theme,
                true, // syntax_enabled
                None, // terminal_width
                crate::ui::layout::TableOverflowPolicy::WrapCells,
            );
            offset += lines.len();
            continue;
        }
        let role = if is_user {
            RoleKind::User
        } else {
            RoleKind::Assistant
        };
        let (lines, ranges, _) = MarkdownRenderer::new(
            role,
            &msg.content,
            theme,
            MarkdownRendererConfig {
                collect_span_metadata: false,
                syntax_highlighting: false,
                width: None,
            },
        )
        .render();
        for (start, len, content) in ranges {
            out.push((offset + start, len, content));
        }
        offset += lines.len();
    }
    out
}

/// Render message with width/policy and collect code block ranges aligned to produced lines.
fn render_message_with_ranges_with_width_and_policy(
    role: RoleKind,
    content: &str,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
    table_policy: crate::ui::layout::TableOverflowPolicy,
) -> RenderedLinesWithMetadata {
    let config = MarkdownRendererConfig {
        collect_span_metadata: true,
        syntax_highlighting: syntax_enabled,
        width: Some(MarkdownWidthConfig {
            terminal_width,
            table_policy,
        }),
    };
    MarkdownRenderer::new(role, content, theme, config).render()
}

/// Compute code block ranges aligned to width-aware rendering and table layout.
pub fn compute_codeblock_ranges_with_width_and_policy(
    messages: &VecDeque<crate::core::message::Message>,
    theme: &Theme,
    terminal_width: Option<usize>,
    policy: crate::ui::layout::TableOverflowPolicy,
    syntax_enabled: bool,
) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let (lines, _, _) = render_message_with_ranges_with_width_and_policy(
                    RoleKind::Assistant,
                    &msg.content,
                    theme,
                    syntax_enabled,
                    terminal_width,
                    policy,
                );
                offset += lines.len();
            }
            "user" => {
                let (lines, ranges, _) = render_message_with_ranges_with_width_and_policy(
                    RoleKind::User,
                    &msg.content,
                    theme,
                    syntax_enabled,
                    terminal_width,
                    policy,
                );
                for (start, len, content) in ranges {
                    out.push((offset + start, len, content));
                }
                offset += lines.len();
            }
            _ => {
                let (lines, ranges, _) = render_message_with_ranges_with_width_and_policy(
                    RoleKind::Assistant,
                    &msg.content,
                    theme,
                    syntax_enabled,
                    terminal_width,
                    policy,
                );
                for (start, len, content) in ranges {
                    out.push((offset + start, len, content));
                }
                offset += lines.len();
            }
        }
    }
    out
}
/// Provides only content and optional language hint for each code block, in order of appearance.
pub fn compute_codeblock_contents_with_lang(
    messages: &VecDeque<crate::core::message::Message>,
) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for msg in messages {
        if msg.role == "system" {
            continue;
        }
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);
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
        compute_codeblock_ranges, render_message_markdown_details_with_policy,
        render_message_markdown_opts_with_width, MarkdownRenderer, MarkdownRendererConfig,
        MarkdownWidthConfig, RoleKind, TableRenderer,
    };
    use crate::core::message::Message;
    use crate::ui::span::SpanKind;
    use crate::ui::theme::Theme;
    use crate::utils::test_utils::SAMPLE_HYPERTEXT_PARAGRAPH;
    use pulldown_cmark::{Options, Parser};
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use std::collections::VecDeque;
    use unicode_width::UnicodeWidthStr;

    fn render_message_with_ranges(
        syntax_enabled: bool,
        content: &str,
        theme: &Theme,
    ) -> (Vec<Line<'static>>, Vec<(usize, usize, String)>) {
        let (lines, ranges, _) = MarkdownRenderer::new(
            RoleKind::Assistant,
            content,
            theme,
            MarkdownRendererConfig {
                collect_span_metadata: false,
                syntax_highlighting: syntax_enabled,
                width: None,
            },
        )
        .render();
        (lines, ranges)
    }

    #[test]
    fn markdown_details_metadata_matches_lines_and_tags() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "Testing metadata with a [link](https://example.com) inside.".into(),
        };

        let details = render_message_markdown_details_with_policy(
            &message,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
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
        let details_with_width = render_message_markdown_details_with_policy(
            &message,
            &theme,
            true,
            width,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
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
    fn metadata_marks_user_prefix() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "user".into(),
            content: "Hello world".into(),
        };

        let details = render_message_markdown_details_with_policy(
            &message,
            &theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );

        let metadata = details.span_metadata.expect("metadata present");
        assert!(!metadata.is_empty());
        let first_line = &metadata[0];
        assert!(!first_line.is_empty());
        assert!(matches!(first_line[0], SpanKind::UserPrefix));
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

        let details = render_message_markdown_details_with_policy(
            &message,
            &theme,
            true,
            Some(50),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
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
    fn shared_renderer_without_metadata_matches_legacy_ranges() {
        let theme = crate::ui::theme::Theme::dark_default();
        let content = "Paragraph before\n\n```\nfirst\nsecond\n```\nparagraph after";

        let (legacy_lines, legacy_ranges) = render_message_with_ranges(false, content, &theme);
        let (lines, ranges, metadata) = MarkdownRenderer::new(
            RoleKind::Assistant,
            content,
            &theme,
            MarkdownRendererConfig {
                collect_span_metadata: false,
                syntax_highlighting: false,
                width: None,
            },
        )
        .render();

        assert_eq!(legacy_lines, lines);
        assert_eq!(legacy_ranges, ranges);
        assert!(
            metadata.is_empty(),
            "metadata should be empty when disabled"
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

        let expected = render_message_markdown_details_with_policy(
            &message,
            &theme,
            true,
            Some(48),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );

        let (lines, ranges, metadata) = MarkdownRenderer::new(
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
            },
        )
        .render();

        assert_eq!(expected.lines, lines);
        assert_eq!(expected.codeblock_ranges, ranges);
        let expected_metadata = expected
            .span_metadata
            .expect("details wrapper should provide metadata");
        assert_eq!(expected_metadata, metadata);
    }

    #[test]
    fn codeblock_ranges_map_correctly() {
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: "before\n```\nline1\nline2\n```\nafter".into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();
        let ranges = compute_codeblock_ranges(&messages, &theme);
        assert_eq!(ranges.len(), 1);
        let (_start, len, content) = &ranges[0];
        assert_eq!(*len, 2);
        assert_eq!(content, "line1\nline2");
    }

    #[test]
    fn width_aware_ranges_align_with_render_wrapping() {
        let theme = crate::ui::theme::Theme::dark_default();
        let mut messages = VecDeque::new();
        // A long paragraph that will wrap, followed by a fenced code block
        messages.push_back(Message {
            role: "assistant".into(),
            content: "This is a very long paragraph that should wrap when rendered at a small width so we can verify that the computed code block start index reflects wrapped lines.\n\n```\nfirst\nsecond\n```".into(),
        });

        let width = Some(20usize);
        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &messages,
            &theme,
            width,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            false,
        );
        assert_eq!(ranges.len(), 1);
        let (start, len, content) = &ranges[0];
        assert_eq!(*len, 2, "two code lines expected");
        assert_eq!(content, "first\nsecond");

        // Render the same message with width and ensure the lines at [start..start+len]
        // correspond to the code lines
        let rendered =
            super::render_message_markdown_opts_with_width(&messages[0], &theme, false, width);
        let lines: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();
        assert!(lines.len() > *start + *len);
        assert_eq!(lines[*start], "first");
        assert_eq!(lines[*start + 1], "second");
    }

    #[test]
    fn markdown_links_wrap_at_word_boundaries_with_width() {
        let theme = crate::ui::theme::Theme::dark_default();
        let message = Message {
            role: "assistant".into(),
            content: "abcd efgh [hypertext dreams](https://docs.hypertext.org) and more text"
                .into(),
        };

        let rendered =
            super::render_message_markdown_opts_with_width(&message, &theme, true, Some(10));
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

        let wider =
            super::render_message_markdown_opts_with_width(&message, &theme, true, Some(15));
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

        let rendered =
            super::render_message_markdown_opts_with_width(&message, &theme, true, Some(158));
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
    fn width_aware_ranges_account_for_preceding_table() {
        let theme = crate::ui::theme::Theme::dark_default();
        let mut messages = VecDeque::new();
        // Message 0: a table
        messages.push_back(Message {
            role: "assistant".into(),
            content: "| A | B |\n|---|---|\n| 1 | 2 |\n".into(),
        });
        // Message 1: a code block
        messages.push_back(Message {
            role: "assistant".into(),
            content: "```\nalpha\nbeta\n```".into(),
        });

        let width = Some(60usize);
        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &messages,
            &theme,
            width,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            false,
        );
        assert_eq!(ranges.len(), 1);
        let (start, len, content) = &ranges[0];
        assert_eq!(*len, 2, "two code lines expected");
        assert_eq!(content, "alpha\nbeta");

        // Build full rendering for both messages and assert selected span matches
        let rendered0 =
            super::render_message_markdown_opts_with_width(&messages[0], &theme, false, width);
        let rendered1 =
            super::render_message_markdown_opts_with_width(&messages[1], &theme, false, width);
        let combined: Vec<String> = rendered0
            .lines
            .iter()
            .chain(rendered1.lines.iter())
            .map(|l| l.to_string())
            .collect();
        assert!(combined.len() > *start + *len);
        assert_eq!(combined[*start], "alpha");
        assert_eq!(combined[*start + 1], "beta");
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
        let rendered = render_message_markdown_opts_with_width(&messages[0], &theme, true, None);

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
        let rendered = render_message_markdown_opts_with_width(&messages[0], &theme, true, None);
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(150));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(60));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(60));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(45));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(20));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(70));
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
        let rendered = render_message_markdown_opts_with_width(
            messages.front().unwrap(),
            &theme,
            true,
            Some(80),
        );
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(120));
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
        let rendered =
            render_message_markdown_opts_with_width(&messages[0], &theme, true, Some(60));
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
}

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

fn flush_code_block_buffer(
    code_block_lines: &mut Vec<String>,
    syntax_enabled: bool,
    language_hint: Option<&str>,
    theme: &Theme,
    lines: &mut Vec<Line<'static>>,
    span_metadata: Option<&mut Vec<Vec<SpanKind>>>,
    ranges: &mut Vec<(usize, usize, String)>,
) {
    if code_block_lines.is_empty() {
        return;
    }

    let start = lines.len();
    let joined = code_block_lines.join("\n");
    let produced_lines = if syntax_enabled {
        crate::utils::syntax::highlight_code_block(language_hint.unwrap_or(""), &joined, theme)
            .unwrap_or_else(|| plain_codeblock_lines(code_block_lines, theme))
    } else {
        plain_codeblock_lines(code_block_lines, theme)
    };

    if let Some(metadata) = span_metadata {
        for line in produced_lines {
            metadata.push(vec![SpanKind::Text; line.spans.len()]);
            lines.push(line);
        }
    } else {
        lines.extend(produced_lines);
    }

    let end = lines.len();
    if end > start {
        ranges.push((start, end - start, joined));
    }

    code_block_lines.clear();
}

fn detab(s: &str) -> String {
    // Simple, predictable detab: replace tabs with 4 spaces
    s.replace('\t', "    ")
}

/// Build display lines for all messages using markdown rendering
#[cfg(test)]
pub fn build_markdown_display_lines(
    messages: &VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        let rendered = render_message_markdown_opts_with_width(msg, theme, true, None);
        lines.extend(rendered.lines);
    }
    lines
}

pub fn build_plain_display_lines_with_spans(
    messages: &VecDeque<Message>,
    theme: &Theme,
) -> (Vec<Line<'static>>, Vec<MessageLineSpan>) {
    let mut lines = Vec::new();
    let mut spans = Vec::with_capacity(messages.len());
    for msg in messages {
        let start = lines.len();
        match msg.role.as_str() {
            "system" => {
                let (rendered_lines, _, _) = render_message_with_ranges_with_width_and_policy(
                    RoleKind::Assistant,
                    &msg.content,
                    theme,
                    true, // syntax_enabled
                    None, // terminal_width
                    crate::ui::layout::TableOverflowPolicy::WrapCells,
                );
                let len = rendered_lines.len();
                lines.extend(rendered_lines);
                spans.push(MessageLineSpan { start, len });
            }
            "user" => {
                for (i, line) in msg.content.lines().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled("You: ", theme.user_prefix_style),
                            Span::styled(detab(line), theme.user_text_style),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw("     "),
                            Span::styled(detab(line), theme.user_text_style),
                        ]));
                    }
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
                spans.push(MessageLineSpan {
                    start,
                    len: lines.len().saturating_sub(start),
                });
            }
            _ => {
                for line in msg.content.lines() {
                    lines.push(Line::from(Span::styled(
                        detab(line),
                        theme.md_paragraph_style(),
                    )));
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
                spans.push(MessageLineSpan {
                    start,
                    len: lines.len().saturating_sub(start),
                });
            }
        }
    }
    (lines, spans)
}
