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
fn wrapped_list_items_align_under_text() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: "- Parent item that wraps within the width budget and keeps alignment.\n  - Child item that wraps nicely under its parent alignment requirement.\n    - Grandchild entry that wraps and keeps deeper indentation consistent.".into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, true, Some(28));
    let lines = line_texts(&rendered.lines);

    let parent_idx = lines
        .iter()
        .position(|l| l.starts_with("- Parent item"))
        .expect("parent line present");
    let parent_continuation = &lines[parent_idx + 1];
    assert!(
        !parent_continuation.trim().is_empty()
            && !parent_continuation.trim_start().starts_with('-')
    );
    assert_eq!(
        parent_continuation
            .chars()
            .take_while(|c| c.is_whitespace())
            .count(),
        2
    );

    let child_idx = lines
        .iter()
        .position(|l| l.contains("Child item that wraps"))
        .expect("child line present");
    let child_continuation = &lines[child_idx + 1];
    assert!(
        !child_continuation.trim().is_empty() && !child_continuation.trim_start().starts_with('-')
    );
    assert_eq!(
        child_continuation
            .chars()
            .take_while(|c| c.is_whitespace())
            .count(),
        4
    );

    let grandchild_idx = lines
        .iter()
        .position(|l| l.contains("Grandchild entry"))
        .expect("grandchild line present");
    let grandchild_continuation = &lines[grandchild_idx + 1];
    assert!(
        !grandchild_continuation.trim().is_empty()
            && !grandchild_continuation.trim_start().starts_with('-')
    );
    assert_eq!(
        grandchild_continuation
            .chars()
            .take_while(|c| c.is_whitespace())
            .count(),
        6
    );
}

#[test]
fn gfm_callout_blockquotes_render_content() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "> [!NOTE]\n> Always document parser upgrades.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, true, None);
    let lines = line_texts(&rendered.lines);

    // Blockquote contains a paragraph, which adds a trailing blank line
    assert!(
        lines.len() >= 2,
        "expected callout blockquote to render with trailing spacing"
    );
    assert_line_text(&lines, 0, "Always document parser upgrades.");
    assert!(
        lines[1].is_empty(),
        "blockquote rendering should emit a separating blank line"
    );
}

#[test]
fn ordered_list_item_code_block_is_indented_under_marker() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "1. Intro text\n\n   ```\n   fn greet() {}\n   ```\n\n   Follow up text".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let bullet_line = rendered
        .lines
        .iter()
        .find(|line| line.to_string().contains("Intro text"))
        .expect("bullet line present");
    let bullet_marker_width = bullet_line
        .spans
        .first()
        .map(|span| span.content.as_ref().width())
        .expect("marker span present");

    let code_line = rendered
        .lines
        .iter()
        .find(|line| line.to_string().contains("fn greet() {}"))
        .expect("code block line present");
    assert_first_span_is_space_indented(code_line, bullet_marker_width);

    let follow_up_line = rendered
        .lines
        .iter()
        .find(|line| line.to_string().contains("Follow up text"))
        .expect("follow up text present");
    assert_first_span_is_space_indented(follow_up_line, bullet_marker_width);
}

#[test]
fn multi_item_ordered_list_keeps_code_block_with_correct_item() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: "1. **Open a new terminal** on your local machine (keeping your SSH session open) and run `scp` as above.\n2. **Use `scp` in reverse** from the remote side *to* your local machine (if remote can reach your local machine and SSH is accessible), e.g.:\n   ```bash\n   scp /path/to/file you@your_local_IP:/path/to/local/destination/\n   ```\n   But this only works if your local machine is running an SSH server and is network-reachable — rarely the case.\n3. **Use `rsync` over SSH** similarly to `scp`."
                .into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let bullet_two_index = rendered
        .lines
        .iter()
        .position(|line| line.to_string().starts_with("2. "))
        .expect("bullet two present");

    let bullet_two_indent = rendered.lines[bullet_two_index]
        .spans
        .first()
        .map(|span| span.content.as_ref().width())
        .expect("bullet two span");

    let code_block_index = rendered
        .lines
        .iter()
        .enumerate()
        .find_map(|(idx, line)| {
            if line.to_string().contains("scp /path/to/file") {
                Some(idx)
            } else {
                None
            }
        })
        .expect("code block line present");

    assert!(bullet_two_index < code_block_index);

    let code_line = &rendered.lines[code_block_index];
    assert_first_span_is_space_indented(code_line, bullet_two_indent);

    let follow_up_index = rendered
        .lines
        .iter()
        .position(|line| line.to_string().contains("But this only works"))
        .expect("follow up text present");
    assert!(code_block_index < follow_up_index);

    assert_first_span_is_space_indented(&rendered.lines[follow_up_index], bullet_two_indent);
}

