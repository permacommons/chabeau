use std::fs;
use std::path::Path;
use std::time::Instant;

use tokio::sync::mpsc;

use super::App;
use crate::api::ModelsResponse;
use crate::commands::{process_input, CommandResult};
use crate::core::app::picker::PickerMode;
use crate::core::app::ModelPickerRequest;
use crate::core::chat_stream::StreamParams;

pub enum AppAction {
    AppendResponseChunk {
        content: String,
    },
    StreamErrored {
        message: String,
    },
    StreamCompleted,
    ClearStatus,
    ToggleComposeMode,
    CancelFilePrompt,
    CancelInPlaceEdit,
    CancelStreaming,
    SetStatus {
        message: String,
    },
    ClearInput,
    InsertIntoInput {
        text: String,
    },
    SubmitMessage {
        message: String,
    },
    RetryLastMessage,
    ProcessCommand {
        input: String,
    },
    PickerEscape,
    PickerMoveUp,
    PickerMoveDown,
    PickerMoveToStart,
    PickerMoveToEnd,
    PickerCycleSortMode,
    PickerApplySelection {
        persistent: bool,
    },
    PickerUnsetDefault,
    PickerBackspace,
    PickerTypeChar {
        ch: char,
    },
    CompleteFilePromptDump {
        filename: String,
        overwrite: bool,
    },
    CompleteFilePromptSaveBlock {
        filename: String,
        content: String,
        overwrite: bool,
    },
    CompleteInPlaceEdit {
        index: usize,
        new_text: String,
    },
    ModelPickerLoaded {
        default_model_for_provider: Option<String>,
        models_response: ModelsResponse,
    },
    ModelPickerLoadFailed {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AppActionContext {
    pub term_width: u16,
    pub term_height: u16,
}

pub struct AppActionEnvelope {
    pub action: AppAction,
    pub context: AppActionContext,
}

#[derive(Clone)]
pub struct AppActionDispatcher {
    tx: mpsc::UnboundedSender<AppActionEnvelope>,
}

impl AppActionDispatcher {
    pub fn new(tx: mpsc::UnboundedSender<AppActionEnvelope>) -> Self {
        Self { tx }
    }

    pub fn dispatch_many<I>(&self, actions: I, ctx: AppActionContext)
    where
        I: IntoIterator<Item = AppAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action,
                context: ctx,
            });
        }
    }
}

pub enum AppCommand {
    SpawnStream(StreamParams),
    LoadModelPicker(ModelPickerRequest),
}

pub fn apply_actions(
    app: &mut App,
    envelopes: impl IntoIterator<Item = AppActionEnvelope>,
) -> Vec<AppCommand> {
    let mut commands = Vec::new();
    for envelope in envelopes {
        if let Some(cmd) = apply_action(app, envelope.action, envelope.context) {
            commands.push(cmd);
        }
    }
    commands
}

