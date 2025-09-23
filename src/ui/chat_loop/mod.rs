//! Main chat event loop and UI rendering
//!
//! This module contains the main event loop that handles user input, renders the UI,
//! and manages the chat session.

mod keybindings;
mod setup;
mod stream;

use self::keybindings::{
    build_mode_aware_registry, scroll_block_into_view, wrap_next_index, wrap_previous_index,
    KeyContext, KeyLoopAction, KeyResult,
};

use self::setup::bootstrap_app;
use self::stream::{StreamDispatcher, StreamParams, STREAM_END_MARKER};

use crate::commands::process_input;
use crate::commands::CommandResult;
use crate::core::app::App;
use crate::ui::renderer::ui;
use crate::utils::editor::handle_external_editor;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    error::Error,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};

fn language_to_extension(lang: Option<&str>) -> &'static str {
    if let Some(l) = lang {
        let l = l.trim().to_ascii_lowercase();
        return match l.as_str() {
            "rs" | "rust" => "rs",
            "py" | "python" => "py",
            "sh" | "bash" | "zsh" => "sh",
            "js" | "javascript" => "js",
            "ts" | "typescript" => "ts",
            "json" => "json",
            "yaml" | "yml" => "yml",
            "toml" => "toml",
            "md" | "markdown" => "md",
            "go" => "go",
            "java" => "java",
            "c" => "c",
            "cpp" | "c++" | "cc" | "cxx" => "cpp",
            "html" => "html",
            "css" => "css",
            "sql" => "sql",
            _ => "txt",
        };
    }
    "txt"
}

/// Helper to generate status suffix for picker actions (persistent vs session-only)
fn status_suffix(is_persistent: bool) -> &'static str {
    if is_persistent {
        " (saved to config)"
    } else {
        " (session only)"
    }
}

