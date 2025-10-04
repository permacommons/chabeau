//! Main chat event loop and UI rendering
//!
//! This module contains the main event loop that handles user input, renders the UI,
//! and manages the chat session.

mod keybindings;
mod setup;

use self::keybindings::{
    build_mode_aware_registry, scroll_block_into_view, wrap_next_index, wrap_previous_index,
    KeyContext, KeyLoopAction, KeyResult, ModeAwareRegistry,
};

use self::setup::bootstrap_app;

use crate::core::chat_stream::{ChatStreamService, StreamMessage, StreamParams};

use crate::api::models::fetch_models;
use crate::commands::process_input;
use crate::commands::CommandResult;
use crate::core::app::ui_state::FilePromptKind;
use crate::core::app::{
    apply_actions, App, AppAction, AppActionContext, AppActionDispatcher, AppActionEnvelope,
    ModelPickerRequest,
};
use crate::ui::osc_backend::OscBackend;
use crate::ui::renderer::ui;
use crate::utils::editor::{launch_external_editor, ExternalEditorOutcome};
use ratatui::crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::Size, Terminal};
use std::{
    error::Error,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};

type SharedTerminal = Arc<Mutex<Terminal<OscBackend<io::Stdout>>>>;

#[derive(Debug)]
pub enum UiEvent {
    Crossterm(Event),
    RequestRedraw,
}

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

fn spawn_model_picker_loader(
    dispatcher: AppActionDispatcher,
    event_tx: mpsc::UnboundedSender<UiEvent>,
    request: ModelPickerRequest,
) {
    let _ = event_tx.send(UiEvent::RequestRedraw);
    tokio::spawn(async move {
        let ModelPickerRequest {
            client,
            base_url,
            api_key,
            provider_name,
            default_model_for_provider,
        } = request;

        let fetch_result = fetch_models(&client, &base_url, &api_key, &provider_name)
            .await
            .map_err(|e| e.to_string());

        let action = match fetch_result {
            Ok(models_response) => AppAction::ModelPickerLoaded {
                default_model_for_provider,
                models_response,
            },
            Err(e) => AppAction::ModelPickerLoadFailed { error: e },
        };

        dispatcher.dispatch_many([action], AppActionContext::default());
        let _ = event_tx.send(UiEvent::RequestRedraw);
    });
}

async fn is_exit_requested(app: &Arc<Mutex<App>>) -> bool {
    let app_guard = app.lock().await;
    app_guard.ui.exit_requested
}

async fn current_terminal_size(terminal: &SharedTerminal) -> Size {
    let terminal_guard = terminal.lock().await;
    terminal_guard.size().unwrap_or_default()
}

async fn try_draw_frame(
    app: &Arc<Mutex<App>>,
    terminal: &SharedTerminal,
    request_redraw: &mut bool,
    last_draw: &mut Instant,
    frame_duration: Duration,
) -> io::Result<()> {
    if !*request_redraw {
        return Ok(());
    }

    let now = Instant::now();
    if now.duration_since(*last_draw) < frame_duration {
        return Ok(());
    }

    let mut app_guard = app.lock().await;
    let mut terminal_guard = terminal.lock().await;
    terminal_guard.draw(|f| ui(f, &mut app_guard))?;
    *last_draw = now;
    *request_redraw = false;
    Ok(())
}

struct EventProcessingOutcome {
    events_processed: bool,
    request_redraw: bool,
    exit_requested: bool,
}

async fn process_ui_events(
    app: &Arc<Mutex<App>>,
    event_rx: &mut mpsc::UnboundedReceiver<UiEvent>,
    mode_registry: &ModeAwareRegistry,
    dispatcher: &AppActionDispatcher,
    term_size: Size,
    last_input_layout_update: &mut Instant,
) -> Result<EventProcessingOutcome, Box<dyn Error>> {
    let mut outcome = EventProcessingOutcome {
        events_processed: false,
        request_redraw: false,
        exit_requested: false,
    };

    while let Ok(ev) = event_rx.try_recv() {
        outcome.events_processed = true;
        match ev {
            UiEvent::RequestRedraw => {
                outcome.request_redraw = true;
            }
            UiEvent::Crossterm(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                let keyboard_outcome = route_keyboard_event(
                    app,
                    mode_registry,
                    dispatcher,
                    key,
                    term_size,
                    last_input_layout_update,
                )
                .await?;
                if keyboard_outcome.exit_requested {
                    outcome.exit_requested = true;
                    outcome.request_redraw = true;
                    break;
                }
                if keyboard_outcome.request_redraw {
                    outcome.request_redraw = true;
                }
            }
            UiEvent::Crossterm(Event::Paste(text)) => {
                handle_paste_event(app, term_size.width, text, last_input_layout_update).await;
                outcome.request_redraw = true;
            }
            UiEvent::Crossterm(Event::Resize(_, _)) => {
                outcome.request_redraw = true;
            }
            UiEvent::Crossterm(_) => {}
        }
    }

    if outcome.events_processed {
        outcome.request_redraw = true;
    }

    Ok(outcome)
}

struct KeyboardEventOutcome {
    request_redraw: bool,
    exit_requested: bool,
}

