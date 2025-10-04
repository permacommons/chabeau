//! Keybinding handler implementations
//!
//! This module contains all the keybinding handlers organized by functionality:
//! - Basic system operations (quit, clear, escape)
//! - Navigation (arrows, home/end, page up/down)
//! - Text editing (typing, cursor movement, deletion)
//! - Mode switching (block select, edit select)
//! - Complex operations (external editor, message submission)
//! - Mode-specific handlers (picker, edit select, block select)

use crate::commands;
use crate::core::app::App;
use crate::core::chat_stream::ChatStreamService;
use crate::ui::chat_loop::keybindings::registry::{KeyHandler, KeyResult};
use crate::ui::chat_loop::{
    handle_block_select_mode_event, handle_ctrl_j_shortcut, handle_edit_select_mode_event,
    handle_enter_key, handle_external_editor_shortcut, handle_picker_key_event,
    handle_retry_shortcut, KeyLoopAction, UiEvent,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc::UnboundedSender, Mutex};
use tui_textarea::{CursorMove, Input as TAInput, Key as TAKey};

// ============================================================================
// Utility Functions
// ============================================================================

/// Utility function to wrap to previous index in a circular manner
pub fn wrap_previous_index(current: usize, total: usize) -> Option<usize> {
    if total == 0 {
        None
    } else if current == 0 {
        Some(total - 1)
    } else {
        Some(current - 1)
    }
}

/// Utility function to wrap to next index in a circular manner
pub fn wrap_next_index(current: usize, total: usize) -> Option<usize> {
    if total == 0 {
        None
    } else {
        Some((current + 1) % total)
    }
}

/// Utility function to scroll a code block into view
pub fn scroll_block_into_view(
    app_guard: &mut App,
    term_width: u16,
    term_height: u16,
    block_start: usize,
) {
    let lines =
        crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &app_guard.ui.messages,
            &app_guard.ui.theme,
            app_guard.ui.markdown_enabled,
            app_guard.ui.syntax_enabled,
            Some(term_width as usize),
        );
    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
    let available_height = {
        let conversation = app_guard.conversation();
        conversation.calculate_available_height(term_height, input_area_height)
    };
    let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
        &lines,
        term_width,
        available_height,
        block_start,
    );
    let max_scroll = app_guard
        .ui
        .calculate_max_scroll_offset(available_height, term_width);
    app_guard.ui.scroll_offset = desired.min(max_scroll);
}

/// Helper function to recompute input layout if enough time has passed
fn recompute_input_layout_if_due(app: &mut App, term_width: u16, last_update: &mut Instant) {
    if last_update.elapsed() >= Duration::from_millis(16) {
        app.ui.recompute_input_layout_after_edit(term_width);
        *last_update = Instant::now();
    }
}

// ============================================================================
// Basic System Handlers
// ============================================================================

/// Handler for Ctrl+C (quit)
pub struct CtrlCHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlCHandler {
    async fn handle(
        &self,
        _app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        _term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        KeyLoopAction::Break.into()
    }
}

/// Handler for Ctrl+L (clear status)
pub struct CtrlLHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlLHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        _term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        app_guard.conversation().clear_status();
        KeyResult::Handled
    }
}

/// Handler for F4 (toggle compose mode)
pub struct F4Handler;

#[async_trait::async_trait]
impl KeyHandler for F4Handler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        _term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        app_guard.ui.toggle_compose_mode();
        KeyResult::Handled
    }
}

/// Handler for Escape key
pub struct EscapeHandler;

#[async_trait::async_trait]
impl KeyHandler for EscapeHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        _term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        if app_guard.ui.file_prompt().is_some() {
            app_guard.ui.cancel_file_prompt();
            return KeyResult::Handled;
        }
        if app_guard.ui.in_place_edit_index().is_some() {
            app_guard.ui.cancel_in_place_edit();
            app_guard.ui.clear_input();
            return KeyResult::Handled;
        }
        if app_guard.ui.is_streaming {
            app_guard.conversation().cancel_current_stream();
            return KeyResult::Handled;
        }
        KeyResult::NotHandled
    }
}