pub async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
) -> Result<(), Box<dyn Error>> {
    let app = bootstrap_app(model.clone(), log.clone(), provider.clone(), env_only).await?;

    // Sign-off line (no noisy startup banners)
    println!(
        "Chabeau is in the public domain, forever. Contribute: https://github.com/permacommons/chabeau"
    );
    // Color depth print removed; use CHABEAU_COLOR and README tips when debugging

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Arc::new(Mutex::new(Terminal::new(backend)?));

    // Channel for streaming updates with stream ID
    let (stream_tx, mut rx) = mpsc::unbounded_channel::<(String, u64)>();
    let stream_dispatcher = Arc::new(StreamDispatcher::new(stream_tx));

    // Channel for async event processing
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();

    // Spawn async event reader task
    let event_reader_handle = {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            loop {
                // Use a short timeout to prevent blocking
                if let Ok(true) = event::poll(Duration::from_millis(10)) {
                    match event::read() {
                        Ok(ev) => {
                            // Send event to processing queue
                            if event_tx.send(ev).is_err() {
                                // Channel closed, exit
                                break;
                            }
                        }
                        Err(_) => {
                            // Error reading event, continue
                            continue;
                        }
                    }
                } else {
                    // No events available, yield to other tasks
                    tokio::task::yield_now().await;
                }
            }
        })
    };

    // Initialize mode-aware keybinding registry
    let mode_registry = build_mode_aware_registry(stream_dispatcher.clone(), terminal.clone());

    // Drawing cadence control
    let _last_draw = Instant::now();
    let mut _request_redraw = true;
    let mut last_input_layout_update = Instant::now();
    let _last_tick_instant = Instant::now();
    let _last_input_event: Option<Instant> = None;
    let _pressed_keys: Vec<(String, Instant)> = Vec::new();
    // Perf sampling window (1s) and maxima
    let _window_start = Instant::now();
    let _max_tick_ms: u128 = 0;
    let _max_draw_ms: u128 = 0;
    let _max_input_to_draw_ms: u128 = 0;
    let _max_queue_drain_ms: u128 = 0;
    let _max_poll_delay_ms: u128 = 0;

    // Performance logger (enabled when CHABEAU_PERF_LOG=1)
    // Perf logging disabled

    // Main loop
    let result = 'main_loop: loop {
        let _tick_start = Instant::now();
        {
            let mut app_guard = app.lock().await;
            if app_guard.exit_requested {
                break 'main_loop Ok(());
            }
            let mut terminal_guard = terminal.lock().await;
            terminal_guard.draw(|f| ui(f, &mut app_guard))?;
        }
        // Cache terminal size for this tick
        let term_size = {
            let terminal_guard = terminal.lock().await;
            terminal_guard.size().unwrap_or_default()
        };
        // Handle events from async queue
        while let Ok(ev) = event_rx.try_recv() {
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Determine the current context based on app state
                    let context = {
                        let app_guard = app.lock().await;
                        let picker_open = app_guard.model_picker_state().is_some()
                            || app_guard.theme_picker_state().is_some()
                            || app_guard.provider_picker_state().is_some();
                        KeyContext::from_ui_mode(&app_guard.mode, picker_open)
                    };

                    // Smart routing: check if this should be handled as text input first
                    if mode_registry.should_handle_as_text_input(&key, &context) {
                        // Direct text input - bypass registry and go straight to tui-textarea
                        let mut app_guard = app.lock().await;
                        app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                            ta.input(tui_textarea::Input::from(key));
                        });
                        continue;
                    }

                    // Use mode-aware registry for discrete actions
                    let registry_result = mode_registry
                        .handle_key_event(
                            &app,
                            &key,
                            context.clone(),
                            term_size.width,
                            term_size.height,
                            Some(last_input_layout_update),
                        )
                        .await;

                    // Update the layout time if it was modified
                    if let Some(updated_time) = registry_result.updated_layout_time {
                        last_input_layout_update = updated_time;
                    }

                    // Handle registry results
                    match registry_result.result {
                        KeyResult::Exit => break 'main_loop Ok(()),
                        KeyResult::Continue | KeyResult::Handled => continue,
                        KeyResult::NotHandled => {
                            // Key not handled by registry - silently ignore
                        }
                    }
                }
                Event::Paste(text) => {
                    // Handle paste events - sanitize and add the pasted text to input
                    let mut app_guard = app.lock().await;

                    // Sanitize the pasted text to prevent TUI corruption
                    // Convert tabs to spaces and carriage returns to newlines
                    let sanitized_text = text
                        .replace('\t', "    ") // Convert tabs to 4 spaces
                        .replace('\r', "\n") // Convert carriage returns to newlines
                        .chars()
                        .filter(|&c| {
                            // Allow printable characters and newlines, filter out other control characters
                            c == '\n' || !c.is_control()
                        })
                        .collect::<String>();
                    app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                        ta.insert_str(&sanitized_text);
                    });
                    last_input_layout_update = Instant::now();
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let mut app_guard = app.lock().await;
                        app_guard.auto_scroll = false;
                        app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(3);
                    }
                    MouseEventKind::ScrollDown => {
                        let mut app_guard = app.lock().await;
                        app_guard.auto_scroll = false;
                        let input_area_height =
                            app_guard.calculate_input_area_height(term_size.width);
                        let available_height = term_size
                            .height
                            .saturating_sub(input_area_height + 2)
                            .saturating_sub(1);
                        let max_scroll = app_guard
                            .calculate_max_scroll_offset(available_height, term_size.width);
                        app_guard.scroll_offset =
                            (app_guard.scroll_offset.saturating_add(3)).min(max_scroll);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Handle streaming updates - drain all available messages
        let mut received_any = false;
        while let Ok((content, msg_stream_id)) = rx.try_recv() {
            let mut app_guard = app.lock().await;

            // Only process messages from the current stream
            if msg_stream_id != app_guard.current_stream_id {
                // This message is from an old stream, ignore it
                drop(app_guard);
                continue;
            }

            if content == STREAM_END_MARKER {
                // End of streaming - clear the streaming state and finalize response
                app_guard.finalize_response();
                app_guard.is_streaming = false;
                drop(app_guard);
                received_any = true;
            } else if let Some(err) = content.strip_prefix("<<API_ERROR>>") {
                // Display API/network error in the chat area as a system message
                let error_message = format!("Error: {}", err.trim());
                app_guard.add_system_message(error_message);
                // Stop streaming state, since the request failed
                app_guard.is_streaming = false;
                // Ensure the new system message is visible
                let input_area_height = app_guard.calculate_input_area_height(term_size.width);
                let available_height = term_size
                    .height
                    .saturating_sub(input_area_height + 2)
                    .saturating_sub(1);
                app_guard.update_scroll_position(available_height, term_size.width);
                drop(app_guard);
                received_any = true;
            } else {
                let input_area_height = app_guard.calculate_input_area_height(term_size.width);
                let available_height = term_size
                    .height
                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                    .saturating_sub(1); // 1 for title
                app_guard.append_to_response(&content, available_height, term_size.width);
                drop(app_guard);
                received_any = true;
            }
        }
        if received_any {
            continue; // Force a redraw after processing all updates
        }

        // End of loop tick: log if this frame was slow
        // end of iteration
    };

    // Clean up event reader task
    event_reader_handle.abort();

    // Restore terminal
    disable_raw_mode()?;
    {
        let mut terminal_guard = terminal.lock().await;
        execute!(
            terminal_guard.backend_mut(),
            LeaveAlternateScreen,
            DisableBracketedPaste
        )?;
        terminal_guard.show_cursor()?;
    }

    result
}

