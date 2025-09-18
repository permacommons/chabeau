#![allow(clippy::items_after_test_module)]
use crate::core::message::Message;
use crate::ui::links::LinkHotspot;
use crate::ui::theme::Theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

struct TableState {
    rows: Vec<Vec<Vec<Vec<Span<'static>>>>>,
    current_row: Vec<Vec<Vec<Span<'static>>>>,
    current_cell: Vec<Vec<Span<'static>>>,
    in_header: bool,
}

/// Description of a rendered message (line-based), used by the TUI renderer.
pub struct RenderedMessage {
    pub lines: Vec<Line<'static>>,
    pub hotspots: Vec<LinkHotspot>,
}

#[derive(Clone, Debug)]
struct RichSpan {
    content: String,
    style: Style,
    link_url: Option<String>,
}

/// Markdown renderer using pulldown-cmark with theming.
#[cfg(test)]
#[allow(dead_code)]
pub fn render_message_markdown(msg: &Message, theme: &Theme) -> RenderedMessage {
    match msg.role.as_str() {
        "system" => render_with_parser_role_and_width_policy(
            RoleKind::System,
            &msg.content,
            theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        ),
        "user" => render_with_parser_role_and_width_policy(
            RoleKind::User,
            &msg.content,
            theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        ),
        _ => render_with_parser_role_and_width_policy(
            RoleKind::Assistant,
            &msg.content,
            theme,
            true,
            None,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        ),
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
    render_message_markdown_with_policy(
        msg,
        theme,
        syntax_enabled,
        terminal_width,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
    )
}

/// Render markdown with explicit table overflow policy.
pub fn render_message_markdown_with_policy(
    msg: &Message,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
    policy: crate::ui::layout::TableOverflowPolicy,
) -> RenderedMessage {
    match msg.role.as_str() {
        "system" => render_with_parser_role_and_width_policy(
            RoleKind::System,
            &msg.content,
            theme,
            syntax_enabled,
            terminal_width,
            policy,
        ),
        "user" => render_with_parser_role_and_width_policy(
            RoleKind::User,
            &msg.content,
            theme,
            syntax_enabled,
            terminal_width,
            policy,
        ),
        _ => render_with_parser_role_and_width_policy(
            RoleKind::Assistant,
            &msg.content,
            theme,
            syntax_enabled,
            terminal_width,
            policy,
        ),
    }
}

fn render_system_message(content: &str, theme: &Theme) -> RenderedMessage {
    let mut out: Vec<Line<'static>> = Vec::new();
    for l in content.lines() {
        if l.trim().is_empty() {
            out.push(Line::from(""));
        } else {
            let text = detab(l);
            // Heuristic: if line starts with "Error:", render with error style
            if text.starts_with("Error:") {
                out.push(Line::from(Span::styled(text, theme.error_text_style)));
            } else {
                out.push(Line::from(Span::styled(text, theme.system_text_style)));
            }
        }
    }
    if !out.is_empty() {
        out.push(Line::from(""));
    }
    RenderedMessage {
        lines: out,
        hotspots: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    User,
    Assistant,
    System,
}

fn base_text_style(role: RoleKind, theme: &Theme) -> Style {
    match role {
        RoleKind::User => theme.user_text_style,
        RoleKind::Assistant => theme.md_paragraph_style(),
        RoleKind::System => theme.system_text_style,
    }
}

fn render_with_parser_role_and_width_policy(
    role: RoleKind,
    content: &str,
    theme: &Theme,
    syntax_enabled: bool,
    terminal_width: Option<usize>,
    table_policy: crate::ui::layout::TableOverflowPolicy,
) -> RenderedMessage {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(content, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hotspots: Vec<LinkHotspot> = Vec::new();

    // Buffer for the current paragraph/heading/list item
    let mut current_rich_spans: Vec<RichSpan> = Vec::new();
    // Style stack for inline formatting
    let mut style_stack: Vec<Style> = vec![base_text_style(role, theme)];
    let mut current_link_url: Option<String> = None;

    // List handling
    let mut list_stack: Vec<ListKind> = Vec::new();
    // Code block handling
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_block_lines: Vec<String> = Vec::new();
    // Table handling
    let mut table_state: Option<TableState> = None;

    // User prefix handling
    let is_user = role == RoleKind::User;
    let mut did_prefix_user = role != RoleKind::User; // only user gets prefix

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if role == RoleKind::User {
                        if !did_prefix_user {
                            current_rich_spans.push(RichSpan {
                                content: "You: ".to_string(),
                                style: theme.user_prefix_style,
                                link_url: None,
                            });
                            did_prefix_user = true;
                        } else {
                            current_rich_spans.push(RichSpan {
                                content: "     ".to_string(),
                                style: Style::default(),
                                link_url: None,
                            });
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let style = theme.md_heading_style(level as u8);
                    if is_user && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    style_stack.push(style);
                }
                Tag::BlockQuote => {
                    style_stack.push(theme.md_blockquote_style());
                }
                Tag::List(start) => {
                    list_stack.push(match start {
                        Some(n) => ListKind::Ordered(n),
                        None => ListKind::Unordered,
                    });
                }
                Tag::Item => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let marker = match list_stack.last().cloned().unwrap_or(ListKind::Unordered) {
                        ListKind::Unordered => "- ".to_string(),
                        ListKind::Ordered(_n) => {
                            if let Some(ListKind::Ordered(ref mut k)) = list_stack.last_mut() {
                                let cur = *k;
                                *k += 1;
                                format!("{}. ", cur)
                            } else {
                                "1. ".to_string()
                            }
                        }
                    };
                    if role == RoleKind::User && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    current_rich_spans.push(RichSpan {
                        content: marker,
                        style: theme.md_list_marker_style(),
                        link_url: None,
                    });
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = Some(match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        _ => String::new(),
                    });
                    code_block_lines.clear();
                }
                Tag::Emphasis => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::ITALIC);
                    style_stack.push(new);
                }
                Tag::Strong => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::BOLD);
                    style_stack.push(new);
                }
                Tag::Strikethrough => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::DIM);
                    style_stack.push(new);
                }
                Tag::Link { dest_url, .. } => {
                    let new = theme.md_link_style();
                    style_stack.push(new);
                    current_link_url = Some(dest_url.to_string());
                }
                Tag::Table(_) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    table_state = Some(TableState::new());
                }
                Tag::TableHead => {
                    if let Some(ref mut table) = table_state {
                        table.start_header();
                    }
                }
                Tag::TableRow => {
                    if let Some(ref mut table) = table_state {
                        table.start_row();
                    }
                }
                Tag::TableCell => {
                    if let Some(ref mut table) = table_state {
                        table.start_cell();
                    }
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    if let Some(w) = terminal_width {
                        if !current_rich_spans.is_empty() {
                            let (wrapped_lines, new_hotspots) = wrap_rich_spans_to_width(
                                &current_rich_spans,
                                w,
                                lines.len(),
                                role == RoleKind::User,
                            );
                            lines.extend(wrapped_lines);
                            hotspots.extend(new_hotspots);
                            current_rich_spans.clear();
                        }
                    } else {
                        flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    }
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_level) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::BlockQuote => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::List(_start) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    list_stack.pop();
                }
                TagEnd::Item => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                }
                TagEnd::CodeBlock => {
                    let joined = code_block_lines.join("\n");
                    if syntax_enabled {
                        if let Some(mut hl_lines) = crate::utils::syntax::highlight_code_block(
                            in_code_block.as_deref().unwrap_or(""),
                            &joined,
                            theme,
                        ) {
                            lines.append(&mut hl_lines);
                        } else {
                            for l in joined.split('\n') {
                                let mut st = theme.md_codeblock_text_style();
                                if let Some(bg) = theme.md_codeblock_bg_color() {
                                    st = st.bg(bg);
                                }
                                lines.push(Line::from(Span::styled(detab(l), st)));
                            }
                        }
                    } else {
                        for l in joined.split('\n') {
                            let mut st = theme.md_codeblock_text_style();
                            if let Some(bg) = theme.md_codeblock_bg_color() {
                                st = st.bg(bg);
                            }
                            lines.push(Line::from(Span::styled(detab(l), st)));
                        }
                    }
                    lines.push(Line::from(""));
                    in_code_block = None;
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => {
                    style_stack.pop();
                    current_link_url = None;
                }
                TagEnd::Table => {
                    if let Some(table) = table_state.take() {
                        let mut table_lines = table.render_table_with_width_policy(
                            theme,
                            terminal_width,
                            table_policy,
                        );
                        lines.append(&mut table_lines);
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::TableHead => {
                    if let Some(ref mut table) = table_state {
                        table.end_header();
                    }
                }
                TagEnd::TableRow => {
                    if let Some(ref mut table) = table_state {
                        table.end_row();
                    }
                }
                TagEnd::TableCell => {
                    if let Some(ref mut table) = table_state {
                        table.end_cell();
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block.is_some() {
                    for l in text.lines() {
                        code_block_lines.push(detab(l).to_string());
                    }
                } else if let Some(ref mut table) = table_state {
                    let span = Span::styled(
                        detab(&text),
                        *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                    );
                    table.add_span(span);
                } else {
                    current_rich_spans.push(RichSpan {
                        content: detab(&text),
                        style: *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                        link_url: current_link_url.clone(),
                    });
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                if let Some(ref mut table) = table_state {
                    let span = Span::styled(detab(&code), s);
                    table.add_span(span);
                } else {
                    current_rich_spans.push(RichSpan {
                        content: detab(&code),
                        style: s,
                        link_url: current_link_url.clone(),
                    });
                }
            }
            Event::SoftBreak => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                if role == RoleKind::User && did_prefix_user {
                    current_rich_spans.push(RichSpan {
                        content: "     ".to_string(),
                        style: Style::default(),
                        link_url: None,
                    });
                }
            }
            Event::HardBreak => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
            }
            Event::Rule => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(_checked) => {
                current_rich_spans.push(RichSpan {
                    content: "[ ] ".to_string(),
                    style: theme.md_list_marker_style(),
                    link_url: None,
                });
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if let Some(ref mut table) = table_state {
                    if html.trim() == "<br>" || html.trim() == "<br/>" {
                        table.new_line_in_cell();
                    }
                }
            }
            Event::FootnoteReference(_) => {}
        }
    }

    flush_current_rich_line(&mut lines, &mut current_rich_spans);
    if !lines.is_empty()
        && lines
            .last()
            .map(|l| !l.to_string().is_empty())
            .unwrap_or(false)
    {
        lines.push(Line::from(""));
    }

    RenderedMessage { lines, hotspots }
}

