//! Main chat event loop and UI rendering
//!
//! This module contains the main event loop that handles user input, renders the UI,
//! and manages the chat session.

mod setup;
mod stream;

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
use tui_textarea::{CursorMove, Input as TAInput, Key as TAKey};

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
    let mut terminal = Terminal::new(backend)?;

    // Channel for streaming updates with stream ID
    let (stream_tx, mut rx) = mpsc::unbounded_channel::<(String, u64)>();
    let stream_dispatcher = StreamDispatcher::new(stream_tx);

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
            terminal.draw(|f| ui(f, &mut app_guard))?;
        }
        // Cache terminal size for this tick
        let term_size = terminal.size().unwrap_or_default();
        // Local throttle helper
        let mut update_if_due = |app_guard: &mut App| {
            if last_input_layout_update.elapsed() >= Duration::from_millis(16) {
                app_guard.recompute_input_layout_after_edit(term_size.width);
                last_input_layout_update = Instant::now();
            }
        };

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Always allow Ctrl+C to quit, even when a modal is open
                    if matches!(key.code, KeyCode::Char('c'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        break 'main_loop Ok(());
                    }
                    // Clear ephemeral status with Ctrl+L
                    if matches!(key.code, KeyCode::Char('l'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        let mut app_guard = app.lock().await;
                        app_guard.clear_status();
                        continue;
                    }
                    // Toggle compose mode with F4
                    if matches!(key.code, KeyCode::F(4)) {
                        let mut app_guard = app.lock().await;
                        app_guard.toggle_compose_mode();
                        continue;
                    }
                    // If a picker is open, handle navigation/selection first
                    let picker_state = handle_picker_key_event(&app, &key).await;
                    if picker_state.selection.is_some() || picker_state.has_session {
                        continue;
                    }

                    // Global: Ctrl+B to enter block select mode or cycle upward when active
                    if matches!(key.code, KeyCode::Char('b'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                        && handle_ctrl_b_event(&app, term_size.width, term_size.height).await
                    {
                        continue;
                    }

                    // Global: Ctrl+P to enter edit-select mode (or cycle upward)
                    if matches!(key.code, KeyCode::Char('p'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                        && handle_ctrl_p_event(&app, term_size.width, term_size.height).await
                    {
                        continue;
                    }

                    if handle_edit_select_mode_event(&app, &key, term_size.width, term_size.height)
                        .await
                    {
                        continue;
                    }

                    if handle_block_select_mode_event(&app, &key, term_size.width, term_size.height)
                        .await
                    {
                        continue;
                    }

                    match key.code {
                        KeyCode::Home => {
                            let mut app_guard = app.lock().await;
                            app_guard.scroll_to_top();
                        }
                        KeyCode::End => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.scroll_to_bottom_view(available_height, term_size.width);
                        }
                        KeyCode::PageUp => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.page_up(available_height);
                        }
                        KeyCode::PageDown => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.page_down(available_height, term_size.width);
                        }
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            break 'main_loop Ok(());
                        }
                        KeyCode::Char('d')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+D: exit if input is empty, else delete forward
                            let mut app_guard = app.lock().await;
                            if app_guard.get_input_text().is_empty() {
                                break 'main_loop Ok(());
                            } else {
                                app_guard.apply_textarea_edit_and_recompute(
                                    term_size.width,
                                    |ta| {
                                        ta.input_without_shortcuts(TAInput {
                                            key: TAKey::Delete,
                                            ctrl: false,
                                            alt: false,
                                            shift: false,
                                        });
                                    },
                                );
                            }
                        }
                        KeyCode::Char('t')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Handle CTRL+T for external editor
                            let editor_result = {
                                let mut app_guard = app.lock().await;
                                handle_external_editor(&mut app_guard).await
                            };

                            // Force a full redraw after editor
                            terminal.clear()?;

                            match editor_result {
                                Ok(Some(message)) => {
                                    // Editor returned content, send it immediately
                                    let stream_params = {
                                        let mut app_guard = app.lock().await;

                                        // Re-enable auto-scroll when user sends a new message
                                        app_guard.auto_scroll = true;

                                        // Start new stream (this will cancel any existing stream)
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);

                                        // Update scroll position to ensure latest messages are visible
                                        let terminal_size = terminal.size().unwrap_or_default();
                                        let input_area_height = app_guard
                                            .calculate_input_area_height(terminal_size.width);
                                        let available_height = terminal_size
                                            .height
                                            .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                            .saturating_sub(1); // 1 for title
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal_size.width,
                                        );

                                        StreamParams {
                                            client: app_guard.client.clone(),
                                            base_url: app_guard.base_url.clone(),
                                            api_key: app_guard.api_key.clone(),
                                            model: app_guard.model.clone(),
                                            api_messages,
                                            cancel_token,
                                            stream_id,
                                        }
                                    };

                                    // Send the message to API (deduplicated helper)
                                    stream_dispatcher.spawn(stream_params);
                                }
                                Ok(None) => {
                                    // Editor returned no content or user cancelled
                                    let mut app_guard = app.lock().await;
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let input_area_height =
                                        app_guard.calculate_input_area_height(terminal_size.width);
                                    let available_height = terminal_size
                                        .height
                                        .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                        .saturating_sub(1); // 1 for title
                                    app_guard.update_scroll_position(
                                        available_height,
                                        terminal_size.width,
                                    );
                                }
                                Err(e) => {
                                    let mut app_guard = app.lock().await;
                                    app_guard.set_status(format!("Editor error: {}", e));
                                    // Keep view stable; brief corner status is sufficient
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let input_area_height =
                                        app_guard.calculate_input_area_height(terminal_size.width);
                                    let available_height = terminal_size
                                        .height
                                        .saturating_sub(input_area_height + 2)
                                        .saturating_sub(1);
                                    app_guard.update_scroll_position(
                                        available_height,
                                        terminal_size.width,
                                    );
                                }
                            }
                        }
                        KeyCode::Char('r')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Retry the last bot response with debounce protection
                            let stream_params = {
                                let mut app_guard = app.lock().await;

                                // Check debounce at the event level to prevent any processing
                                let now = std::time::Instant::now();
                                if now.duration_since(app_guard.last_retry_time).as_millis() < 200 {
                                    // Too soon since last retry, ignore completely
                                    continue;
                                }

                                let terminal_size = terminal.size().unwrap_or_default();
                                let input_area_height =
                                    app_guard.calculate_input_area_height(terminal_size.width);
                                let available_height = terminal_size
                                    .height
                                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                    .saturating_sub(1); // 1 for title

                                app_guard
                                    .prepare_retry(available_height, terminal_size.width)
                                    .map(|api_messages| {
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();

                                        StreamParams {
                                            client: app_guard.client.clone(),
                                            base_url: app_guard.base_url.clone(),
                                            api_key: app_guard.api_key.clone(),
                                            model: app_guard.model.clone(),
                                            api_messages,
                                            cancel_token,
                                            stream_id,
                                        }
                                    })
                            };

                            if let Some(params) = stream_params {
                                stream_dispatcher.spawn(params);
                            }
                        }
                        KeyCode::Esc => {
                            let mut app_guard = app.lock().await;
                            if app_guard.file_prompt().is_some() {
                                app_guard.cancel_file_prompt();
                                continue;
                            }
                            if app_guard.in_edit_select_mode() {
                                app_guard.exit_edit_select_mode();
                                continue;
                            }
                            if app_guard.in_place_edit_index().is_some() {
                                app_guard.cancel_in_place_edit();
                                app_guard.clear_input();
                                continue;
                            }
                            if app_guard.is_streaming {
                                // Use the new cancellation mechanism
                                app_guard.cancel_current_stream();
                            }
                        }
                        KeyCode::Enter => {
                            let modifiers = key.modifiers;
                            // Handle filename prompt (Enter: save if new; Alt+Enter: overwrite)
                            {
                                let mut app_guard = app.lock().await;
                                if let Some(prompt) = app_guard.file_prompt().cloned() {
                                    let filename = app_guard.get_input_text().trim().to_string();
                                    if filename.is_empty() {
                                        continue;
                                    }
                                    let overwrite = modifiers.contains(event::KeyModifiers::ALT);
                                    match prompt.kind {
                                        crate::core::app::FilePromptKind::Dump => {
                                            // Use commands helper to dump
                                            let res =
                                                crate::commands::dump_conversation_with_overwrite(
                                                    &app_guard, &filename, overwrite,
                                                );
                                            match res {
                                                Ok(()) => {
                                                    app_guard.set_status(format!(
                                                        "Dumped: {}",
                                                        filename
                                                    ));
                                                    app_guard.cancel_file_prompt();
                                                }
                                                Err(e) => {
                                                    let msg = e.to_string();
                                                    if msg.contains("already exists") {
                                                        app_guard
                                                            .set_status("Log file already exists.");
                                                    } else {
                                                        app_guard.set_status(format!(
                                                            "Dump error: {}",
                                                            msg
                                                        ));
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
                                                        app_guard.set_status(format!(
                                                            "Saved to {}",
                                                            filename
                                                        ));
                                                        app_guard.cancel_file_prompt();
                                                    }
                                                    Err(_e) => {
                                                        app_guard
                                                            .set_status("Error saving code block");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                            }
                            // Compose/newline logic:
                            // - Compose mode: Enter inserts newline; Alt+Enter sends
                            // - Normal mode: Alt+Enter inserts newline; Enter sends
                            {
                                let app_guard = app.lock().await;
                                let compose = app_guard.compose_mode;
                                let alt = modifiers.contains(event::KeyModifiers::ALT);
                                drop(app_guard);
                                let should_insert_newline = if compose { !alt } else { alt };
                                if should_insert_newline {
                                    let mut app_guard = app.lock().await;
                                    app_guard.apply_textarea_edit_and_recompute(
                                        term_size.width,
                                        |ta| {
                                            ta.insert_str("\n");
                                        },
                                    );
                                    continue;
                                }
                            }
                            {
                                // If editing in place, apply changes to history instead of sending
                                {
                                    let mut app_guard = app.lock().await;
                                    if let Some(idx) = app_guard.take_in_place_edit_index() {
                                        // Apply edit to the selected user message
                                        if idx < app_guard.messages.len()
                                            && app_guard.messages[idx].role == "user"
                                        {
                                            let new_text = app_guard.get_input_text().to_string();
                                            app_guard.messages[idx].content = new_text;
                                            app_guard.invalidate_prewrap_cache();
                                            // Rewrite log file to reflect in-place edit
                                            let _ = app_guard
                                                .logging
                                                .rewrite_log_without_last_response(
                                                    &app_guard.messages,
                                                );
                                        }
                                        app_guard.clear_input();
                                        continue;
                                    }
                                }
                                let (
                                    should_send_to_api,
                                    api_messages,
                                    client,
                                    model,
                                    api_key,
                                    base_url,
                                    cancel_token,
                                    stream_id,
                                ) = {
                                    let mut app_guard = app.lock().await;
                                    if app_guard.get_input_text().trim().is_empty() {
                                        continue;
                                    }

                                    let input_text = app_guard.get_input_text().to_string();
                                    app_guard.clear_input();

                                    // Process input for commands
                                    match process_input(&mut app_guard, &input_text) {
                                        CommandResult::Continue => {
                                            // Command was processed, don't send to API
                                            // Update scroll position to ensure latest messages are visible
                                            let term_size = terminal.size().unwrap_or_default();
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                            continue;
                                        }
                                        CommandResult::OpenModelPicker => {
                                            // Open model picker asynchronously
                                            match app_guard.open_model_picker().await {
                                                Ok(_) => {
                                                    // Status messages not needed - help is shown in-dialog
                                                }
                                                Err(e) => {
                                                    app_guard.set_status(format!(
                                                        "Model picker error: {}",
                                                        e
                                                    ));
                                                }
                                            }
                                            continue;
                                        }
                                        CommandResult::OpenProviderPicker => {
                                            // Open provider picker
                                            app_guard.open_provider_picker();
                                            // Status messages not needed - help is shown in-dialog
                                            continue;
                                        }
                                        CommandResult::ProcessAsMessage(message) => {
                                            // Re-enable auto-scroll when user sends a new message
                                            app_guard.auto_scroll = true;

                                            // Start new stream (this will cancel any existing stream)
                                            let (cancel_token, stream_id) =
                                                app_guard.start_new_stream();
                                            let api_messages = app_guard.add_user_message(message);

                                            // Update scroll position to ensure latest messages are visible
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                terminal.size().unwrap_or_default().width,
                                            );

                                            (
                                                true,
                                                api_messages,
                                                app_guard.client.clone(),
                                                app_guard.model.clone(),
                                                app_guard.api_key.clone(),
                                                app_guard.base_url.clone(),
                                                cancel_token,
                                                stream_id,
                                            )
                                        }
                                    }
                                };

                                if !should_send_to_api {
                                    continue;
                                }

                                stream_dispatcher.spawn(StreamParams {
                                    client,
                                    base_url,
                                    api_key,
                                    model,
                                    api_messages,
                                    cancel_token,
                                    stream_id,
                                });
                            }
                        }
                        // Ctrl+J: newline in normal mode; send in compose mode
                        KeyCode::Char('j')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            let send_now = {
                                let app_guard = app.lock().await;
                                app_guard.compose_mode && app_guard.file_prompt().is_none()
                            };
                            if !send_now {
                                let mut app_guard = app.lock().await;
                                app_guard.apply_textarea_edit_and_recompute(
                                    term_size.width,
                                    |ta| {
                                        ta.insert_str("\n");
                                    },
                                );
                                last_input_layout_update = Instant::now();
                                continue;
                            }
                            // Send path (same as Enter send)
                            let (
                                should_send_to_api,
                                api_messages,
                                client,
                                model,
                                api_key,
                                base_url,
                                cancel_token,
                                stream_id,
                            ) = {
                                let mut app_guard = app.lock().await;
                                if app_guard.get_input_text().trim().is_empty() {
                                    continue;
                                }

                                let input_text = app_guard.get_input_text().to_string();
                                app_guard.clear_input();

                                match process_input(&mut app_guard, &input_text) {
                                    CommandResult::Continue => {
                                        let term_size = terminal.size().unwrap_or_default();
                                        let input_area_height =
                                            app_guard.calculate_input_area_height(term_size.width);
                                        let available_height = app_guard
                                            .calculate_available_height(
                                                term_size.height,
                                                input_area_height,
                                            );
                                        app_guard.update_scroll_position(
                                            available_height,
                                            term_size.width,
                                        );
                                        continue;
                                    }
                                    CommandResult::OpenModelPicker => {
                                        match app_guard.open_model_picker().await {
                                            Ok(_) => {}
                                            Err(e) => app_guard
                                                .set_status(format!("Model picker error: {}", e)),
                                        }
                                        continue;
                                    }
                                    CommandResult::OpenProviderPicker => {
                                        app_guard.open_provider_picker();
                                        continue;
                                    }
                                    CommandResult::ProcessAsMessage(message) => {
                                        app_guard.auto_scroll = true;
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);
                                        let input_area_height =
                                            app_guard.calculate_input_area_height(term_size.width);
                                        let available_height = app_guard
                                            .calculate_available_height(
                                                term_size.height,
                                                input_area_height,
                                            );
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal.size().unwrap_or_default().width,
                                        );

                                        (
                                            true,
                                            api_messages,
                                            app_guard.client.clone(),
                                            app_guard.model.clone(),
                                            app_guard.api_key.clone(),
                                            app_guard.base_url.clone(),
                                            cancel_token,
                                            stream_id,
                                        )
                                    }
                                }
                            };
                            if !should_send_to_api {
                                continue;
                            }
                            stream_dispatcher.spawn(StreamParams {
                                client,
                                base_url,
                                api_key,
                                model,
                                api_messages,
                                cancel_token,
                                stream_id,
                            });
                        }
                        KeyCode::Char('a')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Forward to textarea (beginning of line)
                            let mut app_guard = app.lock().await;
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input(TAInput::from(key));
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Char('e')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Forward to textarea (end of line)
                            let mut app_guard = app.lock().await;
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input(TAInput::from(key));
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Left => {
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = key.modifiers.contains(event::KeyModifiers::SHIFT);
                            if (compose && !shift) || (!compose && shift) {
                                // Move exactly one character left (ignore selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Back));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll left
                                app_guard.horizontal_scroll_offset =
                                    app_guard.horizontal_scroll_offset.saturating_sub(1);
                            }
                        }
                        KeyCode::Right => {
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = key.modifiers.contains(event::KeyModifiers::SHIFT);
                            if (compose && !shift) || (!compose && shift) {
                                // Move exactly one character right (ignore selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Forward));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll right
                                app_guard.horizontal_scroll_offset =
                                    app_guard.horizontal_scroll_offset.saturating_add(1);
                            }
                        }
                        KeyCode::Char(_) => {
                            let mut app_guard = app.lock().await;
                            // Let textarea handle text input, including multi-byte chars
                            app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                                ta.input(TAInput::from(key));
                            });
                        }
                        KeyCode::Delete => {
                            let mut app_guard = app.lock().await;
                            // Forward delete in input area
                            app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                                ta.input_without_shortcuts(TAInput {
                                    key: TAKey::Delete,
                                    ctrl: false,
                                    alt: false,
                                    shift: false,
                                });
                            });
                        }
                        KeyCode::Backspace => {
                            let mut app_guard = app.lock().await;
                            // Use input_without_shortcuts to ensure Backspace always deletes a single char/newline
                            let input = TAInput::from(key);
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input_without_shortcuts(input);
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Up => {
                            let modifiers = key.modifiers;
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = modifiers.contains(event::KeyModifiers::SHIFT);

                            if (compose && !shift) || (!compose && shift) {
                                // Move cursor up exactly one line (no selection)
                                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Up));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll chat history up
                                app_guard.auto_scroll = false;
                                app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let modifiers = key.modifiers;
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = modifiers.contains(event::KeyModifiers::SHIFT);

                            if (compose && !shift) || (!compose && shift) {
                                // Move cursor down exactly one line (no selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Down));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll chat history down
                                app_guard.auto_scroll = false;
                                let input_area_height =
                                    app_guard.calculate_input_area_height(term_size.width);
                                let available_height = term_size
                                    .height
                                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                    .saturating_sub(1); // 1 for title
                                let max_scroll = app_guard
                                    .calculate_max_scroll_offset(available_height, term_size.width);
                                app_guard.scroll_offset =
                                    (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
                            }
                        }
                        _ => {}
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
                        let input_area_height = app_guard
                            .calculate_input_area_height(terminal.size().unwrap_or_default().width);
                        let available_height = terminal
                            .size()
                            .unwrap_or_default()
                            .height
                            .saturating_sub(input_area_height + 2)
                            .saturating_sub(1);
                        let max_scroll = app_guard.calculate_max_scroll_offset(
                            available_height,
                            terminal.size().unwrap_or_default().width,
                        );
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

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}