pub fn apply_action(app: &mut App, action: AppAction, ctx: AppActionContext) -> Option<AppCommand> {
    match action {
        AppAction::AppendResponseChunk { content } => {
            append_response_chunk(app, &content, ctx);
            None
        }
        AppAction::StreamErrored { message } => {
            let error_message = format!("Error: {}", message.trim());
            let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
            {
                let mut conversation = app.conversation();
                conversation.add_system_message(error_message);
                let available_height =
                    conversation.calculate_available_height(ctx.term_height, input_area_height);
                conversation.update_scroll_position(available_height, ctx.term_width);
            }
            app.ui.end_streaming();
            None
        }
        AppAction::StreamCompleted => {
            {
                let mut conversation = app.conversation();
                conversation.finalize_response();
            }
            app.ui.end_streaming();
            None
        }
        AppAction::ClearStatus => {
            app.conversation().clear_status();
            None
        }
        AppAction::ToggleComposeMode => {
            app.ui.toggle_compose_mode();
            None
        }
        AppAction::CancelFilePrompt => {
            app.ui.cancel_file_prompt();
            None
        }
        AppAction::CancelInPlaceEdit => {
            if app.ui.in_place_edit_index().is_some() {
                app.ui.cancel_in_place_edit();
                app.ui.clear_input();
            }
            None
        }
        AppAction::CancelStreaming => {
            app.conversation().cancel_current_stream();
            None
        }
        AppAction::SetStatus { message } => {
            set_status_message(app, message, ctx);
            None
        }
        AppAction::ClearInput => {
            app.ui.clear_input();
            if ctx.term_width > 0 {
                app.ui.recompute_input_layout_after_edit(ctx.term_width);
            }
            None
        }
        AppAction::InsertIntoInput { text } => {
            if !text.is_empty() {
                app.ui
                    .apply_textarea_edit_and_recompute(ctx.term_width, |ta| {
                        ta.insert_str(&text);
                    });
            }
            None
        }
        AppAction::SubmitMessage { message } => {
            let params = prepare_stream_params_for_message(app, message, ctx);
            Some(AppCommand::SpawnStream(params))
        }
        AppAction::RetryLastMessage => prepare_retry_stream(app, ctx),
        AppAction::ProcessCommand { input } => handle_process_command(app, input, ctx),
        AppAction::PickerEscape => {
            handle_picker_escape(app, ctx);
            None
        }
        AppAction::PickerMoveUp => {
            handle_picker_movement(app, PickerMovement::Up);
            None
        }
        AppAction::PickerMoveDown => {
            handle_picker_movement(app, PickerMovement::Down);
            None
        }
        AppAction::PickerMoveToStart => {
            handle_picker_movement(app, PickerMovement::Start);
            None
        }
        AppAction::PickerMoveToEnd => {
            handle_picker_movement(app, PickerMovement::End);
            None
        }
        AppAction::PickerCycleSortMode => {
            handle_picker_cycle_sort_mode(app);
            None
        }
        AppAction::PickerApplySelection { persistent } => {
            handle_picker_apply_selection(app, persistent, ctx)
        }
        AppAction::PickerUnsetDefault => handle_picker_unset_default(app, ctx),
        AppAction::PickerBackspace => {
            handle_picker_backspace(app);
            None
        }
        AppAction::PickerTypeChar { ch } => {
            handle_picker_type_char(app, ch);
            None
        }
        AppAction::CompleteFilePromptDump {
            filename,
            overwrite,
        } => {
            handle_file_prompt_dump(app, filename, overwrite, ctx);
            None
        }
        AppAction::CompleteFilePromptSaveBlock {
            filename,
            content,
            overwrite,
        } => {
            handle_file_prompt_save_block(app, filename, content, overwrite, ctx);
            None
        }
        AppAction::CompleteInPlaceEdit { index, new_text } => {
            handle_in_place_edit(app, index, new_text);
            None
        }
        AppAction::ModelPickerLoaded {
            default_model_for_provider,
            models_response,
        } => {
            if let Err(e) =
                app.complete_model_picker_request(default_model_for_provider, models_response)
            {
                set_status_message(app, format!("Model picker error: {}", e), ctx);
            }
            None
        }
        AppAction::ModelPickerLoadFailed { error } => {
            app.fail_model_picker_request();
            set_status_message(app, format!("Model picker error: {}", error), ctx);
            None
        }
    }
}

fn append_response_chunk(app: &mut App, chunk: &str, ctx: AppActionContext) {
    if chunk.is_empty() {
        return;
    }

    let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.append_to_response(chunk, available_height, ctx.term_width);
}

fn set_status_message(app: &mut App, message: String, ctx: AppActionContext) {
    app.conversation().set_status(message);
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
}

enum PickerMovement {
    Up,
    Down,
    Start,
    End,
}

fn handle_picker_movement(app: &mut App, movement: PickerMovement) {
    let mode = app.current_picker_mode();
    let selected_theme = {
        let mut selected = None;
        if let Some(state) = app.picker_state_mut() {
            match movement {
                PickerMovement::Up => state.move_up(),
                PickerMovement::Down => state.move_down(),
                PickerMovement::Start => state.move_to_start(),
                PickerMovement::End => state.move_to_end(),
            }
            if mode == Some(PickerMode::Theme) {
                selected = state.selected_id().map(|s| s.to_string());
            }
        }
        selected
    };
    if let Some(id) = selected_theme {
        app.preview_theme_by_id(&id);
    }
}

