#![allow(unused_imports)]
use super::helpers::{
    assert_first_span_is_space_indented, assert_line_text, line_texts, render_markdown_for_test,
};
use crate::core::message::Message;
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
use pulldown_cmark::{Options, Parser};
use ratatui::style::Modifier;
use ratatui::text::Span;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;

#[test]
fn shared_renderer_with_metadata_matches_details_wrapper() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "A [link](https://example.com) and a code block.\n\n```rust\nfn main() {}\n```"
            .into(),
    };

    let expected = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        Some(48),
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );

    let (lines, metadata) = MarkdownRenderer::new(
        RoleKind::Assistant,
        &message.content,
        &theme,
        MarkdownRendererConfig {
            collect_span_metadata: true,
            syntax_highlighting: true,
            width: Some(MarkdownWidthConfig {
                terminal_width: Some(48),
                table_policy: crate::ui::layout::TableOverflowPolicy::WrapCells,
            }),
            user_display_name: None,
        },
    )
    .render();

    assert_eq!(expected.lines, lines);
    let expected_metadata = expected
        .span_metadata
        .expect("details wrapper should provide metadata");
    assert_eq!(expected_metadata, metadata);
}

#[test]
fn markdown_links_wrap_at_word_boundaries_with_width() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "abcd efgh [hypertext dreams](https://docs.hypertext.org) and more text".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, true, Some(10));
    let lines = line_texts(&rendered.lines);
    let combined = lines.join("\n");

    assert!(
        combined.contains("hypertext"),
        "combined output should include the link text: {:?}",
        combined
    );
    assert!(
        !combined.contains("hype\nrtext"),
        "link text should wrap at the space boundary, not mid-word: {:?}",
        combined
    );

    let wider = render_markdown_for_test(&message, &theme, true, Some(15));
    let wider_text = wider
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !wider_text.contains("hype\nrtext"),
        "link text should stay intact even when more columns are available: {:?}",
        wider_text
    );
}

#[test]
fn markdown_links_wrap_in_long_paragraph_without_mid_word_break() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: SAMPLE_HYPERTEXT_PARAGRAPH.to_string(),
    };

    let rendered = render_markdown_for_test(&message, &theme, true, Some(158));
    let combined = rendered
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !combined.contains("hype\nrtext"),
        "wide layout still broke link mid-word: {:?}",
        combined
    );
    assert!(
        combined.contains("hypertext dreams"),
        "link text missing from output: {:?}",
        combined
    );
}

#[test]
fn cell_wraps_at_space_across_spans() {
    // Ensure wrapping prefers spaces even when they occur across styled spans
    let theme = crate::ui::theme::Theme::dark_default();
    let ts = TableRenderer::new();

    let bold = theme.md_paragraph_style().add_modifier(Modifier::BOLD);
    let spans = vec![
        (Span::styled("foo", bold), SpanKind::Text),
        (Span::raw(" "), SpanKind::Text),
        (Span::styled("bar", bold), SpanKind::Text),
    ];

    // Width fits "foo" exactly; space + "bar" should go to next line
    let lines =
        ts.wrap_spans_to_width(&spans, 3, crate::ui::layout::TableOverflowPolicy::WrapCells);
    let rendered: Vec<String> = lines
        .iter()
        .map(|spans| {
            spans
                .iter()
                .map(|(s, _)| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0], "foo");
    assert_eq!(rendered[1], "bar");
}

#[test]
fn cell_wraps_after_hyphen() {
    // Ensure hyphen is treated as a soft break opportunity
    let theme = crate::ui::theme::Theme::dark_default();
    let ts = TableRenderer::new();
    let style = theme.md_paragraph_style();
    let spans = vec![(Span::styled("decision-making", style), SpanKind::Text)];

    // Allow exactly "decision-" on first line
    let lines = ts.wrap_spans_to_width(
        &spans,
        10,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
    );
    let rendered: Vec<String> = lines
        .iter()
        .map(|spans| {
            spans
                .iter()
                .map(|(s, _)| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0], "decision-");
    assert_eq!(rendered[1], "making");
}

#[test]

fn wrapped_code_preserves_metadata_across_lines() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::wrapped_code();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        Some(40), // Narrow width to force wrapping
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    // All code spans should have block_index 0
    let block_indices: Vec<usize> = metadata
        .iter()
        .flat_map(|line| line.iter())
        .filter_map(|k| k.code_block_meta().map(|m| m.block_index()))
        .collect();

    assert!(!block_indices.is_empty(), "Should have code block metadata");
    assert!(
        block_indices.iter().all(|&idx| idx == 0),
        "All wrapped lines should have same block_index"
    );
}

#[test]
fn emphasis_ending_one_after_width() {
    // Test when italic word extends one char past the width
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *fundamentally* good.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(34));
    let lines = line_texts(&rendered.lines);

    // The word "good" should not have leading space
    let good_line = lines
        .iter()
        .find(|l| l.contains("good"))
        .expect("The word 'good' should appear in the wrapped output");
    assert!(
        !good_line.starts_with(" good"),
        "The word 'good' should not have a leading space. Found: '{}'",
        good_line
    );
}

