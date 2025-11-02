use std::error::Error;
use std::io;
use std::time::Instant;

use ratatui::crossterm::event::{self, KeyCode};
use ratatui::Terminal;

use crate::core::app::ui_state::{EditSelectTarget, FilePromptKind};
use crate::core::app::{AppAction, AppActionContext, AppActionDispatcher};
use crate::core::chat_stream::ChatStreamService;
use crate::core::message::{ROLE_ASSISTANT, ROLE_USER};
use crate::ui::osc_backend::OscBackend;
use crate::utils::editor::{launch_external_editor, ExternalEditorOutcome};

use super::keybindings::{
    scroll_block_into_view, wrap_next_index, wrap_previous_index, KeyLoopAction,
};
use super::AppHandle;

pub fn language_to_extension(lang: Option<&str>) -> &'static str {
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

pub async fn handle_edit_select_mode_event(
    app: &AppHandle,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    app.update(|app| {
        if !app.ui.in_edit_select_mode() {
            return false;
        }

        let Some(target) = app.ui.edit_select_target() else {
            return false;
        };

        match key.code {
            KeyCode::Esc => {
                app.ui.exit_edit_select_mode();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match target {
                    EditSelectTarget::User => {
                        if let Some(current) = app.ui.selected_user_message_index() {
                            let prev = {
                                let ui = &app.ui;
                                ui.prev_user_message_index(current)
                                    .or_else(|| ui.last_user_message_index())
                            };
                            if let Some(prev) = prev {
                                app.ui.set_selected_user_message_index(prev);
                                app.conversation().scroll_index_into_view(
                                    prev,
                                    term_width,
                                    term_height,
                                );
                            }
                        } else if let Some(last) = app.ui.last_user_message_index() {
                            app.ui.set_selected_user_message_index(last);
                        }
                    }
                    EditSelectTarget::Assistant => {
                        if let Some(current) = app.ui.selected_assistant_message_index() {
                            let prev = {
                                let ui = &app.ui;
                                ui.prev_assistant_message_index(current)
                                    .or_else(|| ui.last_assistant_message_index())
                            };
                            if let Some(prev) = prev {
                                app.ui.set_selected_assistant_message_index(prev);
                                app.conversation().scroll_index_into_view(
                                    prev,
                                    term_width,
                                    term_height,
                                );
                            }
                        } else if let Some(last) = app.ui.last_assistant_message_index() {
                            app.ui.set_selected_assistant_message_index(last);
                        }
                    }
                }
                true
            }

            KeyCode::Down | KeyCode::Char('j') => {
                match target {
                    EditSelectTarget::User => {
                        if let Some(current) = app.ui.selected_user_message_index() {
                            let next = {
                                let ui = &app.ui;
                                ui.next_user_message_index(current)
                                    .or_else(|| ui.first_user_message_index())
                            };
                            if let Some(next) = next {
                                app.ui.set_selected_user_message_index(next);
                                app.conversation().scroll_index_into_view(
                                    next,
                                    term_width,
                                    term_height,
                                );
                            }
                        } else if let Some(last) = app.ui.last_user_message_index() {
                            app.ui.set_selected_user_message_index(last);
                        }
                    }
                    EditSelectTarget::Assistant => {
                        if let Some(current) = app.ui.selected_assistant_message_index() {
                            let next = {
                                let ui = &app.ui;
                                ui.next_assistant_message_index(current)
                                    .or_else(|| ui.first_assistant_message_index())
                            };
                            if let Some(next) = next {
                                app.ui.set_selected_assistant_message_index(next);
                                app.conversation().scroll_index_into_view(
                                    next,
                                    term_width,
                                    term_height,
                                );
                            }
                        } else if let Some(last) = app.ui.last_assistant_message_index() {
                            app.ui.set_selected_assistant_message_index(last);
                        }
                    }
                }
                true
            }
            KeyCode::Enter => {
                let idx_opt = match target {
                    EditSelectTarget::User => app.ui.selected_user_message_index(),
                    EditSelectTarget::Assistant => app.ui.selected_assistant_message_index(),
                };

                if let Some(idx) = idx_opt {
                    if idx < app.ui.messages.len() {
                        let role_matches = match target {
                            EditSelectTarget::User => app.ui.messages[idx].role == ROLE_USER,
                            EditSelectTarget::Assistant => {
                                app.ui.messages[idx].role == ROLE_ASSISTANT
                            }
                        };

                        if role_matches {
                            let content = app.ui.messages[idx].content.clone();
                            {
                                let mut conversation = app.conversation();
                                conversation.cancel_current_stream();
                            }
                            app.ui.messages.truncate(idx);
                            app.invalidate_prewrap_cache();
                            let user_display_name = app.persona_manager.get_display_name();
                            let _ = app.session.logging.rewrite_log_without_last_response(
                                &app.ui.messages,
                                &user_display_name,
                            );
                            match target {
                                EditSelectTarget::User => {
                                    app.ui.set_input_text(content);
                                }
                                EditSelectTarget::Assistant => {
                                    app.ui.set_input_text_for_assistant_edit(content);
                                }
                            }
                            app.ui.exit_edit_select_mode();
                            app.ui.focus_input();
                            let input_area_height = app.ui.calculate_input_area_height(term_width);
                            {
                                let mut conversation = app.conversation();
                                let available_height = conversation
                                    .calculate_available_height(term_height, input_area_height);
                                conversation.update_scroll_position(available_height, term_width);
                            }
                        }
                    }
                }
                true
            }
            KeyCode::Char('E') | KeyCode::Char('e') => {
                let idx_opt = match target {
                    EditSelectTarget::User => app.ui.selected_user_message_index(),
                    EditSelectTarget::Assistant => app.ui.selected_assistant_message_index(),
                };

                if let Some(idx) = idx_opt {
                    if idx < app.ui.messages.len() {
                        let role_matches = match target {
                            EditSelectTarget::User => app.ui.messages[idx].role == ROLE_USER,
                            EditSelectTarget::Assistant => {
                                app.ui.messages[idx].role == ROLE_ASSISTANT
                            }
                        };

                        if role_matches {
                            let content = app.ui.messages[idx].content.clone();
                            app.ui.set_input_text(content);
                            app.ui.start_in_place_edit(idx);
                            app.ui.exit_edit_select_mode();
                        }
                    }
                }
                true
            }
            KeyCode::Delete => {
                let idx_opt = match target {
                    EditSelectTarget::User => app.ui.selected_user_message_index(),
                    EditSelectTarget::Assistant => app.ui.selected_assistant_message_index(),
                };

                if let Some(idx) = idx_opt {
                    if idx < app.ui.messages.len() {
                        let role_matches = match target {
                            EditSelectTarget::User => app.ui.messages[idx].role == ROLE_USER,
                            EditSelectTarget::Assistant => {
                                app.ui.messages[idx].role == ROLE_ASSISTANT
                            }
                        };

                        if role_matches {
                            {
                                let mut conversation = app.conversation();
                                conversation.cancel_current_stream();
                            }
                            app.ui.messages.truncate(idx);
                            app.invalidate_prewrap_cache();
                            let user_display_name = app.persona_manager.get_display_name();
                            let _ = app.session.logging.rewrite_log_without_last_response(
                                &app.ui.messages,
                                &user_display_name,
                            );
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
                }
                true
            }

            _ => false,
        }
    })
    .await
}

pub async fn handle_block_select_mode_event(
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
            Some(&app.ui.user_display_name),
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
            _ => false,
        }
    })
    .await
}

