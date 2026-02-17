use super::*;
use crate::core::app::actions::{
    apply_action, AppAction, AppActionContext, AppActionDispatcher, AppActionEnvelope, AppCommand,
    InputAction, StreamingAction,
};
use crate::core::app::ui_state::EditSelectTarget;
use crate::core::app::App;
use crate::core::message::{self, Message, TranscriptRole};
use crate::ui::theme::Theme;
use crate::utils::test_utils::create_test_app;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

const TERM_WIDTH: u16 = 80;
const TERM_HEIGHT: u16 = 24;

mod test_helpers {
    use super::*;

    pub(super) fn default_context() -> AppActionContext {
        AppActionContext {
            term_width: TERM_WIDTH,
            term_height: TERM_HEIGHT,
        }
    }
}

use test_helpers::default_context;

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
            role: TranscriptRole::User,
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
            role: TranscriptRole::Assistant,
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
        assert!(envelopes.iter().any(|env| matches!(
            env.action,
            AppAction::Input(InputAction::Compose(ComposeAction::InsertIntoInput { .. }))
        )));

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
        role: TranscriptRole::Assistant,
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
        .find(|msg| message::is_app_message_role(msg.role));
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
        AppAction::Streaming(StreamingAction::StreamErrored {
            message: "API Error:\n```\napi failure\n```\n".into(),
            stream_id: 42,
        }),
        ctx,
    );

    assert!(!app.ui.is_streaming);
    let last_message = app.ui.messages.back().expect("app message added");
    assert_eq!(last_message.role, TranscriptRole::AppError);
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
    apply_action(
        &mut app,
        AppAction::Streaming(StreamingAction::StreamCompleted { stream_id: 7 }),
        ctx,
    );

    assert!(!app.ui.is_streaming);
    assert!(app.session.retrying_message_index.is_none());
}

#[test]
fn submit_message_returns_spawn_command() {
    let mut app = setup_app();
    let ctx = default_context();
    let result = apply_action(
        &mut app,
        AppAction::Streaming(StreamingAction::SubmitMessage {
            message: "Hello".into(),
        }),
        ctx,
    );
    assert!(matches!(result, Some(AppCommand::SpawnStream(_))));
}

#[test]
fn retry_last_message_returns_none_without_history() {
    let mut app = setup_app();
    let ctx = default_context();
    let result = apply_action(
        &mut app,
        AppAction::Streaming(StreamingAction::RetryLastMessage),
        ctx,
    );
    assert!(result.is_none());
}

#[test]
fn retry_last_message_emits_command_with_history() {
    let mut app = setup_app();
    app.ui.messages.push_back(Message {
        role: TranscriptRole::User,
        content: "Hi".into(),
    });
    app.ui.messages.push_back(Message {
        role: TranscriptRole::Assistant,
        content: "Hello".into(),
    });
    app.session.last_retry_time = Instant::now() - Duration::from_millis(500);

    let ctx = default_context();
    let result = apply_action(
        &mut app,
        AppAction::Streaming(StreamingAction::RetryLastMessage),
        ctx,
    );
    assert!(matches!(result, Some(AppCommand::SpawnStream(_))));
}