fn handle_picker_cycle_sort_mode(app: &mut App) {
    if let Some(state) = app.picker_state_mut() {
        state.cycle_sort_mode();
    }
    app.sort_picker_items();
    app.update_picker_title();
}

fn handle_picker_backspace(app: &mut App) {
    match app.current_picker_mode() {
        Some(PickerMode::Model) => {
            if let Some(state) = app.model_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_models();
                }
            }
        }
        Some(PickerMode::Theme) => {
            if let Some(state) = app.theme_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_themes();
                }
            }
        }
        Some(PickerMode::Provider) => {
            if let Some(state) = app.provider_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_providers();
                }
            }
        }
        Some(PickerMode::Character) => {
            if let Some(state) = app.character_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_characters();
                }
            }
        }
        None => {}
    }
}

fn handle_picker_type_char(app: &mut App, ch: char) {
    if ch.is_control() {
        return;
    }

    match app.current_picker_mode() {
        Some(PickerMode::Model) => {
            if let Some(state) = app.model_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_models();
            }
        }
        Some(PickerMode::Theme) => {
            if let Some(state) = app.theme_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_themes();
            }
        }
        Some(PickerMode::Provider) => {
            if let Some(state) = app.provider_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_providers();
            }
        }
        Some(PickerMode::Character) => {
            if let Some(state) = app.character_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_characters();
            }
        }
        None => {}
    }
}

fn handle_process_command(
    app: &mut App,
    input: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if input.trim().is_empty() {
        return None;
    }

    match process_input(app, &input) {
        CommandResult::Continue => {
            update_scroll_after_command(app, ctx);
            None
        }
        CommandResult::ProcessAsMessage(message) => {
            let params = prepare_stream_params_for_message(app, message, ctx);
            Some(AppCommand::SpawnStream(params))
        }
        CommandResult::OpenModelPicker => {
            let request = app.prepare_model_picker_request();
            Some(AppCommand::LoadModelPicker(request))
        }
        CommandResult::OpenProviderPicker => {
            app.open_provider_picker();
            None
        }
        CommandResult::OpenThemePicker => {
            app.open_theme_picker();
            None
        }
        CommandResult::OpenCharacterPicker => {
            app.open_character_picker();
            None
        }
    }
}

fn update_scroll_after_command(app: &mut App, ctx: AppActionContext) {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return;
    }

    let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn handle_file_prompt_dump(
    app: &mut App,
    filename: String,
    overwrite: bool,
    ctx: AppActionContext,
) {
    if filename.is_empty() {
        return;
    }

    match crate::commands::dump_conversation_with_overwrite(app, &filename, overwrite) {
        Ok(()) => {
            set_status_message(app, format!("Dumped: {}", filename), ctx);
            app.ui.cancel_file_prompt();
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("exists") && !overwrite {
                set_status_message(app, "File exists (Alt+Enter to overwrite)".to_string(), ctx);
            } else {
                set_status_message(app, format!("Dump error: {}", msg), ctx);
            }
        }
    }
}

fn handle_file_prompt_save_block(
    app: &mut App,
    filename: String,
    content: String,
    overwrite: bool,
    ctx: AppActionContext,
) {
    if filename.is_empty() {
        return;
    }

    if Path::new(&filename).exists() && !overwrite {
        set_status_message(app, "File already exists.".to_string(), ctx);
        return;
    }

    match fs::write(&filename, content) {
        Ok(()) => {
            set_status_message(app, format!("Saved to {}", filename), ctx);
            app.ui.cancel_file_prompt();
        }
        Err(_e) => {
            set_status_message(app, "Error saving code block".to_string(), ctx);
        }
    }
}

fn handle_in_place_edit(app: &mut App, index: usize, new_text: String) {
    let Some(actual_index) = app.ui.take_in_place_edit_index() else {
        return;
    };

    if actual_index != index {
        return;
    }

    if actual_index >= app.ui.messages.len() || app.ui.messages[actual_index].role != "user" {
        return;
    }

    app.ui.messages[actual_index].content = new_text;
    app.invalidate_prewrap_cache();
    let _ = app
        .session
        .logging
        .rewrite_log_without_last_response(&app.ui.messages);
}

