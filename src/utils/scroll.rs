use crate::core::message::Message;
use crate::ui::theme::Theme;
use ratatui::{
    style::Style,
    text::{Line, Span},
};
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
        let mut lines = Vec::new();
        for msg in messages {
            Self::add_message_lines_with_theme(&mut lines, msg, theme);
        }
        lines
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
            Self::add_message_lines_with_theme(&mut lines, msg, &theme);
        }

        lines
    }

    /// Add lines for a single message to the lines vector
    fn add_message_lines_with_theme(lines: &mut Vec<Line<'static>>, msg: &Message, theme: &Theme) {
        if msg.role == "user" {
            // User messages: cyan with "You:" prefix and proper multiline handling
            let content_lines: Vec<&str> = msg.content.lines().collect();

            if content_lines.is_empty() {
                // Empty content, just show "You: "
                lines.push(Line::from(vec![Span::styled(
                    "You: ",
                    theme.user_prefix_style,
                )]));
            } else {
                // First line gets the "You: " prefix
                lines.push(Line::from(vec![
                    Span::styled("You: ", theme.user_prefix_style),
                    Span::styled(content_lines[0].to_string(), theme.user_text_style),
                ]));

                // Subsequent lines are indented to align with the content
                for content_line in content_lines.iter().skip(1) {
                    if content_line.trim().is_empty() {
                        lines.push(Line::from(""));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "     ", // 5 spaces to align with content after "You: "
                                Style::default(),
                            ),
                            Span::styled(content_line.to_string(), theme.user_text_style),
                        ]));
                    }
                }
            }
            lines.push(Line::from("")); // Empty line for spacing
        } else if msg.role == "system" {
            // System messages: gray/dim color, split content into lines for proper wrapping
            for content_line in msg.content.lines() {
                if content_line.trim().is_empty() {
                    lines.push(Line::from(""));
                } else {
                    lines.push(Line::from(Span::styled(
                        content_line.to_string(),
                        theme.system_text_style,
                    )));
                }
            }
            lines.push(Line::from("")); // Empty line for spacing
        } else if !msg.content.is_empty() {
            // Assistant messages: no prefix, just content in white/default color
            // Split content into lines for proper wrapping
            for content_line in msg.content.lines() {
                if content_line.trim().is_empty() {
                    lines.push(Line::from(""));
                } else {
                    lines.push(Line::from(Span::styled(
                        content_line.to_string(),
                        theme.assistant_text_style,
                    )));
                }
            }
            lines.push(Line::from("")); // Empty line for spacing
        }
    }

    /// Calculate how many wrapped lines the given lines will take
    pub fn calculate_wrapped_line_count(lines: &[Line], terminal_width: u16) -> u16 {
        let mut total_wrapped_lines = 0u16;

        for line in lines {
            let line_text = line.to_string();
            if line_text.is_empty() {
                total_wrapped_lines = total_wrapped_lines.saturating_add(1);
            } else {
                // Trim whitespace to match ratatui's Wrap { trim: true } behavior
                let trimmed_text = line_text.trim();

                if trimmed_text.is_empty() || terminal_width == 0 {
                    total_wrapped_lines = total_wrapped_lines.saturating_add(1);
                } else {
                    // Word-based wrapping to match ratatui's behavior
                    let wrapped_count =
                        Self::calculate_word_wrapped_lines(trimmed_text, terminal_width);
                    total_wrapped_lines = total_wrapped_lines.saturating_add(wrapped_count);
                }
            }
        }

        total_wrapped_lines
    }

    /// Calculate how many lines a single text string will wrap to
    fn calculate_word_wrapped_lines(text: &str, terminal_width: u16) -> u16 {
        let mut current_line_len = 0;
        let mut line_count = 1u16;

        for word in text.split_whitespace() {
            let word_len = word.chars().count();

            // Start new line if adding this word would exceed width
            if current_line_len > 0 && current_line_len + 1 + word_len > terminal_width as usize {
                line_count = line_count.saturating_add(1);
                current_line_len = word_len;
            } else {
                if current_line_len > 0 {
                    current_line_len += 1; // Add space
                }
                current_line_len += word_len;
            }
        }

        line_count
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
    use crate::utils::test_utils::{create_test_message, create_test_messages};
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
}