async fn handle_edit_select_mode_event(
    app: &Arc<Mutex<App>>,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    let mut app_guard = app.lock().await;
    if !app_guard.in_edit_select_mode() {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app_guard.exit_edit_select_mode();
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(current) = app_guard.selected_user_message_index() {
                if let Some(prev) = app_guard
                    .prev_user_message_index(current)
                    .or_else(|| app_guard.last_user_message_index())
                {
                    app_guard.set_selected_user_message_index(prev);
                    app_guard.scroll_index_into_view(prev, term_width, term_height);
                }
            } else if let Some(last) = app_guard.last_user_message_index() {
                app_guard.set_selected_user_message_index(last);
            }
            true
        }

        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(current) = app_guard.selected_user_message_index() {
                if let Some(next) = app_guard
                    .next_user_message_index(current)
                    .or_else(|| app_guard.first_user_message_index())
                {
                    app_guard.set_selected_user_message_index(next);
                    app_guard.scroll_index_into_view(next, term_width, term_height);
                }
            } else if let Some(last) = app_guard.last_user_message_index() {
                app_guard.set_selected_user_message_index(last);
            }
            true
        }
        KeyCode::Enter => {
            if let Some(idx) = app_guard.selected_user_message_index() {
                if idx < app_guard.messages.len() && app_guard.messages[idx].role == "user" {
                    let content = app_guard.messages[idx].content.clone();
                    app_guard.cancel_current_stream();
                    app_guard.messages.truncate(idx);
                    app_guard.invalidate_prewrap_cache();
                    let _ = app_guard
                        .logging
                        .rewrite_log_without_last_response(&app_guard.messages);
                    app_guard.set_input_text(content);
                    app_guard.exit_edit_select_mode();
                    let input_area_height = app_guard.calculate_input_area_height(term_width);
                    let available_height =
                        app_guard.calculate_available_height(term_height, input_area_height);
                    app_guard.update_scroll_position(available_height, term_width);
                }
            }
            true
        }
        KeyCode::Char('E') | KeyCode::Char('e') => {
            if let Some(idx) = app_guard.selected_user_message_index() {
                if idx < app_guard.messages.len() && app_guard.messages[idx].role == "user" {
                    let content = app_guard.messages[idx].content.clone();
                    app_guard.set_input_text(content);
                    app_guard.start_in_place_edit(idx);
                    app_guard.exit_edit_select_mode();
                }
            }
            true
        }
        KeyCode::Delete => {
            if let Some(idx) = app_guard.selected_user_message_index() {
                if idx < app_guard.messages.len() && app_guard.messages[idx].role == "user" {
                    app_guard.cancel_current_stream();
                    app_guard.messages.truncate(idx);
                    app_guard.invalidate_prewrap_cache();
                    let _ = app_guard
                        .logging
                        .rewrite_log_without_last_response(&app_guard.messages);
                    app_guard.exit_edit_select_mode();
                    let input_area_height = app_guard.calculate_input_area_height(term_width);
                    let available_height =
                        app_guard.calculate_available_height(term_height, input_area_height);
                    app_guard.update_scroll_position(available_height, term_width);
                }
            }
            true
        }

        _ => false, // Key not handled - allow fallback handlers to process it
    }
}

