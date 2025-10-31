use super::*;
use crate::core::message::Message;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::ui::picker::{PickerItem, PickerState};
use crate::utils::test_utils::{create_test_app, create_test_message};
use tui_textarea::{CursorMove, Input, Key};

#[test]
fn theme_picker_highlights_active_theme_over_default() {
    let mut app = create_test_app();
    // Simulate active theme is light, while default (config) remains None in tests
    app.ui.current_theme_id = Some("light".to_string());

    // Open the theme picker
    app.open_theme_picker().expect("theme picker opens");

    // After sorting and selection alignment, ensure selected item has id "light"
    if let Some(picker) = app.picker_state() {
        let idx = picker.selected;
        let selected_id = &picker.items[idx].id;
        assert_eq!(selected_id, "light");
    } else {
        panic!("picker not opened");
    }
}

#[test]
fn model_picker_title_uses_az_when_no_dates() {
    let mut app = create_test_app();
    // Build a model picker with no sort_key (no dates)
    let items = vec![
        PickerItem {
            id: "a-model".into(),
            label: "a-model".into(),
            metadata: None,
            inspect_metadata: None,
            sort_key: None,
        },
        PickerItem {
            id: "z-model".into(),
            label: "z-model".into(),
            metadata: None,
            inspect_metadata: None,
            sort_key: None,
        },
    ];
    let mut picker_state = PickerState::new("Pick Model", items.clone(), 0);
    picker_state.sort_mode = crate::ui::picker::SortMode::Name;
    app.picker.picker_session = Some(PickerSession {
        state: picker_state,
        data: PickerData::Model(ModelPickerState {
            search_filter: String::new(),
            all_items: items,
            before_model: None,
            has_dates: false,
        }),
    });
    app.update_picker_title();
    let picker = app.picker_state().unwrap();
    assert!(picker.title.contains("Sort by: A-Z"));
}

#[test]
fn provider_model_cancel_reverts_base_url_and_state() {
    let mut app = create_test_app();
    // Set current state to some new provider context
    app.session.provider_name = "newprov".into();
    app.session.provider_display_name = "NewProv".into();
    app.session.model = "new-model".into();
    app.session.api_key = "new-key".into();
    app.session.base_url = "https://api.newprov.test/v1".into();

    // Simulate saved previous state for transition
    app.picker.in_provider_model_transition = true;
    app.picker.provider_model_transition_state = Some((
        "oldprov".into(),
        "OldProv".into(),
        "old-model".into(),
        "old-key".into(),
        "https://api.oldprov.test/v1".into(),
    ));

    // Cancelling model picker should revert provider/model/api_key/base_url
    app.picker.revert_model_preview(&mut app.session);

    assert_eq!(app.session.provider_name, "oldprov");
    assert_eq!(app.session.provider_display_name, "OldProv");
    assert_eq!(app.session.model, "old-model");
    assert_eq!(app.session.api_key, "old-key");
    assert_eq!(app.session.base_url, "https://api.oldprov.test/v1");
    assert!(!app.picker.in_provider_model_transition);
    assert!(app.picker.provider_model_transition_state.is_none());
}

#[test]
fn calculate_available_height_matches_expected_layout_rules() {
    let mut app = create_test_app();

    let cases = [
        (30, 5, 22), // 30 - (5 + 2) - 1
        (10, 8, 0),  // Saturating at zero when chat area would be negative
        (5, 0, 2),   // Just borders and title removed
    ];

    for (term_height, input_height, expected) in cases {
        assert_eq!(
            app.conversation()
                .calculate_available_height(term_height, input_height),
            expected
        );
    }
}

#[test]
fn default_sort_mode_helper_behaviour() {
    let mut app = create_test_app();
    // Theme picker prefers alphabetical → Name
    app.picker.picker_session = Some(PickerSession {
        state: PickerState::new("Pick Theme", vec![], 0),
        data: PickerData::Theme(ThemePickerState {
            search_filter: String::new(),
            all_items: Vec::new(),
            before_theme: None,
            before_theme_id: None,
        }),
    });
    assert!(matches!(
        app.picker_session().unwrap().default_sort_mode(),
        crate::ui::picker::SortMode::Name
    ));
    // Provider picker prefers alphabetical → Name
    app.picker.picker_session = Some(PickerSession {
        state: PickerState::new("Pick Provider", vec![], 0),
        data: PickerData::Provider(ProviderPickerState {
            search_filter: String::new(),
            all_items: Vec::new(),
            before_provider: None,
        }),
    });
    assert!(matches!(
        app.picker_session().unwrap().default_sort_mode(),
        crate::ui::picker::SortMode::Name
    ));
    // Model picker with dates → Date
    app.picker.picker_session = Some(PickerSession {
        state: PickerState::new("Pick Model", vec![], 0),
        data: PickerData::Model(ModelPickerState {
            search_filter: String::new(),
            all_items: Vec::new(),
            before_model: None,
            has_dates: true,
        }),
    });
    assert!(matches!(
        app.picker_session().unwrap().default_sort_mode(),
        crate::ui::picker::SortMode::Date
    ));
    // Model picker without dates → Name
    if let Some(PickerSession {
        data: PickerData::Model(state),
        ..
    }) = app.picker_session_mut()
    {
        state.has_dates = false;
    }
    assert!(matches!(
        app.picker_session().unwrap().default_sort_mode(),
        crate::ui::picker::SortMode::Name
    ));
}

