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
    sync::Arc,
    time::{Duration, Instant},
};

use ratatui::crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
use ratatui::prelude::Size;
use serde::Serialize;
use serde_json;
use tokio::sync::mpsc;

use crate::api::models::fetch_models;
use crate::character::CharacterService;
use crate::core::app::{
    apply_actions, AppAction, AppActionContext, AppActionDispatcher, AppActionEnvelope, AppCommand,
    InspectMode, ModelPickerRequest,
};
use crate::core::chat_stream::{ChatStreamService, StreamMessage};
use crate::core::mcp_auth::McpTokenStore;
use crate::core::message::AppMessageKind;
use crate::ui::renderer::ui;

use super::keybindings::{build_mode_aware_registry, KeyContext, KeyResult, ModeAwareRegistry};
use super::lifecycle::{
    apply_cursor_color_to_terminal, restore_terminal, setup_terminal, SharedTerminal,
};
use super::setup::bootstrap_app;
use super::AppHandle;

#[derive(Debug)]
pub enum UiEvent {
    Crossterm(Event),
}

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

fn spawn_mcp_initializer(app: AppHandle, dispatcher: AppActionDispatcher) {
    tokio::spawn(async move {
        let mcp_disabled = app.read(|app| app.session.mcp_disabled).await;
        if mcp_disabled {
            dispatcher.dispatch_many([AppAction::McpInitCompleted], AppActionContext::default());
            return;
        }

        let has_enabled_servers = app
            .read(|app| app.mcp.servers().any(|server| server.config.is_enabled()))
            .await;

        if !has_enabled_servers {
            dispatcher.dispatch_many([AppAction::McpInitCompleted], AppActionContext::default());
            return;
        }

        let keyring_enabled = app.read(|app| !app.session.startup_env_only).await;
        let token_store = McpTokenStore::new_with_keyring(keyring_enabled);

        let config = app.read(|app| app.config.clone()).await;
        let mut mcp = crate::mcp::client::McpClientManager::from_config(&config);
        mcp.connect_all(&token_store).await;

        let server_ids: Vec<String> = mcp
            .servers()
            .filter(|server| server.config.is_enabled())
            .map(|server| server.config.id.clone())
            .collect();

        for server_id in server_ids {
            mcp.refresh_tools(&server_id).await;
            mcp.refresh_prompts(&server_id).await;
            mcp.refresh_resources(&server_id).await;
            mcp.refresh_resource_templates(&server_id).await;
        }

        app.update(|app| {
            app.mcp = mcp;
            app.session.mcp_init_in_progress = false;
            app.session.mcp_init_complete = true;
        })
        .await;

        dispatcher.dispatch_many([AppAction::McpInitCompleted], AppActionContext::default());
    });
}

fn spawn_mcp_tool_call(
    app: AppHandle,
    dispatcher: AppActionDispatcher,
    request: crate::core::app::session::ToolCallRequest,
) {
    tokio::spawn(async move {
        let term_size = app.read(|app| app.ui.last_term_size).await;
        let ctx = AppActionContext {
            term_width: term_size.width,
            term_height: term_size.height,
        };

        let mut call_context = match app
            .read(|app| app.mcp.tool_call_context(&request.server_id))
            .await
        {
            Some(context) => context,
            None => {
                dispatcher.dispatch_many(
                    [AppAction::ToolCallCompleted {
                        tool_name: request.tool_name.clone(),
                        tool_call_id: request.tool_call_id.clone(),
                        result: Err("MCP server not available.".to_string()),
                    }],
                    ctx,
                );
                return;
            }
        };

        let result = if request
            .tool_name
            .eq_ignore_ascii_case(crate::mcp::MCP_READ_RESOURCE_TOOL)
        {
            let uri = match request
                .arguments
                .as_ref()
                .and_then(|args| args.get("uri"))
                .and_then(|value| value.as_str())
            {
                Some(uri) => uri.to_string(),
                None => {
                    dispatcher.dispatch_many(
                        [AppAction::ToolCallCompleted {
                            tool_name: request.tool_name.clone(),
                            tool_call_id: request.tool_call_id.clone(),
                            result: Err("Resource read requires uri.".to_string()),
                        }],
                        ctx,
                    );
                    return;
                }
            };

            crate::mcp::client::execute_resource_read(&mut call_context, &uri)
                .await
                .map(|result| serialize_mcp_result(&result))
        } else {
            crate::mcp::client::execute_tool_call(&mut call_context, &request)
                .await
                .map(|result| serialize_mcp_result(&result))
        };

        let session_id = call_context.session_id.clone();
        let error = result.as_ref().err().cloned();
        app.update(|app| {
            app.mcp
                .update_tool_call_session(&call_context.server_id, session_id, error);
        })
        .await;

        dispatcher.dispatch_many(
            [AppAction::ToolCallCompleted {
                tool_name: request.tool_name.clone(),
                tool_call_id: request.tool_call_id.clone(),
                result,
            }],
            ctx,
        );
    });
}

