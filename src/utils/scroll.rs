use crate::core::message::Message;
use crate::ui::markdown::{
    build_markdown_display_lines, build_plain_display_lines, compute_codeblock_ranges,
    render_message_markdown_opts,
};
use crate::ui::theme::Theme;
use ratatui::{text::Line, text::Span};
use std::collections::VecDeque;

/// Handles all scroll-related calculations and line building
pub struct ScrollCalculator;

impl ScrollCalculator {
    /// Build display lines for all messages
    pub fn build_display_lines(messages: &VecDeque<Message>) -> Vec<Line<'static>> {
        // Backwards-compatible default theme
        let theme = Theme::dark_default();
        Self::build_display_lines_with_theme(messages, &theme)
    }

    /// Build display lines using a provided theme
    pub fn build_display_lines_with_theme(
        messages: &VecDeque<Message>,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        build_markdown_display_lines(messages, theme)
    }

    /// Build display lines using theme and flags
    pub fn build_display_lines_with_theme_and_flags(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
    ) -> Vec<Line<'static>> {
        if markdown_enabled {
            // render each with syntax flag
            let mut out = Vec::new();
            for msg in messages {
                let rendered = render_message_markdown_opts(msg, theme, syntax_enabled);
                out.extend(rendered.lines);
            }
            out
        } else {
            build_plain_display_lines(messages, theme)
        }
    }

    /// Build display lines using a provided theme and optionally highlight a selected message
    /// Build display lines and optionally highlight a selected user message (flags aware)
    pub fn build_display_lines_with_theme_and_selection_and_flags(
        messages: &VecDeque<Message>,
        theme: &Theme,
        selected_index: Option<usize>,
        highlight: ratatui::style::Style,
        markdown_enabled: bool,
        syntax_enabled: bool,
    ) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            let mut rendered = if markdown_enabled {
                render_message_markdown_opts(msg, theme, syntax_enabled)
            } else {
                crate::ui::markdown::RenderedMessage {
                    lines: build_plain_display_lines(&VecDeque::from([msg.clone()]), theme),
                }
            };
            if selected_index == Some(i) && msg.role == "user" {
                for l in &mut rendered.lines {
                    if !l.to_string().is_empty() {
                        let text = l.to_string();
                        *l = Line::from(Span::styled(text, theme.user_text_style.patch(highlight)));
                    }
                }
            }
            lines.extend(rendered.lines);
        }
        lines
    }

    /// Build display lines and highlight a selected code block range
    /// Codeblock highlight respecting flags (no-op when markdown disabled)
    pub fn build_display_lines_with_codeblock_highlight_and_flags(
        messages: &VecDeque<Message>,
        theme: &crate::ui::theme::Theme,
        selected_block: Option<usize>,
        highlight: ratatui::style::Style,
        markdown_enabled: bool,
        syntax_enabled: bool,
    ) -> Vec<Line<'static>> {
        let mut lines = if markdown_enabled {
            let mut out = Vec::new();
            for msg in messages {
                let rendered = render_message_markdown_opts(msg, theme, syntax_enabled);
                out.extend(rendered.lines);
            }
            out
        } else {
            build_plain_display_lines(messages, theme)
        };
        if markdown_enabled {
            if let Some(idx) = selected_block {
                let ranges = compute_codeblock_ranges(messages, theme);
                if let Some((start, len, _content)) = ranges.get(idx).cloned() {
                    for i in start..start + len {
                        if i < lines.len() {
                            let text = lines[i].to_string();
                            let st = theme.md_codeblock_text_style().patch(highlight);
                            lines[i] = Line::from(Span::styled(text, st));
                        }
                    }
                }
            }
        }
        lines
    }

    /// Compute a scroll offset that positions the start of a given logical line index
    /// within view, taking wrapping and available height into account. The caller should
    /// clamp the result to the maximum scroll.
    pub fn scroll_offset_to_line_start(
        lines: &[Line],
        terminal_width: u16,
        available_height: u16,
        line_index: usize,
    ) -> u16 {
        let prefix = &lines[..line_index.min(lines.len())];
        let wrapped_to_start = Self::calculate_wrapped_line_count(prefix, terminal_width);
        if wrapped_to_start > available_height.saturating_sub(1) {
            wrapped_to_start.saturating_sub(1)
        } else {
            wrapped_to_start
        }
    }

    /// Build display lines up to a specific message index (inclusive)
    pub fn build_display_lines_up_to(
        messages: &VecDeque<Message>,
        max_index: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for (i, msg) in messages.iter().enumerate() {
            if i > max_index {
                break;
            }
            // Use default theme for backward compatibility
            let theme = Theme::dark_default();
            let rendered = render_message_markdown_opts(msg, &theme, true);
            lines.extend(rendered.lines);
        }

        lines
    }

    /// Calculate how many wrapped lines the given lines will take
    pub fn calculate_wrapped_line_count(lines: &[Line], terminal_width: u16) -> u16 {
        let mut total_wrapped_lines = 0u16;

        for line in lines {
            let line_text = line.to_string();
            if terminal_width == 0 {
                total_wrapped_lines = total_wrapped_lines.saturating_add(1);
                continue;
            }
            if line_text.is_empty() {
                total_wrapped_lines = total_wrapped_lines.saturating_add(1);
                continue;
            }
            // Preserve leading whitespace when computing wrap height; treat whitespace-only lines as 1
            if line_text.chars().all(|c| c.is_whitespace()) {
                total_wrapped_lines = total_wrapped_lines.saturating_add(1);
                continue;
            }
            let wrapped_count =
                Self::calculate_word_wrapped_lines_with_leading(&line_text, terminal_width);
            total_wrapped_lines = total_wrapped_lines.saturating_add(wrapped_count);
        }

        total_wrapped_lines
    }

    /// Calculate how many lines a single text string will wrap to
    fn calculate_word_wrapped_lines_with_leading(text: &str, terminal_width: u16) -> u16 {
        let width = terminal_width as usize;
        let mut current_line_len: usize;
        let mut line_count = 1u16;

        // Count leading spaces explicitly (tabs should be detabbed earlier in markdown)
        let mut chars = text.chars().peekable();
        let mut leading_spaces = 0usize;
        while let Some(&ch) = chars.peek() {
            if ch == ' ' {
                leading_spaces += 1;
                chars.next();
            } else {
                break;
            }
        }
        if leading_spaces >= width && width > 0 {
            // Advance lines for fully consumed widths
            line_count = line_count.saturating_add((leading_spaces / width) as u16);
            current_line_len = leading_spaces % width;
        } else {
            current_line_len = leading_spaces;
        }

        // Process the remainder as words separated by whitespace
        let remainder: String = chars.collect();
        for word in remainder.split_whitespace() {
            let word_len = word.chars().count();
            if current_line_len > 0 {
                // account for one space between words
                if current_line_len + 1 + word_len > width {
                    line_count = line_count.saturating_add(1);
                    current_line_len = word_len;
                } else {
                    current_line_len += 1 + word_len;
                }
            } else {
                // start of line
                current_line_len = word_len;
            }
        }

        if line_count == 0 {
            1
        } else {
            line_count
        }
    }

    // Wrapper only for tests that reference the original name
    #[cfg(test)]
    fn calculate_word_wrapped_lines(text: &str, terminal_width: u16) -> u16 {
        Self::calculate_word_wrapped_lines_with_leading(text, terminal_width)
    }

    /// Calculate scroll offset to show the bottom of all messages
    pub fn calculate_scroll_to_bottom(
        messages: &VecDeque<Message>,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        let lines = Self::build_display_lines(messages);
        let total_wrapped_lines = Self::calculate_wrapped_line_count(&lines, terminal_width);

        if total_wrapped_lines > available_height {
            total_wrapped_lines.saturating_sub(available_height)
        } else {
            0
        }
    }

    /// Calculate scroll offset to show a specific message
    pub fn calculate_scroll_to_message(
        messages: &VecDeque<Message>,
        message_index: usize,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        let lines = Self::build_display_lines_up_to(messages, message_index);
        let wrapped_lines = Self::calculate_wrapped_line_count(&lines, terminal_width);

        if wrapped_lines > available_height {
            wrapped_lines.saturating_sub(available_height)
        } else {
            0
        }
    }

    /// Calculate maximum scroll offset
    pub fn calculate_max_scroll_offset(
        messages: &VecDeque<Message>,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        Self::calculate_scroll_to_bottom(messages, terminal_width, available_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;
    use crate::utils::test_utils::{create_test_message, create_test_messages};
    use ratatui::text::Line as TLine;
    use std::collections::VecDeque;

    #[test]
    fn test_build_display_lines_basic() {
        let messages = create_test_messages();
        let lines = ScrollCalculator::build_display_lines(&messages);

        // Should have lines for each message plus spacing
        // Each message gets 2 lines (content + empty spacing)
        assert_eq!(lines.len(), 8); // 4 messages * 2 lines each

        // Check that user messages start with "You: "
        assert!(lines[0].to_string().starts_with("You: "));
        assert!(lines[4].to_string().starts_with("You: "));

        // Check that assistant messages don't have prefix
        assert!(!lines[2].to_string().starts_with("You: "));
        assert!(!lines[6].to_string().starts_with("You: "));
    }

    #[test]
    fn test_build_display_lines_up_to() {
        let messages = create_test_messages();
        let lines = ScrollCalculator::build_display_lines_up_to(&messages, 1);

        // Should only include first 2 messages (indices 0 and 1)
        assert_eq!(lines.len(), 4); // 2 messages * 2 lines each

        assert!(lines[0].to_string().starts_with("You: Hello"));
        assert!(lines[2].to_string().contains("Hi there!"));
    }

    #[test]
    fn test_calculate_word_wrapped_lines_single_line() {
        // Text that fits in one line
        let wrapped = ScrollCalculator::calculate_word_wrapped_lines("Hello world", 20);
        assert_eq!(wrapped, 1);
    }

    #[test]
    fn test_calculate_word_wrapped_lines_multiple_lines() {
        // Text that needs to wrap
        let text = "This is a very long sentence that will definitely need to wrap";
        let wrapped = ScrollCalculator::calculate_word_wrapped_lines(text, 20);
        assert!(wrapped > 1);
    }

    #[test]
    fn test_calculate_word_wrapped_lines_exact_fit() {
        // Text that exactly fits the width
        let wrapped = ScrollCalculator::calculate_word_wrapped_lines("Hello world test", 16);
        assert_eq!(wrapped, 1);
    }

    #[test]
    fn test_calculate_word_wrapped_lines_single_word_too_long() {
        // Single word longer than width should still count as 1 line
        let wrapped = ScrollCalculator::calculate_word_wrapped_lines(
            "supercalifragilisticexpialidocious",
            10,
        );
        assert_eq!(wrapped, 1);
    }

    #[test]
    fn test_calculate_wrapped_line_count_empty_lines() {
        let lines = vec![Line::from(""), Line::from(""), Line::from("")];
        let count = ScrollCalculator::calculate_wrapped_line_count(&lines, 80);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_calculate_wrapped_line_count_mixed_content() {
        let lines = vec![
            Line::from("Short line"),
            Line::from(""),
            Line::from("This is a much longer line that might wrap depending on terminal width"),
            Line::from("Another short one"),
        ];

        // With wide terminal, should not wrap
        let count_wide = ScrollCalculator::calculate_wrapped_line_count(&lines, 100);
        assert_eq!(count_wide, 4);

        // With narrow terminal, long line should wrap
        let count_narrow = ScrollCalculator::calculate_wrapped_line_count(&lines, 20);
        assert!(count_narrow > 4);
    }

    #[test]
    fn test_calculate_wrapped_line_count_zero_width() {
        let lines = vec![Line::from("Any content")];
        let count = ScrollCalculator::calculate_wrapped_line_count(&lines, 0);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_calculate_scroll_to_bottom_no_scroll_needed() {
        let messages = create_test_messages();
        let scroll = ScrollCalculator::calculate_scroll_to_bottom(&messages, 80, 20);
        // With wide terminal and high available height, no scroll needed
        assert_eq!(scroll, 0);
    }

    #[test]
    fn test_calculate_scroll_to_bottom_scroll_needed() {
        let mut messages = VecDeque::new();
        // Create many messages to force scrolling
        for i in 0..10 {
            messages.push_back(create_test_message("user", &format!("Message {i}")));
            messages.push_back(create_test_message("assistant", &format!("Response {i}")));
        }

        let scroll = ScrollCalculator::calculate_scroll_to_bottom(&messages, 80, 5);
        // With low available height, should need to scroll
        assert!(scroll > 0);
    }

    #[test]
    fn test_calculate_scroll_to_message() {
        let messages = create_test_messages();

        // Scroll to first message should be 0
        let scroll_first = ScrollCalculator::calculate_scroll_to_message(&messages, 0, 80, 10);
        assert_eq!(scroll_first, 0);

        // Scroll to later message might require scrolling
        let scroll_later = ScrollCalculator::calculate_scroll_to_message(&messages, 3, 80, 2);
        assert!(scroll_later > 0);
    }

    #[test]
    fn test_calculate_max_scroll_offset() {
        let messages = create_test_messages();
        let max_scroll = ScrollCalculator::calculate_max_scroll_offset(&messages, 80, 5);
        let scroll_to_bottom = ScrollCalculator::calculate_scroll_to_bottom(&messages, 80, 5);

        // Max scroll should equal scroll to bottom
        assert_eq!(max_scroll, scroll_to_bottom);
    }

    #[test]
    fn test_system_message_formatting() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("system", "System message"));

        let lines = ScrollCalculator::build_display_lines(&messages);
        assert_eq!(lines.len(), 2); // System message + spacing

        // System messages should not have "You: " prefix
        assert!(!lines[0].to_string().starts_with("You: "));
        assert!(lines[0].to_string().contains("System message"));
    }

    #[test]
    fn test_empty_message_content() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("assistant", ""));

        let lines = ScrollCalculator::build_display_lines(&messages);
        // Empty assistant message should not add any lines
        assert_eq!(lines.len(), 0);
    }

    #[test]
    fn test_multiline_assistant_message() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("assistant", "Line 1\nLine 2\n\nLine 4"));

        let lines = ScrollCalculator::build_display_lines(&messages);
        // Should have: Line 1, Line 2, empty line, Line 4, spacing = 5 lines
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn test_multiline_user_message() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("user", "Line 1\nLine 2\n\nLine 4"));

        let lines = ScrollCalculator::build_display_lines(&messages);
        // Should have: "You: Line 1", "     Line 2", empty line, "     Line 4", spacing = 5 lines
        assert_eq!(lines.len(), 5);

        // First line should have "You: " prefix
        assert!(lines[0].to_string().starts_with("You: Line 1"));
        // Second line should be indented
        assert!(lines[1].to_string().starts_with("     Line 2"));
        // Third line should be empty
        assert_eq!(lines[2].to_string(), "");
        // Fourth line should be indented
        assert!(lines[3].to_string().starts_with("     Line 4"));
        // Fifth line should be empty spacing
        assert_eq!(lines[4].to_string(), "");
    }

    #[test]
    fn test_word_wrapping_with_long_paragraph() {
        let long_text = "This is a very long paragraph that contains many words and should definitely wrap across multiple lines when displayed in a narrow terminal window. The wrapping should be word-based, not character-based, to match ratatui's behavior.";

        let wrapped_narrow = ScrollCalculator::calculate_word_wrapped_lines(long_text, 40);
        let wrapped_wide = ScrollCalculator::calculate_word_wrapped_lines(long_text, 300); // Use wider width

        // Should wrap more with narrow width
        assert!(wrapped_narrow > wrapped_wide);
        assert!(wrapped_narrow > 3); // Should definitely wrap
        assert_eq!(wrapped_wide, 1); // Should fit in one line when wide enough
    }

    #[test]
    fn test_trimming_behavior() {
        let lines = vec![
            Line::from("  "),            // Only whitespace
            Line::from("   content   "), // Content with surrounding whitespace
            Line::from(""),              // Empty
        ];

        let count = ScrollCalculator::calculate_wrapped_line_count(&lines, 80);
        // All should count as single lines due to trimming
        assert_eq!(count, 3);
    }

    #[test]
    fn test_selection_highlight_builds_same_number_of_lines() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("user", "Hello"));
        messages.push_back(create_test_message("assistant", "Hi there!"));
        messages.push_back(create_test_message("user", "How are you?"));
        let theme = Theme::dark_default();

        let normal = ScrollCalculator::build_display_lines_with_theme(&messages, &theme);
        let highlighted = ScrollCalculator::build_display_lines_with_theme_and_selection_and_flags(
            &messages,
            &theme,
            Some(2),
            theme.streaming_indicator_style,
            true,
            true,
        );

        assert_eq!(normal.len(), highlighted.len());
    }

    #[test]
    fn test_scroll_offset_to_line_start_basic() {
        // Three lines: short, long, short. Width forces wrapping of the long line.
        let lines = vec![
            TLine::from("aaa"),
            TLine::from("bbb bbb bbb bbb"),
            TLine::from("ccc"),
        ];
        let width = 5u16;
        let available = 5u16;
        let off0 = ScrollCalculator::scroll_offset_to_line_start(&lines, width, available, 0);
        let off1 = ScrollCalculator::scroll_offset_to_line_start(&lines, width, available, 1);
        let off2 = ScrollCalculator::scroll_offset_to_line_start(&lines, width, available, 2);
        assert_eq!(off0, 0);
        assert!(off1 >= 1); // starts after first line
        assert!(off2 >= off1); // further down the view
    }
}