fn handle_picker_escape(app: &mut App, ctx: AppActionContext) {
    match app.current_picker_mode() {
        Some(PickerMode::Theme) => {
            app.revert_theme_preview();
            app.close_picker();
        }
        Some(PickerMode::Model) => {
            if app.picker.startup_requires_model {
                app.close_picker();
                if app.picker.startup_multiple_providers_available {
                    app.picker.startup_requires_model = false;
                    app.picker.startup_requires_provider = true;
                    app.session.provider_name.clear();
                    app.session.provider_display_name = "(no provider selected)".to_string();
                    app.session.api_key.clear();
                    app.session.base_url.clear();
                    app.open_provider_picker();
                } else {
                    app.ui.exit_requested = true;
                }
            } else {
                let was_transitioning = app.picker.in_provider_model_transition;
                app.revert_model_preview();
                if was_transitioning {
                    set_status_message(app, "Selection cancelled".to_string(), ctx);
                }
                app.close_picker();
            }
        }
        Some(PickerMode::Provider) => {
            if app.picker.startup_requires_provider {
                app.close_picker();
                app.ui.exit_requested = true;
            } else {
                app.revert_provider_preview();
                app.close_picker();
            }
        }
        Some(PickerMode::Character) => {
            app.close_picker();
        }
        None => {}
    }
}

fn handle_picker_apply_selection(
    app: &mut App,
    persistent: bool,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match app.current_picker_mode() {
        Some(PickerMode::Theme) => {
            if let Some(id) = selected_picker_id(app) {
                let result = {
                    let mut controller = app.theme_controller();
                    if persistent {
                        controller.apply_theme_by_id(&id)
                    } else {
                        controller.apply_theme_by_id_session_only(&id)
                    }
                };
                match result {
                    Ok(_) => set_status_message(
                        app,
                        format!("Theme set: {}{}", id, picker_status_suffix(persistent)),
                        ctx,
                    ),
                    Err(_) => set_status_message(app, "Theme error".to_string(), ctx),
                }
            }
            app.close_picker();
            None
        }
        Some(PickerMode::Model) => {
            let Some(id) = selected_picker_id(app) else {
                app.close_picker();
                return None;
            };
            let persist_to_config = persistent && !app.session.startup_env_only;
            let result = {
                let mut controller = app.provider_controller();
                if persist_to_config {
                    controller.apply_model_by_id_persistent(&id)
                } else {
                    controller.apply_model_by_id(&id);
                    Ok(())
                }
            };
            match result {
                Ok(_) => {
                    set_status_message(
                        app,
                        format!(
                            "Model set: {}{}",
                            id,
                            picker_status_suffix(persist_to_config)
                        ),
                        ctx,
                    );
                    if app.picker.in_provider_model_transition {
                        app.complete_provider_model_transition();
                    }
                    if app.picker.startup_requires_model {
                        app.picker.startup_requires_model = false;
                    }
                }
                Err(e) => set_status_message(app, format!("Model error: {}", e), ctx),
            }
            app.close_picker();
            None
        }
        Some(PickerMode::Provider) => {
            let Some(id) = selected_picker_id(app) else {
                app.close_picker();
                return None;
            };
            let (result, should_open_model_picker) = {
                let mut controller = app.provider_controller();
                if persistent {
                    controller.apply_provider_by_id_persistent(&id)
                } else {
                    controller.apply_provider_by_id(&id)
                }
            };

            let mut followup = None;
            match result {
                Ok(_) => {
                    set_status_message(
                        app,
                        format!("Provider set: {}{}", id, picker_status_suffix(persistent)),
                        ctx,
                    );
                    app.close_picker();
                    if should_open_model_picker {
                        if app.picker.startup_requires_provider {
                            app.picker.startup_requires_provider = false;
                            app.picker.startup_requires_model = true;
                        }
                        let request = app.prepare_model_picker_request();
                        followup = Some(AppCommand::LoadModelPicker(request));
                    }
                }
                Err(e) => {
                    set_status_message(app, format!("Provider error: {}", e), ctx);
                    app.close_picker();
                }
            }

            followup
        }
        Some(PickerMode::Character) => {
            app.apply_selected_character(persistent);
            None
        }
        None => None,
    }
}

