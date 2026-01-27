//! Keybinding handler implementations
//!
//! This module contains all the keybinding handlers organized by functionality:
//! - Basic system operations (quit, clear, escape)
//! - Navigation (arrows, home/end, page up/down)
//! - Text editing (typing, cursor movement, deletion)
//! - Mode switching (block select, edit select)
//! - Complex operations (external editor, message submission)
//! - Mode-specific handlers (picker, edit select, block select)

use crate::core::app::ui_state::{EditSelectTarget, VerticalCursorDirection};
use crate::core::app::{App, AppAction, AppActionContext, AppActionDispatcher, InspectMode};
use crate::core::chat_stream::ChatStreamService;
use crate::core::message::ROLE_ASSISTANT;
use crate::mcp::permissions::ToolPermissionDecision;
use crate::ui::chat_loop::keybindings::registry::{KeyHandler, KeyResult};
use crate::ui::chat_loop::modes::{
    handle_block_select_mode_event, handle_ctrl_j_shortcut, handle_edit_select_mode_event,
    handle_enter_key, handle_external_editor_shortcut, handle_picker_key_event,
};
use crate::ui::chat_loop::{AppHandle, KeyLoopAction};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::debug;
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
        _app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
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
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let should_clear = app.read(|app| app.ui.status.is_some()).await;

        if should_clear {
            let ctx = AppActionContext {
                term_width,
                term_height,
            };
            dispatcher.dispatch_many([AppAction::ClearStatus], ctx);
        }
        KeyResult::Handled
    }
}

/// Handler for Ctrl+O (inspect tool calls/results).
pub struct CtrlOHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlOHandler {
    async fn handle(
        &self,
        _app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        dispatcher.dispatch_many(
            [AppAction::InspectToolResults],
            AppActionContext {
                term_width,
                term_height,
            },
        );
        KeyResult::Handled
    }
}

/// Handler for F4 (toggle compose mode)
pub struct F4Handler;

#[async_trait::async_trait]
impl KeyHandler for F4Handler {
    async fn handle(
        &self,
        _app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let ctx = AppActionContext {
            term_width,
            term_height,
        };
        dispatcher.dispatch_many([AppAction::ToggleComposeMode], ctx);
        KeyResult::Handled
    }
}

/// Handler for Escape key
pub struct EscapeHandler;

#[async_trait::async_trait]
impl KeyHandler for EscapeHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let actions = app
            .read(|app| {
                let mut actions = Vec::new();
                if app.inspect_state().is_some() {
                    actions.push(AppAction::PickerEscape);
                } else if app.ui.file_prompt().is_some() {
                    actions.push(AppAction::CancelFilePrompt);
                } else if app.ui.mcp_prompt_input().is_some() {
                    actions.push(AppAction::CancelMcpPromptInput);
                } else if app.ui.in_place_edit_index().is_some() {
                    actions.push(AppAction::CancelInPlaceEdit);
                } else if app.ui.is_streaming {
                    actions.push(AppAction::CancelStreaming);
                }
                actions
            })
            .await;

        if actions.is_empty() {
            return KeyResult::NotHandled;
        }

        let ctx = AppActionContext {
            term_width,
            term_height,
        };
        dispatcher.dispatch_many(actions, ctx);
        KeyResult::Handled
    }
}