async fn handle_block_select_mode_event(
    app: &Arc<Mutex<App>>,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    let mut app_guard = app.lock().await;
    if !app_guard.in_block_select_mode() {
        return false;
    }

    let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
        &app_guard.messages,
        &app_guard.theme,
        Some(term_width as usize),
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        app_guard.syntax_enabled,
    );

    match key.code {
        KeyCode::Esc => {
            app_guard.exit_block_select_mode();
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(cur) = app_guard.selected_block_index() {
                let total = ranges.len();
                if let Some(next) = wrap_previous_index(cur, total) {
                    app_guard.set_selected_block_index(next);
                    if let Some((start, _len, _)) = ranges.get(next) {
                        scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                    }
                }
            } else if !ranges.is_empty() {
                app_guard.set_selected_block_index(0);
            }
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(cur) = app_guard.selected_block_index() {
                let total = ranges.len();
                if let Some(next) = wrap_next_index(cur, total) {
                    app_guard.set_selected_block_index(next);
                    if let Some((start, _len, _)) = ranges.get(next) {
                        scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                    }
                }
            } else if !ranges.is_empty() {
                app_guard.set_selected_block_index(0);
            }
            true
        }

        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(cur) = app_guard.selected_block_index() {
                if let Some((_start, _len, content)) = ranges.get(cur) {
                    match crate::utils::clipboard::copy_to_clipboard(content) {
                        Ok(()) => app_guard.set_status("Copied code block"),
                        Err(_e) => app_guard.set_status("Clipboard error"),
                    }
                    app_guard.exit_block_select_mode();
                    app_guard.auto_scroll = true;
                    let input_area_height = app_guard.calculate_input_area_height(term_width);
                    let available_height = term_height
                        .saturating_sub(input_area_height + 2)
                        .saturating_sub(1);
                    app_guard.update_scroll_position(available_height, term_width);
                }
            }
            true
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if let Some(cur) = app_guard.selected_block_index() {
                let contents =
                    crate::ui::markdown::compute_codeblock_contents_with_lang(&app_guard.messages);
                if let Some((content, lang)) = contents.get(cur) {
                    use chrono::Utc;
                    use std::fs;
                    let date = Utc::now().format("%Y-%m-%d");
                    let ext = language_to_extension(lang.as_deref());
                    let filename = format!("chabeau-block-{}.{}", date, ext);
                    if std::path::Path::new(&filename).exists() {
                        app_guard.set_status("File already exists.");
                        app_guard.start_file_prompt_save_block(filename, content.clone());
                    } else {
                        match fs::write(&filename, content) {
                            Ok(()) => app_guard.set_status(format!("Saved to {}", filename)),
                            Err(_e) => app_guard.set_status("Error saving code block"),
                        }
                    }
                    app_guard.exit_block_select_mode();
                    app_guard.auto_scroll = true;
                    let input_area_height = app_guard.calculate_input_area_height(term_width);
                    let available_height = term_height
                        .saturating_sub(input_area_height + 2)
                        .saturating_sub(1);
                    app_guard.update_scroll_position(available_height, term_width);
                }
            }
            true
        }
        _ => false, // Key not handled - allow fallback handlers to process it
    }
}

async fn handle_external_editor_shortcut(
    app: &Arc<Mutex<App>>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    stream_dispatcher: &StreamDispatcher,
    term_width: u16,
    term_height: u16,
) -> Result<Option<KeyLoopAction>, String> {
    let editor_result = {
        let mut app_guard = app.lock().await;
        handle_external_editor(&mut app_guard)
            .await
            .map_err(|e| e.to_string())
    };

    terminal.clear().map_err(|e| e.to_string())?;

    match editor_result {
        Ok(Some(message)) => {
            let mut app_guard = app.lock().await;
            let params =
                prepare_stream_params_for_message(&mut app_guard, message, term_width, term_height);
            drop(app_guard);
            stream_dispatcher.spawn(params);
            Ok(Some(KeyLoopAction::Continue))
        }
        Ok(None) => {
            let mut app_guard = app.lock().await;
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height = term_height
                .saturating_sub(input_area_height + 2)
                .saturating_sub(1);
            app_guard.update_scroll_position(available_height, term_width);
            Ok(Some(KeyLoopAction::Continue))
        }
        Err(e) => {
            let error_msg = e.to_string();
            let mut app_guard = app.lock().await;
            app_guard.set_status(format!("Editor error: {}", error_msg));
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height = term_height
                .saturating_sub(input_area_height + 2)
                .saturating_sub(1);
            app_guard.update_scroll_position(available_height, term_width);
            Ok(Some(KeyLoopAction::Continue))
        }
    }
}

async fn process_input_submission(
    app: &Arc<Mutex<App>>,
    input_text: String,
    term_width: u16,
    term_height: u16,
) -> SubmissionResult {
    let mut app_guard = app.lock().await;

    match process_input(&mut app_guard, &input_text) {
        CommandResult::Continue => {
            let input_area_height = app_guard.calculate_input_area_height(term_width);
            let available_height =
                app_guard.calculate_available_height(term_height, input_area_height);
            app_guard.update_scroll_position(available_height, term_width);
            SubmissionResult::Continue
        }
        CommandResult::OpenModelPicker => {
            if let Err(e) = app_guard.open_model_picker().await {
                app_guard.set_status(format!("Model picker error: {}", e));
            }
            SubmissionResult::Continue
        }
        CommandResult::OpenProviderPicker => {
            app_guard.open_provider_picker();
            SubmissionResult::Continue
        }
        CommandResult::ProcessAsMessage(message) => {
            let params =
                prepare_stream_params_for_message(&mut app_guard, message, term_width, term_height);
            SubmissionResult::Spawn(params)
        }
    }
}