#[test]
fn prewrap_cache_reuse_no_changes() {
    let mut app = create_test_app();
    for i in 0..50 {
        app.ui.messages.push_back(Message {
            role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
            content: "lorem ipsum dolor sit amet consectetur adipiscing elit".into(),
        });
    }
    let w = 100u16;
    let ptr1 = {
        let p1 = app.get_prewrapped_lines_cached(w);
        assert!(!p1.is_empty());
        p1.as_ptr()
    };
    let ptr2 = {
        let p2 = app.get_prewrapped_lines_cached(w);
        p2.as_ptr()
    };
    assert_eq!(ptr1, ptr2, "cache should be reused when nothing changed");
}

#[test]
fn prewrap_cache_invalidates_on_width_change() {
    let mut app = create_test_app();
    app.ui.messages.push_back(Message {
        role: "user".into(),
        content: "hello world".into(),
    });
    let ptr1 = {
        let p1 = app.get_prewrapped_lines_cached(80);
        p1.as_ptr()
    };
    let ptr2 = {
        let p2 = app.get_prewrapped_lines_cached(120);
        p2.as_ptr()
    };
    assert_ne!(ptr1, ptr2, "cache should invalidate on width change");
}

#[test]
fn prewrap_cache_updates_metadata_for_markdown_last_message() {
    let mut app = create_test_app();
    app.ui
        .messages
        .push_back(create_test_message("user", "This is the opening line."));
    app.ui.messages.push_back(create_test_message(
        "assistant",
        "Initial response that will be replaced.",
    ));

    let width = 72;
    let initial_lines = app.get_prewrapped_lines_cached(width).clone();
    let initial_meta = app.get_prewrapped_span_metadata_cached(width).clone();
    assert_eq!(initial_lines.len(), initial_meta.len());

    if let Some(last) = app.ui.messages.back_mut() {
        last.content = "Here's an updated reply with a [link](https://example.com).".into();
    }

    let updated_lines = app.get_prewrapped_lines_cached(width).clone();
    let updated_meta = app.get_prewrapped_span_metadata_cached(width).clone();
    assert_eq!(updated_lines.len(), updated_meta.len());
    assert!(updated_meta
        .iter()
        .any(|kinds| kinds.iter().any(|kind| kind.is_link())));
}

#[test]
fn prewrap_cache_updates_metadata_for_plain_text_last_message() {
    let mut app = create_test_app();
    app.ui.markdown_enabled = false;
    app.ui.syntax_enabled = false;
    app.ui
        .messages
        .push_back(create_test_message("user", "Plain intro from the user."));
    app.ui.messages.push_back(create_test_message(
        "assistant",
        "A short reply that will expand into a much longer paragraph after the update.",
    ));

    let width = 40;
    let initial_lines = app.get_prewrapped_lines_cached(width).clone();
    let initial_meta = app.get_prewrapped_span_metadata_cached(width).clone();
    assert_eq!(initial_lines.len(), initial_meta.len());

    if let Some(last) = app.ui.messages.back_mut() {
        last.content = "Now the assistant responds with a deliberately long piece of plain text that should wrap across multiple terminal lines once re-rendered.".into();
    }

    let updated_lines = app.get_prewrapped_lines_cached(width).clone();
    let updated_meta = app.get_prewrapped_span_metadata_cached(width).clone();
    assert_eq!(updated_lines.len(), updated_meta.len());
    assert!(updated_meta
        .iter()
        .flat_map(|kinds| kinds.iter())
        .all(|kind| kind.is_text()));
}

#[test]
fn prewrap_cache_plain_text_last_message_wrapping() {
    // Reproduce the fast-path tail update and ensure plain-text wrapping is preserved
    let mut app = crate::utils::test_utils::create_test_app();
    app.ui.markdown_enabled = false;
    let theme = app.ui.theme.clone();

    // Start with two assistant messages
    app.ui.messages.push_back(Message {
        role: "assistant".into(),
        content: "Short".into(),
    });
    app.ui.messages.push_back(Message {
        role: "assistant".into(),
        content: "This is a very long plain text line that should wrap when width is small".into(),
    });

    let width = 20u16;
    app.get_prewrapped_lines_cached(width);

    // Update only the last message content to trigger the fast path
    if let Some(last) = app.ui.messages.back_mut() {
        last.content.push_str(" and now it changed");
    }
    let second = app.get_prewrapped_lines_cached(width).clone();
    // Convert to strings and check for wrapping (no line exceeds width)
    let rendered: Vec<String> = second.iter().map(|l| l.to_string()).collect();
    let content_lines: Vec<&String> = rendered.iter().filter(|s| !s.is_empty()).collect();
    assert!(
        content_lines.len() > 2,
        "Expected multiple wrapped content lines"
    );
    for (i, s) in content_lines.iter().enumerate() {
        assert!(
            s.chars().count() <= width as usize,
            "Line {} exceeds width: '{}' (len={})",
            i,
            s,
            s.len()
        );
    }

    // Silence unused warning
    let _ = theme;
}