/// Handler for Ctrl+D
pub struct CtrlDHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlDHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        _term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let is_empty = app.read(|app| app.ui.get_input_text().is_empty()).await;
        if is_empty {
            app.update(|app| {
                app.ui.print_transcript_on_exit = true;
            })
            .await;
            KeyLoopAction::Break.into()
        } else {
            app.update(|app| {
                app.ui.focus_input();
                app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
                    ta.input_without_shortcuts(TAInput {
                        key: TAKey::Delete,
                        ctrl: false,
                        alt: false,
                        shift: false,
                    });
                });
            })
            .await;
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
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut last_update = last_input_layout_update.unwrap_or_else(Instant::now);
        let inspect_active = app.read(|app| app.inspect_state().is_some()).await;
        if inspect_active {
            let page_lines = term_height.saturating_sub(8).max(1) as i32;
            let action = match key.code {
                KeyCode::PageUp => AppAction::PickerInspectScroll { lines: -page_lines },
                KeyCode::PageDown => AppAction::PickerInspectScroll { lines: page_lines },
                KeyCode::Home => AppAction::PickerInspectScrollToStart,
                KeyCode::End => AppAction::PickerInspectScrollToEnd,
                _ => return KeyResult::NotHandled,
            };
            dispatcher.dispatch_many(
                [action],
                AppActionContext {
                    term_width,
                    term_height,
                },
            );
            return KeyResult::Handled;
        }

        app.update(|app| match key.code {
            KeyCode::Home => {
                if app.ui.is_input_focused() {
                    app.ui.move_cursor_to_visual_line_start(term_width);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    app.ui.scroll_to_top();
                }
                KeyResult::Handled
            }
            KeyCode::End => {
                if app.ui.is_input_focused() {
                    app.ui.move_cursor_to_visual_line_end(term_width);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    let input_area_height = app.ui.calculate_input_area_height(term_width);
                    let available_height = {
                        let conversation = app.conversation();
                        conversation.calculate_available_height(term_height, input_area_height)
                    };
                    app.ui.scroll_to_bottom_view(available_height, term_width);
                }
                KeyResult::Handled
            }
            KeyCode::PageUp => {
                if app.ui.is_input_focused() {
                    let page_height = app.ui.calculate_input_area_height(term_width);
                    let steps = usize::from(page_height.saturating_sub(1).max(1));
                    if app.ui.move_cursor_page_in_wrapped_input(
                        term_width,
                        VerticalCursorDirection::Up,
                        steps,
                    ) {
                        recompute_input_layout_if_due(app, term_width, &mut last_update);
                    }
                } else {
                    let input_area_height = app.ui.calculate_input_area_height(term_width);
                    let available_height = {
                        let conversation = app.conversation();
                        conversation.calculate_available_height(term_height, input_area_height)
                    };
                    app.ui.page_up(available_height);
                }
                KeyResult::Handled
            }
            KeyCode::PageDown => {
                if app.ui.is_input_focused() {
                    let page_height = app.ui.calculate_input_area_height(term_width);
                    let steps = usize::from(page_height.saturating_sub(1).max(1));
                    if app.ui.move_cursor_page_in_wrapped_input(
                        term_width,
                        VerticalCursorDirection::Down,
                        steps,
                    ) {
                        recompute_input_layout_if_due(app, term_width, &mut last_update);
                    }
                } else {
                    let input_area_height = app.ui.calculate_input_area_height(term_width);
                    let available_height = {
                        let conversation = app.conversation();
                        conversation.calculate_available_height(term_height, input_area_height)
                    };
                    app.ui.page_down(available_height, term_width);
                }
                KeyResult::Handled
            }
            _ => KeyResult::NotHandled,
        })
        .await
    }
}

/// Handler for arrow keys (Up, Down, Left, Right)
pub struct ArrowKeyHandler;