async fn handle_enter_key(
    app: &Arc<Mutex<App>>,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
    stream_dispatcher: &StreamDispatcher,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let modifiers = key.modifiers;

    {
        let mut app_guard = app.lock().await;
        if let Some(prompt) = app_guard.file_prompt().cloned() {
            let filename = app_guard.get_input_text().trim().to_string();
            if filename.is_empty() {
                return Ok(Some(KeyLoopAction::Continue));
            }
            let overwrite = modifiers.contains(event::KeyModifiers::ALT);
            match prompt.kind {
                crate::core::app::FilePromptKind::Dump => {
                    let res = crate::commands::dump_conversation_with_overwrite(
                        &app_guard, &filename, overwrite,
                    );
                    match res {
                        Ok(()) => {
                            app_guard.set_status(format!("Dumped: {}", filename));
                            app_guard.cancel_file_prompt();
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("exists") && !overwrite {
                                app_guard.set_status("File exists (Alt+Enter to overwrite)");
                            } else {
                                app_guard.set_status(format!("Dump error: {}", msg));
                            }
                        }
                    }
                }
                crate::core::app::FilePromptKind::SaveCodeBlock => {
                    use std::fs;
                    let exists = std::path::Path::new(&filename).exists();
                    if exists && !overwrite {
                        app_guard.set_status("File already exists.");
                    } else if let Some(content) = prompt.content {
                        match fs::write(&filename, content) {
                            Ok(()) => {
                                app_guard.set_status(format!("Saved to {}", filename));
                                app_guard.cancel_file_prompt();
                            }
                            Err(_e) => {
                                app_guard.set_status("Error saving code block");
                            }
                        }
                    }
                }
            }
            return Ok(Some(KeyLoopAction::Continue));
        }
    }

    let should_insert_newline = {
        let app_guard = app.lock().await;
        let compose = app_guard.compose_mode;
        let alt = modifiers.contains(event::KeyModifiers::ALT);
        if compose {
            !alt
        } else {
            alt
        }
    };

    if should_insert_newline {
        let mut app_guard = app.lock().await;
        app_guard.apply_textarea_edit_and_recompute(term_width, |ta| {
            ta.insert_str("\n");
        });
        return Ok(Some(KeyLoopAction::Continue));
    }

    {
        let mut app_guard = app.lock().await;
        if let Some(idx) = app_guard.take_in_place_edit_index() {
            if idx < app_guard.messages.len() && app_guard.messages[idx].role == "user" {
                let new_text = app_guard.get_input_text().to_string();
                app_guard.messages[idx].content = new_text;
                app_guard.invalidate_prewrap_cache();
                let _ = app_guard
                    .logging
                    .rewrite_log_without_last_response(&app_guard.messages);
            }
            app_guard.clear_input();
            return Ok(Some(KeyLoopAction::Continue));
        }
    }

    let input_text = {
        let mut app_guard = app.lock().await;
        if app_guard.get_input_text().trim().is_empty() {
            return Ok(Some(KeyLoopAction::Continue));
        }
        let text = app_guard.get_input_text().to_string();
        app_guard.clear_input();
        text
    };

    match process_input_submission(app, input_text, term_width, term_height).await {
        SubmissionResult::Continue => Ok(Some(KeyLoopAction::Continue)),
        SubmissionResult::Spawn(params) => {
            stream_dispatcher.spawn(params);
            Ok(Some(KeyLoopAction::Continue))
        }
    }
}

async fn handle_ctrl_j_shortcut(
    app: &Arc<Mutex<App>>,
    term_width: u16,
    term_height: u16,
    stream_dispatcher: &StreamDispatcher,
    last_input_layout_update: &mut Instant,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let send_now = {
        let app_guard = app.lock().await;
        app_guard.compose_mode && app_guard.file_prompt().is_none()
    };

    if !send_now {
        let mut app_guard = app.lock().await;
        app_guard.apply_textarea_edit_and_recompute(term_width, |ta| {
            ta.insert_str("\n");
        });
        *last_input_layout_update = Instant::now();
        return Ok(Some(KeyLoopAction::Continue));
    }

    let input_text = {
        let mut app_guard = app.lock().await;
        if app_guard.get_input_text().trim().is_empty() {
            return Ok(Some(KeyLoopAction::Continue));
        }
        let text = app_guard.get_input_text().to_string();
        app_guard.clear_input();
        text
    };

    match process_input_submission(app, input_text, term_width, term_height).await {
        SubmissionResult::Continue => Ok(Some(KeyLoopAction::Continue)),
        SubmissionResult::Spawn(params) => {
            stream_dispatcher.spawn(params);
            Ok(Some(KeyLoopAction::Continue))
        }
    }
}