fn scroll_block_into_view(
    app_guard: &mut App,
    term_width: u16,
    term_height: u16,
    block_start: usize,
) {
    let lines =
        crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &app_guard.messages,
            &app_guard.theme,
            app_guard.markdown_enabled,
            app_guard.syntax_enabled,
            Some(term_width as usize),
        );
    let input_area_height = app_guard.calculate_input_area_height(term_width);
    let available_height = term_height
        .saturating_sub(input_area_height + 2)
        .saturating_sub(1);
    let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
        &lines,
        term_width,
        available_height,
        block_start,
    );
    let max_scroll = app_guard.calculate_max_scroll_offset(available_height, term_width);
    app_guard.scroll_offset = desired.min(max_scroll);
}

async fn handle_ctrl_p_event(app: &Arc<Mutex<App>>, term_width: u16, term_height: u16) -> bool {
    let mut app_guard = app.lock().await;

    if app_guard.last_user_message_index().is_none() {
        app_guard.set_status("No user messages");
        return true;
    }

    if app_guard.in_edit_select_mode() {
        if let Some(current) = app_guard.selected_user_message_index() {
            if let Some(prev) = app_guard
                .prev_user_message_index(current)
                .or_else(|| app_guard.last_user_message_index())
            {
                app_guard.set_selected_user_message_index(prev);
            }
        } else if let Some(last) = app_guard.last_user_message_index() {
            app_guard.set_selected_user_message_index(last);
        }
    } else {
        app_guard.enter_edit_select_mode();
        if let Some(last) = app_guard.last_user_message_index() {
            app_guard.set_selected_user_message_index(last);
        }
    }

    if let Some(idx) = app_guard.selected_user_message_index() {
        app_guard.scroll_index_into_view(idx, term_width, term_height);
    }

    true
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
        }
        _ => {}
    }

    true
}