#[async_trait::async_trait]
impl KeyHandler for ArrowKeyHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut last_update = last_input_layout_update.unwrap_or_else(Instant::now);
        let inspect_mode = app
            .read(|app| app.inspect_state().map(|state| state.mode))
            .await;
        if let Some(mode) = inspect_mode {
            let action = match key.code {
                KeyCode::Up => Some(AppAction::PickerInspectScroll { lines: -1 }),
                KeyCode::Down => Some(AppAction::PickerInspectScroll { lines: 1 }),
                KeyCode::Left => match mode {
                    InspectMode::ToolCalls { .. } => {
                        Some(AppAction::InspectToolResultsStep { delta: -1 })
                    }
                    InspectMode::Static => None,
                },
                KeyCode::Right => match mode {
                    InspectMode::ToolCalls { .. } => {
                        Some(AppAction::InspectToolResultsStep { delta: 1 })
                    }
                    InspectMode::Static => None,
                },
                _ => None,
            };
            if let Some(action) = action {
                dispatcher.dispatch_many(
                    [action],
                    AppActionContext {
                        term_width,
                        term_height,
                    },
                );
            }
            if matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
            ) {
                return KeyResult::Handled;
            }
        }

        app.update(|app| match key.code {
            KeyCode::Left => {
                if app.ui.is_input_focused() {
                    app.ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Back));
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    app.ui.horizontal_scroll_offset =
                        app.ui.horizontal_scroll_offset.saturating_sub(1);
                }
                KeyResult::Handled
            }
            KeyCode::Right => {
                if app.ui.is_input_focused() {
                    app.ui
                        .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Forward));
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    app.ui.horizontal_scroll_offset =
                        app.ui.horizontal_scroll_offset.saturating_add(1);
                }
                KeyResult::Handled
            }
            KeyCode::Up => {
                if app.ui.is_input_focused() {
                    app.ui
                        .move_cursor_in_wrapped_input(term_width, VerticalCursorDirection::Up);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    app.ui.auto_scroll = false;
                    app.ui.scroll_offset = app.ui.scroll_offset.saturating_sub(1);
                }
                KeyResult::Handled
            }
            KeyCode::Down => {
                if app.ui.is_input_focused() {
                    app.ui
                        .move_cursor_in_wrapped_input(term_width, VerticalCursorDirection::Down);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                } else {
                    app.ui.auto_scroll = false;
                    let input_area_height = app.ui.calculate_input_area_height(term_width);
                    let available_height = {
                        let conversation = app.conversation();
                        conversation.calculate_available_height(term_height, input_area_height)
                    };
                    let max_scroll = app
                        .ui
                        .calculate_max_scroll_offset(available_height, term_width);
                    app.ui.scroll_offset = (app.ui.scroll_offset.saturating_add(1)).min(max_scroll);
                }
                KeyResult::Handled
            }
            _ => KeyResult::NotHandled,
        })
        .await
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
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        _term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        let mut last_update = last_input_layout_update.unwrap_or_else(Instant::now);

        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.update(|app| {
                    app.ui.focus_input();
                    app.ui.move_cursor_to_visual_line_start(term_width);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                    KeyResult::Handled
                })
                .await
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.update(|app| {
                    app.ui.focus_input();
                    app.ui.move_cursor_to_visual_line_end(term_width);
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                    KeyResult::Handled
                })
                .await
            }
            KeyCode::Char(c) => {
                if c == 'j' && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return KeyResult::NotHandled;
                }
                app.update(|app| {
                    app.ui.focus_input();
                    app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
                        ta.input(TAInput::from(*key));
                    });
                    KeyResult::Handled
                })
                .await
            }
            KeyCode::Delete => {
                app.update(|app| {
                    app.ui.focus_input();
                    app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
                        ta.input_without_shortcuts(TAInput {
                            key: TAKey::Delete,
                            ctrl: false,
                            alt: false,
                            shift: false,
                        });
                    });
                    KeyResult::Handled
                })
                .await
            }
            KeyCode::Backspace => {
                let input = TAInput::from(*key);
                app.update(|app| {
                    app.ui.focus_input();
                    app.ui.apply_textarea_edit(|ta| {
                        ta.input_without_shortcuts(input);
                    });
                    recompute_input_layout_if_due(app, term_width, &mut last_update);
                    KeyResult::Handled
                })
                .await
            }
            _ => KeyResult::NotHandled,
        }
    }
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
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        app.update(|app| {
            if !app.ui.markdown_enabled {
                app.conversation()
                    .set_status("Markdown disabled (/markdown on)");
                return KeyResult::Handled;
            }

            // Use cached metadata instead of recomputing
            let metadata = app.get_prewrapped_span_metadata_cached(term_width);
            let blocks = crate::ui::span::extract_code_blocks(metadata);

            if app.ui.in_block_select_mode() {
                if let Some(cur) = app.ui.selected_block_index() {
                    let total = blocks.len();
                    if let Some(next) = wrap_previous_index(cur, total) {
                        app.ui.set_selected_block_index(next);
                        if let Some(block) = blocks.get(next) {
                            scroll_block_into_view(app, term_width, term_height, block.start_line);
                        }
                    }
                }
            } else if blocks.is_empty() {
                app.conversation().set_status("No code blocks");
            } else {
                let last = blocks.len().saturating_sub(1);
                app.ui.enter_block_select_mode(last);
                if let Some(block) = blocks.get(last) {
                    scroll_block_into_view(app, term_width, term_height, block.start_line);
                }
            }

            KeyResult::Handled
        })
        .await
    }
}

