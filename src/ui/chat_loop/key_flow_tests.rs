//! End-to-end key flows: keys go through `route_keyboard_event`, queued
//! actions are applied until the queue is empty (as the event loop does), and
//! assertions run against app state and a rendered frame.

use super::*;
use crate::core::app::actions::{AppActionEnvelope, AppCommand};
use crate::core::app::picker::PickerMode;
use crate::core::app::App;
use crate::core::message::TranscriptRole;
use crate::ui::osc_backend::OscBackend;
use crate::ui::theme::Theme;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

const TERM_WIDTH: u16 = 80;
const TERM_HEIGHT: u16 = 24;
const MAX_SETTLE_ROUNDS: usize = 32;

/// Drives an app through the real key routing and action reducers.
///
/// Commands that would spawn background work (streams, model loads, MCP
/// calls) are recorded instead of executed so tests stay offline.
struct KeyFlow {
    app: AppHandle,
    registry: ModeAwareRegistry,
    dispatcher: AppActionDispatcher,
    action_rx: mpsc::UnboundedReceiver<AppActionEnvelope>,
    commands: Vec<&'static str>,
    exit_requested: bool,
    last_input_layout_update: Instant,
}

impl KeyFlow {
    fn new(app: App) -> Self {
        let (tx, action_rx) = mpsc::unbounded_channel();
        let (stream_service, _stream_rx) = ChatStreamService::new();
        Self {
            app: AppHandle::new(Arc::new(Mutex::new(app))),
            registry: build_mode_aware_registry(Arc::new(stream_service), offline_terminal()),
            dispatcher: AppActionDispatcher::new(tx),
            action_rx,
            commands: Vec::new(),
            exit_requested: false,
            last_input_layout_update: Instant::now(),
        }
    }

    fn with_default_app() -> Self {
        Self::new(App::new_test_app(Theme::dark_default(), true, true))
    }

    async fn press(&mut self, code: KeyCode) {
        self.press_with(code, KeyModifiers::NONE).await;
    }

