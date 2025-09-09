use crate::core::message::Message;
#[cfg(test)]
use crate::ui::markdown::build_markdown_display_lines;
use crate::ui::markdown::{
    build_plain_display_lines, compute_codeblock_ranges, render_message_markdown_opts,
};
use crate::ui::theme::Theme;
use ratatui::{text::Line, text::Span};
use std::collections::VecDeque;

/// Handles all scroll-related calculations and line building
pub struct ScrollCalculator;

impl ScrollCalculator {
    /// Pre-wrap the given lines to a specific width, preserving styles and wrapping at word
    /// boundaries consistent with the input wrapper (also breaks long tokens when needed).
    /// This allows rendering without ratatui's built-in wrapping, ensuring counts match output.
    pub fn prewrap_lines(lines: &[Line], terminal_width: u16) -> Vec<Line<'static>> {
        let width = terminal_width as usize;
        // Fast path: zero width, just clone as owned
        if width == 0 {
            let mut out = Vec::with_capacity(lines.len());
            for line in lines {
                if line.spans.is_empty() {
                    out.push(Line::from(""));
                } else {
                    let spans: Vec<Span<'static>> = line
                        .spans
                        .iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style))
                        .collect();
                    out.push(Line::from(spans));
                }
            }
            return out;
        }

        let mut out: Vec<Line<'static>> = Vec::new();

        for line in lines {
            if line.spans.is_empty() {
                out.push(Line::from(""));
                continue;
            }

            // Helpers to manage styled span appends
            let emit_line = |collector: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
                out.push(Line::from(std::mem::take(collector)));
            };
            let append_run =
                |collector: &mut Vec<Span<'static>>, style: ratatui::style::Style, text: &str| {
                    if text.is_empty() {
                        return;
                    }
                    if let Some(last) = collector.last_mut() {
                        if last.style == style {
                            let combined = format!("{}{}", last.content, text);
                            let st = last.style;
                            *last = Span::styled(combined, st);
                            return;
                        }
                    }
                    collector.push(Span::styled(text.to_string(), style));
                };

            let mut cur_spans: Vec<Span<'static>> = Vec::new();
            let mut cur_len: usize = 0;
            let mut emitted_any = false;

            // Current word accumulated as styled segments
            let mut word_segs: Vec<(Vec<char>, ratatui::style::Style)> = Vec::new();
            let mut word_len: usize = 0;

            let flush_word = |cur_spans: &mut Vec<Span<'static>>,
                              out: &mut Vec<Line<'static>>,
                              cur_len: &mut usize,
                              emitted_any: &mut bool,
                              word_segs: &mut Vec<(Vec<char>, ratatui::style::Style)>,
                              word_len: &mut usize| {
                if *word_len == 0 {
                    return;
                }
                // Wrap before word if it doesn't fit
                if *cur_len > 0 && *cur_len + *word_len > width {
                    emit_line(cur_spans, out);
                    *emitted_any = true;
                    *cur_len = 0;
                }
                // Place the word, chunking if needed
                let mut seg_idx = 0usize;
                let mut seg_pos = 0usize;
                let mut remaining = *word_len;
                while remaining > 0 {
                    let space_left = width.saturating_sub(*cur_len);
                    let take = remaining.min(space_left.max(1));
                    let mut to_take = take;
                    while to_take > 0 && seg_idx < word_segs.len() {
                        let (seg_chars, seg_style) = &word_segs[seg_idx];
                        let seg_rem = seg_chars.len().saturating_sub(seg_pos);
                        let here = to_take.min(seg_rem);
                        if here > 0 {
                            let slice: String = seg_chars[seg_pos..seg_pos + here].iter().collect();
                            append_run(cur_spans, *seg_style, &slice);
                            *cur_len += here;
                            to_take -= here;
                            seg_pos += here;
                        }
                        if seg_pos >= seg_chars.len() {
                            seg_idx += 1;
                            seg_pos = 0;
                        }
                    }
                    remaining -= take;
                    if remaining > 0 {
                        emit_line(cur_spans, out);
                        *emitted_any = true;
                        *cur_len = 0;
                    }
                }
                word_segs.clear();
                *word_len = 0;
            };

            for s in &line.spans {
                for ch in s.content.chars() {
                    if ch == ' ' {
                        // Place accumulated word before handling space
                        flush_word(
                            &mut cur_spans,
                            &mut out,
                            &mut cur_len,
                            &mut emitted_any,
                            &mut word_segs,
                            &mut word_len,
                        );

                        // Add a single space if it fits; otherwise wrap and skip leading space
                        if cur_len < width {
                            append_run(&mut cur_spans, s.style, " ");
                            cur_len += 1;
                        } else {
                            emit_line(&mut cur_spans, &mut out);
                            emitted_any = true;
                            cur_len = 0;
                        }
                    } else {
                        // Accumulate into current word, merging by style
                        if let Some((last_text, last_style)) = word_segs.last_mut() {
                            if *last_style == s.style {
                                last_text.push(ch);
                            } else {
                                word_segs.push((vec![ch], s.style));
                            }
                        } else {
                            word_segs.push((vec![ch], s.style));
                        }
                        word_len += 1;
                    }
                }
            }

            // Flush any remaining word and finalize the line
            flush_word(
                &mut cur_spans,
                &mut out,
                &mut cur_len,
                &mut emitted_any,
                &mut word_segs,
                &mut word_len,
            );

            if !cur_spans.is_empty() {
                emit_line(&mut cur_spans, &mut out);
                emitted_any = true;
            }
            if !emitted_any {
                // Preserve a single empty visual line for whitespace-only inputs
                out.push(Line::from(""));
            }
        }

        out
    }
    /// Build display lines for all messages (tests only)
    #[cfg(test)]
    pub fn build_display_lines(messages: &VecDeque<Message>) -> Vec<Line<'static>> {
        // Backwards-compatible default theme
        let theme = Theme::dark_default();
        Self::build_display_lines_with_theme(messages, &theme)
    }

    /// Build display lines using a provided theme (tests only)
    #[cfg(test)]
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
        let pre = Self::prewrap_lines(lines, terminal_width);
        pre.len() as u16
    }

    /// Calculate how many lines a single text string will wrap to
    #[cfg(test)]
    fn calculate_word_wrapped_lines_with_leading(text: &str, terminal_width: u16) -> u16 {
        let width = terminal_width as usize;
        if width == 0 {
            return 1;
        }

        // Always at least one visual line
        let mut line_count: u16 = 1;
        let mut current_len: usize;

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
        if leading_spaces >= width {
            line_count = line_count.saturating_add((leading_spaces / width) as u16);
            current_len = leading_spaces % width;
        } else {
            current_len = leading_spaces;
        }

        // Process remainder as words, but break overlong words to avoid undercounting.
        let remainder: String = chars.collect();
        for word in remainder.split_whitespace() {
            let mut word_len = word.chars().count();

            // Insert a single space before the word if not at line start
            if current_len > 0 {
                if current_len + 1 > width {
                    line_count = line_count.saturating_add(1);
                    current_len = 0;
                } else {
                    current_len += 1;
                }
            }

            // Place the word, chunking if it exceeds the available width
            loop {
                let space_left = width.saturating_sub(current_len);
                if word_len <= space_left {
                    current_len += word_len;
                    break;
                }
                if space_left > 0 {
                    // Fill the current line and wrap
                    word_len -= space_left;
                    line_count = line_count.saturating_add(1);
                    current_len = 0;
                } else {
                    // No space left, wrap to new line
                    line_count = line_count.saturating_add(1);
                    current_len = 0;
                }
            }
        }

        line_count.max(1)
    }

    // Wrapper only for tests that reference the original name
    #[cfg(test)]
    fn calculate_word_wrapped_lines(text: &str, terminal_width: u16) -> u16 {
        Self::calculate_word_wrapped_lines_with_leading(text, terminal_width)
    }

    /// Calculate scroll offset to show the bottom of all messages
    #[cfg(test)]
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
    #[cfg(test)]
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
    use std::time::Instant;

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
        // Single word longer than width should wrap across multiple lines
        let wrapped = ScrollCalculator::calculate_word_wrapped_lines(
            "supercalifragilisticexpialidocious",
            10,
        );
        assert!(wrapped > 1);
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
    fn perf_prewrap_short_history() {
        // Synthetic short history with styled spans to exercise prewrap mapping
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();

        // Build ~30 lines alternating user/assistant, with words split into separate spans
        let base = "lorem ipsum dolor sit amet consectetur adipiscing elit";
        for i in 0..15 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            messages.push_back(create_test_message(role, base));
            messages.push_back(create_test_message(role, base));
        }

        // Render to styled lines (markdown on, syntax off for speed)
        let lines = ScrollCalculator::build_display_lines_with_theme_and_flags(
            &messages, &theme, true, false,
        );

        // Time multiple prewrap passes to smooth out noise
        let width: u16 = 100;
        let iters = 50;
        let start = Instant::now();
        let mut total_lines = 0usize;
        for _ in 0..iters {
            let pre = ScrollCalculator::prewrap_lines(&lines, width);
            total_lines += pre.len();
        }
        let elapsed = start.elapsed();

        // Performance threshold for short histories. Keep total under ~90ms
        // for 50 iterations on a small set of lines.
        assert!(
            elapsed.as_millis() < 90,
            "prewrap too slow: {:?} for {} total prewrapped lines",
            elapsed,
            total_lines
        );
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

    #[test]
    fn test_prewrap_paragraph_no_leading_spaces_or_lonely_dot() {
        let paragraph = "The way language shapes our perception of reality is something that linguists and philosophers have debated for centuries. Do we think in words, or do words simply provide a framework for thoughts that exist beyond language? Some cultures have dozens of words for different types of snow, while others have elaborate systems for describing relationships between family members. These linguistic differences suggest that our vocabulary doesn't just describe our world - it actually influences how we see and understand it.";
        let width: u16 = 143;
        let line = TLine::from(paragraph);
        let pre = ScrollCalculator::prewrap_lines(&[line], width);
        assert!(!pre.is_empty());
        for l in pre {
            let s = l.to_string();
            assert!(
                !s.starts_with(' '),
                "wrapped line starts with space: '{} '",
                s
            );
            assert_ne!(s.trim(), ".", "wrapped line became a lonely '.'");
        }
    }
}