fn serialize_mcp_result<T: Serialize>(result: &T) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|_| "Unable to serialize MCP result.".to_string())
}

fn spawn_mcp_prompt_call(
    app: AppHandle,
    dispatcher: AppActionDispatcher,
    request: crate::core::app::session::McpPromptRequest,
) {
    tokio::spawn(async move {
        let term_size = app.read(|app| app.ui.last_term_size).await;
        let ctx = AppActionContext {
            term_width: term_size.width,
            term_height: term_size.height,
        };

        let mut call_context = match app
            .read(|app| app.mcp.prompt_call_context(&request.server_id))
            .await
        {
            Some(context) => context,
            None => {
                dispatcher.dispatch_many(
                    [AppAction::McpPromptCompleted {
                        request,
                        result: Err("MCP server not available.".to_string()),
                    }],
                    ctx,
                );
                return;
            }
        };

        let result = crate::mcp::client::execute_prompt(&mut call_context, &request).await;

        let session_id = call_context.session_id.clone();
        let error = result.as_ref().err().cloned();
        app.update(|app| {
            app.mcp
                .update_prompt_session(&call_context.server_id, session_id, error);
        })
        .await;

        dispatcher.dispatch_many([AppAction::McpPromptCompleted { request, result }], ctx);
    });
}