    async fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let outcome = route_keyboard_event(
            &self.app,
            &self.registry,
            &self.dispatcher,
            KeyEvent::new(code, modifiers),
            Size::new(TERM_WIDTH, TERM_HEIGHT),
            &mut self.last_input_layout_update,
        )
        .await
        .expect("key routing should succeed");
        self.exit_requested |= outcome.exit_requested;
        self.settle().await;
    }

    async fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.press(KeyCode::Char(ch)).await;
        }
    }

    /// Applies queued actions until none remain, including actions that
    /// reducers enqueue while handling earlier ones.
    async fn settle(&mut self) {
        for _ in 0..MAX_SETTLE_ROUNDS {
            let mut pending = Vec::new();
            while let Ok(envelope) = self.action_rx.try_recv() {
                pending.push(envelope);
            }
            if pending.is_empty() {
                return;
            }
            let commands = self.app.update(|app| apply_actions(app, pending)).await;
            self.commands.extend(commands.iter().map(command_name));
        }
        panic!("action queue did not settle after {MAX_SETTLE_ROUNDS} rounds");
    }

    async fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        self.app.read(f).await
    }

    async fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        self.app.update(f).await
    }

    async fn input(&self) -> String {
        self.read(|app| app.ui.get_input_text().to_string()).await
    }

    async fn status(&self) -> Option<String> {
        self.read(|app| app.ui.status.clone()).await
    }

    async fn selected_label(&self) -> Option<String> {
        self.read(|app| {
            app.picker_state()
                .and_then(|state| state.get_selected_item())
                .map(|item| item.label.clone())
        })
        .await
    }

    /// Draws one frame with the real renderer and returns the screen as text.
    async fn render(&self) -> String {
        let backend = TestBackend::new(TERM_WIDTH, TERM_HEIGHT);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        self.app
            .update(|app| {
                terminal.draw(|f| ui(f, app)).expect("frame should render");
            })
            .await;
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content
            .chunks(width)
            .map(|row| {
                let line: String = row.iter().map(|cell| cell.symbol()).collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The registry needs a stdout terminal for the external-editor handler.
/// A fixed viewport avoids querying a real TTY, and these tests never draw
/// to it.
fn offline_terminal() -> SharedTerminal {
    let options = TerminalOptions {
        viewport: Viewport::Fixed(Rect::new(0, 0, TERM_WIDTH, TERM_HEIGHT)),
    };
    let terminal = Terminal::with_options(OscBackend::new(std::io::stdout()), options)
        .expect("fixed-viewport terminal");
    Arc::new(Mutex::new(terminal))
}

fn command_name(cmd: &AppCommand) -> &'static str {
    match cmd {
        AppCommand::SpawnStream(_) => "SpawnStream",
        AppCommand::LoadModelPicker(_) => "LoadModelPicker",
        AppCommand::RunMcpTool(_) => "RunMcpTool",
        AppCommand::RunMcpPrompt(_) => "RunMcpPrompt",
        AppCommand::RunMcpSampling(_) => "RunMcpSampling",
        AppCommand::SendMcpServerError { .. } => "SendMcpServerError",
        AppCommand::RefreshMcp { .. } => "RefreshMcp",
    }
}

/// Opens the theme picker the way a user would: typing `/theme` and Enter.
async fn open_theme_picker(flow: &mut KeyFlow) {
    flow.type_text("/theme").await;
    flow.press(KeyCode::Enter).await;
    let mode = flow.read(|app| app.current_picker_mode()).await;
    assert_eq!(
        mode,
        Some(PickerMode::Theme),
        "/theme should open the theme picker"
    );
}

/// Marks the currently selected picker item as the saved default, matching
/// how pickers label defaults with a trailing `*`.
async fn mark_selected_as_default(flow: &KeyFlow) -> String {
    flow.update(|app| {
        let state = app.picker_state_mut().expect("picker open");
        let item = &mut state.items[state.selected];
        item.label.push('*');
        item.label.clone()
    })
    .await
}

#[tokio::test]
async fn typing_and_enter_submits_message_and_starts_stream() {
    let mut flow = KeyFlow::with_default_app();

    flow.type_text("Hello there").await;
    assert_eq!(flow.input().await, "Hello there");

    flow.press(KeyCode::Enter).await;

    assert_eq!(flow.input().await, "", "input clears after submit");
    let last_user = flow
        .read(|app| {
            app.ui
                .messages
                .iter()
                .rev()
                .find(|m| m.role == TranscriptRole::User)
                .map(|m| m.content.clone())
        })
        .await;
    assert_eq!(last_user.as_deref(), Some("Hello there"));
    assert_eq!(flow.commands, vec!["SpawnStream"]);
    assert!(flow.render().await.contains("Hello there"));
}

#[tokio::test]
async fn shift_tab_does_not_insert_text_while_typing() {
    let mut flow = KeyFlow::with_default_app();

    flow.type_text("ab").await;
    flow.press_with(KeyCode::BackTab, KeyModifiers::SHIFT).await;
    flow.type_text("c").await;

    assert_eq!(flow.input().await, "abc");
}

#[tokio::test]
async fn shift_tab_does_not_insert_tab_in_file_prompt() {
    let mut flow = KeyFlow::with_default_app();
    flow.update(|app| app.ui.start_file_prompt_dump("dump".into()))
        .await;

    flow.press_with(KeyCode::BackTab, KeyModifiers::SHIFT).await;
    flow.type_text(".md").await;

    assert_eq!(flow.input().await, "dump.md");
}

#[tokio::test]
async fn theme_picker_navigates_and_escape_closes_without_applying() {
    let mut flow = KeyFlow::with_default_app();
    let theme_before = flow.read(|app| app.ui.current_theme_id.clone()).await;

    open_theme_picker(&mut flow).await;
    assert_eq!(flow.input().await, "", "command text is consumed");
    let first = flow.selected_label().await.expect("selection");

    flow.press(KeyCode::Down).await;
    let second = flow.selected_label().await.expect("selection");
    assert_ne!(first, second, "Down moves the selection");

    flow.press(KeyCode::Up).await;
    assert_eq!(flow.selected_label().await.as_deref(), Some(first.as_str()));

    flow.press(KeyCode::Esc).await;
    let (picker_open, theme_after) = flow
        .read(|app| {
            (
                app.active_picker().is_some(),
                app.ui.current_theme_id.clone(),
            )
        })
        .await;
    assert!(!picker_open, "Esc closes the picker");
    assert_eq!(theme_after, theme_before, "Esc keeps the original theme");
}

#[tokio::test]
async fn picker_typing_filters_instead_of_editing_input() {
    let mut flow = KeyFlow::with_default_app();
    open_theme_picker(&mut flow).await;

    flow.type_text("zzz-no-such-theme").await;

    assert_eq!(flow.input().await, "", "keys go to the picker filter");
    let visible = flow
        .read(|app| app.picker_state().map(|s| s.items.len()).unwrap_or(0))
        .await;
    assert_eq!(visible, 0, "filter hides non-matching themes");

    for _ in 0.."zzz-no-such-theme".len() {
        flow.press(KeyCode::Backspace).await;
    }
    let restored = flow
        .read(|app| app.picker_state().map(|s| s.items.len()).unwrap_or(0))
        .await;
    assert!(restored > 1, "clearing the filter restores the list");
}

#[tokio::test]
async fn delete_on_non_default_picker_item_explains_and_keeps_picker_open() {
    let mut flow = KeyFlow::with_default_app();
    open_theme_picker(&mut flow).await;
    let before = flow.selected_label().await.expect("selection");
    assert!(!before.ends_with('*'), "test config has no default theme");

    flow.press(KeyCode::Delete).await;

    assert_eq!(
        flow.status().await.as_deref(),
        Some("Del key only works on default items (marked with *)")
    );
    assert_eq!(flow.selected_label().await, Some(before));
    assert_eq!(
        flow.read(|app| app.current_picker_mode()).await,
        Some(PickerMode::Theme)
    );
    assert!(flow
        .render()
        .await
        .contains("Del key only works on default items"));
}

#[tokio::test]
async fn delete_on_default_theme_clears_default_and_refreshes_picker() {
    let mut flow = KeyFlow::with_default_app();
    open_theme_picker(&mut flow).await;
    flow.press(KeyCode::Down).await;
    let default_label = mark_selected_as_default(&flow).await;
    let default_id = flow
        .read(|app| {
            app.picker_state()
                .and_then(|s| s.selected_id())
                .map(str::to_string)
        })
        .await
        .expect("selection");

    flow.press(KeyCode::Delete).await;

    assert_eq!(
        flow.status().await,
        Some(format!("Removed default: {default_id}"))
    );
    let (mode, labels) = flow
        .read(|app| {
            (
                app.current_picker_mode(),
                app.picker_state()
                    .map(|s| s.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
        })
        .await;
    assert_eq!(mode, Some(PickerMode::Theme), "picker stays open after Del");
    assert!(
        labels.iter().all(|label| !label.ends_with('*')),
        "refreshed picker no longer marks a default: {labels:?}"
    );
    assert!(!labels.contains(&default_label));
    assert!(flow.render().await.contains("Removed default"));
}