async fn route_keyboard_event(
    app: &Arc<Mutex<App>>,
    mode_registry: &ModeAwareRegistry,
    dispatcher: &AppActionDispatcher,
    key: event::KeyEvent,
    term_size: Size,
    last_input_layout_update: &mut Instant,
) -> Result<KeyboardEventOutcome, Box<dyn Error>> {
    let context = {
        let app_guard = app.lock().await;
        let picker_open = app_guard.model_picker_state().is_some()
            || app_guard.theme_picker_state().is_some()
            || app_guard.provider_picker_state().is_some();
        KeyContext::from_ui_mode(&app_guard.ui.mode, picker_open)
    };

    if mode_registry.should_handle_as_text_input(&key, &context) {
        let mut app_guard = app.lock().await;
        app_guard
            .ui
            .apply_textarea_edit_and_recompute(term_size.width, |ta| {
                ta.input(tui_textarea::Input::from(key));
            });
        return Ok(KeyboardEventOutcome {
            request_redraw: true,
            exit_requested: false,
        });
    }

    let registry_result = mode_registry
        .handle_key_event(
            app,
            dispatcher,
            &key,
            context,
            term_size.width,
            term_size.height,
            Some(*last_input_layout_update),
        )
        .await;

    if let Some(updated_time) = registry_result.updated_layout_time {
        *last_input_layout_update = updated_time;
    }

    let outcome = match registry_result.result {
        KeyResult::Exit => KeyboardEventOutcome {
            request_redraw: true,
            exit_requested: true,
        },
        KeyResult::Continue | KeyResult::Handled => KeyboardEventOutcome {
            request_redraw: true,
            exit_requested: false,
        },
        KeyResult::NotHandled => KeyboardEventOutcome {
            request_redraw: false,
            exit_requested: false,
        },
    };

    Ok(outcome)
}

async fn handle_paste_event(
    app: &Arc<Mutex<App>>,
    term_width: u16,
    text: String,
    last_input_layout_update: &mut Instant,
) {
    let mut app_guard = app.lock().await;
    let sanitized_text = text
        .replace('\t', "    ")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect::<String>();
    app_guard
        .ui
        .apply_textarea_edit_and_recompute(term_width, |ta| {
            ta.insert_str(&sanitized_text);
        });
    *last_input_layout_update = Instant::now();
}

fn process_stream_updates(
    dispatcher: &AppActionDispatcher,
    rx: &mut mpsc::UnboundedReceiver<(StreamMessage, u64)>,
    term_width: u16,
    term_height: u16,
    current_stream_id: u64,
) -> bool {
    let mut received_any = false;
    let mut coalesced_chunks = String::new();
    let mut followup_actions = Vec::new();

    while let Ok((message, msg_stream_id)) = rx.try_recv() {
        if msg_stream_id != current_stream_id {
            continue;
        }

        match message {
            StreamMessage::Chunk(content) => {
                coalesced_chunks.push_str(&content);
            }
            StreamMessage::Error(err) => {
                followup_actions.push(AppAction::StreamErrored { message: err });
            }
            StreamMessage::End => followup_actions.push(AppAction::StreamCompleted),
        }

        received_any = true;
    }

    if !received_any {
        return false;
    }

    let ctx = AppActionContext {
        term_width,
        term_height,
    };

    let mut actions = Vec::with_capacity(1 + followup_actions.len());
    let chunk = std::mem::take(&mut coalesced_chunks);
    if !chunk.is_empty() {
        actions.push(AppAction::AppendResponseChunk { content: chunk });
    }
    actions.extend(followup_actions);

    if !actions.is_empty() {
        dispatcher.dispatch_many(actions, ctx);
    }

    true
}

async fn drain_action_queue(
    app: &Arc<Mutex<App>>,
    stream_service: &ChatStreamService,
    action_rx: &mut mpsc::UnboundedReceiver<AppActionEnvelope>,
) -> bool {
    let mut pending = Vec::new();
    while let Ok(envelope) = action_rx.try_recv() {
        pending.push(envelope);
    }

    if pending.is_empty() {
        return false;
    }

    let mut app_guard = app.lock().await;
    let commands = apply_actions(&mut app_guard, pending);
    drop(app_guard);
    for cmd in commands {
        match cmd {
            crate::core::app::AppCommand::SpawnStream(params) => {
                stream_service.spawn_stream(params);
            }
        }
    }
    true
}