#[test]
fn nested_bullet_lists_render_with_indentation() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "* Item 1\n    * Sub-item 1.1\n    * Sub-item 1.2\n        * Sub-sub-item 1.2.1"
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Verify that nested items have leading spaces
    // Line 0 should be "- Item 1" (no indent)
    // Line 1 should be "  - Sub-item 1.1" (2 space indent from parent "- " marker)
    // Line 2 should be "  - Sub-item 1.2" (2 space indent)
    // Line 3 should be "    - Sub-sub-item 1.2.1" (4 space indent: 2 from first level + 2 from second level)

    assert!(
        lines.len() >= 4,
        "Should have at least 4 lines, got {}",
        lines.len()
    );
    assert!(
        lines[0].starts_with("- "),
        "First item should start with '- ', got: '{}'",
        lines[0]
    );
    assert!(
        lines[1].starts_with("  - "),
        "Sub-item should have 2-space indent, got: '{}'",
        lines[1]
    );
    assert!(
        lines[2].starts_with("  - "),
        "Sub-item should have 2-space indent, got: '{}'",
        lines[2]
    );
    assert!(
        lines[3].starts_with("    - "),
        "Sub-sub-item should have 4-space indent, got: '{}'",
        lines[3]
    );
}

#[test]
fn nested_lists_dont_add_blank_lines_between_same_level_items() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content:
            "- Budget tree, branch one\n  - Emergency fund\n    - Sub-sticky note\n  - Groceries"
                .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the indices of key items
    let emergency_idx = lines.iter().position(|l| l.contains("Emergency")).unwrap();
    let sub_sticky_idx = lines.iter().position(|l| l.contains("Sub-sticky")).unwrap();
    let groceries_idx = lines.iter().position(|l| l.contains("Groceries")).unwrap();

    // After "Sub-sticky note" ends its nested list, "Groceries" should immediately follow
    // without any blank lines, since they're both at the same level (level 2)
    assert_eq!(
        groceries_idx,
        sub_sticky_idx + 1,
        "Groceries should come immediately after Sub-sticky note without blank lines. Lines: {:#?}",
        lines
    );

    // Verify the structure is correct
    assert!(
        emergency_idx < sub_sticky_idx,
        "Emergency should come before Sub-sticky"
    );
    assert!(
        sub_sticky_idx < groceries_idx,
        "Sub-sticky should come before Groceries"
    );
}

#[test]
fn list_with_source_blank_lines_preserves_spacing_between_top_level_items() {
    // When the markdown source has blank lines between top-level list items,
    // those should be preserved to provide visual breathing room
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: "- Strategic Foundations\n  - Long-Horizon Thinking\n    - Scenario Branches\n\n- Implementation Patterns\n  - Knowledge Architecture\n    - Modular repositories\n\n- Resilience\n  - Stressors".into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find indices
    let implementation_idx = lines
        .iter()
        .position(|l| l.contains("Implementation"))
        .unwrap();
    let resilience_idx = lines.iter().position(|l| l.contains("Resilience")).unwrap();

    // Check if there's a blank line between Strategic section and Implementation section
    let has_blank_before_implementation = lines[implementation_idx - 1].trim().is_empty();

    // Check if there's a blank line between Implementation section and Resilience section
    let has_blank_before_resilience = lines[resilience_idx - 1].trim().is_empty();

    assert!(
            has_blank_before_implementation,
            "Should have blank line before 'Implementation Patterns' (source has blank line). Lines: {:#?}",
            lines
        );
    assert!(
        has_blank_before_resilience,
        "Should have blank line before 'Resilience' (source has blank line). Lines: {:#?}",
        lines
    );
}