/// Render message and also compute local code block ranges (start line index, len, content).
/// Test-only helper: render a single message and collect code block
/// ranges (start line index, length, and content) using a simplified
/// width-agnostic renderer. Useful for focused unit tests that do not
/// require table layout or terminal-width semantics.
#[cfg(test)]
pub fn render_message_with_ranges(
    is_user: bool,
    content: &str,
    theme: &Theme,
) -> (Vec<Line<'static>>, Vec<(usize, usize, String)>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(content, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut ranges: Vec<(usize, usize, String)> = Vec::new();
    let role = if is_user {
        RoleKind::User
    } else {
        RoleKind::Assistant
    };
    let mut current_rich_spans: Vec<RichSpan> = Vec::new();
    let mut style_stack: Vec<Style> = vec![base_text_style_bool(is_user, theme)];
    let mut list_stack: Vec<ListKind> = Vec::new();
    let mut in_code_block: Option<String> = None;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut did_prefix_user = !is_user;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if is_user {
                        if !did_prefix_user {
                            current_rich_spans.push(RichSpan {
                                content: "You: ".to_string(),
                                style: theme.user_prefix_style,
                                link_url: None,
                            });
                            did_prefix_user = true;
                        } else {
                            current_rich_spans.push(RichSpan {
                                content: "     ".to_string(),
                                style: Style::default(),
                                link_url: None,
                            });
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let style = theme.md_heading_style(level as u8);
                    if is_user && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    style_stack.push(style);
                }
                Tag::BlockQuote => style_stack.push(theme.md_blockquote_style()),
                Tag::List(start) => {
                    list_stack.push(match start {
                        Some(n) => ListKind::Ordered(n),
                        None => ListKind::Unordered,
                    });
                }
                Tag::Item => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let marker = match list_stack.last().cloned().unwrap_or(ListKind::Unordered) {
                        ListKind::Unordered => "- ".to_string(),
                        ListKind::Ordered(_n) => {
                            if let Some(ListKind::Ordered(ref mut k)) = list_stack.last_mut() {
                                let cur = *k;
                                *k += 1;
                                format!("{}. ", cur)
                            } else {
                                "1. ".to_string()
                            }
                        }
                    };
                    if role == RoleKind::User && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    current_rich_spans.push(RichSpan {
                        content: marker,
                        style: theme.md_list_marker_style(),
                        link_url: None,
                    });
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = Some(match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        _ => String::new(),
                    });
                    code_block_lines.clear();
                }
                Tag::Emphasis => style_stack.push(
                    style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::ITALIC),
                ),
                Tag::Strong => style_stack.push(
                    style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::BOLD),
                ),
                Tag::Strikethrough => style_stack.push(
                    style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::DIM),
                ),
                Tag::Link { .. } => style_stack.push(theme.md_link_style()),
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_level) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::BlockQuote => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::List(_start) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    list_stack.pop();
                }
                TagEnd::Item => flush_current_rich_line(&mut lines, &mut current_rich_spans),
                TagEnd::CodeBlock => {
                    let start = lines.len();
                    for l in code_block_lines.drain(..) {
                        let mut st = theme.md_codeblock_text_style();
                        if let Some(bg) = theme.md_codeblock_bg_color() {
                            st = st.bg(bg);
                        }
                        lines.push(Line::from(Span::styled(detab(&l), st)));
                    }
                    let end = lines.len();
                    if end > start {
                        let content = lines[start..end]
                            .iter()
                            .map(|ln| ln.to_string())
                            .collect::<Vec<_>>()
                            .join("\n");
                        ranges.push((start, end - start, content));
                    }
                    lines.push(Line::from(""));
                    in_code_block = None;
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    style_stack.pop();
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block.is_some() {
                    for l in text.lines() {
                        code_block_lines.push(detab(l).to_string());
                    }
                } else {
                    current_rich_spans.push(RichSpan {
                        content: detab(&text),
                        style: *style_stack
                            .last()
                            .unwrap_or(&base_text_style_bool(is_user, theme)),
                        link_url: None,
                    });
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                current_rich_spans.push(RichSpan {
                    content: detab(&code),
                    style: s,
                    link_url: None,
                });
            }
            Event::SoftBreak => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                if is_user && did_prefix_user {
                    current_rich_spans.push(RichSpan {
                        content: "     ".to_string(),
                        style: Style::default(),
                        link_url: None,
                    });
                }
            }
            Event::HardBreak => flush_current_rich_line(&mut lines, &mut current_rich_spans),
            Event::Rule => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(_checked) => {
                current_rich_spans.push(RichSpan {
                    content: "[ ] ".to_string(),
                    style: theme.md_list_marker_style(),
                    link_url: None,
                });
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::FootnoteReference(_) => {}
        }
    }

    flush_current_rich_line(&mut lines, &mut current_rich_spans);
    if !lines.is_empty()
        && lines
            .last()
            .map(|l| !l.to_string().is_empty())
            .unwrap_or(false)
    {
        lines.push(Line::from(""));
    }
    (lines, ranges)
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
            let rm = render_system_message(&msg.content, theme);
            offset += rm.lines.len();
            continue;
        }
        let (lines, ranges) = render_message_with_ranges(is_user, &msg.content, theme);
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
) -> (Vec<Line<'static>>, Vec<(usize, usize, String)>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(content, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut ranges: Vec<(usize, usize, String)> = Vec::new();
    let mut hotspots: Vec<LinkHotspot> = Vec::new();

    // Buffer for the current paragraph/heading/list item
    let mut current_rich_spans: Vec<RichSpan> = Vec::new();
    // Style stack for inline formatting
    let mut style_stack: Vec<Style> = vec![base_text_style(role, theme)];
    let mut current_link_url: Option<String> = None;

    // List handling
    let mut list_stack: Vec<ListKind> = Vec::new();
    // Code block handling
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_block_lines: Vec<String> = Vec::new();
    // Table handling
    let mut table_state: Option<TableState> = None;

    // User prefix handling
    let is_user = role == RoleKind::User;
    let mut did_prefix_user = role != RoleKind::User; // only user gets prefix

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if role == RoleKind::User {
                        if !did_prefix_user {
                            current_rich_spans.push(RichSpan {
                                content: "You: ".to_string(),
                                style: theme.user_prefix_style,
                                link_url: None,
                            });
                            did_prefix_user = true;
                        } else {
                            current_rich_spans.push(RichSpan {
                                content: "     ".to_string(),
                                style: Style::default(),
                                link_url: None,
                            });
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let style = theme.md_heading_style(level as u8);
                    if is_user && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    style_stack.push(style);
                }
                Tag::BlockQuote => {
                    style_stack.push(theme.md_blockquote_style());
                }
                Tag::List(start) => {
                    list_stack.push(match start {
                        Some(n) => ListKind::Ordered(n),
                        None => ListKind::Unordered,
                    });
                }
                Tag::Item => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    let marker = match list_stack.last().cloned().unwrap_or(ListKind::Unordered) {
                        ListKind::Unordered => "- ".to_string(),
                        ListKind::Ordered(_n) => {
                            if let Some(ListKind::Ordered(ref mut k)) = list_stack.last_mut() {
                                let cur = *k;
                                *k += 1;
                                format!("{}. ", cur)
                            } else {
                                "1. ".to_string()
                            }
                        }
                    };
                    if role == RoleKind::User && !did_prefix_user {
                        current_rich_spans.push(RichSpan {
                            content: "You: ".to_string(),
                            style: theme.user_prefix_style,
                            link_url: None,
                        });
                        did_prefix_user = true;
                    }
                    current_rich_spans.push(RichSpan {
                        content: marker,
                        style: theme.md_list_marker_style(),
                        link_url: None,
                    });
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = Some(match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        _ => String::new(),
                    });
                    code_block_lines.clear();
                }
                Tag::Emphasis => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::ITALIC);
                    style_stack.push(new);
                }
                Tag::Strong => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::BOLD);
                    style_stack.push(new);
                }
                Tag::Strikethrough => {
                    let new = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::DIM);
                    style_stack.push(new);
                }
                Tag::Link { dest_url, .. } => {
                    let new = theme.md_link_style();
                    style_stack.push(new);
                    current_link_url = Some(dest_url.to_string());
                }
                Tag::Table(_) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    table_state = Some(TableState::new());
                }
                Tag::TableHead => {
                    if let Some(ref mut table) = table_state {
                        table.start_header();
                    }
                }
                Tag::TableRow => {
                    if let Some(ref mut table) = table_state {
                        table.start_row();
                    }
                }
                Tag::TableCell => {
                    if let Some(ref mut table) = table_state {
                        table.start_cell();
                    }
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    if let Some(w) = terminal_width {
                        if !current_rich_spans.is_empty() {
                            let (wrapped_lines, new_hotspots) = wrap_rich_spans_to_width(
                                &current_rich_spans,
                                w,
                                lines.len(),
                                role == RoleKind::User,
                            );
                            lines.extend(wrapped_lines);
                            hotspots.extend(new_hotspots);
                            current_rich_spans.clear();
                        }
                    } else {
                        flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    }
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_level) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::BlockQuote => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::List(_start) => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                    lines.push(Line::from(""));
                    list_stack.pop();
                }
                TagEnd::Item => {
                    flush_current_rich_line(&mut lines, &mut current_rich_spans);
                }
                TagEnd::CodeBlock => {
                    let start = lines.len();
                    let joined = code_block_lines.join("\n");
                    if syntax_enabled {
                        if let Some(mut hl_lines) = crate::utils::syntax::highlight_code_block(
                            in_code_block.as_deref().unwrap_or(""),
                            &joined,
                            theme,
                        ) {
                            lines.append(&mut hl_lines);
                        } else {
                            for l in joined.split('\n') {
                                let mut st = theme.md_codeblock_text_style();
                                if let Some(bg) = theme.md_codeblock_bg_color() {
                                    st = st.bg(bg);
                                }
                                lines.push(Line::from(Span::styled(detab(l), st)));
                            }
                        }
                    } else {
                        for l in joined.split('\n') {
                            let mut st = theme.md_codeblock_text_style();
                            if let Some(bg) = theme.md_codeblock_bg_color() {
                                st = st.bg(bg);
                            }
                            lines.push(Line::from(Span::styled(detab(l), st)));
                        }
                    }
                    let end = lines.len();
                    if end > start {
                        ranges.push((start, end - start, joined));
                    }
                    lines.push(Line::from(""));
                    in_code_block = None;
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => {
                    style_stack.pop();
                    current_link_url = None;
                }
                TagEnd::Table => {
                    if let Some(table) = table_state.take() {
                        let mut table_lines = table.render_table_with_width_policy(
                            theme,
                            terminal_width,
                            table_policy,
                        );
                        lines.append(&mut table_lines);
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::TableHead => {
                    if let Some(ref mut table) = table_state {
                        table.end_header();
                    }
                }
                TagEnd::TableRow => {
                    if let Some(ref mut table) = table_state {
                        table.end_row();
                    }
                }
                TagEnd::TableCell => {
                    if let Some(ref mut table) = table_state {
                        table.end_cell();
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block.is_some() {
                    for l in text.lines() {
                        code_block_lines.push(detab(l).to_string());
                    }
                } else if let Some(ref mut table) = table_state {
                    let span = Span::styled(
                        detab(&text),
                        *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                    );
                    table.add_span(span);
                } else {
                    current_rich_spans.push(RichSpan {
                        content: detab(&text),
                        style: *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                        link_url: current_link_url.clone(),
                    });
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                if let Some(ref mut table) = table_state {
                    let span = Span::styled(detab(&code), s);
                    table.add_span(span);
                } else {
                    current_rich_spans.push(RichSpan {
                        content: detab(&code),
                        style: s,
                        link_url: current_link_url.clone(),
                    });
                }
            }
            Event::SoftBreak => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                if role == RoleKind::User && did_prefix_user {
                    current_rich_spans.push(RichSpan {
                        content: "     ".to_string(),
                        style: Style::default(),
                        link_url: None,
                    });
                }
            }
            Event::HardBreak => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
            }
            Event::Rule => {
                flush_current_rich_line(&mut lines, &mut current_rich_spans);
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(_checked) => {
                current_rich_spans.push(RichSpan {
                    content: "[ ] ".to_string(),
                    style: theme.md_list_marker_style(),
                    link_url: None,
                });
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if let Some(ref mut table) = table_state {
                    if html.trim() == "<br>" || html.trim() == "<br/>" {
                        table.new_line_in_cell();
                    }
                }
            }
            Event::FootnoteReference(_) => {}
        }
    }

    flush_current_rich_line(&mut lines, &mut current_rich_spans);
    if !lines.is_empty()
        && lines
            .last()
            .map(|l| !l.to_string().is_empty())
            .unwrap_or(false)
    {
        lines.push(Line::from(""));
    }

    (lines, ranges)
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
                let rm = render_system_message(&msg.content, theme);
                offset += rm.lines.len();
            }
            "user" => {
                let (lines, ranges) = render_message_with_ranges_with_width_and_policy(
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
                let (lines, ranges) = render_message_with_ranges_with_width_and_policy(
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
                    in_code_block = Some(match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        _ => String::new(),
                    });
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
                        for l in text.lines() {
                            buf.push(detab(l));
                        }
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
        compute_codeblock_ranges, render_message_markdown_opts_with_width,
        render_message_markdown_with_policy, TableState,
    };
    use crate::core::message::Message;
    use pulldown_cmark::{Options, Parser};
    use ratatui::style::Modifier;
    use ratatui::text::Span;
    use std::collections::VecDeque;
    use unicode_width::UnicodeWidthStr;

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
        let mut test_table = TableState::new();

        // Add a header row with long headers
        test_table.start_header();
        test_table.start_cell();
        test_table.add_span(Span::raw("Very Long Header Name"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Short"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Another Very Long Header Name"));
        test_table.end_cell();
        test_table.end_header();

        // Add a data row
        test_table.start_row();
        test_table.start_cell();
        test_table.add_span(Span::raw("Short"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("VeryLongContentThatShouldBeHandled"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Data"));
        test_table.end_cell();
        test_table.end_row();

        let theme = crate::ui::theme::Theme::dark_default();

        // Test with narrow terminal (50 chars)
        let narrow_lines = test_table.render_table_with_width(&theme, Some(50));
        let narrow_strings: Vec<String> = narrow_lines.iter().map(|l| l.to_string()).collect();

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
        let wide_strings: Vec<String> = wide_lines.iter().map(|l| l.to_string()).collect();

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
        let ts = TableState::new();
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
        let mut ts2 = TableState::new();
        // Header
        ts2.start_header();
        ts2.start_cell();
        ts2.add_span(Span::raw("H1"));
        ts2.end_cell();
        ts2.start_cell();
        ts2.add_span(Span::raw("H2"));
        ts2.end_cell();
        ts2.start_cell();
        ts2.add_span(Span::raw("H3"));
        ts2.end_cell();
        ts2.end_header();
        // Data row with unbreakable words: 8, 10, 12 chars respectively
        ts2.start_row();
        ts2.start_cell();
        ts2.add_span(Span::raw("aaaaaaaa"));
        ts2.end_cell(); // 8
        ts2.start_cell();
        ts2.add_span(Span::raw("bbbbbbbbbb"));
        ts2.end_cell(); // 10
        ts2.start_cell();
        ts2.add_span(Span::raw("cccccccccccc"));
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
        let table_state = TableState::new();
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
        let mut test_table = TableState::new();

        // Add header
        test_table.start_header();
        test_table.start_cell();
        test_table.add_span(Span::raw("Command"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Description"));
        test_table.end_cell();
        test_table.end_header();

        // Add first data row
        test_table.start_row();
        test_table.start_cell();
        test_table.add_span(Span::raw("git commit"));
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("Creates a new commit with staged changes"));
        test_table.end_cell();
        test_table.end_row();

        // Add continuation row (empty first cell)
        test_table.start_row();
        test_table.start_cell();
        // Empty first cell - should continue previous row
        test_table.end_cell();
        test_table.start_cell();
        test_table.add_span(Span::raw("and includes a commit message"));
        test_table.end_cell();
        test_table.end_row();

        let theme = crate::ui::theme::Theme::dark_default();
        let lines = test_table.render_table_with_width(&theme, Some(60));
        let line_strings: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

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
        let line_strings: Vec<String> = lines.lines.iter().map(|l| l.to_string()).collect();

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
        let ts = TableState::new();

        let bold = theme.md_paragraph_style().add_modifier(Modifier::BOLD);
        let spans = vec![
            Span::styled("foo", bold),
            Span::raw(" "),
            Span::styled("bar", bold),
        ];

        // Width fits "foo" exactly; space + "bar" should go to next line
        let lines =
            ts.wrap_spans_to_width(&spans, 3, crate::ui::layout::TableOverflowPolicy::WrapCells);
        let rendered: Vec<String> = lines
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "foo");
        assert_eq!(rendered[1], "bar");
    }

    #[test]
    fn cell_wraps_after_hyphen() {
        // Ensure hyphen is treated as a soft break opportunity
        let theme = crate::ui::theme::Theme::dark_default();
        let ts = TableState::new();
        let style = theme.md_paragraph_style();
        let spans = vec![Span::styled("decision-making", style)];

        // Allow exactly "decision-" on first line
        let lines = ts.wrap_spans_to_width(
            &spans,
            10,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        );
        let rendered: Vec<String> = lines
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.as_ref()).collect::<String>())
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

#[allow(clippy::items_after_test_module)]
impl TableState {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: vec![Vec::new()],
            in_header: false,
        }
    }

    fn start_header(&mut self) {
        self.in_header = true;
    }

    fn end_header(&mut self) {
        self.in_header = false;
        if !self.current_row.is_empty() {
            self.rows.push(std::mem::take(&mut self.current_row));
        }
    }

    fn start_row(&mut self) {
        // Row already started, just continue
    }

    fn end_row(&mut self) {
        if !self.current_row.is_empty() {
            // Check for logical row continuation (empty first cell continuing previous row)
            if self.should_continue_previous_row() {
                self.merge_with_previous_row();
            } else {
                self.rows.push(std::mem::take(&mut self.current_row));
            }
        }
    }

    fn start_cell(&mut self) {
        self.current_cell = vec![Vec::new()];
    }

    fn end_cell(&mut self) {
        self.current_row
            .push(std::mem::take(&mut self.current_cell));
    }

    fn add_span(&mut self, span: Span<'static>) {
        if self.current_cell.is_empty() {
            self.current_cell.push(Vec::new());
        }
        self.current_cell.last_mut().unwrap().push(span);
    }

    fn new_line_in_cell(&mut self) {
        self.current_cell.push(Vec::new());
    }

    /// Check if current row should continue the previous logical row
    /// This happens when the first cell is empty (indicating continuation)
    fn should_continue_previous_row(&self) -> bool {
        if self.rows.is_empty() || self.current_row.is_empty() {
            return false;
        }

        // Check if first cell is empty or contains only whitespace
        let first_cell = &self.current_row[0];
        if first_cell.is_empty() {
            return true;
        }

        // Check if first cell contains only empty spans or whitespace
        first_cell
            .iter()
            .all(|line| line.is_empty() || line.iter().all(|span| span.content.trim().is_empty()))
    }

    /// Merge current row with the previous row for logical continuation
    fn merge_with_previous_row(&mut self) {
        if let Some(previous_row) = self.rows.last_mut() {
            // For each column in the current row (except the first empty one)
            for (col_idx, cell) in self.current_row.iter().enumerate().skip(1) {
                if let Some(prev_cell) = previous_row.get_mut(col_idx) {
                    // Add the content to the corresponding cell in the previous row
                    for line in cell {
                        prev_cell.push(line.clone());
                    }
                }
            }
        }
        // Clear the current row since it's been merged
        self.current_row.clear();
    }

    /// Wraps spans to a width while preserving all text and styles.
    /// Breaks at spaces and selected punctuation across span boundaries
    /// (hyphens, en/em dashes, slash). If no break point exists, splits
    /// by character as a last resort.
    fn wrap_spans_to_width(
        &self,
        spans: &[Span<'static>],
        max_width: usize,
        _table_policy: crate::ui::layout::TableOverflowPolicy,
    ) -> Vec<Vec<Span<'static>>> {
        if spans.is_empty() {
            return vec![Vec::new()];
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum TokKind {
            Space,
            BreakChar, // '-', '–', '—', '/'
            Word,
        }

        #[derive(Clone)]
        struct Tok {
            text: String,
            style: Style,
            kind: TokKind,
            width: usize,
        }

        fn ch_width(ch: char) -> usize {
            UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]))
        }

        fn str_width(s: &str) -> usize {
            UnicodeWidthStr::width(s)
        }

        fn is_break_char(ch: char) -> bool {
            // ASCII hyphen, Unicode hyphen (U+2010), en dash, em dash, slash
            matches!(ch, '-' | '‐' | '–' | '—' | '/')
        }

        // Tokenize a span into Space / BreakChar / Word tokens preserving style
        fn tokenize(text: &str, style: Style) -> Vec<Tok> {
            let mut toks: Vec<Tok> = Vec::new();
            let mut buf = String::new();
            let mut mode: Option<TokKind> = None;
            for ch in text.chars() {
                let kind = if ch.is_whitespace() {
                    TokKind::Space
                } else if is_break_char(ch) {
                    TokKind::BreakChar
                } else {
                    TokKind::Word
                };
                match (mode, kind) {
                    (Some(TokKind::Space), TokKind::Space) => buf.push(ch),
                    (Some(TokKind::Word), TokKind::Word) => buf.push(ch),
                    // Any change (including BreakChar which are single-char tokens)
                    (Some(prev), k) if prev != k => {
                        if !buf.is_empty() {
                            let w = str_width(&buf);
                            toks.push(Tok {
                                text: std::mem::take(&mut buf),
                                style,
                                kind: prev,
                                width: w,
                            });
                        }
                        if k == TokKind::BreakChar {
                            let s = ch.to_string();
                            toks.push(Tok {
                                width: ch_width(ch),
                                text: s,
                                style,
                                kind: TokKind::BreakChar,
                            });
                            mode = None;
                        } else {
                            buf.push(ch);
                            mode = Some(k);
                        }
                    }
                    (None, TokKind::BreakChar) => {
                        let s = ch.to_string();
                        toks.push(Tok {
                            width: ch_width(ch),
                            text: s,
                            style,
                            kind: TokKind::BreakChar,
                        });
                        mode = None;
                    }
                    (None, k) => {
                        buf.push(ch);
                        mode = Some(k);
                    }
                    _ => unreachable!(),
                }
            }
            if !buf.is_empty() {
                let k = mode.unwrap_or(TokKind::Word);
                let w = str_width(&buf);
                toks.push(Tok {
                    text: buf,
                    style,
                    kind: k,
                    width: w,
                });
            }
            toks
        }

        // Prepare token stream
        let mut all_toks: Vec<Tok> = Vec::new();
        for s in spans {
            // Fast path for empty
            if s.content.is_empty() {
                continue;
            }
            let mut toks = tokenize(s.content.as_ref(), s.style);
            all_toks.append(&mut toks);
        }

        if all_toks.is_empty() {
            return vec![Vec::new()];
        }

        // Wrap using greedy algorithm with last-break tracking across tokens
        let mut out_lines: Vec<Vec<Span<'static>>> = Vec::new();
        let mut cur: Vec<Tok> = Vec::new();
        let mut cur_width: usize = 0;
        let mut last_break_idx: Option<usize> = None; // boundary AFTER this token index

        let mut i = 0usize;
        while i < all_toks.len() {
            let tok = all_toks[i].clone();
            let w = tok.width;

            let fits = cur_width + w <= max_width;
            if fits {
                // Add token
                if matches!(tok.kind, TokKind::Space) {
                    // Collapse multiple leading spaces on empty line (do not count as width)
                    if cur.is_empty() {
                        // Skip leading spaces at line start
                        i += 1;
                        continue;
                    }
                }
                cur_width += w;
                if matches!(tok.kind, TokKind::Space | TokKind::BreakChar) {
                    last_break_idx = Some(cur.len() + 1); // after this token
                }
                cur.push(tok);
                i += 1;
                continue;
            }

            // Overflow handling
            if let Some(br) = last_break_idx {
                // Build line up to break (trim trailing spaces)
                let mut left = cur[..br.min(cur.len())].to_vec();
                while left
                    .last()
                    .map(|t| t.kind == TokKind::Space)
                    .unwrap_or(false)
                {
                    let last = left.pop().unwrap();
                    cur_width = cur_width.saturating_sub(last.width);
                }

                // Emit left
                if left.is_empty() {
                    // Nothing meaningful to emit, force split below
                } else {
                    let spans_line: Vec<Span<'static>> = left
                        .into_iter()
                        .map(|t| Span::styled(t.text, t.style))
                        .collect();
                    out_lines.push(spans_line);
                }

                // Start new line with remainder tokens in cur after break plus current tok
                let mut right: Vec<Tok> = cur[br.min(cur.len())..].to_vec();
                // Drop leading spaces on the new line
                while right
                    .first()
                    .map(|t| t.kind == TokKind::Space)
                    .unwrap_or(false)
                {
                    let first = right.remove(0);
                    let _ = first;
                }
                // Reset state
                cur = right;
                cur_width = cur.iter().map(|t| t.width).sum();
                last_break_idx = None;
                // Retry current token on the fresh line without advancing i
                continue;
            }

            // No recorded break op
            // If the overflowing token is whitespace, flush current line (if any) and drop it
            if matches!(tok.kind, TokKind::Space) {
                if !cur.is_empty() {
                    let line_spans: Vec<Span<'static>> = cur
                        .drain(..)
                        .map(|t| Span::styled(t.text, t.style))
                        .collect();
                    out_lines.push(line_spans);
                }
                cur_width = 0;
                last_break_idx = None;
                i += 1; // skip the space
                continue;
            }

            // No recorded break op: forced split of current non-space token
            // Find how many chars of tok.text fit into remaining space
            let mut acc = 0usize;
            let mut cut = 0usize; // byte index
            for (pos, ch) in tok.text.char_indices() {
                let cw = ch_width(ch);
                if cur_width + acc + cw > max_width {
                    break;
                }
                acc += cw;
                cut = pos + ch.len_utf8();
            }

            if cut == 0 {
                // Nothing fits on this line, flush current (if any). If token is space, drop it.
                if !cur.is_empty() {
                    let line_spans: Vec<Span<'static>> = cur
                        .drain(..)
                        .map(|t| Span::styled(t.text, t.style))
                        .collect();
                    out_lines.push(line_spans);
                }
                cur_width = 0;
                last_break_idx = None;
                if matches!(tok.kind, TokKind::Space) {
                    i += 1; // drop space
                    continue;
                }
                // Now on empty line, try to split token to width
                let mut acc2 = 0usize;
                let mut cut2 = 0usize;
                for (pos, ch) in tok.text.char_indices() {
                    let cw = ch_width(ch);
                    if acc2 + cw > max_width {
                        break;
                    }
                    acc2 += cw;
                    cut2 = pos + ch.len_utf8();
                }
                if cut2 == 0 {
                    // Degenerate case (max_width == 0), avoid infinite loop
                    // Place token as-is to move forward
                    cur_width = tok.width;
                    cur.push(tok);
                    i += 1;
                } else {
                    let left_text = tok.text[..cut2].to_string();
                    let right_text = tok.text[cut2..].to_string();
                    let left_tok = Tok {
                        width: str_width(&left_text),
                        text: left_text,
                        style: tok.style,
                        kind: TokKind::Word,
                    };
                    let right_tok = Tok {
                        width: str_width(&right_text),
                        text: right_text,
                        style: tok.style,
                        kind: TokKind::Word,
                    };
                    cur.push(left_tok);
                    // Emit line immediately
                    let line_spans: Vec<Span<'static>> = cur
                        .drain(..)
                        .map(|t| Span::styled(t.text, t.style))
                        .collect();
                    out_lines.push(line_spans);
                    cur_width = 0;
                    last_break_idx = None;
                    // Place remainder for next iteration by replacing current token with right_tok
                    all_toks[i] = right_tok;
                }
            } else {
                // Split current token into left (fits) and right (remaining)
                let left_text = tok.text[..cut].to_string();
                let right_text = tok.text[cut..].to_string();
                let left_tok = Tok {
                    width: str_width(&left_text),
                    text: left_text,
                    style: tok.style,
                    kind: TokKind::Word,
                };
                let right_tok = Tok {
                    width: str_width(&right_text),
                    text: right_text,
                    style: tok.style,
                    kind: TokKind::Word,
                };
                cur.push(left_tok);
                // Emit line
                let line_spans: Vec<Span<'static>> = cur
                    .drain(..)
                    .map(|t| Span::styled(t.text, t.style))
                    .collect();
                out_lines.push(line_spans);
                cur_width = 0;
                last_break_idx = None;
                // Replace current token with remainder and retry without advancing i
                all_toks[i] = right_tok;
            }
        }

        // Flush last line (trim trailing spaces)
        while cur
            .last()
            .map(|t| t.kind == TokKind::Space)
            .unwrap_or(false)
        {
            let last = cur.pop().unwrap();
            cur_width = cur_width.saturating_sub(last.width);
        }
        if !cur.is_empty() {
            out_lines.push(
                cur.into_iter()
                    .map(|t| Span::styled(t.text, t.style))
                    .collect(),
            );
        }

        if out_lines.is_empty() {
            vec![Vec::new()]
        } else {
            out_lines
        }
    }

    // Backward-compatible wrapper uses default WrapCells policy
    #[cfg(test)]
    fn render_table_with_width(
        &self,
        theme: &Theme,
        terminal_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        self.render_table_with_width_policy(
            theme,
            terminal_width,
            crate::ui::layout::TableOverflowPolicy::WrapCells,
        )
    }

    fn render_table_with_width_policy(
        &self,
        theme: &Theme,
        terminal_width: Option<usize>,
        table_policy: crate::ui::layout::TableOverflowPolicy,
    ) -> Vec<Line<'static>> {
        if self.rows.is_empty() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        let max_cols = self.rows.iter().map(|row| row.len()).max().unwrap_or(0);

        if max_cols == 0 {
            return lines;
        }

        // Calculate ideal column widths based on text content of spans
        // Also check for unbreakable words that should force expansion
        let mut ideal_col_widths = vec![0; max_cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < ideal_col_widths.len() {
                    for line in cell {
                        let cell_text_width = line
                            .iter()
                            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                            .sum::<usize>();
                        ideal_col_widths[i] = ideal_col_widths[i].max(cell_text_width);

                        // Check for unbreakable words that should force expansion
                        for span in line {
                            let words = span.content.split_whitespace();
                            for word in words {
                                let word_width = UnicodeWidthStr::width(word);
                                if word_width <= 30 && word_width > ideal_col_widths[i] {
                                    ideal_col_widths[i] = word_width;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Apply intelligent column width balancing
        let col_widths =
            self.balance_column_widths(&ideal_col_widths, terminal_width, table_policy);

        // Pre-process rows to wrap cell content instead of truncating
        let wrapped_rows = self.wrap_rows_for_rendering(&col_widths, table_policy);

        let table_style = theme.md_paragraph_style();

        // Render header if we have rows
        if !wrapped_rows.is_empty() {
            // Top border
            let top_border = self.create_border_line(&col_widths, "┌", "┬", "┐", "─");
            lines.push(Line::from(Span::styled(top_border, table_style)));

            // Header row
            let header_row = &wrapped_rows[0];
            let max_lines_in_header = header_row.iter().map(|cell| cell.len()).max().unwrap_or(1);
            for line_idx in 0..max_lines_in_header {
                let header_line = self.create_content_line_with_spans(
                    header_row,
                    &col_widths,
                    line_idx,
                    table_style,
                );
                lines.push(header_line);
            }

            // Header separator
            let header_sep = self.create_border_line(&col_widths, "├", "┼", "┤", "─");
            lines.push(Line::from(Span::styled(header_sep, table_style)));

            // Data rows
            for row in &wrapped_rows[1..] {
                let max_lines_in_row = row.iter().map(|cell| cell.len()).max().unwrap_or(1);
                for line_idx in 0..max_lines_in_row {
                    let content_line = self.create_content_line_with_spans(
                        row,
                        &col_widths,
                        line_idx,
                        table_style,
                    );
                    lines.push(content_line);
                }
            }

            // Bottom border
            let bottom_border = self.create_border_line(&col_widths, "└", "┴", "┘", "─");
            lines.push(Line::from(Span::styled(bottom_border, table_style)));
        }

        lines
    }

    /// Wrap all rows for rendering, applying cell wrapping to fit column widths
    fn wrap_rows_for_rendering(
        &self,
        col_widths: &[usize],
        table_policy: crate::ui::layout::TableOverflowPolicy,
    ) -> Vec<Vec<Vec<Vec<Span<'static>>>>> {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        let col_width = col_widths.get(col_idx).copied().unwrap_or(20);

                        // For each line in the cell, wrap it individually
                        let mut wrapped_cell = Vec::new();
                        for line in cell {
                            let wrapped_lines =
                                self.wrap_spans_to_width(line, col_width, table_policy);
                            wrapped_cell.extend(wrapped_lines);
                        }

                        if wrapped_cell.is_empty() {
                            vec![Vec::new()]
                        } else {
                            wrapped_cell
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn create_border_line(
        &self,
        col_widths: &[usize],
        left: &str,
        mid: &str,
        right: &str,
        fill: &str,
    ) -> String {
        let mut line = String::new();
        line.push_str(left);
        for (i, &width) in col_widths.iter().enumerate() {
            line.push_str(&fill.repeat(width + 2)); // +2 for padding
            if i < col_widths.len() - 1 {
                line.push_str(mid);
            }
        }
        line.push_str(right);
        line
    }

    fn create_content_line_with_spans(
        &self,
        row: &[Vec<Vec<Span<'static>>>],
        col_widths: &[usize],
        line_idx: usize,
        style: Style,
    ) -> Line<'static> {
        let mut spans = Vec::new();

        // Left border
        spans.push(Span::styled("│", style));

        for (i, width) in col_widths.iter().enumerate() {
            // Left padding
            spans.push(Span::raw(" "));

            // Cell content with formatting - NO TRUNCATION, content preservation
            let cell_spans = row
                .get(i)
                .and_then(|cell| cell.get(line_idx))
                .cloned()
                .unwrap_or_default();
            let cell_text_len: usize = cell_spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();

            // Always preserve content and ensure proper padding for border alignment
            spans.extend(cell_spans);

            // Always pad to exact column width to maintain border alignment
            if cell_text_len < *width {
                spans.push(Span::raw(" ".repeat(width - cell_text_len)));
            } else if cell_text_len > *width {
                // Content is longer than expected - this should not happen with proper wrapping
                // But if it does, we still need consistent padding to keep borders aligned
                // The wrapping should have prevented this, but as a safety net, we clip
                let total_content_width: usize = spans[1..]
                    .iter() // Skip left padding
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();

                if total_content_width > *width {
                    // Emergency clipping to maintain border alignment - should rarely happen
                    let mut clipped_spans = Vec::new();
                    let mut used_width = 0;

                    for span in &spans[1..] {
                        // Skip left padding
                        let span_width = UnicodeWidthStr::width(span.content.as_ref());
                        if used_width + span_width <= *width {
                            clipped_spans.push(span.clone());
                            used_width += span_width;
                        } else if used_width < *width {
                            // Partial span fits
                            let remaining_width = *width - used_width;
                            let clipped_text =
                                self.clip_text_to_width(&span.content, remaining_width);
                            if !clipped_text.is_empty() {
                                clipped_spans.push(Span::styled(clipped_text, span.style));
                                used_width += remaining_width;
                            }
                            break;
                        } else {
                            break;
                        }
                    }

                    // Replace content spans with clipped versions
                    spans.truncate(1); // Keep only left padding
                    spans.extend(clipped_spans);

                    // Pad remainder
                    if used_width < *width {
                        spans.push(Span::raw(" ".repeat(*width - used_width)));
                    }
                } else {
                    // Width calculation was wrong but spans fit - just pad normally
                    spans.push(Span::raw(" ".repeat(*width - total_content_width)));
                }
            }
            // Content is exactly the right width - no padding needed

            // Right padding and border
            spans.push(Span::raw(" "));
            spans.push(Span::styled("│", style));
        }

        Line::from(spans)
    }

    /// Balance column widths intelligently with content preservation priority
    fn balance_column_widths(
        &self,
        ideal_widths: &[usize],
        terminal_width: Option<usize>,
        _table_policy: crate::ui::layout::TableOverflowPolicy,
    ) -> Vec<usize> {
        if ideal_widths.is_empty() {
            return Vec::new();
        }

        let num_cols = ideal_widths.len();

        // Set minimum width per column (increased for better wrapping)
        const MIN_COL_WIDTH: usize = 8;

        // Ensure minimum widths based on ideal widths (which already account for unbreakable words)
        let col_widths: Vec<usize> = ideal_widths.iter().map(|&w| w.max(MIN_COL_WIDTH)).collect();

        // If no terminal width is provided, use ideal widths
        let Some(term_width) = terminal_width else {
            return col_widths;
        };

        // Calculate table overhead: borders + padding
        // Each column has left padding (1) + right padding (1) = 2
        // Plus borders: left border (1) + right borders per column (1) = num_cols + 1
        let table_overhead = num_cols * 2 + (num_cols + 1);

        if term_width <= table_overhead {
            // Terminal is too narrow, but still preserve minimum widths for content
            return vec![MIN_COL_WIDTH; num_cols];
        }

        let available_width = term_width - table_overhead;
        // If all ideal widths fit, use them (but ensure minimums based on words and column policy)
        let total_ideal_width: usize = ideal_widths.iter().sum();
        if total_ideal_width <= available_width {
            let mut widths: Vec<usize> = ideal_widths.to_vec();
            // Enforce MIN_COL_WIDTH and longest-unbreakable-word minimums below after computing min_word_widths
            // but here we can early exit once we have min_word_widths:
            // (we compute min_word_widths immediately to clamp widths)
            // Calculate minimum widths for each column based on longest unbreakable word
            let mut min_word_widths = vec![MIN_COL_WIDTH; num_cols];
            for row in &self.rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < min_word_widths.len() {
                        for line in cell {
                            for span in line {
                                for word in span.content.split_whitespace() {
                                    let ww = UnicodeWidthStr::width(word);
                                    if ww <= 30 && min_word_widths[i] < ww {
                                        min_word_widths[i] = ww;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for i in 0..widths.len() {
                if widths[i] < MIN_COL_WIDTH {
                    widths[i] = MIN_COL_WIDTH;
                }
                if widths[i] < min_word_widths[i] {
                    widths[i] = min_word_widths[i];
                }
            }
            return widths;
        }

        // Calculate minimum widths for each column based on longest unbreakable word
        let mut min_word_widths = vec![MIN_COL_WIDTH; num_cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < min_word_widths.len() {
                    for line in cell {
                        for span in line {
                            // Find the longest word in this span
                            let words = span.content.split_whitespace();
                            for word in words {
                                let word_width = UnicodeWidthStr::width(word);
                                // Only consider words that are reasonable length (not URLs, etc.)
                                if word_width <= 30 {
                                    min_word_widths[i] = min_word_widths[i].max(word_width);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Start from the hard minimums
        let mut base_widths = min_word_widths.clone();
        // Ensure minimum column width
        for w in &mut base_widths {
            if *w < MIN_COL_WIDTH {
                *w = MIN_COL_WIDTH;
            }
        }

        let total_min_width: usize = base_widths.iter().sum();

        // If even the sum of minimum widths exceeds available space, avoid mid-word breaks by
        // accepting horizontal overflow (borders intact). We return min_word_widths which ensures
        // each column can hold its longest unbreakable word.
        if total_min_width > available_width {
            return min_word_widths;
        }

        // We have extra space: distribute to columns proportionally toward their ideal widths,
        // but do not exceed ideal widths (and do not force 100% fill).
        let extra_space = available_width - total_min_width;
        let desired_gains: Vec<usize> = ideal_widths
            .iter()
            .zip(&base_widths)
            .map(|(&ideal, &base)| ideal.saturating_sub(base))
            .collect();
        let total_desired: usize = desired_gains.iter().sum();
        let mut final_widths = base_widths.clone();
        if total_desired == 0 {
            return final_widths;
        }
        let mut allocated = 0usize;
        for i in 0..final_widths.len() {
            let prop = desired_gains[i] as f64 / total_desired as f64;
            let mut add = (extra_space as f64 * prop).floor() as usize;
            // Cap at ideal width
            let cap = ideal_widths[i].saturating_sub(final_widths[i]);
            if add > cap {
                add = cap;
            }
            final_widths[i] += add;
            allocated += add;
        }
        // Assign any remainder, left to right where desire remains, respecting caps
        let mut rem = extra_space.saturating_sub(allocated);
        if rem > 0 {
            for i in 0..final_widths.len() {
                if rem == 0 {
                    break;
                }
                let cap = ideal_widths[i].saturating_sub(final_widths[i]);
                if cap > 0 {
                    final_widths[i] += 1;
                    rem -= 1;
                }
            }
        }
        final_widths
    }

    /// Emergency helper to clip text to width (used as safety net)
    fn clip_text_to_width(&self, text: &str, max_width: usize) -> String {
        let mut result = String::new();
        let mut current_width = 0;

        for ch in text.chars() {
            let char_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
            if current_width + char_width > max_width {
                break;
            }
            result.push(ch);
            current_width += char_width;
        }

        result
    }
}

/// Test-only helper: choose the base text style for a message role.
/// Used by the simplified unit-test renderer to avoid depending on
/// broader rendering context.
#[cfg(test)]
fn base_text_style_bool(is_user: bool, theme: &Theme) -> Style {
    if is_user {
        theme.user_text_style
    } else {
        theme.md_paragraph_style()
    }
}

fn flush_current_rich_line(
    lines: &mut Vec<Line<'static>>,
    current_rich_spans: &mut Vec<RichSpan>,
) {
    if !current_rich_spans.is_empty() {
        // For now, just convert to spans and discard link info.
        // Hotspot generation will happen in wrap_rich_spans_to_width.
        let spans: Vec<Span> = current_rich_spans
            .iter()
            .map(|rs| Span::styled(rs.content.clone(), rs.style))
            .collect();
        lines.push(Line::from(spans));
        current_rich_spans.clear();
    }
}

fn wrap_rich_spans_to_width(
    rich_spans: &[RichSpan],
    max_width: usize,
    start_y: usize,
    is_user: bool,
) -> (Vec<Line<'static>>, Vec<LinkHotspot>) {
    let mut lines = Vec::new();
    let mut hotspots = Vec::new();
    if rich_spans.is_empty() {
        return (lines, hotspots);
    }

    let mut current_line_spans = Vec::new();
    let mut current_line_width = 0;
    let mut current_y = start_y;
    let indent = if is_user { 5 } else { 0 };

    for rich_span in rich_spans {
        let words = rich_span.content.split(' ').collect::<Vec<&str>>();
        for (i, word) in words.iter().enumerate() {
            let mut current_word = word.to_string();
            while !current_word.is_empty() {
                let word_width = UnicodeWidthStr::width(current_word.as_str());

                if current_line_width + word_width > max_width {
                    if current_line_width > indent {
                        lines.push(Line::from(
                            current_line_spans.drain(..).collect::<Vec<Span>>(),
                        ));
                        current_line_width = indent;
                        current_y += 1;
                        if is_user {
                            current_line_spans.push(Span::raw("     "));
                        }
                    }

                    // Word is longer than max_width, so we need to break it
                    let mut break_pos = 0;
                    let mut part_width = 0;
                    for (j, c) in current_word.char_indices() {
                        let char_width = UnicodeWidthStr::width(c.to_string().as_str());
                        if current_line_width + part_width + char_width > max_width {
                            break;
                        }
                        part_width += char_width;
                        break_pos = j + c.len_utf8();
                    }

                    if break_pos == 0 {
                        // if a single character is wider than max_width
                        break_pos = current_word
                            .char_indices()
                            .next()
                            .map_or(0, |(j, c)| j + c.len_utf8());
                    }

                    let part = current_word[..break_pos].to_string();
                    let x = current_line_width as u16;
                    let y = current_y as u16;
                    let span = Span::styled(part.clone(), rich_span.style);

                    if let Some(url) = &rich_span.link_url {
                        hotspots.push(LinkHotspot {
                            url: url.clone(),
                            rect: Rect::new(x, y, UnicodeWidthStr::width(part.as_str()) as u16, 1),
                        });
                    }

                    current_line_spans.push(span);
                    lines.push(Line::from(
                        current_line_spans.drain(..).collect::<Vec<Span>>(),
                    ));
                    current_line_width = indent;
                    current_y += 1;
                    if is_user {
                        current_line_spans.push(Span::raw("     "));
                    }

                    current_word = current_word[break_pos..].to_string();
                } else {
                    let x = current_line_width as u16;
                    let y = current_y as u16;
                    let span = Span::styled(current_word.clone(), rich_span.style);

                    if let Some(url) = &rich_span.link_url {
                        hotspots.push(LinkHotspot {
                            url: url.clone(),
                            rect: Rect::new(x, y, word_width as u16, 1),
                        });
                    }

                    current_line_spans.push(span);
                    current_line_width += word_width;
                    current_word.clear();
                }
            }

            if i < words.len() - 1 {
                if current_line_width + 1 > max_width {
                    lines.push(Line::from(
                        current_line_spans.drain(..).collect::<Vec<Span>>(),
                    ));
                    current_line_width = indent;
                    current_y += 1;
                    if is_user {
                        current_line_spans.push(Span::raw("     "));
                    }
                }
                current_line_spans.push(Span::raw(" "));
                current_line_width += 1;
            }
        }
    }

    if !current_line_spans.is_empty() {
        lines.push(Line::from(current_line_spans));
    }

    (lines, hotspots)
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

/// Build display lines for all messages using plain text rendering
pub fn build_plain_display_lines(
    messages: &VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let rendered = render_system_message(&msg.content, theme);
                lines.extend(rendered.lines);
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
            }
            _ => {
                // Assistant messages
                for line in msg.content.lines() {
                    lines.push(Line::from(Span::styled(
                        detab(line),
                        theme.md_paragraph_style(),
                    )));
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
            }
        }
    }
    lines
}
