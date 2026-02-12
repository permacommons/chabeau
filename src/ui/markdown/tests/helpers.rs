use crate::core::message::Message;
use crate::ui::markdown::{render_message_with_config, MessageRenderConfig};

pub fn render_markdown_for_test(
    message: &Message,
    theme: &crate::ui::theme::Theme,
    syntax_enabled: bool,
    width: Option<usize>,
) -> crate::ui::markdown::RenderedMessage {
    let cfg = MessageRenderConfig::markdown(true, syntax_enabled)
        .with_terminal_width(width, crate::ui::layout::TableOverflowPolicy::WrapCells);
    render_message_with_config(message, theme, cfg).into_rendered()
}

pub fn line_texts(lines: &[ratatui::text::Line<'static>]) -> Vec<String> {
    lines.iter().map(|line| line.to_string()).collect()
}

pub fn assert_line_text(lines: &[String], index: usize, expected: &str) {
    assert_eq!(lines.get(index).map(String::as_str), Some(expected));
}

pub fn assert_first_span_is_space_indented(
    line: &ratatui::text::Line<'static>,
    expected_width: usize,
) {
    let indent = line
        .spans
        .first()
        .expect("indent span present")
        .content
        .as_ref();
    assert!(indent.chars().all(|ch| ch == ' '));
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(indent),
        expected_width
    );
}