pub async fn handle_picker_key_event(
    app: &AppHandle,
    dispatcher: &AppActionDispatcher,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) {
    let mut actions = Vec::new();

    let inspect_active = app.read(|app| app.picker_inspect_state().is_some()).await;

    if inspect_active {
        let page_lines = term_height.saturating_sub(8).max(1) as i32;
        match key.code {
            event::KeyCode::Esc => actions.push(AppAction::PickerEscape),
            event::KeyCode::Up => actions.push(AppAction::PickerInspectScroll { lines: -1 }),
            event::KeyCode::Down => actions.push(AppAction::PickerInspectScroll { lines: 1 }),
            event::KeyCode::PageUp => {
                actions.push(AppAction::PickerInspectScroll { lines: -page_lines })
            }
            event::KeyCode::PageDown => {
                actions.push(AppAction::PickerInspectScroll { lines: page_lines })
            }
            event::KeyCode::Home => actions.push(AppAction::PickerInspectScrollToStart),
            event::KeyCode::End => actions.push(AppAction::PickerInspectScrollToEnd),
            _ => {}
        }
    } else {
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
            event::KeyCode::Char('o') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                actions.push(AppAction::PickerInspectSelection);
            }
            event::KeyCode::Char(c) => {
                if !key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    actions.push(AppAction::PickerTypeChar { ch: c });
                }
            }
            _ => {}
        }
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

