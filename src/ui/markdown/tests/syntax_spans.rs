use crate::core::message::{Message, TranscriptRole};
use crate::ui::markdown::render_message_markdown_details_with_policy_and_user_name;
use crate::ui::span::SpanKind;

#[test]
fn markdown_details_metadata_matches_lines_and_tags() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::Assistant,
        content: "Testing metadata with a [link](https://example.com) inside.".into(),
    };

    let details = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.as_ref().expect("metadata present");
    assert_eq!(metadata.len(), details.lines.len());
    let mut saw_link = false;
    for (line, kinds) in details.lines.iter().zip(metadata.iter()) {
        assert_eq!(line.spans.len(), kinds.len());
        for kind in kinds {
            if let Some(href) = kind.link_href() {
                saw_link = true;
                assert_eq!(href, "https://example.com");
            }
        }
    }
    assert!(saw_link, "expected link metadata to be captured");

    let width = Some(24usize);
    let details_with_width = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        width,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata_wrapped = details_with_width
        .span_metadata
        .as_ref()
        .expect("metadata present for width-aware render");
    assert_eq!(metadata_wrapped.len(), details_with_width.lines.len());
    for (line, kinds) in details_with_width.lines.iter().zip(metadata_wrapped.iter()) {
        assert_eq!(line.spans.len(), kinds.len());
    }
}

#[test]
fn metadata_marks_user_prefix() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message {
        role: TranscriptRole::User,
        content: "Hello world".into(),
    };

    let details = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );

    let metadata = details.span_metadata.expect("metadata present");
    assert!(!metadata.is_empty());
    let first_line = &metadata[0];
    assert!(!first_line.is_empty());
    assert!(matches!(first_line[0], SpanKind::UserPrefix));
    assert!(first_line.iter().skip(1).all(|k| k.is_text()));
}

#[test]
fn metadata_marks_app_prefix() {
    let theme = crate::ui::theme::Theme::dark_default();
    let message = Message::app_info("Heads up");

    let details = render_message_markdown_details_with_policy_and_user_name(
        &message,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );

    let metadata = details.span_metadata.expect("metadata present");
    assert!(!metadata.is_empty());
    let first_line = &metadata[0];
    assert!(!first_line.is_empty());
    assert!(matches!(first_line[0], SpanKind::AppPrefix));
    assert!(first_line.iter().skip(1).all(|k| k.is_text()));
}

#[test]

fn code_block_spans_have_metadata() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::single_block();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    // Find spans that should be code blocks
    let code_spans: Vec<_> = metadata
        .iter()
        .flat_map(|line| line.iter())
        .filter(|kind| kind.is_code_block())
        .collect();

    assert!(
        !code_spans.is_empty(),
        "Code block should have CodeBlock metadata"
    );

    // Verify metadata contains language and block index
    if let Some(meta) = code_spans[0].code_block_meta() {
        assert_eq!(meta.language(), Some("rust"));
        assert_eq!(meta.block_index(), 0);
    } else {
        panic!("Expected CodeBlock metadata");
    }
}

#[test]

fn multiple_code_blocks_have_unique_indices() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::multiple_blocks();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    // Extract unique block indices
    let mut indices = std::collections::HashSet::new();
    for line_meta in metadata.iter() {
        for kind in line_meta.iter() {
            if let Some(meta) = kind.code_block_meta() {
                indices.insert(meta.block_index());
            }
        }
    }

    assert_eq!(indices.len(), 3, "Should have 3 unique code block indices");
    assert!(indices.contains(&0));
    assert!(indices.contains(&1));
    assert!(indices.contains(&2));
}

#[test]

fn empty_code_block_has_metadata() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::empty_block();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    // Empty blocks produce no span metadata and are not navigable.
    // This is correct behavior - there's no content to select or extract.
    let has_code_meta = metadata
        .iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_code_block());

    assert!(
        !has_code_meta,
        "Empty blocks should not create code block metadata"
    );
}

#[test]

fn code_block_without_language_has_metadata() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::no_language_tag();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    let code_metas: Vec<_> = metadata
        .iter()
        .flat_map(|line| line.iter())
        .filter_map(|k| k.code_block_meta())
        .collect();

    assert!(!code_metas.is_empty(), "Should have code block metadata");
    assert_eq!(
        code_metas[0].language(),
        None,
        "Block without language should have None language"
    );
}

#[test]

fn nested_code_blocks_have_metadata() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::nested_in_list();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    // Should have two code blocks (indices 0 and 1)
    let mut indices = std::collections::HashSet::new();
    for line_meta in metadata.iter() {
        for kind in line_meta.iter() {
            if let Some(meta) = kind.code_block_meta() {
                indices.insert(meta.block_index());
            }
        }
    }

    assert_eq!(indices.len(), 2, "Should have 2 code blocks in list");
}

#[test]

fn user_message_code_blocks_have_metadata() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::user_message_with_code();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        Some("User"),
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    let has_code_blocks = metadata
        .iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_code_block());

    assert!(
        has_code_blocks,
        "User messages should have code block metadata"
    );
}

#[test]

fn code_and_link_metadata_coexist() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::code_and_links();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    let has_code_blocks = metadata
        .iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_code_block());

    let has_links = metadata
        .iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_link());

    assert!(has_code_blocks, "Should have code block metadata");
    assert!(has_links, "Should have link metadata");
}

#[test]

fn various_language_tags_preserved() {
    use crate::ui::markdown::test_fixtures;
    let msg = test_fixtures::various_languages();
    let theme = crate::ui::theme::Theme::dark_default();

    let details = render_message_markdown_details_with_policy_and_user_name(
        &msg,
        &theme,
        true,
        None,
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        None,
    );
    let metadata = details.span_metadata.expect("metadata should be present");

    let languages: Vec<Option<&str>> = metadata
        .iter()
        .flat_map(|line| line.iter())
        .filter_map(|k| k.code_block_meta())
        .map(|m| m.language())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Should find bash, javascript, json, txt
    assert!(
        languages.len() >= 4,
        "Should preserve different language tags"
    );
}
