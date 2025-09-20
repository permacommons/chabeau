use crate::core::message::Message;
#[cfg(test)]
use crate::ui::markdown::build_markdown_display_lines;
use crate::ui::theme::Theme;
use ratatui::{style::Style, text::Line, text::Span};
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

        let mut out: Vec<Line<'static>> = Vec::with_capacity(lines.len());

        for line in lines {
            if line.spans.is_empty() {
                out.push(Line::from(""));
                continue;
            }

            // Helpers to manage styled span appends
            let emit_line = |collector: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
                out.push(Line::from(std::mem::take(collector)));
            };
            let append_run = |collector: &mut Vec<Span<'static>>,
                              style: ratatui::style::Style,
                              text: &str| {
                if text.is_empty() {
                    return;
                }
                if let Some(last) = collector.last_mut() {
                    if last.style == style {
                        let mut combined = String::with_capacity(last.content.len() + text.len());
                        combined.push_str(&last.content);
                        combined.push_str(text);
                        let st = last.style;
                        *last = Span::styled(combined, st);
                        return;
                    }
                }
                collector.push(Span::styled(text.to_string(), style));
            };

            let mut cur_spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 4);
            let mut cur_len: usize = 0;
            let mut emitted_any = false;

            // Current word accumulated as styled segments
            let mut word_segs: Vec<(Vec<char>, ratatui::style::Style)> =
                Vec::with_capacity(line.spans.len() + 4);
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

    /// Build display lines using theme and flags (test-only)
    #[cfg(test)]
    pub fn build_display_lines_with_theme_and_flags(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
    ) -> Vec<Line<'static>> {
        Self::build_display_lines_with_theme_and_flags_and_width(
            messages,
            theme,
            markdown_enabled,
            syntax_enabled,
            None,
        )
    }

    /// Build display lines using theme, flags, and terminal width for table balancing
    pub fn build_display_lines_with_theme_and_flags_and_width(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
        terminal_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        // Route through the unified layout engine so downstream consumers get the same
        // width-aware line stream everywhere (renderer, scroll math, selection, etc.).
        let cfg = crate::ui::layout::LayoutConfig {
            width: terminal_width,
            markdown_enabled,
            syntax_enabled,
            table_overflow_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
        };
        let layout = crate::ui::layout::LayoutEngine::layout_messages(messages, theme, &cfg);
        layout.lines
    }

    /// Build display lines with selection highlighting and terminal width for table balancing
    pub fn build_display_lines_with_theme_and_selection_and_flags_and_width(
        messages: &VecDeque<Message>,
        theme: &Theme,
        selected_index: Option<usize>,
        highlight: ratatui::style::Style,
        markdown_enabled: bool,
        syntax_enabled: bool,
        terminal_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        let cfg = crate::ui::layout::LayoutConfig {
            width: terminal_width,
            markdown_enabled,
            syntax_enabled,
            table_overflow_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
        };
        let mut layout = crate::ui::layout::LayoutEngine::layout_messages(messages, theme, &cfg);

        if let Some(sel) = selected_index {
            if let Some(msg) = messages.get(sel) {
                if msg.role == "user" {
                    if let Some(span) = layout.message_spans.get(sel) {
                        let highlight_style = theme.selection_highlight_style.patch(highlight);
                        for (offset, line) in layout
                            .lines
                            .iter_mut()
                            .skip(span.start)
                            .take(span.len)
                            .enumerate()
                        {
                            let include_empty = offset < span.len.saturating_sub(1);
                            Self::apply_selection_highlight(
                                line,
                                highlight_style,
                                cfg.width,
                                include_empty,
                            );
                        }
                    }
                }
            }
        }

        layout.lines
    }

    /// Build display lines with codeblock highlighting and terminal width for table balancing
    pub fn build_display_lines_with_codeblock_highlight_and_flags_and_width(
        messages: &VecDeque<Message>,
        theme: &crate::ui::theme::Theme,
        selected_block: Option<usize>,
        highlight: ratatui::style::Style,
        markdown_enabled: bool,
        syntax_enabled: bool,
        terminal_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        let cfg = crate::ui::layout::LayoutConfig {
            width: terminal_width,
            markdown_enabled,
            syntax_enabled,
            table_overflow_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
        };
        let mut layout = crate::ui::layout::LayoutEngine::layout_messages(messages, theme, &cfg);

        if markdown_enabled {
            if let Some(idx) = selected_block {
                if let Some((start, len, _content)) = layout.codeblock_ranges.get(idx).cloned() {
                    let highlight_style = theme.selection_highlight_style.patch(highlight);
                    for line in layout.lines.iter_mut().skip(start).take(len) {
                        Self::apply_selection_highlight(line, highlight_style, cfg.width, true);
                    }
                }
            }
        }

        layout.lines
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

    /// Build display lines up to a specific message index with flags (test-only)
    #[cfg(test)]
    pub fn build_display_lines_up_to_with_flags(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
        up_to_index: usize,
    ) -> Vec<Line<'static>> {
        Self::build_display_lines_up_to_with_flags_and_width(
            messages,
            theme,
            markdown_enabled,
            syntax_enabled,
            up_to_index,
            None,
        )
    }

    /// Build display lines up to a specific message index with terminal width for table balancing
    pub fn build_display_lines_up_to_with_flags_and_width(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
        max_index: usize,
        terminal_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            if i > max_index {
                break;
            }
            let rendered = if markdown_enabled {
                crate::ui::markdown::render_message_markdown_opts_with_width(
                    msg,
                    theme,
                    syntax_enabled,
                    terminal_width,
                )
            } else {
                // Route plain text through the layout engine to apply width-aware wrapping
                let layout = crate::ui::layout::LayoutEngine::layout_plain_text(
                    &VecDeque::from([msg.clone()]),
                    theme,
                    terminal_width,
                    syntax_enabled,
                );
                crate::ui::markdown::RenderedMessage {
                    lines: layout.lines,
                }
            };
            lines.extend(rendered.lines);
        }
        lines
    }

    /// Calculate how many wrapped lines the given lines will take
    pub fn calculate_wrapped_line_count(lines: &[Line], _terminal_width: u16) -> u16 {
        // Lines provided by the unified layout pipeline are already width-aware.
        // Do not perform any additional wrapping here. This is the single source of truth
        // for visual line counts used by scroll calculations.
        lines.len() as u16
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

    /// Calculate scroll offset to show a specific message with exact display flags
    pub fn calculate_scroll_to_message_with_flags(
        messages: &VecDeque<Message>,
        theme: &Theme,
        markdown_enabled: bool,
        syntax_enabled: bool,
        message_index: usize,
        terminal_width: u16,
        available_height: u16,
    ) -> u16 {
        let lines = Self::build_display_lines_up_to_with_flags_and_width(
            messages,
            theme,
            markdown_enabled,
            syntax_enabled,
            message_index,
            Some(terminal_width as usize),
        );
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

impl ScrollCalculator {
    fn apply_selection_highlight(
        line: &mut Line<'static>,
        highlight: Style,
        width: Option<usize>,
        include_empty: bool,
    ) {
        let mut has_content = false;
        // Apply highlight to existing spans (preserves foreground styling)
        for span in &mut line.spans {
            span.style = span.style.patch(highlight);
            if !span.content.trim().is_empty() {
                has_content = true;
            }
        }

        if !has_content && !include_empty {
            return;
        }

        if let Some(target_width) = width {
            if line.spans.is_empty() {
                if include_empty && target_width > 0 {
                    let padding = " ".repeat(target_width);
                    *line = Line::from(Span::styled(padding, highlight));
                }
                return;
            }

            let current_width = line.width();
            if current_width < target_width {
                let padding = " ".repeat(target_width - current_width);
                line.spans.push(Span::styled(padding, highlight));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;
    use crate::utils::test_utils::{create_test_message, create_test_messages};
    use ratatui::style::Style;
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
        let theme = Theme::dark_default();
        let lines = ScrollCalculator::build_display_lines_up_to_with_flags(
            &messages, &theme, true, true, 1,
        );

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
        // Build content via the unified layout engine and compare line counts at different widths
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();
        let content = "Short line\n\nThis is a much longer line that might wrap depending on terminal width\nAnother short one";
        messages.push_back(Message {
            role: "assistant".into(),
            content: content.into(),
        });

        let lines_wide = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages,
            &theme,
            true,
            false,
            Some(100),
        );
        // Use plain text path to ensure paragraph wrap is exercised without markdown semantics
        let lines_narrow = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages,
            &theme,
            false,
            false,
            Some(20),
        );

        // With wide terminal (markdown), and narrow (plain), narrow should produce >= lines
        assert!(lines_narrow.len() >= lines_wide.len());
        // And not fewer lines than wide
        assert!(lines_narrow.len() >= lines_wide.len());
    }

    #[test]
    fn test_calculate_wrapped_line_count_zero_width() {
        let lines = vec![Line::from("Any content")];
        let count = ScrollCalculator::calculate_wrapped_line_count(&lines, 0);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_plain_text_long_line_wrapping() {
        // Plain-text mode should wrap long lines when a terminal width is provided
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();
        let long = "This is a very long plain text line without explicit newlines that should wrap when markdown is disabled";
        messages.push_back(Message {
            role: "assistant".into(),
            content: long.into(),
        });

        let width = 20usize;
        let lines = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages,
            &theme,
            false, // markdown disabled
            false,
            Some(width),
        );
        // Filter to content lines only (non-empty)
        let rendered: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let content_lines: Vec<String> = rendered.into_iter().filter(|s| !s.is_empty()).collect();

        // Should have wrapped into multiple visual lines
        assert!(
            content_lines.len() > 1,
            "Expected multiple wrapped lines in plain-text mode"
        );
        // No content line should exceed the specified width
        for (i, s) in content_lines.iter().enumerate() {
            assert!(
                s.chars().count() <= width,
                "Wrapped line {} exceeds width {}: '{}' (len={})",
                i,
                width,
                s,
                s.len()
            );
        }
        // Content must be preserved (no ellipsis)
        let joined = content_lines.join(" ");
        assert!(!joined.contains('…'));
        assert!(joined.contains("plain text line"));
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
        let theme = Theme::dark_default();
        let scroll_first = ScrollCalculator::calculate_scroll_to_message_with_flags(
            &messages, &theme, true, true, 0, 80, 10,
        );
        assert_eq!(scroll_first, 0);

        // Scroll to later message might require scrolling
        let scroll_later = ScrollCalculator::calculate_scroll_to_message_with_flags(
            &messages, &theme, true, true, 3, 80, 2,
        );
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
    fn table_scroll_height_matches_rendered() {
        // Test that prewrap line count matches rendered line count to prevent
        // unreachable table bottoms due to width mismatches
        let mut messages = VecDeque::new();
        let table_content = r#"Here's a test table:

| Government System | Definition |
|-------------------|------------|
| Democracy | A system where power is vested in the people |
| Dictatorship | A form of government where a single person holds absolute power |
| Monarchy | A form of government with a single ruler |
"#;

        messages.push_back(create_test_message("assistant", table_content));

        let theme = Theme::dark_default();
        let terminal_width = 80u16;

        // Build display lines using the same path as scroll calculations
        // Since we're using the same terminal width for both, the output should be identical
        let display_lines = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages,
            &theme,
            true,
            false,
            Some(terminal_width as usize),
        );
        let scroll_line_count = display_lines.len();

        // Now render using markdown with the same terminal width
        use crate::ui::markdown::render_message_markdown_opts_with_width;
        let rendered = render_message_markdown_opts_with_width(
            &messages[0],
            &theme,
            true,
            Some(terminal_width as usize),
        );
        let rendered_line_count = rendered.lines.len();

        // Key assertion: line counts should match since both use the same width constraint
        assert_eq!(
            scroll_line_count, rendered_line_count,
            "Scroll line count ({}) should match rendered line count ({}). \
                    This ensures scroll calculations are consistent with rendering.",
            scroll_line_count, rendered_line_count
        );

        // Additional check: verify table content is present
        let rendered_str = rendered
            .lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered_str.contains("Democracy") && rendered_str.contains("Dictatorship"),
            "Table content should be present in rendered output"
        );
    }

    #[test]
    fn test_selection_highlight_builds_same_number_of_lines() {
        let mut messages = VecDeque::new();
        messages.push_back(create_test_message("user", "Hello"));
        messages.push_back(create_test_message("assistant", "Hi there!"));
        messages.push_back(create_test_message("user", "How are you?"));
        let theme = Theme::dark_default();
        let highlight = Style::default();

        let normal = ScrollCalculator::build_display_lines_with_theme(&messages, &theme);
        let highlighted =
            ScrollCalculator::build_display_lines_with_theme_and_selection_and_flags_and_width(
                &messages,
                &theme,
                Some(0),
                highlight,
                true,
                true,
                None,
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

        // Performance thresholds for short histories (50 iterations):
        // - Warn at >= 90ms (non-fatal, prints to stderr)
        // - Fail at >= 200ms
        let ms = elapsed.as_millis();
        if ms >= 200 {
            panic!(
                "prewrap extremely slow: {:?} for {} total prewrapped lines",
                elapsed, total_lines
            );
        } else if ms >= 90 {
            eprintln!(
                "Warning: prewrap moderately slow: {:?} for {} total prewrapped lines",
                elapsed, total_lines
            );
        }
    }

    #[test]
    fn perf_prewrap_large_history() {
        // Larger synthetic history to exercise scaling
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();
        let base = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua";
        for i in 0..100 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            messages.push_back(create_test_message(role, base));
            messages.push_back(create_test_message(role, base));
        }

        let lines = ScrollCalculator::build_display_lines_with_theme_and_flags(
            &messages, &theme, true, false,
        );

        let width: u16 = 80;
        let iters = 20;
        let start = Instant::now();
        let mut total_lines = 0usize;
        for _ in 0..iters {
            let pre = ScrollCalculator::prewrap_lines(&lines, width);
            total_lines += pre.len();
        }
        let elapsed = start.elapsed();

        // Warn at moderate times, fail at excessive times for larger histories
        let ms = elapsed.as_millis();
        if ms >= 1000 {
            panic!(
                "prewrap extremely slow (large): {:?} for {} total prewrapped lines",
                elapsed, total_lines
            );
        } else if ms >= 400 {
            eprintln!(
                "Warning: prewrap moderately slow (large): {:?} for {} total prewrapped lines",
                elapsed, total_lines
            );
        }
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

    #[test]
    fn highlight_is_correct_after_wrapped_paragraph() {
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();
        let long_para = "This is a very long line that should wrap multiple times given a small terminal width so that the visual line count before the code block increases significantly.";
        let content = format!("{}\n\n```\ncode1\ncode2\n```", long_para);
        messages.push_back(Message {
            role: "assistant".into(),
            content,
        });

        let highlight = ratatui::style::Style::default();

        let lines =
            ScrollCalculator::build_display_lines_with_codeblock_highlight_and_flags_and_width(
                &messages,
                &theme,
                Some(0), // select first block
                highlight,
                true,     // markdown enabled
                false,    // syntax disabled for deterministic line counts
                Some(20), // small width to force wrapping
            );

        let expected_style = theme
            .md_codeblock_text_style()
            .patch(theme.selection_highlight_style.patch(highlight));

        // Determine expected highlighted range via width-aware ranges
        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &messages,
            &theme,
            Some(20usize),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            false,
        );
        assert_eq!(ranges.len(), 1, "Should have one code block");
        let (start, len, _content) = ranges[0].clone();
        for line in lines.iter().skip(start).take(len) {
            for sp in line
                .spans
                .iter()
                .filter(|span| !span.content.trim().is_empty())
            {
                assert_eq!(sp.style, expected_style, "Code line should be highlighted");
            }
        }
    }

    #[test]
    fn highlight_is_correct_after_table() {
        let theme = Theme::dark_default();
        let mut messages: VecDeque<Message> = VecDeque::new();
        // Message 0: a table that will be rendered before the code block
        messages.push_back(Message {
            role: "assistant".into(),
            content: r#"| A | B |\n|---|---|\n| 1 | 2 |\n"#.to_string(),
        });
        // Message 1: a code block to highlight
        messages.push_back(Message {
            role: "assistant".into(),
            content: "```\nalpha\nbeta\n```\n".to_string(),
        });

        let highlight =
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::BOLD);

        let lines =
            ScrollCalculator::build_display_lines_with_codeblock_highlight_and_flags_and_width(
                &messages,
                &theme,
                Some(0), // select first block
                highlight,
                true,  // markdown enabled
                false, // syntax disabled for determinism
                Some(60),
            );

        // Sanity: parser should find one code block
        let contents = crate::ui::markdown::compute_codeblock_contents_with_lang(&messages);
        assert_eq!(contents.len(), 1, "Parser should detect one code block");

        let expected_style = theme
            .md_codeblock_text_style()
            .patch(theme.selection_highlight_style.patch(highlight));

        // Determine expected highlighted range via width-aware ranges
        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &messages,
            &theme,
            Some(60usize),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            false,
        );
        assert_eq!(ranges.len(), 1, "Should have one code block");
        let (start, len, _content) = ranges[0].clone();
        for line in lines.iter().skip(start).take(len) {
            for sp in line
                .spans
                .iter()
                .filter(|span| !span.content.trim().is_empty())
            {
                assert_eq!(
                    sp.style, expected_style,
                    "Highlight modifiers should be applied"
                );
            }
        }
    }
}