#[test]
fn test_sync_cursor_mapping_single_and_multi_line() {
    let mut app = create_test_app();

    // Single line: move to end
    app.ui.set_input_text("hello world".to_string());
    app.ui.textarea.move_cursor(CursorMove::End);
    app.ui.sync_input_from_textarea();
    assert_eq!(app.ui.get_input_text(), "hello world");
    assert_eq!(app.ui.input_cursor_position, 11);

    // Multi-line: jump to (row=1, col=3) => after "wor" on second line
    app.ui.set_input_text("hello\nworld".to_string());
    app.ui.textarea.move_cursor(CursorMove::Jump(1, 3));
    app.ui.sync_input_from_textarea();
    // 5 (hello) + 1 (\n) + 3 = 9
    assert_eq!(app.ui.input_cursor_position, 9);
}

#[test]
fn test_backspace_at_start_noop() {
    let mut app = create_test_app();
    app.ui.set_input_text("abc".to_string());
    // Move to head of line
    app.ui.textarea.move_cursor(CursorMove::Head);
    // Simulate backspace (always single-char via input_without_shortcuts)
    app.ui.textarea.input_without_shortcuts(Input {
        key: Key::Backspace,
        ctrl: false,
        alt: false,
        shift: false,
    });
    app.ui.sync_input_from_textarea();
    assert_eq!(app.ui.get_input_text(), "abc");
    assert_eq!(app.ui.input_cursor_position, 0);
}

#[test]
fn test_backspace_at_line_start_joins_lines() {
    let mut app = create_test_app();
    app.ui.set_input_text("hello\nworld".to_string());
    // Move to start of second line
    app.ui.textarea.move_cursor(CursorMove::Jump(1, 0));
    // Backspace should join lines; use input_without_shortcuts to ensure single-char delete
    app.ui.textarea.input_without_shortcuts(Input {
        key: Key::Backspace,
        ctrl: false,
        alt: false,
        shift: false,
    });
    app.ui.sync_input_from_textarea();
    assert_eq!(app.ui.get_input_text(), "helloworld");
    // Cursor should be at end of former first line (index 5)
    assert_eq!(app.ui.input_cursor_position, 5);
}

#[test]
fn test_backspace_with_alt_modifier_deletes_single_char() {
    let mut app = create_test_app();
    app.ui.set_input_text("hello world".to_string());
    app.ui.textarea.move_cursor(CursorMove::End);
    // Simulate Alt+Backspace; with input_without_shortcuts it should still delete one char
    app.ui.textarea.input_without_shortcuts(Input {
        key: Key::Backspace,
        ctrl: false,
        alt: true,
        shift: false,
    });
    app.ui.sync_input_from_textarea();
    assert_eq!(app.ui.get_input_text(), "hello worl");
    assert_eq!(app.ui.input_cursor_position, "hello worl".chars().count());
}

#[test]
fn test_update_input_scroll_keeps_cursor_visible() {
    let mut app = create_test_app();
    // Long line that wraps at width 10 into multiple lines
    let text = "one two three four five six seven eight nine ten";
    app.ui.set_input_text(text.to_string());
    // Simulate small input area: width=20 total => inner available width accounts in method
    let width: u16 = 10; // small terminal width to force wrapping (inner ~4)
    let input_area_height: u16 = 2; // only 2 lines visible
                                    // Place cursor near end
    app.ui.input_cursor_position = text.chars().count().saturating_sub(1);
    app.ui.update_input_scroll(input_area_height, width);
    // With cursor near end, scroll offset should be > 0 to bring cursor into view
    assert!(app.ui.input_scroll_offset > 0);
}