/// Handler for Ctrl+D
pub struct CtrlDHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlDHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        if app_guard.ui.get_input_text().is_empty() {
            KeyLoopAction::Break.into()
        } else {
            app_guard
                .ui
                .apply_textarea_edit_and_recompute(term_width, |ta| {
                    ta.input_without_shortcuts(TAInput {
                        key: TAKey::Delete,
                        ctrl: false,
                        alt: false,
                        shift: false,
                    });
                });
            KeyLoopAction::Continue.into()
        }
    }
}

// ============================================================================
// Navigation Handlers
// ============================================================================

/// Handler for navigation keys (Home, End, PageUp, PageDown)
pub struct NavigationHandler;

#[async_trait::async_trait]
impl KeyHandler for NavigationHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        match key.code {
            KeyCode::Home => {
                app_guard.ui.scroll_to_top();
                KeyResult::Handled
            }
            KeyCode::End => {
                let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                let available_height = {
                    let conversation = app_guard.conversation();
                    conversation.calculate_available_height(term_height, input_area_height)
                };
                app_guard
                    .ui
                    .scroll_to_bottom_view(available_height, term_width);
                KeyResult::Handled
            }
            KeyCode::PageUp => {
                let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                let available_height = {
                    let conversation = app_guard.conversation();
                    conversation.calculate_available_height(term_height, input_area_height)
                };
                app_guard.ui.page_up(available_height);
                KeyResult::Handled
            }
            KeyCode::PageDown => {
                let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                let available_height = {
                    let conversation = app_guard.conversation();
                    conversation.calculate_available_height(term_height, input_area_height)
                };
                app_guard.ui.page_down(available_height, term_width);
                KeyResult::Handled
            }
            _ => KeyResult::NotHandled,
        }
    }
}

/// Handler for arrow keys (Up, Down, Left, Right)
pub struct ArrowKeyHandler;

#[async_trait::async_trait]
impl KeyHandler for ArrowKeyHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        let mut last_update = last_input_layout_update.unwrap_or_else(Instant::now);

        match key.code {
            KeyCode::Left => {
                let compose = app_guard.ui.compose_mode;
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if (compose && !shift) || (!compose && shift) {
                    app_guard
                        .ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Back));
                    recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                } else {
                    app_guard.ui.horizontal_scroll_offset =
                        app_guard.ui.horizontal_scroll_offset.saturating_sub(1);
                }
                KeyResult::Handled
            }
            KeyCode::Right => {
                let compose = app_guard.ui.compose_mode;
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if (compose && !shift) || (!compose && shift) {
                    app_guard
                        .ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Forward));
                    recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                } else {
                    app_guard.ui.horizontal_scroll_offset =
                        app_guard.ui.horizontal_scroll_offset.saturating_add(1);
                }
                KeyResult::Handled
            }
            KeyCode::Up => {
                let compose = app_guard.ui.compose_mode;
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);

                if (compose && !shift) || (!compose && shift) {
                    app_guard
                        .ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Up));
                    recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                } else {
                    app_guard.ui.auto_scroll = false;
                    app_guard.ui.scroll_offset = app_guard.ui.scroll_offset.saturating_sub(1);
                }
                KeyResult::Handled
            }
            KeyCode::Down => {
                let compose = app_guard.ui.compose_mode;
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);

                if (compose && !shift) || (!compose && shift) {
                    app_guard
                        .ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Down));
                    recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                } else {
                    app_guard.ui.auto_scroll = false;
                    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                    let available_height = {
                        let conversation = app_guard.conversation();
                        conversation.calculate_available_height(term_height, input_area_height)
                    };
                    let max_scroll = app_guard
                        .ui
                        .calculate_max_scroll_offset(available_height, term_width);
                    app_guard.ui.scroll_offset =
                        (app_guard.ui.scroll_offset.saturating_add(1)).min(max_scroll);
                }
                KeyResult::Handled
            }
            _ => KeyResult::NotHandled,
        }
    }
}

// ============================================================================
// Text Editing Handlers
// ============================================================================

/// Handler for text editing keys
pub struct TextEditingHandler;