async fn handle_retry_shortcut(
    app: &Arc<Mutex<App>>,
    term_width: u16,
    term_height: u16,
    stream_dispatcher: &StreamDispatcher,
) -> bool {
    let maybe_params = {
        let mut app_guard = app.lock().await;
        let now = Instant::now();
        if now.duration_since(app_guard.last_retry_time).as_millis() < 200 {
            return true;
        }

        let input_area_height = app_guard.calculate_input_area_height(term_width);
        let available_height = app_guard.calculate_available_height(term_height, input_area_height);

        app_guard
            .prepare_retry(available_height, term_width)
            .map(|api_messages| {
                let (cancel_token, stream_id) = app_guard.start_new_stream();
                StreamParams {
                    client: app_guard.client.clone(),
                    base_url: app_guard.base_url.clone(),
                    api_key: app_guard.api_key.clone(),
                    provider_name: app_guard.provider_name.clone(),
                    model: app_guard.model.clone(),
                    api_messages,
                    cancel_token,
                    stream_id,
                }
            })
    };

    if let Some(params) = maybe_params {
        stream_dispatcher.spawn(params);
    }

    true
}

fn prepare_stream_params_for_message(
    app_guard: &mut App,
    message: String,
    term_width: u16,
    term_height: u16,
) -> StreamParams {
    app_guard.auto_scroll = true;
    let (cancel_token, stream_id) = app_guard.start_new_stream();
    let api_messages = app_guard.add_user_message(message);
    let input_area_height = app_guard.calculate_input_area_height(term_width);
    let available_height = app_guard.calculate_available_height(term_height, input_area_height);
    app_guard.update_scroll_position(available_height, term_width);

    StreamParams {
        client: app_guard.client.clone(),
        base_url: app_guard.base_url.clone(),
        api_key: app_guard.api_key.clone(),
        provider_name: app_guard.provider_name.clone(),
        model: app_guard.model.clone(),
        api_messages,
        cancel_token,
        stream_id,
    }
}

enum SubmissionResult {
    Continue,
    Spawn(StreamParams),
}