#[test]
fn list_without_source_blank_lines_has_no_spacing_between_top_level_items() {
    // When the markdown source has NO blank lines between top-level list items,
    // they should render consecutively without extra spacing
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "- First section\n  - Nested item\n- Second section\n  - Another nested".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find index
    let second_idx = lines
        .iter()
        .position(|l| l.contains("Second section"))
        .unwrap();

    // Second section should come relatively soon after First section ends
    // There should be no blank line between them since source has none
    // We need to account for the nested item, so check the line before Second section
    let line_before_second = &lines[second_idx - 1];

    assert!(
            !line_before_second.trim().is_empty(),
            "Should NOT have blank line before 'Second section' (source has no blank line). Line before: '{}'. All lines: {:#?}",
            line_before_second,
            lines
        );
}

#[test]
fn list_preceded_by_paragraph_has_blank_line_before() {
    // A list preceded by a paragraph should have a blank line separating them
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "Here is some introductory text.\n\n- First item\n- Second item".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let intro_idx = lines
        .iter()
        .position(|l| l.contains("introductory"))
        .unwrap();
    let first_item_idx = lines.iter().position(|l| l.contains("First item")).unwrap();

    // There should be a blank line between the paragraph and the list
    assert!(
        first_item_idx > intro_idx + 1,
        "Should have blank line between paragraph and list. Lines: {:#?}",
        lines
    );
    assert!(
        lines[intro_idx + 1].trim().is_empty(),
        "Line after paragraph should be blank. Lines: {:#?}",
        lines
    );
}

#[test]
fn list_followed_by_paragraph_has_blank_line_after() {
    // A list followed by a paragraph should have a blank line separating them
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "- First item\n- Second item\n\nThis is concluding text.".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let second_item_idx = lines
        .iter()
        .position(|l| l.contains("Second item"))
        .unwrap();
    let concluding_idx = lines.iter().position(|l| l.contains("concluding")).unwrap();

    // There should be a blank line between the list and the paragraph
    assert!(
        concluding_idx > second_item_idx + 1,
        "Should have blank line between list and paragraph. Lines: {:#?}",
        lines
    );
    assert!(
        lines[second_item_idx + 1].trim().is_empty(),
        "Line after list should be blank. Lines: {:#?}",
        lines
    );
}

#[test]
fn list_preceded_by_heading_has_blank_line_before() {
    // A list preceded by a heading should have a blank line separating them
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "## My Section\n\n- First item\n- Second item".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let heading_idx = lines.iter().position(|l| l.contains("My Section")).unwrap();
    let first_item_idx = lines.iter().position(|l| l.contains("First item")).unwrap();

    // There should be a blank line between the heading and the list
    assert!(
        first_item_idx > heading_idx + 1,
        "Should have blank line between heading and list. Lines: {:#?}",
        lines
    );
    assert!(
        lines[heading_idx + 1].trim().is_empty(),
        "Line after heading should be blank. Lines: {:#?}",
        lines
    );
}

#[test]
fn list_followed_by_heading_has_blank_line_after() {
    // A list followed by a heading should have a blank line separating them
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: "- First item\n- Second item\n\n## Next Section".into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let second_item_idx = lines
        .iter()
        .position(|l| l.contains("Second item"))
        .unwrap();
    let heading_idx = lines
        .iter()
        .position(|l| l.contains("Next Section"))
        .unwrap();

    // There should be a blank line between the list and the heading
    assert!(
        heading_idx > second_item_idx + 1,
        "Should have blank line between list and heading. Lines: {:#?}",
        lines
    );
    assert!(
        lines[second_item_idx + 1].trim().is_empty(),
        "Line after list should be blank. Lines: {:#?}",
        lines
    );
}