#[async_trait::async_trait]
impl KeyHandler for TextEditingHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        _term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        let mut last_update = last_input_layout_update.unwrap_or_else(Instant::now);

        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app_guard.ui.apply_textarea_edit(|ta| {
                    ta.input(TAInput::from(*key));
                });
                recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                KeyResult::Handled
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app_guard.ui.apply_textarea_edit(|ta| {
                    ta.input(TAInput::from(*key));
                });
                recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                KeyResult::Handled
            }
            KeyCode::Char(c) => {
                // Skip Ctrl+J - it has special compose mode logic that needs the stream service
                if c == 'j' && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return KeyResult::NotHandled;
                }
                app_guard
                    .ui
                    .apply_textarea_edit_and_recompute(term_width, |ta| {
                        ta.input(TAInput::from(*key));
                    });
                KeyResult::Handled
            }
            KeyCode::Delete => {
                app_guard
                    .ui
                    .apply_textarea_edit_and_recompute(term_width, |ta| {
                        ta.input_without_shortcuts(TAInput {
                            key: TAKey::Delete,
                            ctrl: false,
                            alt: false,
                            shift: false,
                        });
                    });
                KeyResult::Handled
            }
            KeyCode::Backspace => {
                let input = TAInput::from(*key);
                app_guard.ui.apply_textarea_edit(|ta| {
                    ta.input_without_shortcuts(input);
                });
                recompute_input_layout_if_due(&mut app_guard, term_width, &mut last_update);
                KeyResult::Handled
            }
            _ => KeyResult::NotHandled,
        }
    }
}

// ============================================================================
// Slash Command Autocomplete Handler
// ============================================================================

type CommandMatchFn = dyn Fn(&str) -> Vec<String> + Send + Sync;

pub struct CommandAutocompleteHandler {
    matcher: Arc<CommandMatchFn>,
}

impl CommandAutocompleteHandler {
    pub fn new() -> Self {
        Self {
            matcher: Arc::new(|prefix: &str| {
                commands::matching_commands(prefix)
                    .into_iter()
                    .map(|command| command.name.to_string())
                    .collect()
            }),
        }
    }

    #[cfg(test)]
    pub fn with_matcher(matcher: Arc<CommandMatchFn>) -> Self {
        Self { matcher }
    }
}

impl Default for CommandAutocompleteHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl KeyHandler for CommandAutocompleteHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        let input = app_guard.ui.get_input_text().to_string();

        if !input.starts_with('/') {
            return KeyResult::NotHandled;
        }

        let cursor_char_index = app_guard.ui.input_cursor_position;
        let cursor_byte_index = char_index_to_byte_index(&input, cursor_char_index);

        if cursor_byte_index == 0 {
            return KeyResult::NotHandled;
        }

        let command_region_end = {
            let after_slash = &input[1..];
            after_slash
                .char_indices()
                .find(|(_, c)| c.is_whitespace())
                .map(|(idx, _)| idx + 1)
                .unwrap_or_else(|| input.len())
        };

        if cursor_byte_index > command_region_end {
            return KeyResult::NotHandled;
        }

        let command_prefix = &input[1..cursor_byte_index];
        let matches = (self.matcher)(command_prefix);

        if matches.is_empty() {
            let message = if command_prefix.is_empty() {
                "No commands available".to_string()
            } else {
                format!("No commands matching \"/{}\"", command_prefix)
            };
            app_guard.conversation().set_status(message);
            return KeyResult::Handled;
        }

        let rest = &input[command_region_end..];

        if matches.len() == 1 {
            let command_name = &matches[0];
            let mut new_text = String::with_capacity(1 + command_name.len() + rest.len() + 1);
            new_text.push('/');
            new_text.push_str(command_name);
            if rest.is_empty() {
                new_text.push(' ');
            } else {
                new_text.push_str(rest);
            }

            if new_text != input {
                let target_col_base = command_name.chars().count() + 1;
                let target_col = if rest.is_empty() {
                    target_col_base + 1
                } else {
                    target_col_base
                } as u16;

                app_guard
                    .ui
                    .apply_textarea_edit_and_recompute(term_width, |ta| {
                        ta.select_all();
                        ta.cut();
                        ta.insert_str(&new_text);
                        ta.move_cursor(CursorMove::Jump(0, target_col));
                    });
            }

            return KeyResult::Handled;
        }

        let prefix_char_len = command_prefix.chars().count();
        let common_prefix_len = longest_common_prefix_len(&matches);
        let target_prefix_len = common_prefix_len.max(prefix_char_len);
        let canonical_prefix: String = matches[0].chars().take(target_prefix_len).collect();

        let mut new_text = String::with_capacity(1 + canonical_prefix.len() + rest.len());
        new_text.push('/');
        new_text.push_str(&canonical_prefix);
        new_text.push_str(rest);

        if new_text != input {
            let target_col = (canonical_prefix.chars().count() + 1) as u16;
            app_guard
                .ui
                .apply_textarea_edit_and_recompute(term_width, |ta| {
                    ta.select_all();
                    ta.cut();
                    ta.insert_str(&new_text);
                    ta.move_cursor(CursorMove::Jump(0, target_col));
                });
        }

        let status = matches
            .iter()
            .map(|name| format!("/{}", name))
            .collect::<Vec<_>>()
            .join(", ");
        app_guard
            .conversation()
            .set_status(format!("Commands: {}", status));

        KeyResult::Handled
    }
}