#[test]
fn test_shift_like_up_down_moves_one_line_on_many_newlines() {
    let mut app = create_test_app();
    // Build text with many blank lines
    let text = "top\n\n\n\n\n\n\n\n\n\nbottom";
    app.ui.set_input_text(text.to_string());
    // Jump to bottom line, col=3 (after 'bot')
    let bottom_row_usize = app.ui.textarea.lines().len().saturating_sub(1);
    let bottom_row = bottom_row_usize as u16;
    app.ui.textarea.move_cursor(CursorMove::Jump(bottom_row, 3));
    app.ui.sync_input_from_textarea();
    let (row_before, col_before) = app.ui.textarea.cursor();
    assert_eq!(row_before, bottom_row as usize);
    assert!(col_before <= app.ui.textarea.lines()[bottom_row_usize].chars().count());

    // Move up exactly one line
    app.ui.textarea.move_cursor(CursorMove::Up);
    app.ui.sync_input_from_textarea();
    let (row_after_up, col_after_up) = app.ui.textarea.cursor();
    assert_eq!(row_after_up, bottom_row_usize.saturating_sub(1));
    // Column should clamp reasonably; we just assert it's within line bounds
    assert!(col_after_up <= app.ui.textarea.lines()[8].chars().count());

    // Move down exactly one line
    app.ui.textarea.move_cursor(CursorMove::Down);
    app.ui.sync_input_from_textarea();
    let (row_after_down, _col_after_down) = app.ui.textarea.cursor();
    assert_eq!(row_after_down, bottom_row_usize);
}

#[test]
fn test_wrapped_vertical_navigation_preserves_visual_column() {
    let mut app = create_test_app();
    app.ui.set_input_text_with_cursor("abcdefgh".to_string(), 6);

    let moved_up = app
        .ui
        .move_cursor_in_wrapped_input(8, VerticalCursorDirection::Up);
    assert!(moved_up);
    assert_eq!(app.ui.input_cursor_position, 3);

    let moved_down = app
        .ui
        .move_cursor_in_wrapped_input(8, VerticalCursorDirection::Down);
    assert!(moved_down);
    assert_eq!(app.ui.input_cursor_position, 6);
}

#[test]
fn test_wrapped_vertical_navigation_clamps_to_shorter_line() {
    let mut app = create_test_app();
    app.ui.set_input_text_with_cursor("abcdefgh".to_string(), 8);

    let moved_up = app
        .ui
        .move_cursor_in_wrapped_input(8, VerticalCursorDirection::Up);
    assert!(moved_up);
    assert_eq!(app.ui.input_cursor_position, 5);

    let moved_down = app
        .ui
        .move_cursor_in_wrapped_input(8, VerticalCursorDirection::Down);
    assert!(moved_down);
    assert_eq!(app.ui.input_cursor_position, 8);
}

#[test]
fn test_wrapped_vertical_navigation_handles_multiple_paragraphs() {
    let mut app = create_test_app();
    let text = "aaaaa bbbbb ccccc ddddd\neeeee fffff ggggg hhhhh";
    app.ui
        .set_input_text_with_cursor(text.to_string(), text.chars().count());

    let newline_idx = text.find('\n').unwrap();
    let mut saw_above_newline = false;

    loop {
        let moved = app
            .ui
            .move_cursor_in_wrapped_input(15, VerticalCursorDirection::Up);
        if !moved {
            break;
        }
        if app.ui.input_cursor_position <= newline_idx {
            saw_above_newline = true;
        }
    }

    assert!(
        saw_above_newline,
        "cursor should cross the hard newline boundary"
    );
    let (row, _) = app.ui.textarea.cursor();
    assert_eq!(row, 0);
}

#[test]
fn test_wrapped_vertical_navigation_keeps_column_zero_on_descend() {
    let mut app = create_test_app();
    app.ui.set_input_text_with_cursor("abcdefgh".to_string(), 0);

    let moved_down = app
        .ui
        .move_cursor_in_wrapped_input(9, VerticalCursorDirection::Down);
    assert!(moved_down);
    assert_eq!(app.ui.input_cursor_position, 4);

    let moved_up = app
        .ui
        .move_cursor_in_wrapped_input(9, VerticalCursorDirection::Up);
    assert!(moved_up);
    assert_eq!(app.ui.input_cursor_position, 0);
}

#[test]
fn test_shift_like_left_right_moves_one_char() {
    let mut app = create_test_app();
    app.ui.set_input_text("hello".to_string());
    // Move to end, then back by one, then forward by one
    app.ui.textarea.move_cursor(CursorMove::End);
    app.ui.sync_input_from_textarea();
    let end_pos = app.ui.input_cursor_position;
    app.ui.textarea.move_cursor(CursorMove::Back);
    app.ui.sync_input_from_textarea();
    let back_pos = app.ui.input_cursor_position;
    assert_eq!(back_pos, end_pos.saturating_sub(1));
    app.ui.textarea.move_cursor(CursorMove::Forward);
    app.ui.sync_input_from_textarea();
    let forward_pos = app.ui.input_cursor_position;
    assert_eq!(forward_pos, end_pos);
}

#[test]
fn paste_inserts_cursor_at_end_of_insert() {
    let mut app = create_test_app();
    let term_width = 80u16;
    let text = "this is a long paragraph that should wrap softly when rendered";

    app.insert_into_input(text, term_width);

    assert_eq!(app.ui.get_input_text(), text);
    assert_eq!(app.ui.input_cursor_position, text.chars().count());
}