/// Handler for Ctrl+P (edit select mode)
pub struct CtrlPHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlPHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        app.update(|app| {
            if app.ui.last_user_message_index().is_none() {
                app.conversation().set_status("No user messages");
                return KeyResult::Handled;
            }

            if app.ui.in_edit_select_mode()
                && app.ui.edit_select_target() == Some(EditSelectTarget::User)
            {
                if let Some(current) = app.ui.selected_user_message_index() {
                    let prev = {
                        let ui = &app.ui;
                        ui.prev_user_message_index(current)
                            .or_else(|| ui.last_user_message_index())
                    };
                    if let Some(prev) = prev {
                        app.ui.set_selected_user_message_index(prev);
                    }
                } else if let Some(last) = app.ui.last_user_message_index() {
                    app.ui.set_selected_user_message_index(last);
                }
            } else {
                app.ui.enter_edit_select_mode(EditSelectTarget::User);
                if let Some(last) = app.ui.last_user_message_index() {
                    app.ui.set_selected_user_message_index(last);
                }
            }

            if let Some(idx) = app.ui.selected_user_message_index() {
                app.conversation()
                    .scroll_index_into_view(idx, term_width, term_height);
            }

            KeyResult::Handled
        })
        .await
    }
}

/// Handler for Ctrl+X (assistant edit select mode)
pub struct CtrlXHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlXHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult {
        app.update(|app| {
            if app.ui.last_assistant_message_index().is_none() {
                app.conversation().set_status("No assistant messages");
                return KeyResult::Handled;
            }

            if app.ui.in_edit_select_mode()
                && app.ui.edit_select_target() == Some(EditSelectTarget::Assistant)
            {
                if let Some(current) = app.ui.selected_assistant_message_index() {
                    let prev = {
                        let ui = &app.ui;
                        ui.prev_assistant_message_index(current)
                            .or_else(|| ui.last_assistant_message_index())
                    };
                    if let Some(prev) = prev {
                        app.ui.set_selected_assistant_message_index(prev);
                    }
                } else if let Some(last) = app.ui.last_assistant_message_index() {
                    app.ui.set_selected_assistant_message_index(last);
                }
            } else {
                app.ui.enter_edit_select_mode(EditSelectTarget::Assistant);
                if let Some(last) = app.ui.last_assistant_message_index() {
                    app.ui.set_selected_assistant_message_index(last);
                }
            }

            if let Some(idx) = app.ui.selected_assistant_message_index() {
                app.conversation()
                    .scroll_index_into_view(idx, term_width, term_height);
            }

            KeyResult::Handled
        })
        .await
    }
}

// ============================================================================
// Complex Operation Handlers
// ============================================================================

/// Handler for Ctrl+J (send in compose mode, newline otherwise)
pub struct CtrlJHandler {
    pub stream_service: Arc<ChatStreamService>,
}

#[async_trait::async_trait]
impl KeyHandler for CtrlJHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let mut layout_time = last_input_layout_update.unwrap_or_else(Instant::now);

        match handle_ctrl_j_shortcut(
            dispatcher,
            app,
            term_width,
            term_height,
            &self.stream_service,
            &mut layout_time,
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
}

#[async_trait::async_trait]
impl KeyHandler for EnterHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        match handle_enter_key(
            dispatcher,
            app,
            key,
            term_width,
            term_height,
            &self.stream_service,
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
}

#[async_trait::async_trait]
impl KeyHandler for AltEnterHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        match handle_enter_key(
            dispatcher,
            app,
            key,
            term_width,
            term_height,
            &self.stream_service,
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
pub struct CtrlRHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlRHandler {
    async fn handle(
        &self,
        _app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        dispatcher.dispatch_many(
            [AppAction::RetryLastMessage],
            AppActionContext {
                term_width,
                term_height,
            },
        );
        KeyResult::Continue
    }
}

/// Handler for Ctrl+N (repeat last refine)
pub struct CtrlNHandler;

#[async_trait::async_trait]
impl KeyHandler for CtrlNHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let (last_prompt, can_retry) = app
            .read(|app| {
                let prompt = app.session.last_refine_prompt.clone();
                let can_retry = app
                    .ui
                    .messages
                    .iter()
                    .any(|msg| msg.role == ROLE_ASSISTANT && !msg.content.is_empty());
                (prompt, can_retry)
            })
            .await;

        let ctx = AppActionContext {
            term_width,
            term_height,
        };

        match last_prompt {
            Some(prompt) if can_retry => {
                dispatcher.dispatch_many([AppAction::RefineLastMessage { prompt }], ctx);
                KeyResult::Continue
            }
            Some(_) => {
                dispatcher.dispatch_many(
                    [AppAction::SetStatus {
                        message: "No previous message to refine.".to_string(),
                    }],
                    ctx,
                );
                KeyResult::Handled
            }
            None => {
                dispatcher.dispatch_many(
                    [AppAction::SetStatus {
                        message: "No refine prompt yet (/refine <prompt>).".to_string(),
                    }],
                    ctx,
                );
                KeyResult::Handled
            }
        }
    }
}