fn char_index_to_byte_index(s: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    match s.char_indices().nth(char_index) {
        Some((idx, _)) => idx,
        None => s.len(),
    }
}

fn longest_common_prefix_len(names: &[String]) -> usize {
    if names.is_empty() {
        return 0;
    }

    let mut prefix: Vec<char> = names[0].chars().collect();
    for name in names.iter().skip(1) {
        let mut new_len = 0;
        for (a, b) in prefix.iter().zip(name.chars()) {
            if a.eq_ignore_ascii_case(&b) {
                new_len += 1;
            } else {
                break;
            }
        }
        prefix.truncate(new_len);
        if prefix.is_empty() {
            break;
        }
    }

    prefix.len()
}

// ============================================================================
// Mode Switching Handlers
// ============================================================================

/// Handler for Ctrl+B (block select mode)
pub struct CtrlBHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlBHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;
        if !app_guard.ui.markdown_enabled {
            app_guard
                .conversation()
                .set_status("Markdown disabled (/markdown on)");
            return KeyResult::Handled;
        }

        let blocks = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &app_guard.ui.messages,
            &app_guard.ui.theme,
            Some(term_width as usize),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            app_guard.ui.syntax_enabled,
        );

        if app_guard.ui.in_block_select_mode() {
            if let Some(cur) = app_guard.ui.selected_block_index() {
                let total = blocks.len();
                if let Some(next) = wrap_previous_index(cur, total) {
                    app_guard.ui.set_selected_block_index(next);
                    if let Some((start, _len, _)) = blocks.get(next) {
                        scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                    }
                }
            }
        } else if blocks.is_empty() {
            app_guard.conversation().set_status("No code blocks");
        } else {
            let last = blocks.len().saturating_sub(1);
            app_guard.ui.enter_block_select_mode(last);
            if let Some((start, _len, _)) = blocks.get(last) {
                scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
            }
        }

        KeyResult::Handled
    }
}

/// Handler for Ctrl+P (edit select mode)
pub struct CtrlPHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlPHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut app_guard = app.lock().await;

        if app_guard.ui.last_user_message_index().is_none() {
            app_guard.conversation().set_status("No user messages");
            return KeyResult::Handled;
        }

        if app_guard.ui.in_edit_select_mode() {
            if let Some(current) = app_guard.ui.selected_user_message_index() {
                let prev = {
                    let ui = &app_guard.ui;
                    ui.prev_user_message_index(current)
                        .or_else(|| ui.last_user_message_index())
                };
                if let Some(prev) = prev {
                    app_guard.ui.set_selected_user_message_index(prev);
                }
            } else if let Some(last) = app_guard.ui.last_user_message_index() {
                app_guard.ui.set_selected_user_message_index(last);
            }
        } else {
            app_guard.ui.enter_edit_select_mode();
            if let Some(last) = app_guard.ui.last_user_message_index() {
                app_guard.ui.set_selected_user_message_index(last);
            }
        }

        if let Some(idx) = app_guard.ui.selected_user_message_index() {
            app_guard
                .conversation()
                .scroll_index_into_view(idx, term_width, term_height);
        }

        KeyResult::Handled
    }
}