#[test]
fn strong_emphasis_ending_at_width() {
    // Test with **bold** text
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is **fundamentally** useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(35));
    let lines = line_texts(&rendered.lines);

    let useful_line = lines
        .iter()
        .find(|l| l.contains("useful"))
        .expect("The word 'useful' should appear in the wrapped output");
    assert!(
        !useful_line.starts_with(" useful"),
        "The word 'useful' after bold should not have a leading space. Found: '{}'",
        useful_line
    );
}

#[test]
fn inline_code_ending_at_width() {
    // Test with `code` text
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "The function is `very_important_func` today.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(35));
    let lines = line_texts(&rendered.lines);

    // Debug: print all lines and their details
    eprintln!("\n=== DEBUG inline_code_ending_at_width ===");
    for (i, line_obj) in rendered.lines.iter().enumerate() {
        let line_str: String = line_obj.to_string();
        eprintln!("Line {}: '{}'", i, line_str);
        eprintln!("  Spans: {}", line_obj.spans.len());
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

    let today_line = lines
        .iter()
        .find(|l| l.contains("today"))
        .expect("The word 'today' should appear in the wrapped output");
    assert!(
        !today_line.starts_with(" today"),
        "The word 'today' after inline code should not have a leading space. Found: '{}'",
        today_line
    );
}

#[test]
fn link_ending_at_width() {
    // Test with [text](url) links
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Check out [this important resource](http://example.com) here.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(35));
    let lines = line_texts(&rendered.lines);

    let here_line = lines
        .iter()
        .find(|l| l.contains("here"))
        .expect("The word 'here' should appear in the wrapped output");
    assert!(
        !here_line.starts_with(" here"),
        "The word 'here' after link should not have a leading space. Found: '{}'",
        here_line
    );
}

#[test]
fn strikethrough_ending_at_width() {
    // Test with ~~strikethrough~~ text
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "This approach is ~~fundamentally~~ useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(30));
    let lines = line_texts(&rendered.lines);

    let useful_line = lines
        .iter()
        .find(|l| l.contains("useful"))
        .expect("The word 'useful' should appear in the wrapped output");
    assert!(
        !useful_line.starts_with(" useful"),
        "The word 'useful' after strikethrough should not have a leading space. Found: '{}'",
        useful_line
    );
}

#[test]
fn emphasis_with_punctuation_at_width() {
    // Test emphasis followed by punctuation then space
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *fundamentally*, I think, useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));
    let lines = line_texts(&rendered.lines);

    // The "I think" part should not have inappropriate leading space
    let i_line = lines
        .iter()
        .find(|l| l.contains("I think"))
        .expect("The phrase 'I think' should appear in the wrapped output");
    // Allow leading space if the whole phrase wrapped, but check for double space
    assert!(
        !i_line.starts_with("  "),
        "Should not have double leading space. Found: '{}'",
        i_line
    );
}

#[test]
fn emphasis_with_paren_inside_at_width() {
    // Test paren INSIDE emphasis: *(word)* next
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *(fundamentally)* useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));
    let lines = line_texts(&rendered.lines);

    eprintln!("\n=== DEBUG emphasis_with_paren_inside_at_width (paren INSIDE) ===");
    for (i, line_obj) in rendered.lines.iter().enumerate() {
        let line_str: String = line_obj.to_string();
        eprintln!("Line {}: '{}'", i, line_str);
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

    if lines.len() > 1 {
        let second_line = &lines[1];
        assert!(
            !second_line.starts_with(" useful"),
            "Should not have ' useful' at start of wrapped line. Found: '{}'",
            second_line
        );
    }
}