pub async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
) -> Result<(), Box<dyn Error>> {
    let app = bootstrap_app(model.clone(), log.clone(), provider.clone(), env_only).await?;
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<AppActionEnvelope>();
    let action_dispatcher = AppActionDispatcher::new(action_tx);

    // Sign-off line (no noisy startup banners)
    println!(
        "Chabeau is in the public domain, forever. Contribute: https://github.com/permacommons/chabeau"
    );
    // Color depth print removed; use CHABEAU_COLOR and README tips when debugging

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = OscBackend::new(stdout);
    let terminal = Arc::new(Mutex::new(Terminal::new(backend)?));

    // Channel for streaming updates with stream ID
    let (stream_service, mut rx) = ChatStreamService::new();
    let stream_service = Arc::new(stream_service);

    // Channel for async event processing
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<UiEvent>();

    // Spawn async event reader task
    let event_reader_handle = {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            loop {
                // Use a short timeout to prevent blocking
                if let Ok(true) = event::poll(Duration::from_millis(10)) {
                    match event::read() {
                        Ok(ev) => {
                            if event_tx.send(UiEvent::Crossterm(ev)).is_err() {
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
    let mode_registry =
        build_mode_aware_registry(stream_service.clone(), terminal.clone(), event_tx.clone());

    // Drawing cadence control
    const MAX_FPS: u64 = 60; // Limit to 60 FPS
    let frame_duration = Duration::from_millis(1000 / MAX_FPS);
    let mut last_draw = Instant::now();
    let mut request_redraw = true;
    let mut last_input_layout_update = Instant::now();
    let mut indicator_visible = false;
    let mut last_indicator_frame = Instant::now() - frame_duration;

    // Main loop
    let result = 'main_loop: loop {
        if is_exit_requested(&app).await {
            break 'main_loop Ok(());
        }

        try_draw_frame(
            &app,
            &terminal,
            &mut request_redraw,
            &mut last_draw,
            frame_duration,
        )
        .await?;

        let term_size = current_terminal_size(&terminal).await;

        let event_outcome = process_ui_events(
            &app,
            &mut event_rx,
            &mode_registry,
            &action_dispatcher,
            term_size,
            &mut last_input_layout_update,
        )
        .await?;

        if event_outcome.exit_requested {
            break 'main_loop Ok(());
        }

        if event_outcome.request_redraw {
            request_redraw = true;
        }

        let current_stream_id = {
            let app_guard = app.lock().await;
            app_guard.session.current_stream_id
        };

        let received_any = process_stream_updates(
            &action_dispatcher,
            &mut rx,
            term_size.width,
            term_size.height,
            current_stream_id,
        );

        if received_any {
            request_redraw = true;
        }

        let actions_applied = drain_action_queue(&app, &stream_service, &mut action_rx).await;
        if actions_applied {
            request_redraw = true;
        }

        let indicator_now = {
            let app_guard = app.lock().await;
            app_guard.ui.is_activity_indicator_visible()
        };

        if indicator_now != indicator_visible {
            indicator_visible = indicator_now;
            request_redraw = true;
            if !indicator_now {
                last_indicator_frame = Instant::now() - frame_duration;
            }
        }

        if indicator_now {
            let now = Instant::now();
            if now.duration_since(last_indicator_frame) >= frame_duration {
                request_redraw = true;
                last_indicator_frame = now;
            }
        }

        let idle = !event_outcome.events_processed && !received_any && !request_redraw;

        if idle {
            tokio::time::sleep(Duration::from_millis(16)).await; // ~60 FPS when idle
        }
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
    if !app_guard.ui.in_edit_select_mode() {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app_guard.ui.exit_edit_select_mode();
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(current) = app_guard.ui.selected_user_message_index() {
                let prev = {
                    let ui = &app_guard.ui;
                    ui.prev_user_message_index(current)
                        .or_else(|| ui.last_user_message_index())
                };
                if let Some(prev) = prev {
                    app_guard.ui.set_selected_user_message_index(prev);
                    app_guard
                        .conversation()
                        .scroll_index_into_view(prev, term_width, term_height);
                }
            } else if let Some(last) = app_guard.ui.last_user_message_index() {
                app_guard.ui.set_selected_user_message_index(last);
            }
            true
        }

        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(current) = app_guard.ui.selected_user_message_index() {
                let next = {
                    let ui = &app_guard.ui;
                    ui.next_user_message_index(current)
                        .or_else(|| ui.first_user_message_index())
                };
                if let Some(next) = next {
                    app_guard.ui.set_selected_user_message_index(next);
                    app_guard
                        .conversation()
                        .scroll_index_into_view(next, term_width, term_height);
                }
            } else if let Some(last) = app_guard.ui.last_user_message_index() {
                app_guard.ui.set_selected_user_message_index(last);
            }
            true
        }
        KeyCode::Enter => {
            if let Some(idx) = app_guard.ui.selected_user_message_index() {
                if idx < app_guard.ui.messages.len() && app_guard.ui.messages[idx].role == "user" {
                    let content = app_guard.ui.messages[idx].content.clone();
                    {
                        let mut conversation = app_guard.conversation();
                        conversation.cancel_current_stream();
                    }
                    app_guard.ui.messages.truncate(idx);
                    app_guard.invalidate_prewrap_cache();
                    let _ = app_guard
                        .session
                        .logging
                        .rewrite_log_without_last_response(&app_guard.ui.messages);
                    app_guard.ui.set_input_text(content);
                    app_guard.ui.exit_edit_select_mode();
                    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                    {
                        let mut conversation = app_guard.conversation();
                        let available_height =
                            conversation.calculate_available_height(term_height, input_area_height);
                        conversation.update_scroll_position(available_height, term_width);
                    }
                }
            }
            true
        }
        KeyCode::Char('E') | KeyCode::Char('e') => {
            if let Some(idx) = app_guard.ui.selected_user_message_index() {
                if idx < app_guard.ui.messages.len() && app_guard.ui.messages[idx].role == "user" {
                    let content = app_guard.ui.messages[idx].content.clone();
                    app_guard.ui.set_input_text(content);
                    app_guard.ui.start_in_place_edit(idx);
                    app_guard.ui.exit_edit_select_mode();
                }
            }
            true
        }
        KeyCode::Delete => {
            if let Some(idx) = app_guard.ui.selected_user_message_index() {
                if idx < app_guard.ui.messages.len() && app_guard.ui.messages[idx].role == "user" {
                    {
                        let mut conversation = app_guard.conversation();
                        conversation.cancel_current_stream();
                    }
                    app_guard.ui.messages.truncate(idx);
                    app_guard.invalidate_prewrap_cache();
                    let _ = app_guard
                        .session
                        .logging
                        .rewrite_log_without_last_response(&app_guard.ui.messages);
                    app_guard.ui.exit_edit_select_mode();
                    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                    {
                        let mut conversation = app_guard.conversation();
                        let available_height =
                            conversation.calculate_available_height(term_height, input_area_height);
                        conversation.update_scroll_position(available_height, term_width);
                    }
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
    if !app_guard.ui.in_block_select_mode() {
        return false;
    }

    let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
        &app_guard.ui.messages,
        &app_guard.ui.theme,
        Some(term_width as usize),
        crate::ui::layout::TableOverflowPolicy::WrapCells,
        app_guard.ui.syntax_enabled,
    );

    match key.code {
        KeyCode::Esc => {
            app_guard.ui.exit_block_select_mode();
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(cur) = app_guard.ui.selected_block_index() {
                let total = ranges.len();
                if let Some(next) = wrap_previous_index(cur, total) {
                    app_guard.ui.set_selected_block_index(next);
                    if let Some((start, _len, _)) = ranges.get(next) {
                        scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                    }
                }
            } else if !ranges.is_empty() {
                app_guard.ui.set_selected_block_index(0);
            }
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(cur) = app_guard.ui.selected_block_index() {
                let total = ranges.len();
                if let Some(next) = wrap_next_index(cur, total) {
                    app_guard.ui.set_selected_block_index(next);
                    if let Some((start, _len, _)) = ranges.get(next) {
                        scroll_block_into_view(&mut app_guard, term_width, term_height, *start);
                    }
                }
            } else if !ranges.is_empty() {
                app_guard.ui.set_selected_block_index(0);
            }
            true
        }

        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(cur) = app_guard.ui.selected_block_index() {
                if let Some((_start, _len, content)) = ranges.get(cur) {
                    match crate::utils::clipboard::copy_to_clipboard(content) {
                        Ok(()) => app_guard.conversation().set_status("Copied code block"),
                        Err(_e) => app_guard.conversation().set_status("Clipboard error"),
                    }
                    app_guard.ui.exit_block_select_mode();
                    app_guard.ui.auto_scroll = true;
                    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                    {
                        let mut conversation = app_guard.conversation();
                        let available_height =
                            conversation.calculate_available_height(term_height, input_area_height);
                        conversation.update_scroll_position(available_height, term_width);
                    }
                }
            }
            true
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if let Some(cur) = app_guard.ui.selected_block_index() {
                let contents = crate::ui::markdown::compute_codeblock_contents_with_lang(
                    &app_guard.ui.messages,
                );
                if let Some((content, lang)) = contents.get(cur) {
                    use chrono::Utc;
                    use std::fs;
                    let date = Utc::now().format("%Y-%m-%d");
                    let ext = language_to_extension(lang.as_deref());
                    let filename = format!("chabeau-block-{}.{}", date, ext);
                    if std::path::Path::new(&filename).exists() {
                        app_guard.conversation().set_status("File already exists.");
                        app_guard
                            .ui
                            .start_file_prompt_save_block(filename, content.clone());
                    } else {
                        match fs::write(&filename, content) {
                            Ok(()) => app_guard
                                .conversation()
                                .set_status(format!("Saved to {}", filename)),
                            Err(_e) => app_guard
                                .conversation()
                                .set_status("Error saving code block"),
                        }
                    }
                    app_guard.ui.exit_block_select_mode();
                    app_guard.ui.auto_scroll = true;
                    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
                    {
                        let mut conversation = app_guard.conversation();
                        let available_height =
                            conversation.calculate_available_height(term_height, input_area_height);
                        conversation.update_scroll_position(available_height, term_width);
                    }
                }
            }
            true
        }
        _ => false, // Key not handled - allow fallback handlers to process it
    }
}

async fn handle_external_editor_shortcut(
    dispatcher: &AppActionDispatcher,
    app: &Arc<Mutex<App>>,
    terminal: &mut Terminal<OscBackend<io::Stdout>>,
    term_width: u16,
    term_height: u16,
) -> Result<Option<KeyLoopAction>, String> {
    let initial_text = {
        let app_guard = app.lock().await;
        app_guard.ui.get_input_text().to_string()
    };

    let outcome = match launch_external_editor(&initial_text).await {
        Ok(outcome) => outcome,
        Err(e) => ExternalEditorOutcome {
            message: None,
            status: Some(format!("Editor error: {}", e)),
            clear_input: false,
        },
    };

    terminal.clear().map_err(|e| e.to_string())?;

    let mut actions = Vec::new();
    if let Some(status) = outcome.status {
        actions.push(AppAction::SetStatus { message: status });
    }
    if outcome.clear_input {
        actions.push(AppAction::ClearInput);
    }
    if let Some(message) = outcome.message {
        actions.push(AppAction::SubmitMessage { message });
    }

    if !actions.is_empty() {
        dispatcher.dispatch_many(
            actions,
            AppActionContext {
                term_width,
                term_height,
            },
        );
    }

    Ok(Some(KeyLoopAction::Continue))
}

async fn process_input_submission(
    dispatcher: &AppActionDispatcher,
    app: &Arc<Mutex<App>>,
    input_text: String,
    term_width: u16,
    term_height: u16,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> SubmissionResult {
    let mut app_guard = app.lock().await;

    match process_input(&mut app_guard, &input_text) {
        CommandResult::Continue => {
            let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
            {
                let mut conversation = app_guard.conversation();
                let available_height =
                    conversation.calculate_available_height(term_height, input_area_height);
                conversation.update_scroll_position(available_height, term_width);
            }
            SubmissionResult::Continue
        }
        CommandResult::OpenModelPicker => {
            let request = app_guard.prepare_model_picker_request();
            drop(app_guard);
            spawn_model_picker_loader(dispatcher.clone(), event_tx.clone(), request);
            SubmissionResult::Continue
        }
        CommandResult::OpenProviderPicker => {
            app_guard.open_provider_picker();
            drop(app_guard);
            let _ = event_tx.send(UiEvent::RequestRedraw);
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
    dispatcher: &AppActionDispatcher,
    app: &Arc<Mutex<App>>,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
    stream_service: &ChatStreamService,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let modifiers = key.modifiers;

    {
        let mut app_guard = app.lock().await;
        if let Some(prompt) = app_guard.ui.file_prompt().cloned() {
            let filename = app_guard.ui.get_input_text().trim().to_string();
            if filename.is_empty() {
                return Ok(Some(KeyLoopAction::Continue));
            }
            let overwrite = modifiers.contains(event::KeyModifiers::ALT);
            match prompt.kind {
                FilePromptKind::Dump => {
                    let res = crate::commands::dump_conversation_with_overwrite(
                        &app_guard, &filename, overwrite,
                    );
                    match res {
                        Ok(()) => {
                            app_guard
                                .conversation()
                                .set_status(format!("Dumped: {}", filename));
                            app_guard.ui.cancel_file_prompt();
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("exists") && !overwrite {
                                app_guard
                                    .conversation()
                                    .set_status("File exists (Alt+Enter to overwrite)");
                            } else {
                                app_guard
                                    .conversation()
                                    .set_status(format!("Dump error: {}", msg));
                            }
                        }
                    }
                }
                FilePromptKind::SaveCodeBlock => {
                    use std::fs;
                    let exists = std::path::Path::new(&filename).exists();
                    if exists && !overwrite {
                        app_guard.conversation().set_status("File already exists.");
                    } else if let Some(content) = prompt.content {
                        match fs::write(&filename, content) {
                            Ok(()) => {
                                app_guard
                                    .conversation()
                                    .set_status(format!("Saved to {}", filename));
                                app_guard.ui.cancel_file_prompt();
                            }
                            Err(_e) => {
                                app_guard
                                    .conversation()
                                    .set_status("Error saving code block");
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
        let compose = app_guard.ui.compose_mode;
        let alt = modifiers.contains(event::KeyModifiers::ALT);
        if compose {
            !alt
        } else {
            alt
        }
    };

    if should_insert_newline {
        let mut app_guard = app.lock().await;
        app_guard
            .ui
            .apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.insert_str("\n");
            });
        return Ok(Some(KeyLoopAction::Continue));
    }

    {
        let mut app_guard = app.lock().await;
        if let Some(idx) = app_guard.ui.take_in_place_edit_index() {
            if idx < app_guard.ui.messages.len() && app_guard.ui.messages[idx].role == "user" {
                let new_text = app_guard.ui.get_input_text().to_string();
                app_guard.ui.messages[idx].content = new_text;
                app_guard.invalidate_prewrap_cache();
                let _ = app_guard
                    .session
                    .logging
                    .rewrite_log_without_last_response(&app_guard.ui.messages);
            }
            app_guard.ui.clear_input();
            return Ok(Some(KeyLoopAction::Continue));
        }
    }

    let input_text = {
        let mut app_guard = app.lock().await;
        if app_guard.ui.get_input_text().trim().is_empty() {
            return Ok(Some(KeyLoopAction::Continue));
        }
        let text = app_guard.ui.get_input_text().to_string();
        app_guard.ui.clear_input();
        text
    };

    match process_input_submission(
        dispatcher,
        app,
        input_text,
        term_width,
        term_height,
        event_tx,
    )
    .await
    {
        SubmissionResult::Continue => Ok(Some(KeyLoopAction::Continue)),
        SubmissionResult::Spawn(params) => {
            stream_service.spawn_stream(params);
            Ok(Some(KeyLoopAction::Continue))
        }
    }
}

async fn handle_ctrl_j_shortcut(
    dispatcher: &AppActionDispatcher,
    app: &Arc<Mutex<App>>,
    term_width: u16,
    term_height: u16,
    stream_service: &ChatStreamService,
    last_input_layout_update: &mut Instant,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let send_now = {
        let app_guard = app.lock().await;
        app_guard.ui.compose_mode && app_guard.ui.file_prompt().is_none()
    };

    if !send_now {
        let mut app_guard = app.lock().await;
        app_guard
            .ui
            .apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.insert_str("\n");
            });
        *last_input_layout_update = Instant::now();
        return Ok(Some(KeyLoopAction::Continue));
    }

    let input_text = {
        let mut app_guard = app.lock().await;
        if app_guard.ui.get_input_text().trim().is_empty() {
            return Ok(Some(KeyLoopAction::Continue));
        }
        let text = app_guard.ui.get_input_text().to_string();
        app_guard.ui.clear_input();
        text
    };

    match process_input_submission(
        dispatcher,
        app,
        input_text,
        term_width,
        term_height,
        event_tx,
    )
    .await
    {
        SubmissionResult::Continue => Ok(Some(KeyLoopAction::Continue)),
        SubmissionResult::Spawn(params) => {
            stream_service.spawn_stream(params);
            Ok(Some(KeyLoopAction::Continue))
        }
    }
}

fn prepare_stream_params_for_message(
    app_guard: &mut App,
    message: String,
    term_width: u16,
    term_height: u16,
) -> StreamParams {
    app_guard.ui.auto_scroll = true;
    let input_area_height = app_guard.ui.calculate_input_area_height(term_width);
    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app_guard.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(message);
        let available_height =
            conversation.calculate_available_height(term_height, input_area_height);
        conversation.update_scroll_position(available_height, term_width);
        (cancel_token, stream_id, api_messages)
    };

    StreamParams {
        client: app_guard.session.client.clone(),
        base_url: app_guard.session.base_url.clone(),
        api_key: app_guard.session.api_key.clone(),
        provider_name: app_guard.session.provider_name.clone(),
        model: app_guard.session.model.clone(),
        api_messages,
        cancel_token,
        stream_id,
    }
}

enum SubmissionResult {
    Continue,
    Spawn(StreamParams),
}

async fn handle_picker_key_event(
    app: &Arc<Mutex<App>>,
    dispatcher: &AppActionDispatcher,
    key: &event::KeyEvent,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) {
    let mut app_guard = app.lock().await;
    let current_picker_mode = app_guard.current_picker_mode();
    let provider_name = app_guard.session.provider_name.clone();
    let mut should_request_redraw = false;

    let selection = if let Some(picker) = app_guard.picker_state_mut() {
        match key.code {
            KeyCode::Esc => {
                match current_picker_mode {
                    Some(crate::core::app::PickerMode::Theme) => {
                        app_guard.revert_theme_preview();
                        app_guard.close_picker();
                    }
                    Some(crate::core::app::PickerMode::Model) => {
                        if app_guard.picker.startup_requires_model {
                            // Startup mandatory model selection
                            app_guard.close_picker();
                            if app_guard.picker.startup_multiple_providers_available {
                                // Go back to provider picker per spec
                                app_guard.picker.startup_requires_model = false;
                                app_guard.picker.startup_requires_provider = true;
                                // Clear provider selection in title bar during startup bounce-back
                                app_guard.session.provider_name.clear();
                                app_guard.session.provider_display_name =
                                    "(no provider selected)".to_string();
                                app_guard.session.api_key.clear();
                                app_guard.session.base_url.clear();
                                app_guard.open_provider_picker();
                                should_request_redraw = true;
                            } else {
                                // Exit app if no alternative provider
                                app_guard.ui.exit_requested = true;
                            }
                        } else {
                            app_guard.revert_model_preview();
                            if app_guard.picker.in_provider_model_transition {
                                app_guard.revert_provider_model_transition();
                                app_guard.conversation().set_status("Selection cancelled");
                            }
                            app_guard.close_picker();
                        }
                    }
                    Some(crate::core::app::PickerMode::Provider) => {
                        if app_guard.picker.startup_requires_provider {
                            // Startup mandatory provider selection: exit if cancelled
                            app_guard.close_picker();
                            app_guard.ui.exit_requested = true;
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
                        let res = {
                            let mut controller = app_guard.theme_controller();
                            if is_persistent {
                                controller.apply_theme_by_id(&id)
                            } else {
                                controller.apply_theme_by_id_session_only(&id)
                            }
                        };
                        match res {
                            Ok(_) => app_guard.conversation().set_status(format!(
                                "Theme set: {}{}",
                                id,
                                status_suffix(is_persistent)
                            )),
                            Err(_e) => app_guard.conversation().set_status("Theme error"),
                        }
                    }
                    app_guard.close_picker();
                    Some("__picker_handled__".to_string())
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Model) {
                    if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                        let persist = is_persistent && !app_guard.session.startup_env_only;
                        let res = {
                            let mut controller = app_guard.provider_controller();
                            if persist {
                                controller.apply_model_by_id_persistent(&id)
                            } else {
                                controller.apply_model_by_id(&id);
                                Ok(())
                            }
                        };
                        match res {
                            Ok(_) => {
                                app_guard.conversation().set_status(format!(
                                    "Model set: {}{}",
                                    id,
                                    status_suffix(persist)
                                ));
                                if app_guard.picker.in_provider_model_transition {
                                    app_guard.complete_provider_model_transition();
                                }
                                if app_guard.picker.startup_requires_model {
                                    app_guard.picker.startup_requires_model = false;
                                }
                            }
                            Err(e) => app_guard
                                .conversation()
                                .set_status(format!("Model error: {}", e)),
                        }
                    }
                    app_guard.close_picker();
                    Some("__picker_handled__".to_string())
                } else if current_picker_mode == Some(crate::core::app::PickerMode::Provider) {
                    if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                        let (res, should_open_model_picker) = {
                            let mut controller = app_guard.provider_controller();
                            if is_persistent {
                                controller.apply_provider_by_id_persistent(&id)
                            } else {
                                controller.apply_provider_by_id(&id)
                            }
                        };
                        match res {
                            Ok(_) => {
                                app_guard.conversation().set_status(format!(
                                    "Provider set: {}{}",
                                    id,
                                    status_suffix(is_persistent)
                                ));
                                app_guard.close_picker();
                                if should_open_model_picker {
                                    if app_guard.picker.startup_requires_provider {
                                        app_guard.picker.startup_requires_provider = false;
                                        app_guard.picker.startup_requires_model = true;
                                    }
                                    let request = app_guard.prepare_model_picker_request();
                                    spawn_model_picker_loader(
                                        dispatcher.clone(),
                                        event_tx.clone(),
                                        request,
                                    );
                                }
                            }
                            Err(e) => {
                                app_guard
                                    .conversation()
                                    .set_status(format!("Provider error: {}", e));
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
                            let res = {
                                let mut controller = app_guard.theme_controller();
                                controller.apply_theme_by_id(&id)
                            };
                            match res {
                                Ok(_) => app_guard.conversation().set_status(format!(
                                    "Theme set: {}{}",
                                    id,
                                    status_suffix(true)
                                )),
                                Err(_e) => app_guard.conversation().set_status("Theme error"),
                            }
                        }
                        app_guard.close_picker();
                        Some("__picker_handled__".to_string())
                    }
                    Some(crate::core::app::PickerMode::Model) => {
                        if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                            let persist = !app_guard.session.startup_env_only;
                            let res = {
                                let mut controller = app_guard.provider_controller();
                                if persist {
                                    controller.apply_model_by_id_persistent(&id)
                                } else {
                                    controller.apply_model_by_id(&id);
                                    Ok(())
                                }
                            };
                            match res {
                                Ok(_) => {
                                    app_guard.conversation().set_status(format!(
                                        "Model set: {}{}",
                                        id,
                                        status_suffix(persist)
                                    ));
                                    if app_guard.picker.in_provider_model_transition {
                                        app_guard.complete_provider_model_transition();
                                    }
                                    if app_guard.picker.startup_requires_model {
                                        app_guard.picker.startup_requires_model = false;
                                    }
                                }
                                Err(e) => app_guard
                                    .conversation()
                                    .set_status(format!("Model error: {}", e)),
                            }
                        }
                        app_guard.close_picker();
                        Some("__picker_handled__".to_string())
                    }
                    Some(crate::core::app::PickerMode::Provider) => {
                        if let Some(id) = picker.selected_id().map(|s| s.to_string()) {
                            let (res, should_open_model_picker) = {
                                let mut controller = app_guard.provider_controller();
                                controller.apply_provider_by_id_persistent(&id)
                            };
                            match res {
                                Ok(_) => {
                                    app_guard.conversation().set_status(format!(
                                        "Provider set: {}{}",
                                        id,
                                        status_suffix(true)
                                    ));
                                    app_guard.close_picker();
                                    if should_open_model_picker {
                                        let request = app_guard.prepare_model_picker_request();
                                        spawn_model_picker_loader(
                                            dispatcher.clone(),
                                            event_tx.clone(),
                                            request,
                                        );
                                    }
                                }
                                Err(e) => app_guard
                                    .conversation()
                                    .set_status(format!("Provider error: {}", e)),
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
                                let mut controller = app_guard.provider_controller();
                                controller.unset_default_model(&provider_name)
                            }
                            Some(crate::core::app::PickerMode::Theme) => {
                                let mut controller = app_guard.theme_controller();
                                controller.unset_default_theme()
                            }
                            Some(crate::core::app::PickerMode::Provider) => {
                                let mut controller = app_guard.provider_controller();
                                controller.unset_default_provider()
                            }
                            _ => Err("Unknown picker mode".to_string()),
                        };
                        match result {
                            Ok(_) => {
                                app_guard
                                    .conversation()
                                    .set_status(format!("Removed default: {}", item_id));
                                // Refresh the picker to remove the asterisk
                                match current_picker_mode {
                                    Some(crate::core::app::PickerMode::Model) => {
                                        // Refresh future model list asynchronously
                                        let request = app_guard.prepare_model_picker_request();
                                        spawn_model_picker_loader(
                                            dispatcher.clone(),
                                            event_tx.clone(),
                                            request,
                                        );
                                    }
                                    Some(crate::core::app::PickerMode::Theme) => {
                                        app_guard.open_theme_picker();
                                        should_request_redraw = true;
                                    }
                                    Some(crate::core::app::PickerMode::Provider) => {
                                        app_guard.open_provider_picker();
                                        should_request_redraw = true;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                app_guard
                                    .conversation()
                                    .set_status(format!("Error removing default: {}", e));
                            }
                        }
                    } else {
                        app_guard
                            .conversation()
                            .set_status("Del key only works on default items (marked with *)");
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

    drop(app_guard);
    if should_request_redraw {
        let _ = event_tx.send(UiEvent::RequestRedraw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{
        apply_action, apply_actions, AppAction, AppActionContext, AppActionDispatcher,
        AppActionEnvelope, AppCommand,
    };
    use crate::core::message::Message;
    use crate::ui::theme::Theme;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::{mpsc, Mutex};

    const TERM_WIDTH: u16 = 80;
    const TERM_HEIGHT: u16 = 24;

    fn setup_service() -> (
        ChatStreamService,
        tokio::sync::mpsc::UnboundedReceiver<(StreamMessage, u64)>,
    ) {
        ChatStreamService::new()
    }

    fn setup_app() -> App {
        App::new_bench(Theme::dark_default(), true, true)
    }

    fn default_context() -> AppActionContext {
        AppActionContext {
            term_width: TERM_WIDTH,
            term_height: TERM_HEIGHT,
        }
    }

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

    #[test]
    fn chunk_messages_append_to_response() {
        let mut app = setup_app();
        app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: String::new(),
        });

        let ctx = default_context();
        apply_action(
            &mut app,
            AppAction::AppendResponseChunk {
                content: "Hello".into(),
            },
            ctx,
        );

        assert_eq!(app.ui.current_response, "Hello");
        assert_eq!(app.ui.messages.back().unwrap().content, "Hello");
    }

    #[test]
    fn coalesced_chunks_match_sequential_output() {
        let chunks = ["Hello", " ", "world", "!\n"];

        let mut sequential_app = setup_app();
        sequential_app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: String::new(),
        });

        for chunk in &chunks {
            let input_area_height = sequential_app.ui.calculate_input_area_height(TERM_WIDTH);
            {
                let mut conversation = sequential_app.conversation();
                let available_height =
                    conversation.calculate_available_height(TERM_HEIGHT, input_area_height);
                conversation.append_to_response(chunk, available_height, TERM_WIDTH);
            }
        }

        let mut coalesced_app = setup_app();
        coalesced_app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: String::new(),
        });

        let aggregated = chunks.concat();
        let ctx = default_context();
        apply_action(
            &mut coalesced_app,
            AppAction::AppendResponseChunk {
                content: aggregated,
            },
            ctx,
        );

        assert_eq!(
            coalesced_app.ui.current_response,
            sequential_app.ui.current_response
        );
        assert_eq!(
            coalesced_app.ui.messages.back().unwrap().content,
            sequential_app.ui.messages.back().unwrap().content
        );
    }

    #[test]
    fn error_messages_add_system_entries_and_stop_streaming() {
        let mut app = setup_app();
        app.ui.is_streaming = true;

        let ctx = default_context();
        apply_action(
            &mut app,
            AppAction::StreamErrored {
                message: " api failure \n".into(),
            },
            ctx,
        );

        assert!(!app.ui.is_streaming);
        let last_message = app.ui.messages.back().expect("system message added");
        assert_eq!(last_message.role, "system");
        assert_eq!(last_message.content, "Error: api failure");
    }

    #[test]
    fn end_messages_finalize_responses() {
        let mut app = setup_app();
        app.ui.is_streaming = true;
        app.session.retrying_message_index = Some(0);
        app.ui.current_response = "partial".into();

        let ctx = default_context();
        apply_action(&mut app, AppAction::StreamCompleted, ctx);

        assert!(!app.ui.is_streaming);
        assert!(app.session.retrying_message_index.is_none());
    }

    #[test]
    fn submit_message_returns_spawn_command() {
        let mut app = setup_app();
        let ctx = default_context();
        let result = apply_action(
            &mut app,
            AppAction::SubmitMessage {
                message: "Hello".into(),
            },
            ctx,
        );
        assert!(matches!(result, Some(AppCommand::SpawnStream(_))));
    }

    #[test]
    fn retry_last_message_returns_none_without_history() {
        let mut app = setup_app();
        let ctx = default_context();
        let result = apply_action(&mut app, AppAction::RetryLastMessage, ctx);
        assert!(result.is_none());
    }

    #[test]
    fn retry_last_message_emits_command_with_history() {
        let mut app = setup_app();
        app.ui.messages.push_back(Message {
            role: "user".to_string(),
            content: "Hi".into(),
        });
        app.ui.messages.push_back(Message {
            role: "assistant".to_string(),
            content: "Hello".into(),
        });
        app.session.last_retry_time = Instant::now() - Duration::from_millis(500);

        let ctx = default_context();
        let result = apply_action(&mut app, AppAction::RetryLastMessage, ctx);
        assert!(matches!(result, Some(AppCommand::SpawnStream(_))));
    }

    #[tokio::test]
    async fn process_stream_updates_dispatches_actions() {
        let (service, mut rx) = setup_service();
        service.send_for_test(StreamMessage::Chunk("Hello".into()), 42);
        service.send_for_test(StreamMessage::Chunk(" world".into()), 42);
        service.send_for_test(StreamMessage::Error(" failure ".into()), 99);
        service.send_for_test(StreamMessage::End, 42);

        let app = Arc::new(Mutex::new(setup_app()));
        let (action_tx, mut action_rx) = mpsc::unbounded_channel::<AppActionEnvelope>();
        {
            let mut guard = app.lock().await;
            guard.session.current_stream_id = 42;
            guard.ui.messages.push_back(Message {
                role: "assistant".to_string(),
                content: String::new(),
            });
            guard.ui.is_streaming = true;
        }

        let dispatcher = AppActionDispatcher::new(action_tx);

        let processed = process_stream_updates(&dispatcher, &mut rx, TERM_WIDTH, TERM_HEIGHT, 42);
        assert!(processed);

        {
            let mut guard = app.lock().await;
            let mut envelopes = Vec::new();
            while let Ok(envelope) = action_rx.try_recv() {
                envelopes.push(envelope);
            }
            let commands = apply_actions(&mut guard, envelopes);
            assert!(commands.is_empty());
        }

        let guard = app.lock().await;
        assert_eq!(guard.ui.current_response, "Hello world");
        assert_eq!(guard.ui.messages.back().unwrap().content, "Hello world");
        assert!(!guard.ui.is_streaming);

        let last_message = guard
            .ui
            .messages
            .iter()
            .rev()
            .find(|msg| msg.role == "system");
        assert!(last_message.is_none(), "non-matching error message ignored");
    }
}
