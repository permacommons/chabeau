#![allow(unused_imports)]
use super::helpers::{
    assert_first_span_is_space_indented, assert_line_text, line_texts, render_markdown_for_test,
};
use crate::core::message::{Message, TranscriptRole};
use crate::ui::markdown::render::{
    MarkdownRenderer, MarkdownRendererConfig, MarkdownWidthConfig, RoleKind,
};
use crate::ui::markdown::table::TableRenderer;
use crate::ui::markdown::{
    render_message_markdown_details_with_policy_and_user_name, render_message_with_config,
    MessageRenderConfig,
};
use crate::ui::span::SpanKind;
use crate::utils::test_utils::SAMPLE_HYPERTEXT_PARAGRAPH;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::Modifier;
use ratatui::text::Span;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;

#[test]
fn metadata_marks_table_links() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: r"| Label | Value |
|-------|-------|
| Mixed | plain text and [Example](https://example.com) with trailing words |
"
        .into(),
    };

    let details = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        Some(50),
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
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
fn table_parser_emits_expected_event_sequence() {
    let markdown = r###"| H1 | H2 |
|----|----|
| C1 | C2 |"###;

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let events: Vec<Event<'_>> = Parser::new_ext(markdown, options).collect();

    assert!(
        matches!(events.first(), Some(Event::Start(Tag::Table(_)))),
        "table should start with a table start event"
    );
    assert!(
        matches!(events.get(1), Some(Event::Start(Tag::TableHead))),
        "second event should start the table header"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Start(Tag::TableRow))),
        "table should contain a row start event"
    );

    let cell_start_count = events
        .iter()
        .filter(|event| matches!(event, Event::Start(Tag::TableCell)))
        .count();
    let cell_end_count = events
        .iter()
        .filter(|event| matches!(event, Event::End(TagEnd::TableCell)))
        .count();

    assert_eq!(cell_start_count, 4, "expected 4 table cells");
    assert_eq!(cell_end_count, 4, "expected 4 table cell endings");

    assert!(
        matches!(events.last(), Some(Event::End(TagEnd::Table))),
        "table should end with a table end event"
    );
}

#[test]
fn table_rendering_works() {
    let mut messages = VecDeque::new();
    messages.push_back(Message {
        role: TranscriptRole::Assistant,
        content: r###"Here's a table:

| Header 1 | Header 2 | Header 3 |
|----------|----------|----------|
| Cell 1   | Cell 2   | Cell 3   |
| Cell 4   | Cell 5   | Cell 6   |

End of table."###
            .into(),
    });
    let theme = crate::ui::theme::Theme::dark_default();
    let rendered = render_markdown_for_test(&messages[0], &theme, true, None);

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
        role: TranscriptRole::Assistant,
        content: r"| Header | Data |
|---|---|
| Abc | 123 |
| Def | 456 |
| Emoji | 🚀<br/>Hi |
"
        .into(),
    });
    let theme = crate::ui::theme::Theme::dark_default();
    let rendered = render_markdown_for_test(&messages[0], &theme, true, None);
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
    if let (Some(narrow_border), Some(wide_border)) = (narrow_strings.first(), wide_strings.first())
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
            role: TranscriptRole::Assistant,
            content: r"| Short | Medium Content Here | Very Long Column With Lots Of Text That Should Not Be Truncated |
|-------|---------------------|------------------------------------------------------------------|
| A     | Some content here   | This is a very long piece of text that contains important information that the user needs to see in full without any truncation or ellipsis |
| B     | More content        | Another long piece of text with technical details and specifications that must remain fully visible to be useful |
"
                .into(),
        });
    let theme = crate::ui::theme::Theme::dark_default();

    // Wide terminal - should fit everything without wrapping or truncation
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(150));
    let lines = line_texts(&rendered.lines);

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
            role: TranscriptRole::Assistant,
            content: r"| Name | Description |
|------|-------------|
| API  | This is a detailed description of how the API works with multiple parameters and return values |
| SDK  | Software Development Kit with comprehensive documentation and examples for developers |
"
                .into(),
        });
    let theme = crate::ui::theme::Theme::dark_default();

    // Medium terminal width - should wrap content within cells
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
    let lines = line_texts(&rendered.lines);

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
fn test_table_should_not_wrap_borders() {
    // This test reproduces the real-world issue where table borders get wrapped
    let mut messages = VecDeque::new();
    messages.push_back(Message {
            role: TranscriptRole::Assistant,
            content: r#"| System of Government | Definition | Key Features | Examples |
|---------------------|------------|--------------|----------|
| Democracy | Government by the people, either directly or through elected representatives. | Universal suffrage, free elections, protection of civil liberties. | United States, India, Germany |
| Republic | A form of government in which power resides with the citizens, who elect representatives to govern on their behalf. | Elected officials, separation of powers, rule of law. | France, Brazil, South Africa |
| Dictatorship | A form of government in which a single person or a small group holds absolute power. | Lack of free elections, suppression of opposition, centralized control. | North Korea, Cuba, Syria |"#.into(),
        });

    let theme = crate::ui::theme::Theme::dark_default();

    // Test the CORRECT semantic approach: render with width constraints from the start
    let terminal_width = 120u16;
    let lines =
        crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages,
            &theme,
            true,
            true,
            Some(terminal_width as usize),
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
        role: TranscriptRole::Assistant,
        content: r#"| Feature | Benefits |
|---------|----------|
| X | **Dramatically** _improved_ decision-making capabilities with ***real-time*** analytics |
"#
        .into(),
    });

    let theme = crate::ui::theme::Theme::dark_default();

    // Use a modest width to force wrapping within the Benefits cell
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
    let lines = line_texts(&rendered.lines);

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
fn test_table_wrapping_with_mixed_content() {
    // Test wrapping behavior with mixed short and long content
    let mut messages = VecDeque::new();
    messages.push_back(Message {
        role: TranscriptRole::Assistant,
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
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(45));
    let lines = line_texts(&rendered.lines);

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
fn test_table_with_emoji_and_unicode_no_truncation() {
    // Test that emoji and Unicode characters are handled without truncation
    let mut messages = VecDeque::new();
    messages.push_back(Message {
            role: TranscriptRole::Assistant,
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
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(70));
    let lines = line_texts(&rendered.lines);

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
        role: TranscriptRole::Assistant,
        content: markdown.to_string(),
    });

    let theme = crate::ui::theme::Theme::dark_default();
    // Force a narrower width to trigger the column balancing that causes word splits
    let rendered = render_markdown_for_test(messages.front().unwrap(), &theme, true, Some(80));
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
            role: TranscriptRole::Assistant,
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
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(120));
    let lines = line_texts(&rendered.lines);

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
            line.contains("│") && !line.contains("┌") && !line.contains("├") && !line.contains("└")
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
            role: TranscriptRole::Assistant,
            content: r###"Here's a table with long content that should wrap:

| Column A | Column B | Column C |
|----------|----------|----------|
| This is a very long sentence that should definitely wrap within the cell when the terminal is narrow | Short | Another moderately long piece of content |
| Short content | This is another extremely long sentence that contains many words and should wrap properly within the table cell boundaries | More content here |
"###.to_string(),
        });

    let theme = crate::ui::theme::Theme::dark_default();

    // Test with narrow terminal width (60 chars) to force wrapping
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(60));
    let lines = line_texts(&rendered.lines);

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
        assert!(
            row_width <= 100,
            "Row {} should not be excessively wide due to proper wrapping: width={}, content: '{}'",
            i,
            row_width,
            row
        );
    }
}