/// Handler for Ctrl+T (external editor)
pub struct CtrlTHandler {
    pub terminal:
        Arc<Mutex<ratatui::Terminal<crate::ui::osc_backend::OscBackend<std::io::Stdout>>>>,
}

#[async_trait::async_trait]
impl KeyHandler for CtrlTHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        _key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let mut terminal_guard = self.terminal.lock().await;
        match handle_external_editor_shortcut(
            dispatcher,
            app,
            &mut terminal_guard,
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
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
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
        app: &AppHandle,
        _dispatcher: &AppActionDispatcher,
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
pub struct PickerHandler;

#[async_trait::async_trait]
impl KeyHandler for PickerHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        // Check if there's a picker session before handling the key
        let had_picker_before = app.read(|app| app.picker_session().is_some()).await;

        handle_picker_key_event(app, dispatcher, key, term_width, term_height).await;

        // If we had a picker session before, then the key was handled by the picker
        // (even if it resulted in closing the picker, like Esc does)
        if had_picker_before {
            KeyResult::Continue
        } else {
            KeyResult::NotHandled
        }
    }
}

/// Handler for tool permission prompts (Allow/Deny).
pub struct ToolPromptDecisionHandler;

#[async_trait::async_trait]
impl KeyHandler for ToolPromptDecisionHandler {
    async fn handle(
        &self,
        app: &AppHandle,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        _last_input_layout_update: Option<Instant>,
    ) -> KeyResult {
        let inspect_active = app.read(|app| app.inspect_state().is_some()).await;
        if inspect_active {
            let page_lines = term_height.saturating_sub(8).max(1) as i32;
            let action = match key.code {
                KeyCode::Esc => AppAction::PickerEscape,
                KeyCode::Up => AppAction::PickerInspectScroll { lines: -1 },
                KeyCode::Down => AppAction::PickerInspectScroll { lines: 1 },
                KeyCode::PageUp => AppAction::PickerInspectScroll { lines: -page_lines },
                KeyCode::PageDown => AppAction::PickerInspectScroll { lines: page_lines },
                KeyCode::Home => AppAction::PickerInspectScrollToStart,
                KeyCode::End => AppAction::PickerInspectScrollToEnd,
                _ => return KeyResult::NotHandled,
            };
            dispatcher.dispatch_many(
                [action],
                AppActionContext {
                    term_width,
                    term_height,
                },
            );
            return KeyResult::Handled;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return KeyResult::NotHandled;
        }

        match key.code {
            KeyCode::Enter => {
                debug!("Tool prompt decision: allow once (enter)");
                dispatcher.dispatch_many(
                    [AppAction::ToolPermissionDecision {
                        decision: ToolPermissionDecision::AllowOnce,
                    }],
                    AppActionContext {
                        term_width,
                        term_height,
                    },
                );
                KeyResult::Handled
            }
            KeyCode::Esc => {
                debug!("Tool prompt decision: deny once (esc)");
                dispatcher.dispatch_many(
                    [AppAction::ToolPermissionDecision {
                        decision: ToolPermissionDecision::DenyOnce,
                    }],
                    AppActionContext {
                        term_width,
                        term_height,
                    },
                );
                KeyResult::Handled
            }
            KeyCode::Char(ch) => match ch.to_ascii_lowercase() {
                'a' => {
                    debug!("Tool prompt decision: allow once (a)");
                    dispatcher.dispatch_many(
                        [AppAction::ToolPermissionDecision {
                            decision: ToolPermissionDecision::AllowOnce,
                        }],
                        AppActionContext {
                            term_width,
                            term_height,
                        },
                    );
                    KeyResult::Handled
                }
                's' => {
                    debug!("Tool prompt decision: allow session (s)");
                    dispatcher.dispatch_many(
                        [AppAction::ToolPermissionDecision {
                            decision: ToolPermissionDecision::AllowSession,
                        }],
                        AppActionContext {
                            term_width,
                            term_height,
                        },
                    );
                    KeyResult::Handled
                }
                'd' => {
                    debug!("Tool prompt decision: deny once (d)");
                    dispatcher.dispatch_many(
                        [AppAction::ToolPermissionDecision {
                            decision: ToolPermissionDecision::DenyOnce,
                        }],
                        AppActionContext {
                            term_width,
                            term_height,
                        },
                    );
                    KeyResult::Handled
                }
                'b' => {
                    debug!("Tool prompt decision: block (b)");
                    dispatcher.dispatch_many(
                        [AppAction::ToolPermissionDecision {
                            decision: ToolPermissionDecision::Block,
                        }],
                        AppActionContext {
                            term_width,
                            term_height,
                        },
                    );
                    KeyResult::Handled
                }
                _ => KeyResult::NotHandled,
            },
            _ => KeyResult::NotHandled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        wrap_next_index, wrap_previous_index, ArrowKeyHandler, CtrlLHandler, CtrlNHandler,
        EscapeHandler, F4Handler, KeyHandler, KeyResult,
    };
    use crate::core::app::actions::{
        apply_actions, AppAction, AppActionDispatcher, AppActionEnvelope,
    };
    use crate::core::message::{Message, ROLE_ASSISTANT, ROLE_USER};
    use crate::ui::chat_loop::AppHandle;
    use crate::utils::test_utils::create_test_app;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;
    use tokio::runtime::Runtime;
    use tokio::sync::{mpsc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn test_dispatcher() -> (
        AppActionDispatcher,
        mpsc::UnboundedReceiver<AppActionEnvelope>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (AppActionDispatcher::new(tx), rx)
    }

    #[test]
    fn ctrl_n_handler_dispatches_refine_action() {
        let handler = CtrlNHandler;
        let mut app = create_test_app();
        app.session.last_refine_prompt = Some("Tighten it up".to_string());
        app.ui.messages.push_back(Message {
            role: ROLE_USER.to_string(),
            content: "Question".to_string(),
        });
        app.ui.messages.push_back(Message {
            role: ROLE_ASSISTANT.to_string(),
            content: "Answer".to_string(),
        });

        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Continue);

            let envelope = action_rx.try_recv().expect("expected refine action");
            match envelope.action {
                AppAction::RefineLastMessage { prompt } => {
                    assert_eq!(prompt, "Tighten it up");
                }
                _ => panic!("unexpected action"),
            }
        });
    }

    #[test]
    fn ctrl_n_handler_sets_status_when_prompt_missing() {
        let handler = CtrlNHandler;
        let mut app = create_test_app();
        app.ui.messages.push_back(Message {
            role: ROLE_USER.to_string(),
            content: "Question".to_string(),
        });
        app.ui.messages.push_back(Message {
            role: ROLE_ASSISTANT.to_string(),
            content: "Answer".to_string(),
        });

        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let envelope = action_rx.try_recv().expect("expected status action");
            match envelope.action {
                AppAction::SetStatus { message } => {
                    assert_eq!(message, "No refine prompt yet (/refine <prompt>).");
                }
                _ => panic!("unexpected action"),
            }
        });
    }

    #[test]
    fn ctrl_n_handler_sets_status_when_cannot_retry() {
        let handler = CtrlNHandler;
        let mut app = create_test_app();
        app.session.last_refine_prompt = Some("Polish it".to_string());
        app.ui.messages.push_back(Message {
            role: ROLE_USER.to_string(),
            content: "Question".to_string(),
        });
        // No assistant message added

        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let envelope = action_rx.try_recv().expect("expected status action");
            match envelope.action {
                AppAction::SetStatus { message } => {
                    assert_eq!(message, "No previous message to refine.");
                }
                _ => panic!("unexpected action"),
            }
        });
    }

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
    fn arrow_key_handler_moves_cursor_when_input_focused() {
        let handler = ArrowKeyHandler;
        let mut app = create_test_app();
        app.ui.set_input_text("hi".to_string());
        app.ui.focus_input();

        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, _) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let cursor = app.read(|app| app.ui.get_input_cursor_position()).await;
            assert_eq!(cursor, 1);
        });
    }

    #[test]
    fn arrow_key_handler_scrolls_when_transcript_focused() {
        let handler = ArrowKeyHandler;
        let mut app = create_test_app();
        app.ui.set_input_text("hi".to_string());
        // focus remains on transcript by default
        app.ui.horizontal_scroll_offset = 0;

        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, _) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let (cursor, hscroll) = app
                .read(|app| {
                    (
                        app.ui.get_input_cursor_position(),
                        app.ui.horizontal_scroll_offset,
                    )
                })
                .await;
            // Cursor stays at end while horizontal scroll increases
            assert_eq!(cursor, 2);
            assert_eq!(hscroll, 1);
        });
    }

    #[test]
    fn ctrl_l_handler_emits_clear_status_action() {
        let handler = CtrlLHandler;
        let mut app = create_test_app();
        {
            let mut conversation = app.conversation();
            conversation.set_status("Busy");
        }
        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::ClearStatus)));

            let (commands, status_is_none) = app
                .update(move |app| {
                    let commands = apply_actions(app, envelopes);
                    (commands, app.ui.status.is_none())
                })
                .await;
            assert!(commands.is_empty());
            assert!(status_is_none);
        });
    }

    #[test]
    fn f4_handler_toggles_compose_mode_via_action() {
        let handler = F4Handler;
        let app = AppHandle::new(Arc::new(Mutex::new(create_test_app())));
        let key_event = KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::ToggleComposeMode)));

            let compose_before = app.read(|app| app.ui.compose_mode).await;
            assert!(!compose_before);

            let (commands, compose_after) = app
                .update(move |app| {
                    let commands = apply_actions(app, envelopes);
                    (commands, app.ui.compose_mode)
                })
                .await;
            assert!(commands.is_empty());
            assert!(compose_after);
        });
    }

    #[test]
    fn escape_handler_cancels_file_prompt_via_action() {
        let handler = EscapeHandler;
        let mut app = create_test_app();
        {
            app.ui
                .start_file_prompt_save_block("snippet.rs".into(), "fn main() {}".into());
        }
        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::CancelFilePrompt)));

            let (commands, prompt_cleared, input_empty) = app
                .update(move |app| {
                    let commands = apply_actions(app, envelopes);
                    (
                        commands,
                        app.ui.file_prompt().is_none(),
                        app.ui.get_input_text().is_empty(),
                    )
                })
                .await;
            assert!(commands.is_empty());
            assert!(prompt_cleared);
            assert!(input_empty);
        });
    }

    #[test]
    fn escape_handler_cancels_in_place_edit_via_action() {
        let handler = EscapeHandler;
        let mut app = create_test_app();
        {
            app.ui.start_in_place_edit(0);
            app.ui.set_input_text("editing".into());
        }
        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::CancelInPlaceEdit)));

            let (commands, in_place_cleared, input_empty) = app
                .update(move |app| {
                    let commands = apply_actions(app, envelopes);
                    (
                        commands,
                        app.ui.in_place_edit_index().is_none(),
                        app.ui.get_input_text().is_empty(),
                    )
                })
                .await;
            assert!(commands.is_empty());
            assert!(in_place_cleared);
            assert!(input_empty);
        });
    }

    #[test]
    fn escape_handler_cancels_streaming_via_action() {
        let handler = EscapeHandler;
        let mut app = create_test_app();
        {
            app.session.stream_cancel_token = Some(CancellationToken::new());
            app.ui.begin_streaming();
        }
        let app = AppHandle::new(Arc::new(Mutex::new(app)));
        let key_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let (dispatcher, mut action_rx) = test_dispatcher();
            let result = handler
                .handle(&app, &dispatcher, &key_event, 80, 24, None)
                .await;
            assert_eq!(result, KeyResult::Handled);

            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::CancelStreaming)));

            let (commands, is_streaming, cancel_cleared, interrupted) = app
                .update(move |app| {
                    let commands = apply_actions(app, envelopes);
                    (
                        commands,
                        app.ui.is_streaming,
                        app.session.stream_cancel_token.is_none(),
                        app.ui.stream_interrupted,
                    )
                })
                .await;
            assert!(commands.is_empty());
            assert!(!is_streaming);
            assert!(cancel_cleared);
            assert!(interrupted);
        });
    }
}