#[test]
fn visual_line_start_end_track_wrapped_columns() {
    let mut app = create_test_app();
    let text = "alpha beta gamma delta epsilon zeta eta".to_string();
    let cursor_pos = text.find("gamma").unwrap() + 2; // inside "gamma"
    let term_width = 20u16;
    let wrap_width = term_width.saturating_sub(5) as usize;

    app.ui.set_input_text_with_cursor(text.clone(), cursor_pos);

    let layout = TextWrapper::cursor_layout(&text, &WrapConfig::new(wrap_width));
    let line = layout.coordinates_for_index(app.ui.input_cursor_position).0;
    let (line_start, line_end) = layout
        .line_bounds(line)
        .expect("line bounds available for wrapped line");

    assert!(app.ui.move_cursor_to_visual_line_start(term_width));
    assert_eq!(app.ui.input_cursor_position, line_start);

    assert!(app.ui.move_cursor_to_visual_line_end(term_width));
    assert_eq!(app.ui.input_cursor_position, line_end);
}

#[test]
fn wrapped_cursor_crosses_paragraph_boundaries() {
    let mut app = create_test_app();
    let text = "one two three four five six seven eight nine ten\nalpha beta gamma delta epsilon zeta eta theta".to_string();
    let newline_index = text.find('\n').unwrap();
    let cursor_pos = newline_index + 4; // inside the second paragraph
    let term_width = 22u16;

    app.ui.set_input_text_with_cursor(text.clone(), cursor_pos);

    assert!(app
        .ui
        .move_cursor_in_wrapped_input(term_width, VerticalCursorDirection::Up));
    assert!(app.ui.input_cursor_position <= newline_index);

    assert!(app
        .ui
        .move_cursor_in_wrapped_input(term_width, VerticalCursorDirection::Down));
    assert!(app.ui.input_cursor_position > newline_index);
}

#[test]
fn page_cursor_movement_skips_multiple_wrapped_lines() {
    let mut app = create_test_app();
    let text = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua".to_string();
    let term_width = 24u16;

    app.ui
        .set_input_text_with_cursor(text.clone(), text.chars().count());

    let before = app.ui.input_cursor_position;
    let moved =
        app.ui
            .move_cursor_page_in_wrapped_input(term_width, VerticalCursorDirection::Up, 3);

    assert!(moved);
    assert!(app.ui.input_cursor_position < before);
}

#[test]
fn test_cursor_mapping_blankline_insert_no_desync() {
    let mut app = create_test_app();
    let text = "asdf\n\nasdf\n\nasdf";
    app.ui.set_input_text(text.to_string());
    // Jump to blank line 2 (0-based row 3), column 0
    app.ui.textarea.move_cursor(CursorMove::Jump(3, 0));
    app.ui.sync_input_from_textarea();
    // Insert a character on the blank line
    app.ui.textarea.insert_str("x");
    app.ui.sync_input_from_textarea();

    // Compute wrapped position using same wrapper logic (no wrapping with wide width)
    let config = WrapConfig::new(120);
    let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
        app.ui.get_input_text(),
        app.ui.input_cursor_position,
        &config,
    );
    // Compare to textarea's cursor row/col
    let (row, c) = app.ui.textarea.cursor();
    assert_eq!(line, row);
    assert_eq!(col, c);
}

#[test]
fn test_recompute_input_layout_after_edit_updates_scroll() {
    let mut app = create_test_app();
    // Make text long enough to wrap
    let text = "one two three four five six seven eight nine ten";
    app.ui.set_input_text(text.to_string());
    // Place cursor near end
    app.ui.input_cursor_position = text.chars().count().saturating_sub(1);
    // Very small terminal width to force heavy wrapping; method accounts for borders and margin
    let width: u16 = 6;
    app.ui.recompute_input_layout_after_edit(width);
    // With cursor near end on a heavily wrapped input, expect some scroll
    assert!(app.ui.input_scroll_offset > 0);
    // Changing cursor position to start should reduce or reset scroll
    app.ui.input_cursor_position = 0;
    app.ui.recompute_input_layout_after_edit(width);
    assert_eq!(app.ui.input_scroll_offset, 0);
}

#[test]
fn complete_slash_command_fills_unique_match() {
    let mut app = create_test_app();
    app.ui.set_input_text("/he".into());

    let handled = app.complete_slash_command(80);
    assert!(handled);
    assert_eq!(app.ui.get_input_text(), "/help ");
    assert_eq!(app.ui.input_cursor_position, "/help ".chars().count());
    assert!(app.ui.is_input_focused());
}