#[test]
fn complex_nested_lists_with_long_text_preserve_blank_lines() {
    // Test complex nested markdown with multiple levels, long wrapping text,
    // and blank lines at various nesting depths
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: r#"### Architecture Overview

1. **Primary Concept: The Architecture of a Modern Knowledge System**
   In designing a contemporary knowledge system, several foundational components must be conceptualized, integrated, and optimized for scalability. The architecture should balance information retrieval efficiency, semantic accuracy, and human-centered accessibility.
   Below is a structured decomposition of its design hierarchy:

   - **Layer One: Data Acquisition and Normalization**
     Collecting heterogeneous data streams across structured and unstructured sources forms the backbone of long-term informational reliability.
     Examples include web-scraped data, curated research papers, user-generated content, and transaction logs.

     - **Sub-layer A: Source Validation**
       - Ensure authenticity through cryptographic checksums.
       - Implement redundancy detection using fuzzy hashing.
       - Maintain timestamp precision to establish causal consistency.

     - **Sub-layer B: Normalization Pipeline**
       - Convert text encodings to UTF-8 for cross-platform compatibility.
       - Apply tokenization with semantic segmentation to retain linguistic intent.

   - **Layer Two: Knowledge Representation**
     Once normalized, data should be molded into adaptive knowledge graphs or relational mappings.
     These structures serve to bridge connections across domains, entities, and abstract relationships.

     - **Sub-layer A: Ontological Framework**
       - Define entities, attributes, and relations using logical formalisms.
       - Incorporate context-sensitive nodes for ambiguous linguistic references.
"#.into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, false, Some(80));
    let lines = line_texts(&rendered.lines);

    // Debug: print all lines
    eprintln!("\n=== RENDERED OUTPUT ===");
    for (i, line) in lines.iter().enumerate() {
        eprintln!("{:3}: '{}'", i, line);
    }
    eprintln!("=== END OUTPUT ===\n");

    // Find key elements
    let heading_idx = lines
        .iter()
        .position(|l| l.contains("Architecture Overview"))
        .unwrap();
    let primary_idx = lines
        .iter()
        .position(|l| l.contains("Primary Concept"))
        .unwrap();
    let layer_one_idx = lines.iter().position(|l| l.contains("Layer One")).unwrap();
    let sublayer_a_idx = lines
        .iter()
        .position(|l| l.contains("Sub-layer A"))
        .unwrap();
    let ensure_auth_idx = lines
        .iter()
        .position(|l| l.contains("Ensure authenticity"))
        .unwrap();
    let implement_red_idx = lines
        .iter()
        .position(|l| l.contains("Implement redundancy"))
        .unwrap();
    let sublayer_b_idx = lines
        .iter()
        .position(|l| l.contains("Sub-layer B"))
        .unwrap();
    let layer_two_idx = lines.iter().position(|l| l.contains("Layer Two")).unwrap();

    // === EXPECTATION 1: Blank line after heading ===
    assert!(
        lines[heading_idx + 1].trim().is_empty(),
        "Should have blank line after heading. Line {}: '{}'",
        heading_idx + 1,
        lines[heading_idx + 1]
    );

    // === EXPECTATION 2: Ordered list item starts after blank line ===
    assert_eq!(
        heading_idx + 2,
        primary_idx,
        "Ordered list should start right after blank line following heading"
    );

    // === EXPECTATION 3: Blank line before Layer One (nested bullet in ordered list) ===
    // Source has blank line after "Below is a structured decomposition..." and before Layer One
    let line_before_layer_one = &lines[layer_one_idx - 1];
    assert!(
        line_before_layer_one.trim().is_empty(),
        "Should have blank line before 'Layer One' (source has blank line). Line {}: '{}'",
        layer_one_idx - 1,
        line_before_layer_one
    );

    // === EXPECTATION 4: Blank line before Sub-layer A ===
    // Source has blank line after "Examples include web-scraped data..." and before Sub-layer A
    let line_before_sublayer_a = &lines[sublayer_a_idx - 1];
    assert!(
        line_before_sublayer_a.trim().is_empty(),
        "Should have blank line before 'Sub-layer A' (source has blank line). Line {}: '{}'",
        sublayer_a_idx - 1,
        line_before_sublayer_a
    );

    // === EXPECTATION 5: No blank lines within Sub-layer A's nested items ===
    // The three items under Sub-layer A should be consecutive (no blank lines in source)
    let line_after_ensure = &lines[ensure_auth_idx + 1];
    assert!(
            !line_after_ensure.trim().is_empty(),
            "Should NOT have blank line after 'Ensure authenticity' (no blank line in source). Line {}: '{}'",
            ensure_auth_idx + 1,
            line_after_ensure
        );

    let line_after_implement = &lines[implement_red_idx + 1];
    assert!(
            !line_after_implement.trim().is_empty(),
            "Should NOT have blank line after 'Implement redundancy' (no blank line in source). Line {}: '{}'",
            implement_red_idx + 1,
            line_after_implement
        );

    // === EXPECTATION 6: Blank line before Sub-layer B ===
    // Source has blank line after last item of Sub-layer A and before Sub-layer B
    let line_before_sublayer_b = &lines[sublayer_b_idx - 1];
    assert!(
        line_before_sublayer_b.trim().is_empty(),
        "Should have blank line before 'Sub-layer B' (source has blank line). Line {}: '{}'",
        sublayer_b_idx - 1,
        line_before_sublayer_b
    );

    // === EXPECTATION 7: Blank line before Layer Two ===
    // Source has blank line after last item of Sub-layer B and before Layer Two
    let line_before_layer_two = &lines[layer_two_idx - 1];
    assert!(
        line_before_layer_two.trim().is_empty(),
        "Should have blank line before 'Layer Two' (source has blank line). Line {}: '{}'",
        layer_two_idx - 1,
        line_before_layer_two
    );
}