fn spawn_mcp_refresh(app: AppHandle, dispatcher: AppActionDispatcher, server_id: String) {
    tokio::spawn(async move {
        let mcp_disabled = app.read(|app| app.session.mcp_disabled).await;
        if mcp_disabled {
            app.update(|app| {
                app.conversation().add_app_message(
                    AppMessageKind::Info,
                    "MCP: **disabled for this session**\nMCP refresh skipped.".to_string(),
                );
                app.ui.focus_transcript();
                app.clear_status();
                app.ui
                    .end_activity(crate::core::app::ActivityKind::McpRefresh);
            })
            .await;
            dispatcher.dispatch_many([AppAction::ClearStatus], AppActionContext::default());
            return;
        }

        let keyring_enabled = app
            .read(|app| !cfg!(test) && !app.session.startup_env_only)
            .await;
        let token_store = McpTokenStore::new_with_keyring(keyring_enabled);

        let mut mcp = app.read(|app| app.mcp.clone()).await;
        mcp.connect_server(&server_id, &token_store).await;
        mcp.refresh_tools(&server_id).await;
        mcp.refresh_resources(&server_id).await;
        mcp.refresh_resource_templates(&server_id).await;
        mcp.refresh_prompts(&server_id).await;

        let output = mcp.server(&server_id).map(|server| {
            crate::commands::build_mcp_server_output(server, keyring_enabled, &token_store)
        });

        app.update(|app| {
            app.mcp = mcp;
            if let Some(message) = output {
                app.conversation()
                    .add_app_message(AppMessageKind::Info, message);
                app.ui.focus_transcript();
                let term_size = app.ui.last_term_size;
                if term_size.width > 0 && term_size.height > 0 {
                    let input_area_height = app.input_area_height(term_size.width);
                    let mut conversation = app.conversation();
                    let available_height = conversation
                        .calculate_available_height(term_size.height, input_area_height);
                    conversation.update_scroll_position(available_height, term_size.width);
                }
            } else {
                app.conversation()
                    .set_status(format!("Unknown MCP server: {}", server_id));
            }
            app.clear_status();
            app.ui
                .end_activity(crate::core::app::ActivityKind::McpRefresh);
        })
        .await;

        dispatcher.dispatch_many([AppAction::ClearStatus], AppActionContext::default());
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
            dispatcher.dispatch_many(
                [AppAction::InspectToolResultsToggleView],
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

    dispatcher.dispatch_many(
        [AppAction::InsertIntoInput {
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
                followup_actions.push(AppAction::StreamToolCallDelta {
                    delta,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::App { kind, content } => {
                followup_actions.push(AppAction::StreamAppMessage {
                    kind,
                    message: content,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::Error(err) => {
                followup_actions.push(AppAction::StreamErrored {
                    message: err,
                    stream_id: msg_stream_id,
                });
            }
            StreamMessage::End => followup_actions.push(AppAction::StreamCompleted {
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
            actions.push(AppAction::AppendResponseChunk {
                content: chunk,
                stream_id,
            });
        }
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
            AppCommand::SpawnStream(params) => {
                stream_service.spawn_stream(params);
            }
            AppCommand::LoadModelPicker(request) => {
                spawn_model_picker_loader(dispatcher.clone(), request);
            }
            AppCommand::RunMcpTool(request) => {
                spawn_mcp_tool_call(app.clone(), dispatcher.clone(), request);
            }
            AppCommand::RunMcpPrompt(request) => {
                spawn_mcp_prompt_call(app.clone(), dispatcher.clone(), request);
            }
            AppCommand::RefreshMcp { server_id } => {
                spawn_mcp_refresh(app.clone(), dispatcher.clone(), server_id);
            }
        }
    }
    true
}

fn spawn_event_reader(event_tx: mpsc::UnboundedSender<UiEvent>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Ok(true) = event::poll(Duration::from_millis(10)) {
                match event::read() {
                    Ok(ev) => {
                        if event_tx.send(UiEvent::Crossterm(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        continue;
                    }
                }
            } else {
                tokio::task::yield_now().await;
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
    character: Option<String>,
    persona: Option<String>,
    preset: Option<String>,
    disable_mcp: bool,
    character_service: CharacterService,
) -> Result<(), Box<dyn Error>> {
    let app = bootstrap_app(
        model.clone(),
        log.clone(),
        provider.clone(),
        env_only,
        character,
        persona,
        preset,
        disable_mcp,
        character_service,
    )
    .await?;

    app.update(|app| {
        app.conversation().show_character_greeting_if_needed();
    })
    .await;

    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<AppActionEnvelope>();
    let action_dispatcher = AppActionDispatcher::new(action_tx);
    let has_enabled_mcp = app
        .read(|app| app.mcp.servers().any(|server| server.config.is_enabled()))
        .await;
    if has_enabled_mcp {
        app.update(|app| {
            app.session.mcp_init_in_progress = true;
            app.session.mcp_init_complete = false;
        })
        .await;
    }
    spawn_mcp_initializer(app.clone(), action_dispatcher.clone());

    println!(
        "Chabeau is in the public domain, forever. Contribute: https://github.com/permacommons/chabeau"
    );

    let initial_cursor_color = app.read(|app| app.ui.theme.input_cursor_color).await;
    let terminal = setup_terminal(initial_cursor_color)?;
    let mut active_cursor_color = initial_cursor_color;

    let (stream_service, mut rx) = ChatStreamService::new();
    let stream_service = Arc::new(stream_service);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<UiEvent>();
    let event_reader_handle = spawn_event_reader(event_tx.clone());

    let mode_registry = build_mode_aware_registry(stream_service.clone(), terminal.clone());

    const MAX_FPS: u64 = 60;
    let frame_duration = Duration::from_millis(1000 / MAX_FPS);
    let mut last_draw = Instant::now();
    let mut request_redraw = true;
    let mut last_input_layout_update = Instant::now();
    let mut indicator_visible = false;
    let mut last_indicator_frame = Instant::now() - frame_duration;
    let mut tool_prompt_visible = false;
    let mut last_tool_prompt_frame = Instant::now() - frame_duration;

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
        app.update(|app| {
            app.ui.last_term_size = term_size;
        })
        .await;

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

        let idle = !event_outcome.events_processed && !received_any && !request_redraw;

        if idle {
            tokio::time::sleep(Duration::from_millis(16)).await;
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
mod tests {
    use super::*;
    use crate::core::app::actions::{
        apply_action, AppAction, AppActionContext, AppActionDispatcher, AppActionEnvelope,
        AppCommand,
    };
    use crate::core::app::ui_state::EditSelectTarget;
    use crate::core::app::App;
    use crate::core::message::{self, Message, ROLE_APP_ERROR, ROLE_ASSISTANT, ROLE_USER};
    use crate::ui::theme::Theme;
    use crate::utils::test_utils::create_test_app;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;
    use tokio::sync::Mutex;

    const TERM_WIDTH: u16 = 80;
    const TERM_HEIGHT: u16 = 24;

    fn new_dispatcher() -> AppActionDispatcher {
        let (tx, _rx) = mpsc::unbounded_channel();
        AppActionDispatcher::new(tx)
    }

    fn new_app_handle() -> AppHandle {
        AppHandle::new(Arc::new(Mutex::new(create_test_app())))
    }

    fn setup_service() -> (
        ChatStreamService,
        tokio::sync::mpsc::UnboundedReceiver<(StreamMessage, u64)>,
    ) {
        ChatStreamService::new()
    }

    fn setup_app() -> App {
        App::new_test_app(Theme::dark_default(), true, true)
    }

    fn default_context() -> AppActionContext {
        AppActionContext {
            term_width: TERM_WIDTH,
            term_height: TERM_HEIGHT,
        }
    }

    #[tokio::test]
    async fn tab_autocompletes_slash_commands() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.set_input_text("/he".into());
            app.ui.focus_input();
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(outcome.request_redraw);

        let (input, focus_is_input) = app
            .read(|app| {
                (
                    app.ui.get_input_text().to_string(),
                    app.ui.is_input_focused(),
                )
            })
            .await;
        assert_eq!(input, "/help ");
        assert!(focus_is_input);
    }

    #[tokio::test]
    async fn tab_toggles_focus_without_slash_prefix() {
        let app = new_app_handle();
        app.update(|app| app.ui.focus_transcript()).await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(outcome.request_redraw);
        assert!(app.read(|app| app.ui.is_input_focused()).await);

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(outcome.request_redraw);
        assert!(app.read(|app| app.ui.is_transcript_focused()).await);
    }

    #[tokio::test]
    async fn tab_does_not_switch_focus_in_edit_select_mode() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.messages.push_back(Message {
                role: ROLE_USER.to_string(),
                content: "hello".into(),
            });
            app.ui.enter_edit_select_mode(EditSelectTarget::User);
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(!outcome.request_redraw);
        let (focus_is_transcript, in_edit_select) = app
            .read(|app| (app.ui.is_transcript_focused(), app.ui.in_edit_select_mode()))
            .await;
        assert!(focus_is_transcript);
        assert!(in_edit_select);
    }

    #[tokio::test]
    async fn tab_does_not_switch_focus_in_assistant_edit_select_mode() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.messages.push_back(Message {
                role: ROLE_ASSISTANT.to_string(),
                content: "response".into(),
            });
            app.ui.enter_edit_select_mode(EditSelectTarget::Assistant);
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(!outcome.request_redraw);
        let (focus_is_transcript, in_edit_select, target_is_assistant) = app
            .read(|app| {
                (
                    app.ui.is_transcript_focused(),
                    app.ui.in_edit_select_mode(),
                    app.ui.edit_select_target() == Some(EditSelectTarget::Assistant),
                )
            })
            .await;
        assert!(focus_is_transcript);
        assert!(in_edit_select);
        assert!(target_is_assistant);
    }

    #[tokio::test]
    async fn tab_does_not_switch_focus_in_block_select_mode() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.enter_block_select_mode(0);
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("tab handling should succeed");

        assert!(!outcome.request_redraw);
        let (focus_is_transcript, in_block_select) = app
            .read(|app| {
                (
                    app.ui.is_transcript_focused(),
                    app.ui.in_block_select_mode(),
                )
            })
            .await;
        assert!(focus_is_transcript);
        assert!(in_block_select);
    }

    #[tokio::test]
    async fn arrow_key_in_file_prompt_keeps_transcript_focus() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.start_file_prompt_dump("dump.txt".into());
            app.ui.focus_transcript();
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("arrow handling should succeed");

        assert!(!outcome.request_redraw);
        let focus_is_transcript = app.read(|app| app.ui.is_transcript_focused()).await;
        assert!(focus_is_transcript);
    }

    #[tokio::test]
    async fn typing_in_file_prompt_refocuses_input() {
        let app = new_app_handle();
        app.update(|app| {
            app.ui.start_file_prompt_dump("dump".into());
            app.ui.focus_transcript();
        })
        .await;

        let dispatcher = new_dispatcher();
        let mode_registry = ModeAwareRegistry::new();
        let mut last_update = Instant::now();

        let outcome = route_keyboard_event(
            &app,
            &mode_registry,
            &dispatcher,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut last_update,
        )
        .await
        .expect("typing should succeed");

        assert!(outcome.request_redraw);
        let (text, focus_is_input) = app
            .read(|app| {
                (
                    app.ui.get_input_text().to_string(),
                    app.ui.is_input_focused(),
                )
            })
            .await;
        assert_eq!(text, "dumpz");
        assert!(focus_is_input);
    }

    #[test]
    fn sanitize_paste_text_removes_control_characters() {
        let input = "Hello\tworld\r\nThis is\x01fine";
        let sanitized = sanitize_pasted_text(input);
        assert_eq!(sanitized, "Hello    world\nThis isfine");
    }

    #[test]
    fn handle_paste_event_dispatches_insert_action() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let (dispatcher, mut rx) = {
                let (tx, rx) = mpsc::unbounded_channel();
                (AppActionDispatcher::new(tx), rx)
            };
            let mut last_update = Instant::now();

            handle_paste_event(
                &dispatcher,
                TERM_WIDTH,
                TERM_HEIGHT,
                "paste\tinput".into(),
                &mut last_update,
            )
            .await;

            let envelopes: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert!(envelopes
                .iter()
                .any(|env| matches!(env.action, AppAction::InsertIntoInput { .. })));

            let mut app = setup_app();
            let commands = apply_actions(&mut app, envelopes);
            assert!(commands.is_empty());
            assert_eq!(app.ui.get_input_text(), "paste    input");
        });
    }

    #[test]
    fn process_stream_updates_dispatches_actions() {
        let (service, mut rx) = setup_service();
        service.send_for_test(StreamMessage::Chunk("Hello".into()), 42);
        service.send_for_test(StreamMessage::Chunk(" world".into()), 42);
        service.send_for_test(StreamMessage::Error(" failure ".into()), 99);
        service.send_for_test(StreamMessage::End, 42);

        let mut app = setup_app();
        let (action_tx, mut action_rx) = mpsc::unbounded_channel::<AppActionEnvelope>();
        app.session.current_stream_id = 42;
        app.ui.messages.push_back(Message {
            role: ROLE_ASSISTANT.to_string(),
            content: String::new(),
        });
        app.ui.is_streaming = true;

        let dispatcher = AppActionDispatcher::new(action_tx);

        let processed = process_stream_updates(&dispatcher, &mut rx, TERM_WIDTH, TERM_HEIGHT, 42);
        assert!(processed);

        let mut envelopes = Vec::new();
        while let Ok(envelope) = action_rx.try_recv() {
            envelopes.push(envelope);
        }
        let commands = apply_actions(&mut app, envelopes);
        assert!(commands.is_empty());

        assert_eq!(app.ui.messages.back().unwrap().content, "Hello world");
        assert!(!app.ui.is_streaming);

        let last_message = app
            .ui
            .messages
            .iter()
            .rev()
            .find(|msg| message::is_app_message_role(&msg.role));
        assert!(last_message.is_none(), "non-matching error message ignored");
    }

    #[test]
    fn error_messages_add_system_entries_and_stop_streaming() {
        let mut app = setup_app();
        app.ui.is_streaming = true;
        app.session.current_stream_id = 42;

        let ctx = default_context();
        apply_action(
            &mut app,
            AppAction::StreamErrored {
                message: "API Error:\n```\napi failure\n```\n".into(),
                stream_id: 42,
            },
            ctx,
        );

        assert!(!app.ui.is_streaming);
        let last_message = app.ui.messages.back().expect("app message added");
        assert_eq!(last_message.role, ROLE_APP_ERROR);
        assert_eq!(last_message.content, "API Error:\n```\napi failure\n```");
    }

    #[test]
    fn end_messages_finalize_responses() {
        let mut app = setup_app();
        app.ui.is_streaming = true;
        app.session.retrying_message_index = Some(0);
        app.session.current_stream_id = 7;
        app.ui.current_response = "partial".into();

        let ctx = default_context();
        apply_action(&mut app, AppAction::StreamCompleted { stream_id: 7 }, ctx);

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
}