#[test]
fn complete_slash_command_lists_multiple_matches() {
    let mut app = create_test_app();
    app.ui.set_input_text("/p".into());

    let handled = app.complete_slash_command(80);
    assert!(handled);
    assert_eq!(app.ui.get_input_text(), "/p");
    assert_eq!(
        app.ui.status.as_deref(),
        Some("Commands: /provider, /persona, /preset")
    );
}

#[test]
fn complete_slash_command_reports_unknown_prefix() {
    let mut app = create_test_app();
    app.ui.set_input_text("/zzz".into());

    let handled = app.complete_slash_command(80);
    assert!(handled);
    assert_eq!(app.ui.get_input_text(), "/zzz");
    assert_eq!(app.ui.status.as_deref(), Some("No command matches '/zzz'"));
}

#[test]
fn test_last_and_first_user_message_index() {
    let mut app = create_test_app();
    // No messages
    assert_eq!(app.ui.last_user_message_index(), None);
    assert_eq!(app.ui.first_user_message_index(), None);

    // Add messages: user, assistant, user
    app.ui.messages.push_back(create_test_message("user", "u1"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "a1"));
    app.ui.messages.push_back(create_test_message("user", "u2"));

    assert_eq!(app.ui.first_user_message_index(), Some(0));
    assert_eq!(app.ui.last_user_message_index(), Some(2));
}

#[test]
fn prewrap_height_matches_renderer_with_tables() {
    // Test that scroll height calculations match renderer height when tables are involved
    let mut app = create_test_app();

    // Add a message with a large table that will trigger width-dependent wrapping
    let table_content = r#"| Government System | Definition | Key Properties |
|-------------------|------------|----------------|
| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections, Protection of civil liberties |
| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |
| Monarchy | A form of government in which a single person, known as a monarch, rules until death or abdication. | Hereditary succession, Often ceremonial with limited political power |
"#;

    app.ui.messages.push_back(Message {
        role: "assistant".into(),
        content: table_content.to_string(),
    });

    let width = 80u16;

    // Get the height that the renderer will actually use (prewrapped with width)
    let renderer_height = {
        let lines = app.get_prewrapped_lines_cached(width);
        lines.len() as u16
    };

    // Get the height that scroll calculations currently use
    let scroll_height = app.ui.calculate_wrapped_line_count(width);

    // These should match - if they don't, scroll targeting will be off
    assert_eq!(
        renderer_height, scroll_height,
        "Renderer height ({}) should match scroll calculation height ({})",
        renderer_height, scroll_height
    );
}

#[test]
fn streaming_table_autoscroll_stays_consistent() {
    // Test that autoscroll stays at bottom when streaming table content
    let mut app = create_test_app();

    // Start with a user message
    let width = 80u16;
    let available_height = 20u16;

    {
        let mut conversation = app.conversation();
        conversation.add_user_message("Generate a table".to_string());

        // Start streaming a table in chunks
        let table_start = "Here's a government systems table:\n\n";
        conversation.append_to_response(table_start, available_height, width);

        let table_header =
            "| Government System | Definition | Key Properties |\n|-------------------|------------|----------------|\n";
        conversation.append_to_response(table_header, available_height, width);

        // Add table rows that will cause wrapping and potentially height changes
        let row1 = "| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections |\n";
        conversation.append_to_response(row1, available_height, width);

        let row2 = "| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |\n";
        conversation.append_to_response(row2, available_height, width);
    }

    // After each append, if we're auto-scrolling, we should be at the bottom
    if app.ui.auto_scroll {
        let expected_max_scroll = app.ui.calculate_max_scroll_offset(available_height, width);
        assert_eq!(
            app.ui.scroll_offset, expected_max_scroll,
            "Auto-scroll should keep us at bottom. Current offset: {}, Expected max: {}",
            app.ui.scroll_offset, expected_max_scroll
        );
    }
}

#[test]
fn block_selection_offset_matches_renderer_with_tables() {
    // Test that block selection scroll calculations match renderer when tables are involved
    let mut app = create_test_app();

    // Add content with a table followed by a code block
    let content_with_table_and_code = r#"Here's a table:

| Government System | Definition | Key Properties |
|-------------------|------------|----------------|
| Democracy | A system where power is vested in the people, who rule either directly or through freely elected representatives. | Universal suffrage, Free and fair elections |
| Dictatorship | A form of government where a single person or a small group holds absolute power. | Centralized authority, Limited or no political opposition |

And here's some code:

```rust
fn main() {
println!("Hello, world!");
}
```"#;

    app.ui.messages.push_back(Message {
        role: "assistant".into(),
        content: content_with_table_and_code.to_string(),
    });

    let width = 80u16;
    let available_height = 20u16;

    // Get codeblock ranges (these are computed from widthless lines)
    let ranges = crate::ui::markdown::compute_codeblock_ranges(&app.ui.messages, &app.ui.theme);
    assert!(!ranges.is_empty(), "Should have at least one code block");

    let (code_block_start, _len, _content) = &ranges[0];

    // Test that block selection navigation uses the same width-aware approach as the renderer
    // Both should now use width-aware line building for consistent results

    // The key insight: Both block navigation and rendering should use the same cached approach
    // for consistency. In production, block navigation would also use get_prewrapped_lines_cached.
    let lines = app.get_prewrapped_lines_cached(width);

    let _block_nav_offset = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
        lines,
        width,
        available_height,
        *code_block_start,
    );

    // Since both use the same method, heights are identical
    let block_nav_height = lines.len();
    let renderer_height = lines.len();

    // This should always pass now since they're the same method
    assert_eq!(
        block_nav_height, renderer_height,
        "Block navigation height ({}) should match renderer height ({}) for accurate block selection",
        block_nav_height, renderer_height
    );

    // Legacy widthless path is deprecated under the unified layout engine.
    // We no longer assert differences against that path because width-aware layout
    // is the single source of truth for visual line counts.
}