async fn handle_picker_key_event(app: &Arc<Mutex<App>>, key: &event::KeyEvent) {
    let mut app_guard = app.lock().await;
    let current_picker_mode = app_guard.current_picker_mode();
    let provider_name = app_guard.provider_name.clone();

    let selection = if let Some(picker) = app_guard.picker_state_mut() {
        match key.code {
            KeyCode::Esc => {
                match current_picker_mode {
                    Some(crate::core::app::PickerMode::Theme) => {
                        app_guard.revert_theme_preview();
                        app_guard.close_picker();
                    }
                    Some(crate::core::app::PickerMode::Model) => {
                        if app_guard.startup_requires_model {
                            // Startup mandatory model selection
                            app_guard.close_picker();
                            if app_guard.startup_multiple_providers_available {
                                // Go back to provider picker per spec
                                app_guard.startup_requires_model = false;
                                app_guard.startup_requires_provider = true;
                                // Clear provider selection in title bar during startup bounce-back
                                app_guard.provider_name.clear();
                                app_guard.provider_display_name =
                                    "(no provider selected)".to_string();
                                app_guard.api_key.clear();
                                app_guard.base_url.clear();
                                app_guard.open_provider_picker();
                            } else {
                                // Exit app if no alternative provider
                                app_guard.exit_requested = true;
                            }
                        } else {
                            app_guard.revert_model_preview();
                            if app_guard.in_provider_model_transition {
                                app_guard.revert_provider_model_transition();
                                app_guard.set_status("Selection cancelled");
                            }
                            app_guard.close_picker();
                        }
                    }
                    Some(crate::core::app::PickerMode::Provider) => {
                        if app_guard.startup_requires_provider {
                            // Startup mandatory provider selection: exit if cancelled
                            app_guard.close_picker();
                            app_guard.exit_requested = true;
                        } else {
                            app_guard.revert_provider_preview();
                            app_guard.close_picker();
                        }
                    }
                    _ => {}
                }
                None
            }
            KeyCode::Up => {
                picker.move_up();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::Down => {
                picker.move_down();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::Char('k') => {
                picker.move_up();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::Char('j') if !key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                picker.move_down();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::Home => {
                picker.move_to_start();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::End => {
                picker.move_to_end();
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    picker.selected_id().map(|s| s.to_string())
                } else {
                    None
                }
            }
            KeyCode::F(6) => {
                picker.cycle_sort_mode();
                // Re-sort and update title
                let _ = picker; // Release borrow
                app_guard.sort_picker_items();
                app_guard.update_picker_title();
                None
            }
            // Apply selection: Enter (Alt=Persist) or Ctrl+J (Persist)
            KeyCode::Enter | KeyCode::Char('j')
                if key.code == KeyCode::Enter
                    || key.modifiers.contains(event::KeyModifiers::CONTROL) =>
            {
                let is_persistent = if key.code == KeyCode::Enter {
                    key.modifiers.contains(event::KeyModifiers::ALT)
                } else {
                    true
                };
                // Common apply path
                // Theme
                if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                        let res = if is_persistent {
                            app_guard.apply_theme_by_id(&id)
                        } else {
                            app_guard.apply_theme_by_id_session_only(&id)
                        };
                        match res {
                            Ok(_) => app_guard.set_status(format!(
                                "Theme set: {}{}",
                                id,
                                status_suffix(is_persistent)
                            )),
                            Err(_e) => app_guard.set_status("Theme error"),
                        }
                    }
                    app_guard.close_picker();
                    Some("__picker_handled__".to_string())
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Model) {
                    if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                        let persist = is_persistent && !app_guard.startup_env_only;
                        let res = if persist {
                            app_guard.apply_model_by_id_persistent(&id)
                        } else {
                            app_guard.apply_model_by_id(&id);
                            Ok(())
                        };
                        match res {
                            Ok(_) => {
                                app_guard.set_status(format!(
                                    "Model set: {}{}",
                                    id,
                                    status_suffix(persist)
                                ));
                                if app_guard.in_provider_model_transition {
                                    app_guard.complete_provider_model_transition();
                                }
                                if app_guard.startup_requires_model {
                                    app_guard.startup_requires_model = false;
                                }
                            }
                            Err(e) => app_guard.set_status(format!("Model error: {}", e)),
                        }
                    }
                    app_guard.close_picker();
                    Some("__picker_handled__".to_string())
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Provider) {
                    if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                        let (res, should_open_model_picker) = if is_persistent {
                            app_guard.apply_provider_by_id_persistent(&id)
                        } else {
                            app_guard.apply_provider_by_id(&id)
                        };
                        match res {
                            Ok(_) => {
                                app_guard.set_status(format!(
                                    "Provider set: {}{}",
                                    id,
                                    status_suffix(is_persistent)
                                ));
                                app_guard.close_picker();
                                if should_open_model_picker {
                                    if app_guard.startup_requires_provider {
                                        app_guard.startup_requires_provider = false;
                                        app_guard.startup_requires_model = true;
                                    }
                                    let app_clone = app.clone();
                                    tokio::spawn(async move {
                                        let mut app_guard = app_clone.lock().await;
                                        let _ = app_guard.open_model_picker().await;
                                    });
                                }
                            }
                            Err(e) => {
                                app_guard.set_status(format!("Provider error: {}", e));
                                app_guard.close_picker();
                            }
                        }
                    }
                    Some("__picker_handled__".to_string())
                } else {
                    Some("__picker_handled__".to_string())
                }
            }
            // Ctrl+J: persist selection to config (documented only in /help)
            KeyCode::Char('j') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                match current_picker_mode {
                    Some(crate::core::app::PickerMode::Theme) => {
                        if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                            match app_guard.apply_theme_by_id(&id) {
                                Ok(_) => app_guard.set_status(format!(
                                    "Theme set: {}{}",
                                    id,
                                    status_suffix(true)
                                )),
                                Err(_e) => app_guard.set_status("Theme error"),
                            }
                        }
                        app_guard.close_picker();
                        Some("__picker_handled__".to_string())
                    }
                    Some(crate::core::app::PickerMode::Model) => {
                        if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                            let persist = !app_guard.startup_env_only;
                            let res = if persist {
                                app_guard.apply_model_by_id_persistent(&id)
                            } else {
                                app_guard.apply_model_by_id(&id);
                                Ok(())
                            };
                            match res {
                                Ok(_) => {
                                    app_guard.set_status(format!(
                                        "Model set: {}{}",
                                        id,
                                        status_suffix(persist)
                                    ));
                                    if app_guard.in_provider_model_transition {
                                        app_guard.complete_provider_model_transition();
                                    }
                                    if app_guard.startup_requires_model {
                                        app_guard.startup_requires_model = false;
                                    }
                                }
                                Err(e) => app_guard.set_status(format!("Model error: {}", e)),
                            }
                        }
                        app_guard.close_picker();
                        Some("__picker_handled__".to_string())
                    }
                    Some(crate::core::app::PickerMode::Provider) => {
                        if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                            let (res, should_open_model_picker) =
                                app_guard.apply_provider_by_id_persistent(&id);
                            match res {
                                Ok(_) => {
                                    app_guard.set_status(format!(
                                        "Provider set: {}{}",
                                        id,
                                        status_suffix(true)
                                    ));
                                    app_guard.close_picker();
                                    if should_open_model_picker {
                                        let app_clone = app.clone();
                                        tokio::spawn(async move {
                                            let mut app_guard = app_clone.lock().await;
                                            let _ = app_guard.open_model_picker().await;
                                        });
                                    }
                                }
                                Err(e) => app_guard.set_status(format!("Provider error: {}", e)),
                            }
                        }
                        Some("__picker_handled__".to_string())
                    }
                    _ => Some("__picker_handled__".to_string()),
                }
            }
            KeyCode::Delete => {
                // Del key to unset defaults - only works if current selection is a default (has *)
                if let Some(selected_item) = picker.get_selected_item() {
                    if selected_item.label.ends_with('*') {
                        let item_id = selected_item.id.clone();

                        // Release picker borrow by ending the scope
                        let _ = picker;

                        let result = match current_picker_mode {
                            Some(crate::core::app::PickerMode::Model) => {
                                app_guard.unset_default_model(&provider_name)
                            }
                            Some(crate::core::app::PickerMode::Theme) => {
                                app_guard.unset_default_theme()
                            }
                            Some(crate::core::app::PickerMode::Provider) => {
                                app_guard.unset_default_provider()
                            }
                            _ => Err("Unknown picker mode".to_string()),
                        };
                        match result {
                            Ok(_) => {
                                app_guard.set_status(format!("Removed default: {}", item_id));
                                // Refresh the picker to remove the asterisk
                                match current_picker_mode {
                                    Some(crate::core::app::PickerMode::Model) => {
                                        // Store app reference for async refresh
                                        let app_clone = app.clone();
                                        tokio::spawn(async move {
                                            let mut app_guard = app_clone.lock().await;
                                            let _ = app_guard.open_model_picker().await;
                                        });
                                    }
                                    Some(crate::core::app::PickerMode::Theme) => {
                                        app_guard.open_theme_picker();
                                    }
                                    Some(crate::core::app::PickerMode::Provider) => {
                                        app_guard.open_provider_picker();
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                app_guard.set_status(format!("Error removing default: {}", e));
                            }
                        }
                    } else {
                        app_guard.set_status("Del key only works on default items (marked with *)");
                    }
                }
                None
            }
            KeyCode::Backspace => {
                if current_picker_mode == Some(crate::core::app::PickerMode::Model) {
                    if let Some(state) = app_guard.model_picker_state_mut() {
                        if !state.search_filter.is_empty() {
                            state.search_filter.pop();
                            app_guard.filter_models();
                        }
                    }
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    if let Some(state) = app_guard.theme_picker_state_mut() {
                        if !state.search_filter.is_empty() {
                            state.search_filter.pop();
                            app_guard.filter_themes();
                        }
                    }
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Provider) {
                    if let Some(state) = app_guard.provider_picker_state_mut() {
                        if !state.search_filter.is_empty() {
                            state.search_filter.pop();
                            app_guard.filter_providers();
                        }
                    }
                }
                None
            }
            KeyCode::Char(c) => {
                if current_picker_mode == Some(crate::core::app::PickerMode::Model) {
                    // Add character to filter for model picker
                    if !c.is_control() {
                        if let Some(state) = app_guard.model_picker_state_mut() {
                            state.search_filter.push(c);
                            app_guard.filter_models();
                        }
                    }
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
                    // Add character to filter for theme picker
                    if !c.is_control() {
                        if let Some(state) = app_guard.theme_picker_state_mut() {
                            state.search_filter.push(c);
                            app_guard.filter_themes();
                        }
                    }
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Provider) {
                    // Add character to filter for provider picker
                    if !c.is_control() {
                        if let Some(state) = app_guard.provider_picker_state_mut() {
                            state.search_filter.push(c);
                            app_guard.filter_providers();
                        }
                    }
                }
                None
            }
            // No block actions in picker modes
            _ => None,
        }
    } else {
        None
    };

    if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
        if let Some(selected_id) = selection.as_ref() {
            if selected_id != "__picker_handled__" {
                app_guard.preview_theme_by_id(selected_id);
            }
        }
    }

    // Theme preview handling
    if current_picker_mode == Some(crate::core::app::PickerMode::Theme) {
        if let Some(selected_id) = selection.as_ref() {
            if selected_id != "__picker_handled__" {
                app_guard.preview_theme_by_id(selected_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{wrap_next_index, wrap_previous_index};

    #[test]
    fn wrap_previous_handles_empty() {
        assert_eq!(wrap_previous_index(0, 0), None);
    }

    #[test]
    fn wrap_previous_wraps_to_end() {
        assert_eq!(wrap_previous_index(0, 5), Some(4));
    }

    #[test]
    fn wrap_next_handles_empty() {
        assert_eq!(wrap_next_index(0, 0), None);
    }

    #[test]
    fn wrap_next_wraps_to_start() {
        assert_eq!(wrap_next_index(4, 5), Some(0));
    }
}