async fn handle_ctrl_b_event(app: &Arc<Mutex<App>>, term_width: u16, term_height: u16) -> bool {
    let mut app_guard = app.lock().await;
    if !app_guard.markdown_enabled {
        app_guard.set_status("Markdown disabled (/markdown on)");
        return true;
    }

    let blocks = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
        &app_guard.messages,
        &app_guard.theme,
        Some(term_width as usize),
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        app_guard.syntax_enabled,
    );

    if app_guard.in_block_select_mode() {
        if let Some(cur) = app_guard.selected_block_index() {
            let total = blocks.len();
            if let Some(next) = wrap_previous_index(cur, total) {
                app_guard.set_selected_block_index(next);
                if let Some((start, _len, _)) = blocks.get(next) {
                    scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                }
            }
        }
    } else if blocks.is_empty() {
        app_guard.set_status("No code blocks");
    } else {
        let last = blocks.len().saturating_sub(1);
        app_guard.enter_block_select_mode(last);
        if let Some((start, _len, _)) = blocks.get(last) {
            scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
        }
    }

    true
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
        }
        _ => {}
    }

    true
}

struct PickerEventResult {
    selection: Option<String>,
    has_session: bool,
}

async fn handle_picker_key_event(
    app: &Arc<Mutex<App>>,
    key: &event::KeyEvent,
) -> PickerEventResult {
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

    let has_session = app_guard.picker_session().is_some();
    PickerEventResult {
        selection,
        has_session,
    }
}

fn wrap_previous_index(current: usize, total: usize) -> Option<usize> {
    if total == 0 {
        None
    } else if current == 0 {
        Some(total - 1)
    } else {
        Some(current - 1)
    }
}

fn wrap_next_index(current: usize, total: usize) -> Option<usize> {
    if total == 0 {
        None
    } else {
        Some((current + 1) % total)
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