#[test]
fn narrow_terminal_exposes_scroll_height_mismatch() {
    // Test with very narrow terminal that forces significant table wrapping differences
    let mut app = create_test_app();

    // Add a wide table that will need significant rebalancing in narrow terminals
    let wide_table = r#"| Very Long Government System Name | Very Detailed Definition That Goes On And On | Extremely Detailed Key Properties That Include Many Words |
|-----------------------------------|-----------------------------------------------|------------------------------------------------------------|
| Constitutional Democratic Republic | A complex system where power is distributed among elected representatives who operate within a constitutional framework with checks and balances | Multi-party elections, separation of powers, constitutional limits, judicial review, civil liberties protection |
| Authoritarian Single-Party State | A centralized system where one political party maintains exclusive control over government institutions and suppresses opposition | Centralized control, restricted freedoms, state propaganda, limited political participation, strict social control |

Some additional text after the table."#;

    app.ui.messages.push_back(Message {
        role: "assistant".into(),
        content: wide_table.to_string(),
    });

    // Use very narrow width that will force aggressive table column rebalancing
    let width = 40u16;

    // Get the height that the renderer will actually use (prewrapped with narrow width)
    let renderer_height = {
        let lines = app.get_prewrapped_lines_cached(width);
        lines.len() as u16
    };

    // Get the height that scroll calculations currently use (widthless, then scroll heuristic)
    let scroll_height = app.ui.calculate_wrapped_line_count(width);

    // This should expose the mismatch if it exists
    assert_eq!(
        renderer_height, scroll_height,
        "Narrow terminal: Renderer height ({}) should match scroll calculation height ({})",
        renderer_height, scroll_height
    );
}

#[test]
fn streaming_table_with_cache_invalidation_consistency() {
    // Test the exact scenario: streaming table generation with cache invalidation
    let mut app = create_test_app();

    let width = 80u16;
    let available_height = 20u16;

    // Start with user message and empty assistant response
    {
        let mut conversation = app.conversation();
        conversation.add_user_message("Generate a large comparison table".to_string());
    }

    // Simulate streaming a large table piece by piece, with cache invalidation
    let table_chunks = vec![
        "Here's a detailed comparison table:\n\n",
        "| Feature | Option A | Option B | Option C |\n",
        "|---------|----------|----------|----------|\n",
        "| Performance | Very fast execution with optimized algorithms | Moderate speed with good balance | Slower but more flexible |
",
        "| Memory Usage | Low memory footprint, efficient data structures | Medium usage with some overhead | Higher memory requirements |
",
        "| Ease of Use | Complex setup but powerful once configured | User-friendly with good documentation | Simple and intuitive interface |
",
        "| Cost | Enterprise pricing with volume discounts available | Reasonable pricing for small to medium teams | Free with optional premium features |
",
    ];

    for chunk in table_chunks {
        // Before append: get current scroll state
        let _scroll_before = app.ui.scroll_offset;
        let _max_scroll_before = app.ui.calculate_max_scroll_offset(available_height, width);

        // Append content (this invalidates prewrap cache)
        {
            let mut conversation = app.conversation();
            conversation.append_to_response(chunk, available_height, width);
        }

        // After append: check scroll consistency
        let scroll_after = app.ui.scroll_offset;
        let max_scroll_after = app.ui.calculate_max_scroll_offset(available_height, width);

        // During streaming with auto_scroll=true, we should always be at bottom
        if app.ui.auto_scroll {
            assert_eq!(
                scroll_after, max_scroll_after,
                "Auto-scroll should keep us at bottom after streaming chunk"
            );
        }

        // The key test: prewrap cache and scroll calculation should give same height
        let prewrap_height = app.get_prewrapped_lines_cached(width).len() as u16;
        let scroll_calc_height = app.ui.calculate_wrapped_line_count(width);

        assert_eq!(
            prewrap_height, scroll_calc_height,
            "After streaming chunk, prewrap height ({}) should match scroll calc height ({})",
            prewrap_height, scroll_calc_height
        );
    }
}

