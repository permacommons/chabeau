//! Keybinding handlers for navigation and textarea editing
//!
//! This module contains helper functions for handling various keybinding clusters
//! to reduce the size of the main event loop match statement.

use crate::core::app::App;
use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
use std::time::Instant;
use tui_textarea::{CursorMove, Input as TAInput, Key as TAKey};

/// Handles Home/End/PageUp/PageDown navigation keys
pub async fn handle_navigation_keys(
    app_guard: &mut App,
    key_code: ratatui::crossterm::event::KeyCode,
    term_width: u16,
    term_height: u16,
) -> bool {
    use ratatui::crossterm::event::KeyCode;

    match key_code {
        KeyCode::Home => {
            app_guard.scroll_to_top();
            true
        }
        KeyCode::End => {
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height = term_height
                .saturating_sub(input_area_height + 2)
                .saturating_sub(1);
            app_guard.scroll_to_bottom_view(available_height, term_width);
            true
        }
        KeyCode::PageUp => {
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height = term_height
                .saturating_sub(input_area_height + 2)
                .saturating_sub(1);
            app_guard.page_up(available_height);
            true
        }
        KeyCode::PageDown => {
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height = term_height
                .saturating_sub(input_area_height + 2)
                .saturating_sub(1);
            app_guard.page_down(available_height, term_width);
            true
        }
        _ => false,
    }
}

/// Handles arrow key navigation (Left/Right/Up/Down)
pub async fn handle_arrow_keys(
    app_guard: &mut App,
    key: &KeyEvent,
    term_width: u16,
    term_height: u16,
    last_input_layout_update: &mut Instant,
) -> bool {
    use ratatui::crossterm::event::KeyCode;

    match key.code {
        KeyCode::Left => {
            let compose = app_guard.compose_mode;
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            if (compose && !shift) || (!compose && shift) {
                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Back));
                recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            } else {
                app_guard.horizontal_scroll_offset =
                    app_guard.horizontal_scroll_offset.saturating_sub(1);
            }
            true
        }
        KeyCode::Right => {
            let compose = app_guard.compose_mode;
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            if (compose && !shift) || (!compose && shift) {
                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Forward));
                recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            } else {
                app_guard.horizontal_scroll_offset =
                    app_guard.horizontal_scroll_offset.saturating_add(1);
            }
            true
        }
        KeyCode::Up => {
            let compose = app_guard.compose_mode;
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);

            if (compose && !shift) || (!compose && shift) {
                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Up));
                recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            } else {
                app_guard.auto_scroll = false;
                app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(1);
            }
            true
        }
        KeyCode::Down => {
            let compose = app_guard.compose_mode;
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);

            if (compose && !shift) || (!compose && shift) {
                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Down));
                recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            } else {
                app_guard.auto_scroll = false;
                let input_area_height = app_guard.calculate_input_area_height(term_width);
                let available_height = term_height
                    .saturating_sub(input_area_height + 2)
                    .saturating_sub(1);
                let max_scroll =
                    app_guard.calculate_max_scroll_offset(available_height, term_width);
                app_guard.scroll_offset =
                    (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
            }
            true
        }
        _ => false,
    }
}

/// Handles textarea editing keys (Ctrl+A, Ctrl+E, Delete, Backspace, regular chars)
pub async fn handle_textarea_editing_keys(
    app_guard: &mut App,
    key: &KeyEvent,
    term_width: u16,
    last_input_layout_update: &mut Instant,
) -> bool {
    use ratatui::crossterm::event::KeyCode;

    match key.code {
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_guard.apply_textarea_edit(|ta| {
                ta.input(TAInput::from(*key));
            });
            recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            true
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app_guard.apply_textarea_edit(|ta| {
                ta.input(TAInput::from(*key));
            });
            recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            true
        }
        KeyCode::Char(_) => {
            app_guard.apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.input(TAInput::from(*key));
            });
            true
        }
        KeyCode::Delete => {
            app_guard.apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.input_without_shortcuts(TAInput {
                    key: TAKey::Delete,
                    ctrl: false,
                    alt: false,
                    shift: false,
                });
            });
            true
        }
        KeyCode::Backspace => {
            let input = TAInput::from(*key);
            app_guard.apply_textarea_edit(|ta| {
                ta.input_without_shortcuts(input);
            });
            recompute_input_layout_if_due(app_guard, term_width, last_input_layout_update);
            true
        }
        _ => false,
    }
}

/// Helper function to recompute input layout if enough time has passed
fn recompute_input_layout_if_due(app: &mut App, term_width: u16, last_update: &mut Instant) {
    use std::time::Duration;

    if last_update.elapsed() >= Duration::from_millis(16) {
        app.recompute_input_layout_after_edit(term_width);
        *last_update = Instant::now();
    }
}
