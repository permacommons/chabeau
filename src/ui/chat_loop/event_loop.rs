//! Event polling, dispatching, and UI rendering loop.
//!
//! This module orchestrates the main event loop for the chat interface.
//! It polls terminal input, resolves mode-aware keybindings, dispatches
//! high-level actions, handles background commands like streaming and
//! model fetching, and triggers UI redraws.
//!
//! The event loop wraps the shared [`App`](crate::core::app::App) in an
//! async mutex and coordinates with Tokio tasks for concurrent operations.

use std::{
    error::Error,
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const ACTIVE_POLL_INTERVAL_MS: u64 = 10;
const ANIMATION_POLL_INTERVAL_MS: u64 = 16;
const IDLE_POLL_INTERVAL_MS: u64 = 100;
const IDLE_SLEEP_MS: u64 = 50;
const INPUT_BURST_POLL_INTERVAL_MS: u64 = 1;
const INPUT_BURST_WINDOW_MS: u64 = 200;

use crate::character::CharacterService;
use crate::core::app::{
    apply_actions, AppActionContext, AppActionDispatcher, AppActionEnvelope, AppCommand,
    ComposeAction, InspectAction, InspectMode, StreamingAction,
};
use crate::core::chat_stream::{ChatStreamService, StreamMessage};
use crate::ui::renderer::ui;
use ratatui::crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
use ratatui::prelude::Size;
use tokio::sync::mpsc;

use super::executors::mcp_init::spawn_mcp_initializer;
use super::executors::mcp_tools::{
    spawn_mcp_prompt_call, spawn_mcp_refresh, spawn_mcp_sampling_call, spawn_mcp_server_error,
    spawn_mcp_tool_call,
};
use super::executors::model_loader::spawn_model_picker_loader;
use super::executors::ExecutorContext;
use super::keybindings::{
    build_mode_aware_registry, KeyContext, KeyExecutionContext, KeyHandlingContext, KeyResult,
    ModeAwareRegistry,
};
use super::lifecycle::{
    apply_cursor_color_to_terminal, restore_terminal, setup_terminal, SharedTerminal,
};
use super::setup::bootstrap_app;
use super::AppHandle;

pub struct RunChatOptions {
    pub model: String,
    pub log: Option<String>,
    pub provider: Option<String>,
    pub env_only: bool,
    pub character: Option<String>,
    pub persona: Option<String>,
    pub preset: Option<String>,
    pub disable_mcp: bool,
    pub character_service: CharacterService,
}

#[derive(Debug)]
pub enum UiEvent {
    Crossterm(Event),
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
    resized: bool,
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
        resized: false,
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
                handle_paste_event(
                    dispatcher,
                    term_size.width,
                    term_size.height,
                    text,
                    last_input_layout_update,
                )
                .await;
                outcome.request_redraw = true;
            }
            UiEvent::Crossterm(Event::Resize(_, _)) => {
                outcome.request_redraw = true;
                outcome.resized = true;
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
                || app.provider_picker_state().is_some()
                || app.character_picker_state().is_some()
                || app.persona_picker_state().is_some()
                || app.preset_picker_state().is_some();
            KeyContext::from_ui_mode(&app.ui.mode, picker_open)
        })
        .await;

    if key.code == event::KeyCode::Tab && key.modifiers.is_empty() {
        let inspect_mode = app
            .read(|app| app.inspect_state().map(|state| state.mode))
            .await;
        if matches!(
            inspect_mode,
            Some(InspectMode::ToolCalls {
                kind: crate::core::app::ToolInspectKind::Result,
                ..
            })
        ) {
            dispatcher.dispatch_input_many(
                [InspectAction::ToggleView],
                AppActionContext {
                    term_width: term_size.width,
                    term_height: term_size.height,
                },
            );
            return Ok(KeyboardEventOutcome {
                request_redraw: true,
                exit_requested: false,
            });
        }
    }

    if matches!(
        key.code,
        event::KeyCode::Char('c') | event::KeyCode::Char('C')
    ) && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
    {
        let inspect_mode = app
            .read(|app| app.inspect_state().map(|state| state.mode))
            .await;
        if matches!(inspect_mode, Some(InspectMode::ToolCalls { .. })) {
            dispatcher.dispatch_input_many(
                [InspectAction::Copy],
                AppActionContext {
                    term_width: term_size.width,
                    term_height: term_size.height,
                },
            );
            return Ok(KeyboardEventOutcome {
                request_redraw: true,
                exit_requested: false,
            });
        }
    }

    if matches!(
        key.code,
        event::KeyCode::Char('d') | event::KeyCode::Char('D')
    ) && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
    {
        let inspect_mode = app
            .read(|app| app.inspect_state().map(|state| state.mode))
            .await;
        if matches!(inspect_mode, Some(InspectMode::ToolCalls { .. })) {
            dispatcher.dispatch_input_many(
                [InspectAction::ToggleDecode],
                AppActionContext {
                    term_width: term_size.width,
                    term_height: term_size.height,
                },
            );
            return Ok(KeyboardEventOutcome {
                request_redraw: true,
                exit_requested: false,
            });
        }
    }

    if key.code == event::KeyCode::Tab
        && !matches!(context, KeyContext::Picker)
        && key.modifiers.is_empty()
    {
        if matches!(context, KeyContext::EditSelect | KeyContext::BlockSelect) {
            return Ok(KeyboardEventOutcome {
                request_redraw: false,
                exit_requested: false,
            });
        }

        let should_complete = app
            .read(|app| app.ui.is_input_active() && app.ui.get_input_text().starts_with('/'))
            .await;

        if should_complete {
            let handled = app
                .update(|app| app.complete_slash_command(term_size.width))
                .await;
            return Ok(KeyboardEventOutcome {
                request_redraw: handled,
                exit_requested: false,
            });
        }

        app.update(|app| app.ui.toggle_focus()).await;
        return Ok(KeyboardEventOutcome {
            request_redraw: true,
            exit_requested: false,
        });
    }

    let mut handle_as_text_input = mode_registry.should_handle_as_text_input(&key, &context);

    if handle_as_text_input && matches!(context, KeyContext::FilePrompt) {
        let input_focused = app.read(|app| app.ui.is_input_focused()).await;
        let is_plain_character = matches!(key.code, event::KeyCode::Char(_))
            && !key.modifiers.contains(KeyModifiers::CONTROL);

        if !input_focused && !is_plain_character {
            handle_as_text_input = false;
        }
    }

    if app.read(|app| app.inspect_state().is_some()).await {
        handle_as_text_input = false;
    }

    if handle_as_text_input {
        app.update(|app| {
            app.ui.focus_input();
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
            &key,
            context,
            KeyExecutionContext { app, dispatcher },
            KeyHandlingContext {
                term_width: term_size.width,
                term_height: term_size.height,
                last_input_layout_update: Some(*last_input_layout_update),
            },
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

pub(crate) fn sanitize_pasted_text(text: &str) -> String {
    let without_crlf = text.replace("\r\n", "\n");
    let without_cr = without_crlf.replace('\r', "\n");
    let expanded_tabs = without_cr.replace('\t', "    ");
    expanded_tabs
        .chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect()
}

pub(crate) async fn handle_paste_event(
    dispatcher: &AppActionDispatcher,
    term_width: u16,
    term_height: u16,
    text: String,
    last_input_layout_update: &mut Instant,
) {
    let sanitized_text = sanitize_pasted_text(&text);
    if sanitized_text.is_empty() {
        return;
    }

    dispatcher.dispatch_input_many(
        [ComposeAction::InsertIntoInput {
            text: sanitized_text,
        }],
        AppActionContext {
            term_width,
            term_height,
        },
    );
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
    let mut chunk_stream_id = None;

    while let Ok((message, msg_stream_id)) = rx.try_recv() {
        if msg_stream_id != current_stream_id {
            continue;
        }

        match message {
            StreamMessage::Chunk(content) => {
                coalesced_chunks.push_str(&content);
                chunk_stream_id = Some(msg_stream_id);
            }
            StreamMessage::ToolCallDelta(delta) => {
                followup_actions.push(StreamingAction::StreamToolCallDelta {
                    delta,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::App { kind, content } => {
                followup_actions.push(StreamingAction::StreamAppMessage {
                    kind,
                    message: content,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::Error(err) => {
                followup_actions.push(StreamingAction::StreamErrored {
                    message: err,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::End => followup_actions.push(StreamingAction::StreamCompleted {
                stream_id: msg_stream_id,
            }),
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
        if let Some(stream_id) = chunk_stream_id {
            actions.push(StreamingAction::AppendResponseChunk {
                content: chunk,
                stream_id,
            });
        }
    }
    actions.extend(followup_actions);

    if !actions.is_empty() {
        dispatcher.dispatch_streaming_many(actions, ctx);
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
            AppCommand::SpawnStream(params) => {
                stream_service.spawn_stream(params);
            }
            AppCommand::LoadModelPicker(request) => {
                spawn_model_picker_loader(dispatcher.clone(), request);
            }
            AppCommand::RunMcpTool(request) => {
                let context = ExecutorContext::from_app(app.clone(), dispatcher.clone()).await;
                spawn_mcp_tool_call(context, request);
            }
            AppCommand::RunMcpPrompt(request) => {
                let context = ExecutorContext::from_app(app.clone(), dispatcher.clone()).await;
                spawn_mcp_prompt_call(context, request);
            }
            AppCommand::RunMcpSampling(request) => {
                let context = ExecutorContext::from_app(app.clone(), dispatcher.clone()).await;
                spawn_mcp_sampling_call(context, *request);
            }
            AppCommand::SendMcpServerError {
                server_id,
                request_id,
                error,
            } => {
                let context = ExecutorContext::from_app(app.clone(), dispatcher.clone()).await;
                spawn_mcp_server_error(context, server_id, request_id, error);
            }
            AppCommand::RefreshMcp { server_id } => {
                let context = ExecutorContext::from_app(app.clone(), dispatcher.clone()).await;
                spawn_mcp_refresh(context, server_id);
            }
        }
    }
    true
}

fn spawn_event_reader(
    event_tx: mpsc::UnboundedSender<UiEvent>,
    poll_interval_ms: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let interval = poll_interval_ms.load(Ordering::Relaxed).max(1);
            if let Ok(true) = event::poll(Duration::from_millis(interval)) {
                match event::read() {
                    Ok(ev) => {
                        if event_tx.send(UiEvent::Crossterm(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        tokio::task::yield_now().await;
                        continue;
                    }
                }
            } else {
                tokio::task::yield_now().await;
            }
        }
    })
}

pub async fn run_chat(options: RunChatOptions) -> Result<(), Box<dyn Error>> {
    let app = bootstrap_app(options).await?;

    app.update(|app| {
        app.conversation().show_character_greeting_if_needed();
    })
    .await;

    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<AppActionEnvelope>();
    let action_dispatcher = AppActionDispatcher::new(action_tx);
    let (mcp_request_tx, mut mcp_request_rx) =
        mpsc::unbounded_channel::<crate::mcp::events::McpServerRequest>();
    app.update(|app| {
        app.mcp.set_request_sender(mcp_request_tx.clone());
    })
    .await;
    {
        let app = app.clone();
        let dispatcher = action_dispatcher.clone();
        tokio::spawn(async move {
            while let Some(request) = mcp_request_rx.recv().await {
                let term_size = app.read(|app| app.ui.last_term_size).await;
                let ctx = AppActionContext {
                    term_width: term_size.width,
                    term_height: term_size.height,
                };
                dispatcher.dispatch_many(
                    [StreamingAction::McpServerRequestReceived {
                        request: Box::new(request),
                    }],
                    ctx,
                );
            }
        });
    }
    let has_enabled_mcp = app
        .read(|app| app.mcp.servers().any(|server| server.config.is_enabled()))
        .await;
    if has_enabled_mcp {
        app.update(|app| {
            app.session.mcp_init.begin();
        })
        .await;
    }
    spawn_mcp_initializer(
        app.clone(),
        action_dispatcher.clone(),
        mcp_request_tx.clone(),
    );

    println!(
        "Chabeau is in the public domain, forever. Contribute: https://github.com/permacommons/chabeau"
    );

    let initial_cursor_color = app.read(|app| app.ui.theme.input_cursor_color).await;
    let terminal = setup_terminal(initial_cursor_color)?;
    let mut active_cursor_color = initial_cursor_color;

    let (stream_service, mut rx) = ChatStreamService::new();
    let stream_service = Arc::new(stream_service);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<UiEvent>();
    let poll_interval_ms = Arc::new(AtomicU64::new(ACTIVE_POLL_INTERVAL_MS));
    let event_reader_handle = spawn_event_reader(event_tx.clone(), poll_interval_ms.clone());

    let mode_registry = build_mode_aware_registry(stream_service.clone(), terminal.clone());

    const MAX_FPS: u64 = 60;
    let frame_duration = Duration::from_millis(1000 / MAX_FPS);
    let mut last_draw = Instant::now();
    let mut request_redraw = true;
    let mut last_input_layout_update = Instant::now();
    let mut last_input_event = Instant::now() - Duration::from_millis(INPUT_BURST_WINDOW_MS);
    let mut indicator_visible = false;
    let mut last_indicator_frame = Instant::now() - frame_duration;
    let mut tool_prompt_visible = false;
    let mut last_tool_prompt_frame = Instant::now() - frame_duration;

    let mut term_size = current_terminal_size(&terminal).await;
    app.update(|app| {
        app.ui.last_term_size = term_size;
    })
    .await;

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

        if event_outcome.events_processed {
            last_input_event = Instant::now();
        }

        if event_outcome.resized {
            term_size = current_terminal_size(&terminal).await;
            app.update(|app| {
                app.ui.last_term_size = term_size;
            })
            .await;
        }

        if event_outcome.request_redraw {
            request_redraw = true;
        }

        let theme_cursor_color = app.read(|app| app.ui.theme.input_cursor_color).await;
        if theme_cursor_color != active_cursor_color {
            apply_cursor_color_to_terminal(&terminal, theme_cursor_color).await?;
            active_cursor_color = theme_cursor_color;
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
        let tool_prompt_now = app.read(|app| app.ui.tool_prompt().is_some()).await;

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

        if tool_prompt_now != tool_prompt_visible {
            tool_prompt_visible = tool_prompt_now;
            request_redraw = true;
            if !tool_prompt_visible {
                last_tool_prompt_frame = Instant::now() - frame_duration;
            }
        }

        if tool_prompt_visible {
            let now = Instant::now();
            if now.duration_since(last_tool_prompt_frame) >= frame_duration {
                request_redraw = true;
                last_tool_prompt_frame = now;
            }
        }

        let animating = indicator_visible || tool_prompt_visible;
        let in_input_burst = Instant::now().duration_since(last_input_event)
            < Duration::from_millis(INPUT_BURST_WINDOW_MS);
        let idle = !event_outcome.events_processed
            && !received_any
            && !request_redraw
            && !animating
            && !in_input_burst;
        let desired_poll_ms = if in_input_burst {
            INPUT_BURST_POLL_INTERVAL_MS
        } else if animating {
            ANIMATION_POLL_INTERVAL_MS
        } else if idle {
            IDLE_POLL_INTERVAL_MS
        } else {
            ACTIVE_POLL_INTERVAL_MS
        };
        poll_interval_ms.store(desired_poll_ms, Ordering::Relaxed);

        if idle {
            tokio::time::sleep(Duration::from_millis(IDLE_SLEEP_MS)).await;
        }
    };

    event_reader_handle.abort();
    restore_terminal(&terminal).await?;

    let (should_print, last_term_size) = app
        .read(|app| (app.ui.print_transcript_on_exit, app.ui.last_term_size))
        .await;

    if should_print {
        app.update(|app| {
            let lines = app.ui.get_prewrapped_lines_cached(last_term_size.width);
            for line in lines {
                println!("{line}");
            }
        })
        .await;
    }

    result
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod event_loop_tests;