// ============================================================================
// Complex Operation Handlers
// ============================================================================

/// Handler for Ctrl+J (send in compose mode, newline otherwise)
pub struct CtrlJHandler {
    pub stream_service: Arc<ChatStreamService>,
    pub event_tx: UnboundedSender<UiEvent>,
}

#[async_trait::async_trait]
impl KeyHandler for CtrlJHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let mut layout_time = last_input_layout_update.unwrap_or_else(Instant::now);

        match handle_ctrl_j_shortcut(
            app,
            term_width,
            term_height,
            &self.stream_service,
            &mut layout_time,
            &self.event_tx,
        )
        .await
        {
            Ok(Some(KeyLoopAction::Break)) => KeyResult::Exit,
            Ok(Some(KeyLoopAction::Continue)) => KeyResult::Continue,
            Ok(None) => KeyResult::Handled,
            Err(_) => KeyResult::NotHandled,
        }
    }
}

/// Handler for Enter key (submit message)
pub struct EnterHandler {
    pub stream_service: Arc<ChatStreamService>,
    pub event_tx: UnboundedSender<UiEvent>,
}

#[async_trait::async_trait]
impl KeyHandler for EnterHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        match handle_enter_key(
            app,
            key,
            term_width,
            term_height,
            &self.stream_service,
            &self.event_tx,
        )
        .await
        {
            Ok(Some(KeyLoopAction::Break)) => KeyResult::Exit,
            Ok(Some(KeyLoopAction::Continue)) => KeyResult::Continue,
            Ok(None) => KeyResult::Handled,
            Err(_) => KeyResult::NotHandled,
        }
    }
}

/// Handler for Alt+Enter key (context-dependent behavior)
pub struct AltEnterHandler {
    pub stream_service: Arc<ChatStreamService>,
    pub event_tx: UnboundedSender<UiEvent>,
}

#[async_trait::async_trait]
impl KeyHandler for AltEnterHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        match handle_enter_key(
            app,
            key,
            term_width,
            term_height,
            &self.stream_service,
            &self.event_tx,
        )
        .await
        {
            Ok(Some(KeyLoopAction::Break)) => KeyResult::Exit,
            Ok(Some(KeyLoopAction::Continue)) => KeyResult::Continue,
            Ok(None) => KeyResult::Handled,
            Err(_) => KeyResult::NotHandled,
        }
    }
}

/// Handler for Ctrl+R (retry last message)
pub struct CtrlRHandler {
    pub stream_service: Arc<ChatStreamService>,
}

#[async_trait::async_trait]
impl KeyHandler for CtrlRHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        if handle_retry_shortcut(app, term_width, term_height, &self.stream_service).await {
            KeyResult::Continue
        } else {
            KeyResult::NotHandled
        }
    }
}

/// Handler for Ctrl+T (external editor)
pub struct CtrlTHandler {
    pub stream_service: Arc<ChatStreamService>,
    pub terminal:
        Arc<Mutex<ratatui::Terminal<crate::ui::osc_backend::OscBackend<std::io::Stdout>>>>,
}

#[async_trait::async_trait]
impl KeyHandler for CtrlTHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let mut terminal_guard = self.terminal.lock().await;
        match handle_external_editor_shortcut(
            app,
            &mut terminal_guard,
            &self.stream_service,
            term_width,
            term_height,
        )
        .await
        {
            Ok(Some(KeyLoopAction::Break)) => KeyResult::Exit,
            Ok(Some(KeyLoopAction::Continue)) => KeyResult::Continue,
            Ok(None) => KeyResult::Handled,
            Err(_e) => {
                // Error is already handled by handle_external_editor_shortcut
                KeyResult::Continue
            }
        }
    }
}

// ============================================================================
// Mode-Specific Handlers
// ============================================================================

