use crate::core::message::Message;
use crate::ui::theme::Theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;

/// Description of a rendered message (line-based), used by the TUI renderer.
pub struct RenderedMessage {
    pub lines: Vec<Line<'static>>,
}

/// Markdown renderer using pulldown-cmark with theming.
#[cfg(test)]
pub fn render_message_markdown(msg: &Message, theme: &Theme) -> RenderedMessage {
    match msg.role.as_str() {
        "system" => render_with_parser_role(RoleKind::System, &msg.content, theme, true),
        "user" => render_with_parser_role(RoleKind::User, &msg.content, theme, true),
        _ => render_with_parser_role(RoleKind::Assistant, &msg.content, theme, true),
    }
}

/// Render markdown with options to enable/disable syntax highlighting.
pub fn render_message_markdown_opts(
    msg: &Message,
    theme: &Theme,
    syntax_enabled: bool,
) -> RenderedMessage {
    match msg.role.as_str() {
        "system" => render_with_parser_role(RoleKind::System, &msg.content, theme, syntax_enabled),
        "user" => render_with_parser_role(RoleKind::User, &msg.content, theme, syntax_enabled),
        _ => render_with_parser_role(RoleKind::Assistant, &msg.content, theme, syntax_enabled),
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
    RenderedMessage { lines: out }
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

fn render_with_parser_role(
    role: RoleKind,
    content: &str,
    theme: &Theme,
    syntax_enabled: bool,
) -> RenderedMessage {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(content, options);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Inline buffer for current line
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    // Style stack for inline formatting
    let mut style_stack: Vec<Style> = vec![base_text_style(role, theme)];

    // List handling
    let mut list_stack: Vec<ListKind> = Vec::new();
    // Code block handling
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_block_lines: Vec<String> = Vec::new();

    // User prefix handling
    let is_user = role == RoleKind::User;
    let mut did_prefix_user = role != RoleKind::User; // only user gets prefix

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if role == RoleKind::User {
                        if !did_prefix_user {
                            current_spans.push(Span::styled("You: ", theme.user_prefix_style));
                            did_prefix_user = true;
                        } else {
                            current_spans.push(Span::raw("     "));
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    flush_current_line(&mut lines, &mut current_spans);
                    let style = theme.md_heading_style(level as u8);
                    if is_user && !did_prefix_user {
                        current_spans.push(Span::styled("You: ", theme.user_prefix_style));
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
                    flush_current_line(&mut lines, &mut current_spans);
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
                        current_spans.push(Span::styled("You: ", theme.user_prefix_style));
                        did_prefix_user = true;
                    }
                    current_spans.push(Span::styled(marker, theme.md_list_marker_style()));
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
                Tag::Link { .. } => {
                    let new = theme.md_link_style();
                    style_stack.push(new);
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_level) => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::BlockQuote => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::List(_start) => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    list_stack.pop();
                }
                TagEnd::Item => {
                    flush_current_line(&mut lines, &mut current_spans);
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
                    current_spans.push(Span::styled(
                        detab(&text),
                        *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                    ))
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                current_spans.push(Span::styled(detab(&code), s));
            }
            Event::SoftBreak => {
                // Treat soft breaks as new lines
                flush_current_line(&mut lines, &mut current_spans);
                // For user messages, indent continuation lines
                if role == RoleKind::User && did_prefix_user {
                    current_spans.push(Span::raw("     "));
                }
            }
            Event::HardBreak => {
                flush_current_line(&mut lines, &mut current_spans);
            }
            Event::Rule => {
                flush_current_line(&mut lines, &mut current_spans);
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(_checked) => {
                current_spans.push(Span::styled("[ ] ", theme.md_list_marker_style()));
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::FootnoteReference(_) => {
                // Ignore advanced/HTML/Math for TUI rendering
            }
        }
    }

    flush_current_line(&mut lines, &mut current_spans);
    if !lines.is_empty()
        && lines
            .last()
            .map(|l| !l.to_string().is_empty())
            .unwrap_or(false)
    {
        lines.push(Line::from(""));
    }

    RenderedMessage { lines }
}

/// Render message and also compute local code block ranges (start line index, len, content).
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
    let mut current_spans: Vec<Span<'static>> = Vec::new();
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
                            current_spans.push(Span::styled("You: ", theme.user_prefix_style));
                            did_prefix_user = true;
                        } else {
                            current_spans.push(Span::raw("     "));
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    flush_current_line(&mut lines, &mut current_spans);
                    let style = theme.md_heading_style(level as u8);
                    if is_user && !did_prefix_user {
                        current_spans.push(Span::styled("You: ", theme.user_prefix_style));
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
                    flush_current_line(&mut lines, &mut current_spans);
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
                        current_spans.push(Span::styled("You: ", theme.user_prefix_style));
                        did_prefix_user = true;
                    }
                    current_spans.push(Span::styled(marker, theme.md_list_marker_style()));
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
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_level) => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::BlockQuote => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    style_stack.pop();
                }
                TagEnd::List(_start) => {
                    flush_current_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                    list_stack.pop();
                }
                TagEnd::Item => flush_current_line(&mut lines, &mut current_spans),
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
                    current_spans.push(Span::styled(
                        detab(&text),
                        *style_stack
                            .last()
                            .unwrap_or(&base_text_style_bool(is_user, theme)),
                    ))
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                current_spans.push(Span::styled(detab(&code), s));
            }
            Event::SoftBreak => {
                flush_current_line(&mut lines, &mut current_spans);
                if is_user && did_prefix_user {
                    current_spans.push(Span::raw("     "));
                }
            }
            Event::HardBreak => flush_current_line(&mut lines, &mut current_spans),
            Event::Rule => {
                flush_current_line(&mut lines, &mut current_spans);
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(_checked) => {
                current_spans.push(Span::styled("[ ] ", theme.md_list_marker_style()));
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::FootnoteReference(_) => {}
        }
    }

    flush_current_line(&mut lines, &mut current_spans);
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
    use super::*;
    use crate::core::message::Message;
    use std::collections::VecDeque;

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
}