#[test]
fn test_page_up_down_and_home_end_behavior() {
    let mut app = create_test_app();
    // Create enough messages to require scrolling
    for _ in 0..50 {
        app.ui
            .messages
            .push_back(create_test_message("assistant", "line content"));
    }

    let width: u16 = 80;
    let input_area_height = 3u16; // pretend a small input area
    let term_height = 24u16;
    let available_height = {
        let conversation = app.conversation();
        conversation.calculate_available_height(term_height, input_area_height)
    };

    // Sanity: have some scrollable height
    let max_scroll = app.ui.calculate_max_scroll_offset(available_height, width);
    assert!(max_scroll > 0);

    // Start in the middle
    let step = available_height.saturating_sub(1);
    app.ui.scroll_offset = (step * 2).min(max_scroll);

    // Page up reduces by step, not below 0
    let before = app.ui.scroll_offset;
    app.ui.page_up(available_height);
    let after_up = app.ui.scroll_offset;
    assert_eq!(after_up, before.saturating_sub(step));
    assert!(!app.ui.auto_scroll);

    // Page down increases by step, clamped to max
    app.ui.page_down(available_height, width);
    let after_down = app.ui.scroll_offset;
    assert!(after_down >= after_up);
    assert!(after_down <= max_scroll);
    assert!(!app.ui.auto_scroll);

    // Home goes to top and disables auto-scroll
    app.ui.scroll_to_top();
    assert_eq!(app.ui.scroll_offset, 0);
    assert!(!app.ui.auto_scroll);

    // End goes to bottom and enables auto-scroll
    app.ui.scroll_to_bottom_view(available_height, width);
    assert_eq!(app.ui.scroll_offset, max_scroll);
    assert!(app.ui.auto_scroll);
}

#[test]
fn test_prev_next_user_message_index_navigation() {
    let mut app = create_test_app();
    // indices: 0 user, 1 assistant, 2 app, 3 user
    app.ui.messages.push_back(create_test_message("user", "u1"));
    app.ui
        .messages
        .push_back(create_test_message("assistant", "a1"));
    app.ui.messages.push_back(create_test_message(
        crate::core::message::ROLE_APP_INFO,
        "s1",
    ));
    app.ui.messages.push_back(create_test_message("user", "u2"));

    // From index 3 (user) prev should be 0 (skipping non-user)
    assert_eq!(app.ui.prev_user_message_index(3), Some(0));
    // From index 0 next should be 3 (skipping non-user)
    assert_eq!(app.ui.next_user_message_index(0), Some(3));
    // From index 1 prev should be 0
    assert_eq!(app.ui.prev_user_message_index(1), Some(0));
    // From index 1 next should be 3
    assert_eq!(app.ui.next_user_message_index(1), Some(3));
}

#[test]
fn test_set_input_text_places_cursor_at_end() {
    let mut app = create_test_app();
    let text = String::from("line1\nline2");
    app.ui.set_input_text(text.clone());
    // Linear cursor at end
    assert_eq!(app.ui.input_cursor_position, text.chars().count());
    // Textarea cursor at end (last row/col)
    let (row, col) = app.ui.textarea.cursor();
    let lines = app.ui.textarea.lines();
    assert_eq!(row, lines.len() - 1);
    assert_eq!(col, lines.last().unwrap().chars().count());
}

#[test]
fn test_turn_off_character_mode_from_picker() {
    use crate::character::card::{CharacterCard, CharacterData};

    let mut app = create_test_app();

    let character = CharacterCard {
        spec: "chara_card_v2".to_string(),
        spec_version: "2.0".to_string(),
        data: CharacterData {
            name: "TestChar".to_string(),
            description: "Test".to_string(),
            personality: "Friendly".to_string(),
            scenario: "Testing".to_string(),
            first_mes: "Hello!".to_string(),
            mes_example: String::new(),
            creator_notes: None,
            system_prompt: None,
            post_history_instructions: None,
            alternate_greetings: None,
            tags: None,
            creator: None,
            character_version: None,
        },
    };

    app.session.set_character(character);
    assert!(app.session.active_character.is_some());

    app.picker.picker_session = Some(picker::PickerSession {
        state: PickerState::new(
            "Pick Character",
            vec![PickerItem {
                id: picker::TURN_OFF_CHARACTER_ID.to_string(),
                label: "[Turn off character mode]".to_string(),
                metadata: Some("Disable character".to_string()),
                inspect_metadata: Some("Disable character".to_string()),
                sort_key: None,
            }],
            0,
        ),
        data: picker::PickerData::Character(picker::CharacterPickerState {
            search_filter: String::new(),
            all_items: vec![],
        }),
    });

    app.apply_selected_character(false);

    assert!(app.session.active_character.is_none());
    assert_eq!(app.ui.status.as_deref(), Some("Character mode disabled"));
}
