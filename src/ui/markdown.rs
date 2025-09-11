use crate::core::message::Message;
use crate::ui::theme::Theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

struct TableState {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    current_row: Vec<Vec<Span<'static>>>,
    current_cell: Vec<Span<'static>>,
    in_header: bool,
}


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
                Tag::Table(_) => {
                    flush_current_line(&mut lines, &mut current_spans);
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
                TagEnd::Table => {
                    if let Some(table) = table_state.take() {
                        let mut table_lines = table.render_table(theme);
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
                    current_spans.push(Span::styled(
                        detab(&text),
                        *style_stack.last().unwrap_or(&base_text_style(role, theme)),
                    ))
                }
            }
            Event::Code(code) => {
                let s = theme.md_inline_code_style();
                let span = Span::styled(detab(&code), s);
                if let Some(ref mut table) = table_state {
                    table.add_span(span);
                } else {
                    current_spans.push(span);
                }
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

    #[test]
    fn debug_table_events() {
        let markdown = r#"| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |"#;

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
            content: r#"Here's a table:

| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |

End of table."#
                .into(),
        });
        let theme = crate::ui::theme::Theme::dark_default();
        let rendered = render_message_markdown(&messages[0], &theme);

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
}


impl TableState {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: Vec::new(),
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
            self.rows.push(std::mem::take(&mut self.current_row));
        }
    }

    fn start_cell(&mut self) {
        self.current_cell.clear();
    }

    fn end_cell(&mut self) {
        self.current_row
            .push(std::mem::take(&mut self.current_cell));
    }

    fn add_span(&mut self, span: Span<'static>) {
        self.current_cell.push(span);
    }

    fn render_table(&self, theme: &Theme) -> Vec<Line<'static>> {
        if self.rows.is_empty() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        let max_cols = self.rows.iter().map(|row| row.len()).max().unwrap_or(0);

        if max_cols == 0 {
            return lines;
        }

        // Calculate column widths based on text content of spans
        let mut col_widths = vec![0; max_cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < col_widths.len() {
                    let cell_text_width = cell.iter().map(|span| span.content.len()).sum::<usize>();
                    col_widths[i] = col_widths[i].max(cell_text_width);
                }
            }
        }

        // Ensure minimum width of 3 for each column
        for width in &mut col_widths {
            *width = (*width).max(3);
        }

        let table_style = theme.md_paragraph_style();

        // Render header if we have rows
        if !self.rows.is_empty() {
            // Top border
            let top_border = self.create_border_line(&col_widths, "┌", "┬", "┐", "─");
            lines.push(Line::from(Span::styled(top_border, table_style)));

            // Header row
            let header_line = self.create_content_line_with_spans(&self.rows[0], &col_widths);
            lines.push(header_line);

            // Header separator
            let header_sep = self.create_border_line(&col_widths, "├", "┼", "┤", "─");
            lines.push(Line::from(Span::styled(header_sep, table_style)));

            // Data rows
            for row in &self.rows[1..] {
                let content_line = self.create_content_line_with_spans(row, &col_widths);
                lines.push(content_line);
            }

            // Bottom border
            let bottom_border = self.create_border_line(&col_widths, "└", "┴", "┘", "─");
            lines.push(Line::from(Span::styled(bottom_border, table_style)));
        }

        lines
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

    fn create_content_line_with_spans(&self, row: &[Vec<Span<'static>>], col_widths: &[usize]) -> Line<'static> {
        let mut spans = Vec::new();

        // Left border
        spans.push(Span::raw("│"));

        for (i, width) in col_widths.iter().enumerate() {
            // Left padding
            spans.push(Span::raw(" "));

            // Cell content with formatting
            let cell_spans = row.get(i).cloned().unwrap_or_default();
            let cell_text_len: usize = cell_spans.iter().map(|s| s.content.len()).sum();

            // Add the formatted spans
            spans.extend(cell_spans);

            // Right padding to fill column width
            if cell_text_len < *width {
                spans.push(Span::raw(" ".repeat(width - cell_text_len)));
            }

            // Right padding and border
            spans.push(Span::raw(" "));
            spans.push(Span::raw("│"));
        }

        Line::from(spans)
    }

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

/// Build display lines for all messages using markdown rendering
#[cfg(test)]
pub fn build_markdown_display_lines(
    messages: &VecDeque<Message>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for msg in messages {
        let rendered = render_message_markdown_opts(msg, theme, true);
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
                    lines.push(Line::from(Span::styled(detab(line), theme.md_paragraph_style())));
                }
                if !msg.content.is_empty() {
                    lines.push(Line::from(""));
                }
            }
        }
    }
    lines
}