/// Handler for edit select mode navigation
pub struct EditSelectHandler;

#[async_trait::async_trait]
impl KeyHandler for EditSelectHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        if handle_edit_select_mode_event(app, key, term_width, term_height).await {
            KeyResult::Continue
        } else {
            KeyResult::NotHandled
        }
    }
}

/// Handler for block select mode navigation
pub struct BlockSelectHandler;

#[async_trait::async_trait]
impl KeyHandler for BlockSelectHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        if handle_block_select_mode_event(app, key, term_width, term_height).await {
            KeyResult::Continue
        } else {
            KeyResult::NotHandled
        }
    }
}

/// Handler for picker navigation (model/theme selection)
pub struct PickerHandler {
    pub event_tx: UnboundedSender<UiEvent>,
}

#[async_trait::async_trait]
impl KeyHandler for PickerHandler {
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        key: &KeyEvent,
        _term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        // Check if there's a picker session before handling the key
        let had_picker_before = {
            let app_guard = app.lock().await;
            app_guard.picker_session().is_some()
        };

        handle_picker_key_event(app, key, &self.event_tx).await;

        // If we had a picker session before, then the key was handled by the picker
        // (even if it resulted in closing the picker, like Esc does)
        if had_picker_before {
            KeyResult::Continue
        } else {
            KeyResult::NotHandled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        wrap_next_index, wrap_previous_index, CommandAutocompleteHandler, KeyHandler, KeyResult,
    };
    use crate::utils::test_utils::create_test_app;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;
    use tokio::runtime::Runtime;
    use tokio::sync::Mutex;

    #[test]
    fn wrap_previous_index_wraps_to_end() {
        assert_eq!(wrap_previous_index(0, 0), None);
        assert_eq!(wrap_previous_index(0, 3), Some(2));
        assert_eq!(wrap_previous_index(2, 3), Some(1));
    }

    #[test]
    fn wrap_next_index_wraps_to_start() {
        assert_eq!(wrap_next_index(0, 0), None);
        assert_eq!(wrap_next_index(2, 3), Some(0));
        assert_eq!(wrap_next_index(1, 3), Some(2));
    }

    #[test]
    fn tab_completion_single_match_completes_command() {
        let suggestions = Arc::new(vec!["model".to_string()]);
        let handler = CommandAutocompleteHandler::with_matcher({
            let suggestions = Arc::clone(&suggestions);
            Arc::new(move |prefix: &str| {
                let lower = prefix.to_ascii_lowercase();
                suggestions
                    .iter()
                    .filter(|name| name.starts_with(&lower))
                    .cloned()
                    .collect()
            })
        });

        let mut app = create_test_app();
        app.ui.set_input_text("/mo".to_string());
        let app = Arc::new(Mutex::new(app));

        let key_event = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let result = handler.handle(&app, &key_event, 80, 24, None).await;
            assert_eq!(result, KeyResult::Handled);

            let app_guard = app.lock().await;
            assert_eq!(app_guard.ui.get_input_text(), "/model ");
        });
    }

    #[test]
    fn tab_completion_multiple_matches_extends_prefix_and_sets_status() {
        let suggestions = Arc::new(vec!["theme".to_string(), "theory".to_string()]);
        let handler = CommandAutocompleteHandler::with_matcher({
            let suggestions = Arc::clone(&suggestions);
            Arc::new(move |prefix: &str| {
                let lower = prefix.to_ascii_lowercase();
                suggestions
                    .iter()
                    .filter(|name| name.starts_with(&lower))
                    .cloned()
                    .collect()
            })
        });

        let mut app = create_test_app();
        app.ui.set_input_text("/t".to_string());
        let app = Arc::new(Mutex::new(app));

        let key_event = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let result = handler.handle(&app, &key_event, 80, 24, None).await;
            assert_eq!(result, KeyResult::Handled);

            let app_guard = app.lock().await;
            assert_eq!(app_guard.ui.get_input_text(), "/the");
            assert_eq!(
                app_guard.ui.status.as_deref(),
                Some("Commands: /theme, /theory")
            );
        });
    }
}