#[test]
fn blank_line_before_paragraph_doesnt_cause_blank_before_later_list_item() {
    // Regression test: prev_was_blank persisting across paragraph text would cause
    // a later list item to incorrectly get a blank line before it.
    // Scenario: item, paragraph, blank, paragraph, item - the second item should NOT
    // get a blank line because there's no blank immediately before it.
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"- First item
Paragraph text after first item.

More paragraph text.
- Second item (should have NO blank before it)"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the items
    let first_idx = lines.iter().position(|l| l.contains("First item")).unwrap();
    let second_idx = lines
        .iter()
        .position(|l| l.contains("Second item"))
        .unwrap();

    // The second item should NOT have a blank line before it from our preprocessing
    // (it may have one from TagEnd::Paragraph, but that's a separate rendering decision)
    // What we're testing is that the blank line before "More paragraph text" doesn't
    // cause our preprocessing to mark the second item as needing a blank line.

    // If the bug exists, find_items_needing_blank_lines would return {1} (second item)
    // With the fix, it should return {} (no items need blank lines from preprocessing)
    // We can't directly test the set, but we can verify the item doesn't get EXTRA spacing

    // Actually, let me verify by checking there's only one blank line before second item
    // (from TagEnd::Paragraph), not two (from both TagEnd::Paragraph and our preprocessing)
    let mut blank_count = 0;
    for line in lines.iter().take(second_idx).skip(first_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have at most 2 blank lines (one after "Paragraph text" paragraph end,
    // one for the explicit blank line in source before "More paragraph text")
    // If our preprocessing bug existed, there'd be an additional blank before the second item
    assert!(
            blank_count <= 2,
            "Should have at most 2 blank lines between items (not from preprocessing bug). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}

#[test]
fn lists_with_numeric_text_before_them_dont_shift_indices() {
    // Regression test: lines starting with digits like "2024 roadmap" should not be
    // counted as list items, which would shift indices and cause blank lines
    // to appear at wrong positions
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"2024 roadmap includes several initiatives.

1. First initiative
2. Second initiative

3. Third initiative (after blank line)"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the items
    let first_idx = lines
        .iter()
        .position(|l| l.contains("First initiative"))
        .unwrap();
    let second_idx = lines
        .iter()
        .position(|l| l.contains("Second initiative"))
        .unwrap();
    let third_idx = lines
        .iter()
        .position(|l| l.contains("Third initiative"))
        .unwrap();

    // No blank line between first and second (they're consecutive in source)
    assert_eq!(
        second_idx,
        first_idx + 1,
        "Second item should immediately follow first. Lines: {:#?}",
        lines
    );

    // Blank line before third (source has blank line)
    let line_before_third = &lines[third_idx - 1];
    assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );
}

#[test]
fn lists_with_plus_markers_preserve_blank_lines() {
    // Regression test: + markers should be recognized as list items and preserve
    // blank lines from source, just like - and * markers
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"+ First item
+ Second item

+ Third item (after blank line)
+ Fourth item"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the items
    let first_idx = lines.iter().position(|l| l.contains("First item")).unwrap();
    let second_idx = lines
        .iter()
        .position(|l| l.contains("Second item"))
        .unwrap();
    let third_idx = lines.iter().position(|l| l.contains("Third item")).unwrap();
    let fourth_idx = lines
        .iter()
        .position(|l| l.contains("Fourth item"))
        .unwrap();

    // No blank line between first and second (consecutive in source)
    assert_eq!(
        second_idx,
        first_idx + 1,
        "Second item should immediately follow first. Lines: {:#?}",
        lines
    );

    // Blank line before third (source has blank line)
    let line_before_third = &lines[third_idx - 1];
    assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );

    // No blank line between third and fourth (consecutive in source)
    assert_eq!(
        fourth_idx,
        third_idx + 1,
        "Fourth item should immediately follow third. Lines: {:#?}",
        lines
    );
}

#[test]
fn code_blocks_dont_shift_list_item_indices() {
    // Regression test: lines inside fenced code blocks that look like list items
    // should not increment item_index, which would shift indices and cause
    // blank lines to appear at wrong positions
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"Example code:
```
- not a real item
- also not real
```

- First real item
- Second real item

- Third real item (should have blank before)"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the real items (not the ones in the code block)
    let first_idx = lines
        .iter()
        .position(|l| l.contains("First real item"))
        .unwrap();
    let second_idx = lines
        .iter()
        .position(|l| l.contains("Second real item"))
        .unwrap();
    let third_idx = lines
        .iter()
        .position(|l| l.contains("Third real item"))
        .unwrap();

    // No blank line between first and second (consecutive in source)
    assert_eq!(
        second_idx,
        first_idx + 1,
        "Second item should immediately follow first. Lines: {:#?}",
        lines
    );

    // Blank line before third (source has blank line)
    let line_before_third = &lines[third_idx - 1];
    assert!(
            line_before_third.trim().is_empty(),
            "Should have blank line before third item (source has blank line). Line {}: '{}'. Lines: {:#?}",
            third_idx - 1,
            line_before_third,
            lines
        );
}

#[test]
fn list_items_with_multiple_paragraphs_preserve_blank_lines() {
    // Regression test: blank lines between paragraphs within a single list item
    // should be preserved, not suppressed by the "skip blank after paragraph in list" logic
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"- First paragraph in item

  Second paragraph in same item

- Next item"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the paragraphs
    let first_para_idx = lines
        .iter()
        .position(|l| l.contains("First paragraph"))
        .unwrap();
    let second_para_idx = lines
        .iter()
        .position(|l| l.contains("Second paragraph"))
        .unwrap();
    let next_item_idx = lines.iter().position(|l| l.contains("Next item")).unwrap();

    // Should have a blank line between the two paragraphs within the same item
    assert!(
            second_para_idx > first_para_idx + 1,
            "Second paragraph should have blank line before it. First at {}, Second at {}. Lines: {:#?}",
            first_para_idx,
            second_para_idx,
            lines
        );

    // Verify there's actually a blank line
    let line_between = &lines[first_para_idx + 1];
    assert!(
        line_between.trim().is_empty(),
        "Should have blank line between paragraphs in same item. Line {}: '{}'. Lines: {:#?}",
        first_para_idx + 1,
        line_between,
        lines
    );

    // Should also have blank line before next item
    let line_before_next = &lines[next_item_idx - 1];
    assert!(
        line_before_next.trim().is_empty(),
        "Should have blank line before next item. Line {}: '{}'. Lines: {:#?}",
        next_item_idx - 1,
        line_before_next,
        lines
    );
}

#[test]
fn list_items_preserve_blank_lines_before_all_block_elements() {
    // Regression test: blank lines before code blocks, nested lists, and blockquotes
    // within list items should be preserved, not just blank lines before paragraphs
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"- Introduction paragraph

  ```python
  code_example()
  ```

- Main point about something

  - Nested item one
  - Nested item two

- Context paragraph

  > Important quote here"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Test 1: Blank line before code block
    let intro_idx = lines
        .iter()
        .position(|l| l.contains("Introduction"))
        .unwrap();
    let code_idx = lines
        .iter()
        .position(|l| l.contains("code_example"))
        .unwrap();
    assert!(
        code_idx > intro_idx + 1,
        "Code block should have blank line before it. Intro at {}, Code at {}. Lines: {:#?}",
        intro_idx,
        code_idx,
        lines
    );

    // Test 2: Blank line before nested list
    let main_point_idx = lines.iter().position(|l| l.contains("Main point")).unwrap();
    let nested_one_idx = lines
        .iter()
        .position(|l| l.contains("Nested item one"))
        .unwrap();
    assert!(
        nested_one_idx > main_point_idx + 1,
        "Nested list should have blank line before it. Main at {}, Nested at {}. Lines: {:#?}",
        main_point_idx,
        nested_one_idx,
        lines
    );

    // Test 3: Blank line before blockquote
    let context_idx = lines
        .iter()
        .position(|l| l.contains("Context paragraph"))
        .unwrap();
    let quote_idx = lines
        .iter()
        .position(|l| l.contains("Important quote"))
        .unwrap();
    assert!(
        quote_idx > context_idx + 1,
        "Blockquote should have blank line before it. Context at {}, Quote at {}. Lines: {:#?}",
        context_idx,
        quote_idx,
        lines
    );
}

#[test]
fn list_paragraphs_keep_indent_after_blank_lines() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: r#"- **Primary Concept**
  In designing a contemporary knowledge system, several foundational components must be conceptualized.

  Once normalized, data should be molded into adaptive knowledge graphs or relational mappings.
- Next item"#
                .into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let detail_idx = lines
        .iter()
        .position(|l| l.contains("In designing a contemporary"))
        .unwrap();
    let followup_idx = lines
        .iter()
        .position(|l| l.contains("Once normalized"))
        .unwrap();

    assert!(
        lines[detail_idx].starts_with("  In designing"),
        "Detail paragraph should be indented under the list marker. Line: '{}'",
        lines[detail_idx]
    );
    assert!(
        lines[followup_idx].starts_with("  Once normalized"),
        "Follow-up paragraph should reuse list indent after blank line. Line: '{}'",
        lines[followup_idx]
    );
}

#[test]
fn list_paragraphs_with_soft_breaks_keep_indent() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
            role: "assistant".into(),
            content: r#"- **Primary Concept: The Architecture of a Modern Knowledge System**
  In designing a contemporary knowledge system, several foundational components must be conceptualized, integrated, and optimized for scalability.
  The architecture should balance **information retrieval efficiency**, **semantic accuracy**, and **human-centered accessibility**.
  Below is a structured decomposition of its design hierarchy:"#
                .into(),
        };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let designing_idx = lines
        .iter()
        .position(|l| l.contains("In designing a contemporary"))
        .unwrap();
    let architecture_idx = lines
        .iter()
        .position(|l| l.contains("The architecture should balance"))
        .unwrap();
    let below_idx = lines
        .iter()
        .position(|l| l.contains("Below is a structured decomposition"))
        .unwrap();

    assert!(
        lines[designing_idx].starts_with("  In designing"),
        "First soft-wrapped paragraph line should be indented under marker. Line: '{}'",
        lines[designing_idx]
    );
    assert!(
        lines[architecture_idx].starts_with("  The architecture"),
        "Second soft-wrapped paragraph line should keep list indent. Line: '{}'",
        lines[architecture_idx]
    );
    assert!(
        lines[below_idx].starts_with("  Below is"),
        "Third soft-wrapped paragraph line should keep list indent. Line: '{}'",
        lines[below_idx]
    );
}