pub async fn handle_external_editor_shortcut(
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

pub async fn process_input_submission(
    dispatcher: &AppActionDispatcher,
    app: &AppHandle,
    term_width: u16,
    term_height: u16,
) {
    let (input_text, editing_assistant) = app
        .read(|app| {
            let text = app.ui.get_input_text().to_string();
            let editing = app.ui.is_editing_assistant_message();
            if text.trim().is_empty() {
                (None, editing)
            } else {
                (Some(text), editing)
            }
        })
        .await;

    let Some(input_text) = input_text else {
        return;
    };

    let ctx = AppActionContext {
        term_width,
        term_height,
    };

    if editing_assistant {
        dispatcher.dispatch_many(
            [
                AppAction::CompleteAssistantEdit {
                    content: input_text,
                },
                AppAction::ClearInput,
            ],
            ctx,
        );
        return;
    }

    dispatcher.dispatch_many(
        [
            AppAction::ClearInput,
            AppAction::ProcessCommand { input: input_text },
        ],
        ctx,
    );
}

pub async fn handle_enter_key(
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

pub async fn handle_ctrl_j_shortcut(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{
        apply_actions, AppAction, AppActionDispatcher, AppActionEnvelope, AppCommand,
    };
    use crate::core::chat_stream::ChatStreamService;
    use crate::core::message::{Message, ROLE_ASSISTANT, ROLE_USER};
    use crate::ui::theme::Theme;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;
    use tokio::runtime::Runtime;
    use tokio::sync::{mpsc, Mutex};

    fn setup_app() -> AppHandle {
        let app = crate::core::app::App::new_test_app(Theme::dark_default(), true, true);
        AppHandle::new(Arc::new(Mutex::new(app)))
    }

    fn dispatcher() -> (
        AppActionDispatcher,
        mpsc::UnboundedReceiver<AppActionEnvelope>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (AppActionDispatcher::new(tx), rx)
    }

    #[test]
    fn language_extension_detection() {
        assert_eq!(language_to_extension(Some("rs")), "rs");
        assert_eq!(language_to_extension(Some("unknown")), "txt");
        assert_eq!(language_to_extension(None), "txt");
    }

    #[test]
    fn enter_key_dispatches_process_command_action() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.set_input_text("hello".into());
                })
                .await;

            let (dispatcher, mut rx) = dispatcher();
            let (stream_service, _rx) = ChatStreamService::new();
            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

            let result = handle_enter_key(&dispatcher, &handle, &key, 80, 24, &stream_service)
                .await
                .expect("enter result");
            assert_eq!(result, Some(KeyLoopAction::Continue));

            let envelopes: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert_eq!(envelopes.len(), 2);
            assert!(matches!(envelopes[0].action, AppAction::ClearInput));
            match &envelopes[1].action {
                AppAction::ProcessCommand { input } => assert_eq!(input, "hello"),
                _ => panic!("unexpected action"),
            }

            let commands = handle.update(|app| apply_actions(app, envelopes)).await;
            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], AppCommand::SpawnStream(_)));
        });
    }

    #[test]
    fn enter_key_completes_file_prompt_dump() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.start_file_prompt_dump("dump.txt".into());
                    app.ui.set_input_text("dump.txt".into());
                })
                .await;

            let (dispatcher, mut rx) = dispatcher();
            let (stream_service, _rx) = ChatStreamService::new();
            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

            let result = handle_enter_key(&dispatcher, &handle, &key, 80, 24, &stream_service)
                .await
                .expect("enter result");
            assert_eq!(result, Some(KeyLoopAction::Continue));

            let envelopes: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert_eq!(envelopes.len(), 1);
            match &envelopes[0].action {
                AppAction::CompleteFilePromptDump { filename, .. } => {
                    assert_eq!(filename, "dump.txt")
                }
                _ => panic!("unexpected action"),
            }
        });
    }

    #[test]
    fn edit_select_enter_refocuses_input() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.messages.push_back(Message {
                        role: ROLE_USER.to_string(),
                        content: "rewrite me".into(),
                    });
                    app.ui.enter_edit_select_mode(EditSelectTarget::User);
                })
                .await;

            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let handled = handle_edit_select_mode_event(&handle, &key, 80, 24).await;
            assert!(handled);

            let (input_text, focus_is_input, in_edit_select) = handle
                .read(|app| {
                    (
                        app.ui.get_input_text().to_string(),
                        app.ui.is_input_focused(),
                        app.ui.in_edit_select_mode(),
                    )
                })
                .await;

            assert_eq!(input_text, "rewrite me");
            assert!(focus_is_input);
            assert!(!in_edit_select);
        });
    }

    #[test]
    fn assistant_edit_select_enter_loads_message_into_input() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.messages.push_back(Message {
                        role: ROLE_USER.to_string(),
                        content: "keep".into(),
                    });
                    app.ui.messages.push_back(Message {
                        role: ROLE_ASSISTANT.to_string(),
                        content: "adjust me".into(),
                    });
                    app.ui.enter_edit_select_mode(EditSelectTarget::Assistant);
                })
                .await;

            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let handled = handle_edit_select_mode_event(&handle, &key, 80, 24).await;
            assert!(handled);

            let (input_text, edit_flag, remaining_messages) = handle
                .read(|app| {
                    (
                        app.ui.get_input_text().to_string(),
                        app.ui.is_editing_assistant_message(),
                        app.ui.messages.len(),
                    )
                })
                .await;

            assert_eq!(input_text, "adjust me");
            assert!(edit_flag);
            assert_eq!(remaining_messages, 1);
        });
    }

    #[test]
    fn assistant_edit_select_delete_truncates_without_flag() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.messages.push_back(Message {
                        role: ROLE_ASSISTANT.to_string(),
                        content: "to remove".into(),
                    });
                    app.ui.messages.push_back(Message {
                        role: ROLE_USER.to_string(),
                        content: "later".into(),
                    });
                    app.ui.enter_edit_select_mode(EditSelectTarget::Assistant);
                })
                .await;

            let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
            let handled = handle_edit_select_mode_event(&handle, &key, 80, 24).await;
            assert!(handled);

            let (message_count, edit_flag) = handle
                .read(|app| (app.ui.messages.len(), app.ui.is_editing_assistant_message()))
                .await;

            assert_eq!(message_count, 0);
            assert!(!edit_flag);
        });
    }

    #[test]
    fn assistant_edit_submission_appends_message_without_resend() {
        let runtime = Runtime::new().expect("runtime");
        runtime.block_on(async {
            let handle = setup_app();
            handle
                .update(|app| {
                    app.ui.messages.push_back(Message {
                        role: ROLE_USER.to_string(),
                        content: "keep".into(),
                    });
                    app.ui.messages.push_back(Message {
                        role: ROLE_ASSISTANT.to_string(),
                        content: "to edit".into(),
                    });
                    app.ui.enter_edit_select_mode(EditSelectTarget::Assistant);
                })
                .await;

            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let handled = handle_edit_select_mode_event(&handle, &key, 80, 24).await;
            assert!(handled);

            handle
                .update(|app| {
                    app.ui.set_input_text_for_assistant_edit("revised".into());
                })
                .await;

            let (dispatcher, mut rx) = dispatcher();
            process_input_submission(&dispatcher, &handle, 80, 24).await;

            let envelopes: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert_eq!(envelopes.len(), 2);
            match &envelopes[0].action {
                AppAction::CompleteAssistantEdit { content } => {
                    assert_eq!(content, "revised");
                }
                _ => panic!("unexpected action"),
            }
            assert!(matches!(envelopes[1].action, AppAction::ClearInput));

            let commands = handle.update(|app| apply_actions(app, envelopes)).await;
            assert!(commands.is_empty());

            let (message_count, last_role, last_content, editing_flag) = handle
                .read(|app| {
                    let last = app.ui.messages.back().cloned();
                    (
                        app.ui.messages.len(),
                        last.as_ref().map(|m| m.role.clone()),
                        last.map(|m| m.content.clone()),
                        app.ui.is_editing_assistant_message(),
                    )
                })
                .await;

            assert_eq!(message_count, 2);
            assert_eq!(last_role.as_deref(), Some(ROLE_ASSISTANT));
            assert_eq!(last_content.as_deref(), Some("revised"));
            assert!(!editing_flag);
        });
    }
}