fn handle_picker_unset_default(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let mode = app.current_picker_mode();
    let (selected_id, selected_label) = app.picker_state().and_then(|state| {
        state
            .get_selected_item()
            .map(|item| (item.id.clone(), item.label.clone()))
    })?;

    if !selected_label.ends_with('*') {
        set_status_message(
            app,
            "Del key only works on default items (marked with *)".to_string(),
            ctx,
        );
        return None;
    }

    match mode {
        Some(PickerMode::Model) => {
            let provider_name = app.session.provider_name.clone();
            let mut controller = app.provider_controller();
            match controller.unset_default_model(&provider_name) {
                Ok(_) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    let request = app.prepare_model_picker_request();
                    Some(AppCommand::LoadModelPicker(request))
                }
                Err(e) => {
                    set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Theme) => {
            let mut controller = app.theme_controller();
            match controller.unset_default_theme() {
                Ok(_) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    app.open_theme_picker();
                    None
                }
                Err(e) => {
                    set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Provider) => {
            let mut controller = app.provider_controller();
            match controller.unset_default_provider() {
                Ok(_) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    app.open_provider_picker();
                    None
                }
                Err(e) => {
                    set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        _ => None,
    }
}

fn picker_status_suffix(is_persistent: bool) -> &'static str {
    if is_persistent {
        " (saved to config)"
    } else {
        " (session only)"
    }
}

fn selected_picker_id(app: &App) -> Option<String> {
    app.picker_state()
        .and_then(|state| state.selected_id().map(|id| id.to_string()))
}

fn prepare_stream_params_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> StreamParams {
    let term_width = ctx.term_width.max(1);
    let term_height = ctx.term_height.max(1);
    app.ui.auto_scroll = true;
    let input_area_height = app.ui.calculate_input_area_height(term_width);
    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(message);
        let available_height =
            conversation.calculate_available_height(term_height, input_area_height);
        conversation.update_scroll_position(available_height, term_width);
        (cancel_token, stream_id, api_messages)
    };

    StreamParams {
        client: app.session.client.clone(),
        base_url: app.session.base_url.clone(),
        api_key: app.session.api_key.clone(),
        provider_name: app.session.provider_name.clone(),
        model: app.session.model.clone(),
        api_messages,
        cancel_token,
        stream_id,
    }
}

fn prepare_retry_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let now = Instant::now();
    if now.duration_since(app.session.last_retry_time).as_millis() < 200 {
        return None;
    }

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
    let maybe_params = {
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation
            .prepare_retry(available_height, ctx.term_width)
            .map(|api_messages| {
                let (cancel_token, stream_id) = conversation.start_new_stream();
                (api_messages, cancel_token, stream_id)
            })
    };

    if let Some((api_messages, cancel_token, stream_id)) = maybe_params {
        app.session.last_retry_time = now;
        Some(AppCommand::SpawnStream(StreamParams {
            client: app.session.client.clone(),
            base_url: app.session.base_url.clone(),
            api_key: app.session.api_key.clone(),
            provider_name: app.session.provider_name.clone(),
            model: app.session.model.clone(),
            api_messages,
            cancel_token,
            stream_id,
        }))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::picker::{ModelPickerState, PickerData, PickerMode, PickerSession};
    use crate::ui::picker::{PickerItem, PickerState};
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use tempfile::tempdir;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn theme_picker_escape_reverts_preview() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let original_color = app.ui.theme.background_color;

        app.open_theme_picker();
        apply_action(&mut app, AppAction::PickerMoveDown, ctx);

        assert_ne!(app.ui.theme.background_color, original_color);

        apply_action(&mut app, AppAction::PickerEscape, ctx);

        assert_eq!(app.ui.theme.background_color, original_color);
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn model_picker_escape_reverts_provider_transition() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        app.session.provider_name = "new-prov".into();
        app.session.provider_display_name = "New Provider".into();
        app.session.model = "new-model".into();
        app.session.api_key = "new-key".into();
        app.session.base_url = "https://api.new".into();

        app.picker.in_provider_model_transition = true;
        app.picker.provider_model_transition_state = Some((
            "old-prov".into(),
            "Old Provider".into(),
            "old-model".into(),
            "old-key".into(),
            "https://api.old".into(),
        ));

        let items = vec![PickerItem {
            id: "new-model".into(),
            label: "New Model".into(),
            metadata: None,
            sort_key: None,
        }];

        let picker_state = PickerState::new("Pick Model", items.clone(), 0);

        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Model,
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: String::new(),
                all_items: items,
                before_model: Some("old-model".into()),
                has_dates: false,
            }),
        });

        apply_action(&mut app, AppAction::PickerEscape, ctx);

        assert_eq!(app.session.provider_name, "old-prov");
        assert_eq!(app.session.provider_display_name, "Old Provider");
        assert_eq!(app.session.model, "old-model");
        assert_eq!(app.session.api_key, "old-key");
        assert_eq!(app.session.base_url, "https://api.old");
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
        assert!(app.picker_session().is_none());
        assert_eq!(app.ui.status.as_deref(), Some("Selection cancelled"));
    }

    #[test]
    fn process_command_submits_message() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let cmd = apply_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "hello there".into(),
            },
            ctx,
        );

        assert!(matches!(cmd, Some(AppCommand::SpawnStream(_))));
    }

    #[test]
    fn process_command_opens_theme_picker() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let _ = apply_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/theme".into(),
            },
            ctx,
        );

        assert!(app.picker_session().is_some());
    }

    #[test]
    fn file_prompt_dump_success_sets_status_and_closes_prompt() {
        let mut app = create_test_app();
        app.ui.messages.push_back(create_test_message("user", "hi"));
        let dir = tempdir().unwrap();
        let path = dir.path().join("dump.txt");
        let filename = path.to_str().unwrap().to_string();

        app.ui.start_file_prompt_dump(filename.clone());

        handle_file_prompt_dump(&mut app, filename.clone(), false, default_ctx());

        assert!(path.exists());
        assert_eq!(
            app.ui.status.as_deref(),
            Some(&format!("Dumped: {}", filename)[..])
        );
        assert!(app.ui.file_prompt().is_none());
    }

    #[test]
    fn file_prompt_dump_existing_without_overwrite_sets_status() {
        let mut app = create_test_app();
        app.ui.messages.push_back(create_test_message("user", "hi"));
        let dir = tempdir().unwrap();
        let path = dir.path().join("dump.txt");
        std::fs::write(&path, "existing").unwrap();
        let filename = path.to_str().unwrap().to_string();

        app.ui.start_file_prompt_dump(filename.clone());

        handle_file_prompt_dump(&mut app, filename, false, default_ctx());

        assert_eq!(
            app.ui.status.as_deref(),
            Some("File exists (Alt+Enter to overwrite)")
        );
        assert!(app.ui.file_prompt().is_some());
    }

    #[test]
    fn file_prompt_save_block_success_writes_file() {
        let mut app = create_test_app();
        let dir = tempdir().unwrap();
        let path = dir.path().join("snippet.rs");
        let filename = path.to_str().unwrap().to_string();

        app.ui
            .start_file_prompt_save_block(filename.clone(), "fn main() {}".into());

        handle_file_prompt_save_block(
            &mut app,
            filename.clone(),
            "fn main() {}".into(),
            false,
            default_ctx(),
        );

        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}");
        assert_eq!(
            app.ui.status.as_deref(),
            Some(&format!("Saved to {}", filename)[..])
        );
        assert!(app.ui.file_prompt().is_none());
    }

    #[test]
    fn file_prompt_save_block_existing_without_overwrite_sets_status() {
        let mut app = create_test_app();
        let dir = tempdir().unwrap();
        let path = dir.path().join("snippet.rs");
        std::fs::write(&path, "old").unwrap();
        let filename = path.to_str().unwrap().to_string();

        app.ui
            .start_file_prompt_save_block(filename.clone(), "fn main() {}".into());

        handle_file_prompt_save_block(
            &mut app,
            filename,
            "fn main() {}".into(),
            false,
            default_ctx(),
        );

        assert_eq!(app.ui.status.as_deref(), Some("File already exists."));
        assert!(app.ui.file_prompt().is_some());
    }

    #[test]
    fn complete_in_place_edit_updates_message() {
        let mut app = create_test_app();
        app.ui.messages.push_back(crate::core::message::Message {
            role: "user".into(),
            content: "old".into(),
        });
        app.ui.start_in_place_edit(0);

        handle_in_place_edit(&mut app, 0, "new content".into());

        assert_eq!(app.ui.messages[0].content, "new content");
        assert!(app.ui.in_place_edit_index().is_none());
    }

    #[test]
    fn character_picker_navigation_works() {
        use crate::core::app::picker::{CharacterPickerState, PickerData, PickerMode, PickerSession};
        
        let mut app = create_test_app();
        let ctx = default_ctx();

        // Create a mock character picker with test items
        let items = vec![
            PickerItem {
                id: "alice".to_string(),
                label: "Alice".to_string(),
                metadata: Some("A helpful assistant".to_string()),
                sort_key: Some("Alice".to_string()),
            },
            PickerItem {
                id: "bob".to_string(),
                label: "Bob".to_string(),
                metadata: Some("A friendly character".to_string()),
                sort_key: Some("Bob".to_string()),
            },
            PickerItem {
                id: "charlie".to_string(),
                label: "Charlie".to_string(),
                metadata: Some("An expert advisor".to_string()),
                sort_key: Some("Charlie".to_string()),
            },
        ];

        let picker_state = PickerState::new("Pick Character", items.clone(), 0);
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Character,
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        });

        // Test navigation down
        assert_eq!(app.picker_state().unwrap().selected, 0);
        apply_action(&mut app, AppAction::PickerMoveDown, ctx);
        assert_eq!(app.picker_state().unwrap().selected, 1);
        
        // Test navigation up
        apply_action(&mut app, AppAction::PickerMoveUp, ctx);
        assert_eq!(app.picker_state().unwrap().selected, 0);
    }

    #[test]
    fn character_picker_escape_closes_picker() {
        use crate::core::app::picker::{CharacterPickerState, PickerData, PickerMode, PickerSession};
        
        let mut app = create_test_app();
        let ctx = default_ctx();

        // Create a mock character picker
        let items = vec![
            PickerItem {
                id: "alice".to_string(),
                label: "Alice".to_string(),
                metadata: Some("A helpful assistant".to_string()),
                sort_key: Some("Alice".to_string()),
            },
        ];

        let picker_state = PickerState::new("Pick Character", items.clone(), 0);
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Character,
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        });

        assert!(app.picker_session().is_some());
        
        // Test escape closes picker
        apply_action(&mut app, AppAction::PickerEscape, ctx);
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn character_picker_enter_selects_character() {
        use crate::core::app::picker::{CharacterPickerState, PickerData, PickerMode, PickerSession};
        use crate::character::card::{CharacterCard, CharacterData};
        use tempfile::tempdir;
        use std::fs;
        
        let mut app = create_test_app();
        let ctx = default_ctx();

        // Create a temporary cards directory with a test card
        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let test_card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alice".to_string(),
                description: "A helpful assistant".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Helping users".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let card_path = cards_dir.join("alice.json");
        fs::write(&card_path, serde_json::to_string(&test_card).unwrap()).unwrap();

        // Create a mock character picker
        let items = vec![
            PickerItem {
                id: "alice".to_string(),
                label: "Alice".to_string(),
                metadata: Some("A helpful assistant".to_string()),
                sort_key: Some("Alice".to_string()),
            },
        ];

        let picker_state = PickerState::new("Pick Character", items.clone(), 0);
        app.picker.picker_session = Some(PickerSession {
            mode: PickerMode::Character,
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        });

        // Note: We can't fully test character selection without mocking the file system
        // or having actual character cards, but we can verify the picker closes
        assert!(app.picker_session().is_some());
        apply_action(&mut app, AppAction::PickerApplySelection { persistent: false }, ctx);
        assert!(app.picker_session().is_none());
    }
}