#[test]
fn nested_lists_with_single_blank_line_dont_double_space() {
    // Regression test: when a list item contains a nested list with a single blank
    // line before it, we should render ONE blank line, not two (one from TagEnd::Paragraph
    // peeking ahead and seeing Tag::List, and another from Tag::Item preprocessing)
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"- parent

  - child one
  - child two"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let parent_idx = lines.iter().position(|l| l.contains("parent")).unwrap();
    let child_idx = lines.iter().position(|l| l.contains("child one")).unwrap();

    // Count blank lines between parent and child
    let mut blank_count = 0;
    for line in lines.iter().take(child_idx).skip(parent_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have exactly 1 blank line, not 2
    assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between parent and nested child (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}

#[test]
fn blockquote_followed_by_list_has_single_blank_line() {
    // Regression test: when a blockquote is followed by a list with a single blank
    // line between them, we should render ONE blank line, not two (one from TagEnd::Paragraph
    // inside the blockquote, and another from TagEnd::BlockQuote)
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"> "Relax," it squeals, "we're diversified in hope and overdue library fines."

- **Merit:** it funds the dream of four walls and a window box."#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let quote_idx = lines.iter().position(|l| l.contains("Relax")).unwrap();
    let list_idx = lines.iter().position(|l| l.contains("Merit")).unwrap();

    // Count blank lines between blockquote and list
    let mut blank_count = 0;
    for line in lines.iter().take(list_idx).skip(quote_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have exactly 1 blank line, not 2
    assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and list (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}

#[test]
fn blockquote_followed_by_paragraph_has_single_blank_line() {
    // Similar issue: blockquote followed by a paragraph should preserve
    // the single blank line from the source, not double it
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"> Important quote here.

This is a paragraph after the quote."#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let quote_idx = lines
        .iter()
        .position(|l| l.contains("Important quote"))
        .unwrap();
    let para_idx = lines
        .iter()
        .position(|l| l.contains("This is a paragraph"))
        .unwrap();

    // Count blank lines between blockquote and paragraph
    let mut blank_count = 0;
    for line in lines.iter().take(para_idx).skip(quote_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have exactly 1 blank line, not 2
    assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and paragraph (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}

#[test]
fn blockquote_followed_by_heading_has_single_blank_line() {
    // Same issue with headings after blockquotes
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"> Important quote here.

## Next Section"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    let quote_idx = lines
        .iter()
        .position(|l| l.contains("Important quote"))
        .unwrap();
    let heading_idx = lines
        .iter()
        .position(|l| l.contains("Next Section"))
        .unwrap();

    // Count blank lines between blockquote and heading
    let mut blank_count = 0;
    for line in lines.iter().take(heading_idx).skip(quote_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have exactly 1 blank line, not 2
    assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote and heading (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}

#[test]
fn blockquote_with_code_block_followed_by_paragraph() {
    // Test what happens when a blockquote contains a code block
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: "assistant".into(),
        content: r#"> ```python
> code_here()
> ```

Next paragraph"#
            .into(),
    };

    let rendered = render_markdown_for_test(&message, &theme, false, None);
    let lines = line_texts(&rendered.lines);

    // Find the code block and next paragraph
    let code_idx = lines.iter().position(|l| l.contains("code_here")).unwrap();
    let para_idx = lines
        .iter()
        .position(|l| l.contains("Next paragraph"))
        .unwrap();

    // Count blank lines between code block and paragraph
    let mut blank_count = 0;
    for line in lines.iter().take(para_idx).skip(code_idx + 1) {
        if line.trim().is_empty() {
            blank_count += 1;
        }
    }

    // Should have exactly 1 blank line, not 2
    assert_eq!(
            blank_count, 1,
            "Should have exactly 1 blank line between blockquote code block and next paragraph (source has 1). Found {}. Lines: {:#?}",
            blank_count,
            lines
        );
}