#[test]
fn emphasis_with_paren_outside_one_past_width() {
    // Test paren OUTSIDE emphasis, but paren is exactly 1 char past boundary
    // "Space exploration is " = 21 chars
    // "fundamentally_x" = 15 chars
    // Total = 36 chars (exactly at width)
    // Then ")" is at position 37 (1 past boundary)
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *fundamentally_x*) useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));
    let lines = line_texts(&rendered.lines);

    eprintln!("\n=== DEBUG emphasis_with_paren_outside_one_past_width ===");
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

    // With backtracking, the styled word wraps with punctuation and space preserved
    assert_eq!(lines[0], "Space exploration is ");
    assert_eq!(lines[1], "fundamentally_x) useful.");
}

#[test]
fn code_with_closing_paren_at_width() {
    // Test inline code followed by closing paren: `code`) next
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "The function is `very_important_func`) today.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));
    let lines = line_texts(&rendered.lines);

    eprintln!("\n=== DEBUG code_with_closing_paren_at_width ===");
    for (i, line_obj) in rendered.lines.iter().enumerate() {
        let line_str: String = line_obj.to_string();
        eprintln!("Line {}: '{}'", i, line_str);
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

    if lines.len() > 1 {
        let second_line = &lines[1];
        assert!(
            !second_line.starts_with(") today"),
            "Should not have ') today' at start of wrapped line. Found: '{}'",
            second_line
        );
    }
}

#[test]
fn emphasis_with_multiple_adjacent_punctuation() {
    // Test multiple punctuation characters adjacent to styled word
    // "Space exploration is fundamentally" = 34 chars
    // Then "))) more" - width limit is 37, so first ) fits but not all
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *fundamentally*))) more.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(37));
    let lines = line_texts(&rendered.lines);

    eprintln!("\n=== DEBUG emphasis_with_multiple_adjacent_punctuation ===");
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

    // First ) fits (34 + 1 = 35 < 37), so it gets extracted
    // But we should extract ALL adjacent punctuation, not just one
    assert_eq!(lines[0], "Space exploration is fundamentally)))");
    assert_eq!(lines[1], "more.");
}

#[test]
fn emphasis_preserves_style_during_backtracking() {
    // Critical test: verify that styled text preserves its styling when backtracked
    // "Space exploration is fundamentally_x" = 36 chars exactly
    // ") useful" doesn't fit, triggers backtracking
    // The word "fundamentally_x" MUST remain italic on the wrapped line
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Space exploration is *fundamentally_x*) useful.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(36));

    eprintln!("\n=== DEBUG emphasis_preserves_style_during_backtracking ===");
    for (i, line_obj) in rendered.lines.iter().enumerate() {
        let line_str: String = line_obj.to_string();
        eprintln!("Line {}: '{}'", i, line_str);
        for (j, span) in line_obj.spans.iter().enumerate() {
            eprintln!(
                "    Span {}: content='{}' style={:?}",
                j, span.content, span.style
            );
        }
    }
    eprintln!("=== END DEBUG ===\n");

    // Line 1 should have "fundamentally_x" with italic styling
    let line1_spans = &rendered.lines[1].spans;

    // Find the span containing "fundamentally_x"
    let fundamentally_span = line1_spans
        .iter()
        .find(|span| span.content.contains("fundamentally_x"))
        .expect("Should find 'fundamentally_x' on line 1");

    // Verify it has italic styling (ITALIC modifier should be set)
    assert!(
        fundamentally_span
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::ITALIC),
        "The word 'fundamentally_x' must preserve italic styling after backtracking. \
             Expected ITALIC modifier but got style: {:?}",
        fundamentally_span.style
    );
}

#[test]
fn emphasis_fills_entire_width_with_adjacent_punct() {
    // Critical edge case: styled word that fills entire width by itself,
    // followed by adjacent punctuation. This could cause infinite loop
    // in backtracking if not handled correctly.
    let theme = crate::ui::theme::Theme::dark_default();

    // Create a word that's exactly 36 chars
    let word = "a_very_long_italicized_word_here"; // 32 chars
    let message = Message {
        role: "assistant".into(),
        content: format!("*{}*) more.", word),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(32));
    let lines = line_texts(&rendered.lines);

    // If we get here without hanging, the bug is fixed
    // The word should appear somewhere in output
    let combined = lines.join("");
    assert!(
        combined.contains(word),
        "Should not hang and should preserve word. Output: {:?}",
        lines
    );
}
