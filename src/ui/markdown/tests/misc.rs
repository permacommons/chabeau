use super::helpers::{assert_line_text, line_texts, render_markdown_for_test};
use crate::core::message::{Message, TranscriptRole};
use crate::ui::markdown::table::TableRenderer;
use crate::ui::markdown::{render_message_with_config, MessageRenderConfig};
use crate::ui::span::SpanKind;
use ratatui::text::Span;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;

#[test]
fn tool_call_arguments_do_not_render_markdown() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message::tool_call("lookup | Arguments: q=\"**bold**\"".to_string());

    let rendered = render_markdown_for_test(&message, &theme, true, None);
    let rendered_text = rendered
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(rendered_text.contains("**bold**"));
}

#[test]
fn markdown_images_emit_clickable_links() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: "Look at this sketch: ![diagram](https://example.com/diagram.png) neat, right?"
            .into(),
    };

    let cfg = MessageRenderConfig::markdown(true, false).with_span_metadata();
    let details = render_message_with_config(&message, &theme, cfg);
    let metadata = details.span_metadata.expect("metadata present");
    let mut saw_image_link = false;
    for kinds in metadata {
        for kind in kinds {
            if let Some(meta) = kind.link_meta() {
                if meta.href() == "https://example.com/diagram.png" {
                    saw_image_link = true;
                }
            }
        }
    }

    assert!(
        saw_image_link,
        "expected image alt text to emit a hyperlink"
    );

    let rendered_text = details
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(rendered_text.contains("diagram"));
}

#[test]
fn horizontal_rules_render_as_centered_lines() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: "Above\n\n---\n\nBelow".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, true, Some(50));
    let hr_line = rendered
        .lines
        .iter()
        .find(|line| line.to_string().contains('─'))
        .expect("horizontal rule should render");

    let hr_text = hr_line.to_string();
    assert_eq!(UnicodeWidthStr::width(hr_text.as_str()), 50);

    let hr_chars: Vec<char> = hr_text.chars().collect();
    let first_rule_idx = hr_chars
        .iter()
        .position(|c| *c == '─')
        .expect("rule characters present");
    let rule_len = hr_chars[first_rule_idx..]
        .iter()
        .take_while(|c| **c == '─')
        .count();
    let right_padding = hr_chars.len().saturating_sub(first_rule_idx + rule_len);

    assert_eq!(first_rule_idx, 5);
    assert_eq!(rule_len, 40);
    assert_eq!(right_padding, 5);

    let rule_span = hr_line
        .spans
        .iter()
        .find(|s| s.content.as_ref().contains('─'))
        .expect("rule span present");
    assert_eq!(rule_span.style, theme.md_rule_style());
}

#[test]
fn superscript_and_subscript_render_without_markers() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: "Subscripts: ~abc~ alongside superscripts: ^def^.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, true, None);
    let lines = line_texts(&rendered.lines);

    assert!(
        lines.len() >= 2,
        "expected rendered output to include paragraph and trailing blank line"
    );
    assert_line_text(&lines, 0, "Subscripts: abc alongside superscripts: def.");
    assert!(
        lines[1].is_empty(),
        "renderer should emit blank line after paragraph"
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
fn test_extremely_narrow_terminal_no_truncation() {
    // Test that even extremely narrow terminals never truncate content
    let mut messages = VecDeque::new();
    messages.push_back(Message {
        role: TranscriptRole::Assistant,
        content: r"| A | B |
|---|---|
| VeryLongUnbreakableWord | AnotherLongWord |
"
        .into(),
    });
    let theme = crate::ui::theme::Theme::dark_default();

    // Extremely narrow terminal (20 chars)
    let rendered = render_markdown_for_test(&messages[0], &theme, true, Some(20));
    let lines = line_texts(&rendered.lines);

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
fn emphasis_with_standalone_paren() {
    // Test punctuation that "stands alone" (has space before it)
    // "Space exploration is fundamentally" = 34 chars
    // If we add " )" that would be 36 chars (at width limit)
    // But let's test when the ) is 1 past by using fundamentally_x
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: "Space exploration is *fundamentally_x* )".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));
    let lines = line_texts(&rendered.lines);

    eprintln!("\n=== DEBUG emphasis_with_standalone_paren ===");
    for (i, line_obj) in rendered.lines.iter().enumerate() {
        let line_str: String = line_obj.to_string();
        eprintln!(
            "Line {}: '{}' (width={})",
            i,
            line_str,
            line_str.chars().count()
        );
        for (j, span) in line_obj.spans.iter().enumerate() {
            eprintln!(
                "    Span {}: content='{}' width={}",
                j,
                span.content,
                span.content.width()
            );
        }
    }
    eprintln!("=== END DEBUG ===\n");

    // The ) has a space before it, so it "stands alone" and can wrap by itself
    // We should NOT backtrack and merge fundamentally_x with )
    // Expected: Line 1 ends with "fundamentally_x", Line 2 is just ")"
    assert_eq!(lines[0], "Space exploration is fundamentally_x");
    assert_eq!(lines[1], ")");
}
