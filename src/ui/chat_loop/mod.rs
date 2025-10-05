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

use crate::core::chat_stream::{ChatStreamService, StreamMessage};

use crate::api::models::fetch_models;
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

#[derive(Clone)]
pub struct AppHandle {
    inner: Arc<Mutex<App>>,
}

impl AppHandle {
    pub fn new(inner: Arc<Mutex<App>>) -> Self {
        Self { inner }
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, App> {
        self.inner.lock().await
    }

    pub async fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let guard = self.inner.lock().await;
        f(&guard)
    }

    pub async fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut guard = self.inner.lock().await;
        f(&mut guard)
    }
}

#[derive(Debug)]
pub enum UiEvent {
    Crossterm(Event),
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
fn spawn_model_picker_loader(dispatcher: AppActionDispatcher, request: ModelPickerRequest) {
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
    });
}

async fn is_exit_requested(app: &AppHandle) -> bool {
    app.read(|app| app.ui.exit_requested).await
}

async fn current_terminal_size(terminal: &SharedTerminal) -> Size {
    let terminal_guard = terminal.lock().await;
    terminal_guard.size().unwrap_or_default()
}

async fn try_draw_frame(
    app: &AppHandle,
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

    let mut terminal_guard = terminal.lock().await;
    (app.update(|app| terminal_guard.draw(|f| ui(f, app))).await)?;
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
    app: &AppHandle,
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
    app: &AppHandle,
    mode_registry: &ModeAwareRegistry,
    dispatcher: &AppActionDispatcher,
    key: event::KeyEvent,
    term_size: Size,
    last_input_layout_update: &mut Instant,
) -> Result<KeyboardEventOutcome, Box<dyn Error>> {
    let context = app
        .read(|app| {
            let picker_open = app.model_picker_state().is_some()
                || app.theme_picker_state().is_some()
                || app.provider_picker_state().is_some();
            KeyContext::from_ui_mode(&app.ui.mode, picker_open)
        })
        .await;

    if mode_registry.should_handle_as_text_input(&key, &context) {
        app.update(|app| {
            app.ui
                .apply_textarea_edit_and_recompute(term_size.width, |ta| {
                    ta.input(tui_textarea::Input::from(key));
                });
        })
        .await;
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
    app: &AppHandle,
    term_width: u16,
    text: String,
    last_input_layout_update: &mut Instant,
) {
    let sanitized_text = text
        .replace('\t', "    ")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect::<String>();
    app.update(|app| {
        app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
            ta.insert_str(&sanitized_text);
        });
    })
    .await;
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
    app: &AppHandle,
    dispatcher: &AppActionDispatcher,
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

    let commands = app.update(|app| apply_actions(app, pending)).await;
    for cmd in commands {
        match cmd {
            crate::core::app::AppCommand::SpawnStream(params) => {
                stream_service.spawn_stream(params);
            }
            crate::core::app::AppCommand::LoadModelPicker(request) => {
                spawn_model_picker_loader(dispatcher.clone(), request);
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
    let mode_registry = build_mode_aware_registry(stream_service.clone(), terminal.clone());

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

        let current_stream_id = app.read(|app| app.session.current_stream_id).await;

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

        let actions_applied =
            drain_action_queue(&app, &action_dispatcher, &stream_service, &mut action_rx).await;
        if actions_applied {
            request_redraw = true;
        }

        let indicator_now = app.read(|app| app.ui.is_activity_indicator_visible()).await;

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
    app: &AppHandle,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    app.update(|app| {
        if !app.ui.in_edit_select_mode() {
            return false;
        }

        match key.code {
            KeyCode::Esc => {
                app.ui.exit_edit_select_mode();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(current) = app.ui.selected_user_message_index() {
                    let prev = {
                        let ui = &app.ui;
                        ui.prev_user_message_index(current)
                            .or_else(|| ui.last_user_message_index())
                    };
                    if let Some(prev) = prev {
                        app.ui.set_selected_user_message_index(prev);
                        app.conversation()
                            .scroll_index_into_view(prev, term_width, term_height);
                    }
                } else if let Some(last) = app.ui.last_user_message_index() {
                    app.ui.set_selected_user_message_index(last);
                }
                true
            }

            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(current) = app.ui.selected_user_message_index() {
                    let next = {
                        let ui = &app.ui;
                        ui.next_user_message_index(current)
                            .or_else(|| ui.first_user_message_index())
                    };
                    if let Some(next) = next {
                        app.ui.set_selected_user_message_index(next);
                        app.conversation()
                            .scroll_index_into_view(next, term_width, term_height);
                    }
                } else if let Some(last) = app.ui.last_user_message_index() {
                    app.ui.set_selected_user_message_index(last);
                }
                true
            }
            KeyCode::Enter => {
                if let Some(idx) = app.ui.selected_user_message_index() {
                    if idx < app.ui.messages.len() && app.ui.messages[idx].role == "user" {
                        let content = app.ui.messages[idx].content.clone();
                        {
                            let mut conversation = app.conversation();
                            conversation.cancel_current_stream();
                        }
                        app.ui.messages.truncate(idx);
                        app.invalidate_prewrap_cache();
                        let _ = app
                            .session
                            .logging
                            .rewrite_log_without_last_response(&app.ui.messages);
                        app.ui.set_input_text(content);
                        app.ui.exit_edit_select_mode();
                        let input_area_height = app.ui.calculate_input_area_height(term_width);
                        {
                            let mut conversation = app.conversation();
                            let available_height = conversation
                                .calculate_available_height(term_height, input_area_height);
                            conversation.update_scroll_position(available_height, term_width);
                        }
                    }
                }
                true
            }
            KeyCode::Char('E') | KeyCode::Char('e') => {
                if let Some(idx) = app.ui.selected_user_message_index() {
                    if idx < app.ui.messages.len() && app.ui.messages[idx].role == "user" {
                        let content = app.ui.messages[idx].content.clone();
                        app.ui.set_input_text(content);
                        app.ui.start_in_place_edit(idx);
                        app.ui.exit_edit_select_mode();
                    }
                }
                true
            }
            KeyCode::Delete => {
                if let Some(idx) = app.ui.selected_user_message_index() {
                    if idx < app.ui.messages.len() && app.ui.messages[idx].role == "user" {
                        {
                            let mut conversation = app.conversation();
                            conversation.cancel_current_stream();
                        }
                        app.ui.messages.truncate(idx);
                        app.invalidate_prewrap_cache();
                        let _ = app
                            .session
                            .logging
                            .rewrite_log_without_last_response(&app.ui.messages);
                        app.ui.exit_edit_select_mode();
                        let input_area_height = app.ui.calculate_input_area_height(term_width);
                        {
                            let mut conversation = app.conversation();
                            let available_height = conversation
                                .calculate_available_height(term_height, input_area_height);
                            conversation.update_scroll_position(available_height, term_width);
                        }
                    }
                }
                true
            }

            _ => false, // Key not handled - allow fallback handlers to process it
        }
    })
    .await
}

async fn handle_block_select_mode_event(
    app: &AppHandle,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    app.update(|app| {
        if !app.ui.in_block_select_mode() {
            return false;
        }

        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
            &app.ui.messages,
            &app.ui.theme,
            Some(term_width as usize),
            crate::ui::layout::TableOverflowPolicy::WrapCells,
            app.ui.syntax_enabled,
        );

        match key.code {
            KeyCode::Esc => {
                app.ui.exit_block_select_mode();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(cur) = app.ui.selected_block_index() {
                    let total = ranges.len();
                    if let Some(next) = wrap_previous_index(cur, total) {
                        app.ui.set_selected_block_index(next);
                        if let Some((start, _len, _)) = ranges.get(next) {
                            scroll_block_into_view(app, term_width, term_height, *start);
                        }
                    }
                } else if !ranges.is_empty() {
                    app.ui.set_selected_block_index(0);
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(cur) = app.ui.selected_block_index() {
                    let total = ranges.len();
                    if let Some(next) = wrap_next_index(cur, total) {
                        app.ui.set_selected_block_index(next);
                        if let Some((start, _len, _)) = ranges.get(next) {
                            scroll_block_into_view(app, term_width, term_height, *start);
                        }
                    }
                } else if !ranges.is_empty() {
                    app.ui.set_selected_block_index(0);
                }
                true
            }

            KeyCode::Char('c') | KeyCode::Char('C') => {
                if let Some(cur) = app.ui.selected_block_index() {
                    if let Some((_start, _len, content)) = ranges.get(cur) {
                        match crate::utils::clipboard::copy_to_clipboard(content) {
                            Ok(()) => app.conversation().set_status("Copied code block"),
                            Err(_e) => app.conversation().set_status("Clipboard error"),
                        }
                        app.ui.exit_block_select_mode();
                        app.ui.auto_scroll = true;
                        let input_area_height = app.ui.calculate_input_area_height(term_width);
                        {
                            let mut conversation = app.conversation();
                            let available_height = conversation
                                .calculate_available_height(term_height, input_area_height);
                            conversation.update_scroll_position(available_height, term_width);
                        }
                    }
                }
                true
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(cur) = app.ui.selected_block_index() {
                    let contents =
                        crate::ui::markdown::compute_codeblock_contents_with_lang(&app.ui.messages);
                    if let Some((content, lang)) = contents.get(cur) {
                        use chrono::Utc;
                        use std::fs;
                        let date = Utc::now().format("%Y-%m-%d");
                        let ext = language_to_extension(lang.as_deref());
                        let filename = format!("chabeau-block-{}.{}", date, ext);
                        if std::path::Path::new(&filename).exists() {
                            app.conversation().set_status("File already exists.");
                            app.ui
                                .start_file_prompt_save_block(filename, content.clone());
                        } else {
                            match fs::write(&filename, content) {
                                Ok(()) => app
                                    .conversation()
                                    .set_status(format!("Saved to {}", filename)),
                                Err(_e) => app.conversation().set_status("Error saving code block"),
                            }
                        }
                        app.ui.exit_block_select_mode();
                        app.ui.auto_scroll = true;
                        let input_area_height = app.ui.calculate_input_area_height(term_width);
                        {
                            let mut conversation = app.conversation();
                            let available_height = conversation
                                .calculate_available_height(term_height, input_area_height);
                            conversation.update_scroll_position(available_height, term_width);
                        }
                    }
                }
                true
            }
            _ => false, // Key not handled - allow fallback handlers to process it
        }
    })
    .await
}

async fn handle_external_editor_shortcut(
    dispatcher: &AppActionDispatcher,
    app: &AppHandle,
    terminal: &mut Terminal<OscBackend<io::Stdout>>,
    term_width: u16,
    term_height: u16,
) -> Result<Option<KeyLoopAction>, String> {
    let initial_text = app.read(|app| app.ui.get_input_text().to_string()).await;

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
    app: &AppHandle,
    term_width: u16,
    term_height: u16,
) {
    let input_text = app
        .read(|app| {
            let text = app.ui.get_input_text().to_string();
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .await;

    let Some(input_text) = input_text else {
        return;
    };

    dispatcher.dispatch_many(
        [
            AppAction::ClearInput,
            AppAction::ProcessCommand { input: input_text },
        ],
        AppActionContext {
            term_width,
            term_height,
        },
    );
}

async fn handle_enter_key(
    dispatcher: &AppActionDispatcher,
    app: &AppHandle,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
    _stream_service: &ChatStreamService,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let modifiers = key.modifiers;

    let file_prompt_action = app
        .read(|app| {
            app.ui.file_prompt().cloned().map(|prompt| {
                let filename = app.ui.get_input_text().trim().to_string();
                let overwrite = modifiers.contains(event::KeyModifiers::ALT);
                (prompt, filename, overwrite)
            })
        })
        .await;

    if let Some((prompt, filename, overwrite)) = file_prompt_action {
        if filename.is_empty() {
            return Ok(Some(KeyLoopAction::Continue));
        }

        let ctx = AppActionContext {
            term_width,
            term_height,
        };

        match prompt.kind {
            FilePromptKind::Dump => {
                dispatcher.dispatch_many(
                    [AppAction::CompleteFilePromptDump {
                        filename,
                        overwrite,
                    }],
                    ctx,
                );
            }
            FilePromptKind::SaveCodeBlock => {
                if let Some(content) = prompt.content {
                    dispatcher.dispatch_many(
                        [AppAction::CompleteFilePromptSaveBlock {
                            filename,
                            content,
                            overwrite,
                        }],
                        ctx,
                    );
                }
            }
        }

        return Ok(Some(KeyLoopAction::Continue));
    }

    let should_insert_newline = app
        .read(|app| {
            let compose = app.ui.compose_mode;
            let alt = modifiers.contains(event::KeyModifiers::ALT);
            if compose {
                !alt
            } else {
                alt
            }
        })
        .await;

    if should_insert_newline {
        app.update(|app| {
            app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.insert_str("\n");
            });
        })
        .await;
        return Ok(Some(KeyLoopAction::Continue));
    }

    let in_place_edit = app
        .read(|app| {
            app.ui
                .in_place_edit_index()
                .map(|idx| (idx, app.ui.get_input_text().to_string()))
        })
        .await;

    if let Some((idx, new_text)) = in_place_edit {
        dispatcher.dispatch_many(
            [
                AppAction::CompleteInPlaceEdit {
                    index: idx,
                    new_text,
                },
                AppAction::ClearInput,
            ],
            AppActionContext {
                term_width,
                term_height,
            },
        );
        return Ok(Some(KeyLoopAction::Continue));
    }

    process_input_submission(dispatcher, app, term_width, term_height).await;
    Ok(Some(KeyLoopAction::Continue))
}

async fn handle_ctrl_j_shortcut(
    dispatcher: &AppActionDispatcher,
    app: &AppHandle,
    term_width: u16,
    term_height: u16,
    _stream_service: &ChatStreamService,
    last_input_layout_update: &mut Instant,
) -> Result<Option<KeyLoopAction>, Box<dyn Error>> {
    let send_now = app
        .read(|app| app.ui.compose_mode && app.ui.file_prompt().is_none())
        .await;

    if !send_now {
        app.update(|app| {
            app.ui.apply_textarea_edit_and_recompute(term_width, |ta| {
                ta.insert_str("\n");
            });
        })
        .await;
        *last_input_layout_update = Instant::now();
        return Ok(Some(KeyLoopAction::Continue));
    }

    process_input_submission(dispatcher, app, term_width, term_height).await;
    Ok(Some(KeyLoopAction::Continue))
}

async fn handle_picker_key_event(
    dispatcher: &AppActionDispatcher,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) {
    let mut actions = Vec::new();

    match key.code {
        event::KeyCode::Esc => actions.push(AppAction::PickerEscape),
        event::KeyCode::Up => actions.push(AppAction::PickerMoveUp),
        event::KeyCode::Down => actions.push(AppAction::PickerMoveDown),
        event::KeyCode::Char('k') => actions.push(AppAction::PickerMoveUp),
        event::KeyCode::Char('j') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
            actions.push(AppAction::PickerApplySelection { persistent: true });
        }
        event::KeyCode::Char('j') => actions.push(AppAction::PickerMoveDown),
        event::KeyCode::Home => actions.push(AppAction::PickerMoveToStart),
        event::KeyCode::End => actions.push(AppAction::PickerMoveToEnd),
        event::KeyCode::F(6) => actions.push(AppAction::PickerCycleSortMode),
        event::KeyCode::Enter => {
            let persistent = key.modifiers.contains(event::KeyModifiers::ALT);
            actions.push(AppAction::PickerApplySelection { persistent });
        }
        event::KeyCode::Delete => actions.push(AppAction::PickerUnsetDefault),
        event::KeyCode::Backspace => actions.push(AppAction::PickerBackspace),
        event::KeyCode::Char(c) => {
            if !key.modifiers.contains(event::KeyModifiers::CONTROL) {
                actions.push(AppAction::PickerTypeChar { ch: c });
            }
        }
        _ => {}
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