/// Build display lines for all messages using the lightweight markdown renderer.
#[cfg(test)]
pub fn build_markdown_display_lines(
    messages: &std::collections::VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        let rendered = render_message_markdown(msg, theme);
        lines.extend(rendered.lines);
    }
    lines
}

/// Plain renderer without markdown parsing; keeps prefixes and spacing.
pub fn build_plain_display_lines(
    messages: &std::collections::VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let rm = render_system_message(&msg.content, theme);
                lines.extend(rm.lines);
            }
            "user" => {
                let mut first = true;
                for l in msg.content.lines() {
                    if l.trim().is_empty() {
                        lines.push(Line::from(""));
                    } else if first {
                        lines.push(Line::from(Span::styled(
                            format!("You: {}", detab(l)),
                            theme.user_text_style,
                        )));
                        first = false;
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("     {}", detab(l)),
                            theme.user_text_style,
                        )));
                    }
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
            }
            _ => {
                for l in msg.content.lines() {
                    if l.trim().is_empty() {
                        lines.push(Line::from(""));
                    } else {
                        lines.push(Line::from(Span::styled(
                            detab(l),
                            theme.md_paragraph_style(),
                        )));
                    }
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod plain_tests {
    use super::*;
    use crate::core::message::Message;

    #[test]
    fn plain_user_message_prefix_and_indent() {
        let theme = crate::ui::theme::Theme::dark_default();
        let messages = std::collections::VecDeque::from(vec![Message {
            role: "user".into(),
            content: "Line1\nLine2".into(),
        }]);
        let lines = build_plain_display_lines(&messages, &theme);
        assert!(lines[0].to_string().starts_with("You: Line1"));
        assert!(lines[1].to_string().starts_with("     Line2"));
        assert_eq!(lines[2].to_string(), "");
    }

    #[test]
    fn plain_system_message_spacing() {
        let theme = crate::ui::theme::Theme::dark_default();
        let messages = std::collections::VecDeque::from(vec![Message {
            role: "system".into(),
            content: "Notice".into(),
        }]);
        let lines = build_plain_display_lines(&messages, &theme);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].to_string(), "");
    }
}

// Theme style selection moved into Theme methods (see src/ui/theme.rs)

#[derive(Clone, Copy, Debug)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

fn base_text_style_bool(is_user: bool, theme: &Theme) -> Style {
    if is_user {
        theme.user_text_style
    } else {
        theme.md_paragraph_style()
    }
}

fn flush_current_line(lines: &mut Vec<Line<'static>>, current_spans: &mut Vec<Span<'static>>) {
    if !current_spans.is_empty() {
        lines.push(Line::from(std::mem::take(current_spans)));
    }
}

fn detab(s: &str) -> String {
    // Simple, predictable detab: replace tabs with 4 spaces
    s.replace('\t', "    ")
}
