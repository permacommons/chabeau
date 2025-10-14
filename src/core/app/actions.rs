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
use crate::core::config::Config;
use crate::core::message::AppMessageKind;

pub enum AppAction {
    AppendResponseChunk {
        content: String,
        stream_id: u64,
    },
    StreamErrored {
        message: String,
        stream_id: u64,
    },
    StreamCompleted {
        stream_id: u64,
    },
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
        AppAction::AppendResponseChunk { content, stream_id } => {
            if stream_id != app.session.current_stream_id {
                return None;
            }
            append_response_chunk(app, &content, ctx);
            None
        }
        AppAction::StreamErrored { message, stream_id } => {
            if stream_id != app.session.current_stream_id {
                return None;
            }
            let error_message = message.trim().to_string();
            let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
            {
                let mut conversation = app.conversation();
                conversation.remove_trailing_empty_assistant_messages();
                conversation.add_app_message(AppMessageKind::Error, error_message);
                let available_height =
                    conversation.calculate_available_height(ctx.term_height, input_area_height);
                conversation.update_scroll_position(available_height, ctx.term_width);
            }
            app.ui.end_streaming();
            None
        }
        AppAction::StreamCompleted { stream_id } => {
            if stream_id != app.session.current_stream_id {
                return None;
            }
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
        Some(PickerMode::Persona) => {
            if let Some(state) = app.persona_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_personas();
                }
            }
        }
        Some(PickerMode::Preset) => {
            if let Some(state) = app.preset_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_presets();
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
        Some(PickerMode::Persona) => {
            if let Some(state) = app.persona_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_personas();
            }
        }
        Some(PickerMode::Preset) => {
            if let Some(state) = app.preset_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_presets();
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
            // Show character greeting if needed (after character activation)
            app.conversation().show_character_greeting_if_needed();
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
        CommandResult::OpenPersonaPicker => {
            app.open_persona_picker();
            None
        }
        CommandResult::OpenPresetPicker => {
            app.open_preset_picker();
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
    let user_display_name = app.persona_manager.get_display_name();
    let _ = app
        .session
        .logging
        .rewrite_log_without_last_response(&app.ui.messages, &user_display_name);
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
        Some(PickerMode::Persona) => {
            app.close_picker();
        }
        Some(PickerMode::Preset) => {
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
                    // Load default character for this provider/model if configured
                    load_default_character_if_configured(app, ctx);
                    // Load default persona for this provider/model if configured
                    load_default_persona_if_configured(app, ctx);
                    // Load default preset for this provider/model if configured
                    load_default_preset_if_configured(app, ctx);
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
                    } else {
                        // Load default character for this provider/model if configured
                        load_default_character_if_configured(app, ctx);
                        // Load default persona for this provider/model if configured
                        load_default_persona_if_configured(app, ctx);
                        // Load default preset for this provider/model if configured
                        load_default_preset_if_configured(app, ctx);
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
        Some(PickerMode::Persona) => {
            app.apply_selected_persona(persistent);
            None
        }
        Some(PickerMode::Preset) => {
            app.apply_selected_preset(persistent);
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
        Some(PickerMode::Character) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match Config::mutate(move |config| {
                config.unset_default_character(&provider_name, &model);
                Ok(())
            }) {
                Ok(_) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    app.open_character_picker();
                    None
                }
                Err(e) => {
                    set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Persona) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match app
                .persona_manager
                .unset_default_for_provider_model_persistent(&provider_name, &model)
            {
                Ok(()) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    app.open_persona_picker();
                    None
                }
                Err(e) => {
                    set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Preset) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match app
                .preset_manager
                .unset_default_for_provider_model_persistent(&provider_name, &model)
            {
                Ok(()) => {
                    set_status_message(app, format!("Removed default: {}", selected_id), ctx);
                    app.open_preset_picker();
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

/// Load default character for current provider/model if one is configured
fn push_error_app_message(app: &mut App, message: String, ctx: AppActionContext) {
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        conversation.add_app_message(AppMessageKind::Error, message);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    } else {
        app.conversation()
            .add_app_message(AppMessageKind::Error, message);
    }
}

fn load_default_character_if_configured(app: &mut App, ctx: AppActionContext) {
    let cfg = Config::load_test_safe();
    if let Some(default_name) =
        cfg.get_default_character(&app.session.provider_name, &app.session.model)
    {
        match app.character_service.load_default_for_session(
            &app.session.provider_name,
            &app.session.model,
            &cfg,
        ) {
            Ok(Some((_name, card))) => {
                app.session.set_character(card);
                // Show character greeting if present
                app.conversation().show_character_greeting_if_needed();
            }
            Ok(None) => {}
            Err(e) => {
                push_error_app_message(
                    app,
                    format!("Could not load default character '{}': {}", default_name, e),
                    ctx,
                );
            }
        }
    }
}

/// Load default persona for current provider/model if one is configured
fn load_default_persona_if_configured(app: &mut App, ctx: AppActionContext) {
    // Don't load default persona if one is already active (e.g., from CLI)
    if app.persona_manager.get_active_persona().is_some() {
        return;
    }

    if let Some(persona_id) = app
        .persona_manager
        .get_default_for_provider_model(&app.session.provider_name, &app.session.model)
    {
        let persona_id = persona_id.to_string(); // Clone to avoid borrow issues
        match app.persona_manager.set_active_persona(&persona_id) {
            Ok(()) => {
                let persona_name = app
                    .persona_manager
                    .get_active_persona()
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                app.ui.update_user_display_name(persona_name);
            }
            Err(e) => {
                push_error_app_message(
                    app,
                    format!("Could not load default persona '{}': {}", persona_id, e),
                    ctx,
                );
            }
        }
    }
}

/// Load default preset for current provider/model if one is configured
fn load_default_preset_if_configured(app: &mut App, ctx: AppActionContext) {
    if app.preset_manager.get_active_preset().is_some() {
        return;
    }

    if let Some(preset_id) = app
        .preset_manager
        .get_default_for_provider_model(&app.session.provider_name, &app.session.model)
    {
        let preset_id = preset_id.to_string();
        if let Err(e) = app.preset_manager.set_active_preset(&preset_id) {
            push_error_app_message(
                app,
                format!("Could not load default preset '{}': {}", preset_id, e),
                ctx,
            );
        }
    }
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
    use crate::core::message::{ROLE_APP_ERROR, ROLE_ASSISTANT, ROLE_USER};
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
    fn stream_errored_drops_empty_assistant_placeholder() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = apply_action(
            &mut app,
            AppAction::SubmitMessage {
                message: "Hello there".into(),
            },
            ctx,
        );

        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            Some(_) => panic!("unexpected app command returned for submit message"),
            None => panic!("expected spawn stream command"),
        };

        assert!(app
            .ui
            .messages
            .iter()
            .any(|msg| msg.role == ROLE_ASSISTANT && msg.content.is_empty()));

        let result = apply_action(
            &mut app,
            AppAction::StreamErrored {
                message: " network failure ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());

        assert!(app
            .ui
            .messages
            .iter()
            .all(|msg| msg.role != ROLE_ASSISTANT || !msg.content.trim().is_empty()));

        let last_message = app
            .ui
            .messages
            .back()
            .expect("expected trailing error message");
        assert_eq!(last_message.role, ROLE_APP_ERROR);
        assert_eq!(last_message.content, "network failure");

        assert_eq!(app.ui.messages.len(), 2);
        let first = app.ui.messages.front().expect("missing user message");
        assert_eq!(first.role, ROLE_USER);
        assert_eq!(first.content, "Hello there");
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
        use crate::core::app::picker::{
            CharacterPickerState, PickerData, PickerMode, PickerSession,
        };

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
        use crate::core::app::picker::{
            CharacterPickerState, PickerData, PickerMode, PickerSession,
        };

        let mut app = create_test_app();
        let ctx = default_ctx();

        // Create a mock character picker
        let items = vec![PickerItem {
            id: "alice".to_string(),
            label: "Alice".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];

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
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::core::app::picker::{
            CharacterPickerState, PickerData, PickerMode, PickerSession,
        };
        use std::fs;
        use tempfile::tempdir;

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
        let items = vec![PickerItem {
            id: "alice".to_string(),
            label: "Alice".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];

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
        apply_action(
            &mut app,
            AppAction::PickerApplySelection { persistent: false },
            ctx,
        );
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn character_picker_marks_default_with_asterisk() {
        use crate::core::app::picker::{
            CharacterPickerState, PickerData, PickerMode, PickerSession,
        };

        let mut app = create_test_app();

        // Create a mock character picker with alice marked as default
        // (In real usage, the asterisk is added by open_character_picker based on config)
        let items = vec![
            PickerItem {
                id: "alice".to_string(),
                label: "alice*".to_string(), // Should have asterisk
                metadata: Some("A helpful assistant".to_string()),
                sort_key: Some("alice".to_string()),
            },
            PickerItem {
                id: "bob".to_string(),
                label: "bob".to_string(), // No asterisk
                metadata: Some("Another character".to_string()),
                sort_key: Some("bob".to_string()),
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

        // Verify alice has asterisk
        let selected_item = app.picker_state().unwrap().get_selected_item().unwrap();
        assert_eq!(selected_item.label, "alice*");
    }

    #[test]
    fn model_selection_loads_default_character() {
        use crate::character::card::{CharacterCard, CharacterData};
        use std::fs;
        use tempfile::tempdir;

        let mut app = create_test_app();

        // Create a temporary cards directory with a test card
        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let test_card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "alice".to_string(),
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

        // Set CHABEAU_CONFIG_DIR to use temp directory
        std::env::set_var("CHABEAU_CONFIG_DIR", temp_dir.path());

        // Verify no character is active initially
        assert!(app.session.active_character.is_none());

        // Call the helper function directly (simulating model selection)
        load_default_character_if_configured(&mut app, AppActionContext::default());
        load_default_persona_if_configured(&mut app, AppActionContext::default());

        // Verify character is still not loaded (no default set)
        assert!(app.session.active_character.is_none());
        // Verify persona is still not loaded (no default set)
        assert!(app.persona_manager.get_active_persona().is_none());

        // Clean up
        std::env::remove_var("CHABEAU_CONFIG_DIR");
    }

    #[test]
    fn test_load_default_persona_if_configured() {
        use crate::core::config::{Config, Persona};

        // Create a config with test personas
        let mut config = Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    display_name: "Alice".to_string(),
                    bio: Some("A developer persona".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    display_name: "Bob".to_string(),
                    bio: Some("A student persona".to_string()),
                },
            ],
            ..Default::default()
        };

        // Set up a default persona for the current provider/model (test_test-model)
        config.set_default_persona(
            "test".to_string(),
            "test-model".to_string(),
            "alice-dev".to_string(),
        );

        // Create app with personas
        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        // Initially no persona should be active
        assert!(app.persona_manager.get_active_persona().is_none());

        // Call the helper function directly (simulating model selection)
        load_default_persona_if_configured(&mut app, AppActionContext::default());

        // Verify persona is now loaded
        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "alice-dev");
        assert_eq!(app.ui.user_display_name, "Alice");
    }

    #[test]
    fn test_load_default_persona_respects_existing_active_persona() {
        use crate::core::config::{Config, Persona};

        // Create a config with test personas
        let config = Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    display_name: "Alice".to_string(),
                    bio: Some("A developer persona".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    display_name: "Bob".to_string(),
                    bio: Some("A student persona".to_string()),
                },
            ],
            ..Default::default()
        };

        // Set up a default persona for the current provider/model (test_test-model)
        let mut config = config;
        config.set_default_persona(
            "test".to_string(),
            "test-model".to_string(),
            "alice-dev".to_string(),
        );

        // Create app with personas
        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        // Activate a different persona first (simulating CLI persona)
        app.persona_manager
            .set_active_persona("bob-student")
            .unwrap();
        app.ui.update_user_display_name("Bob".to_string());

        // Call the helper function (simulating model selection)
        load_default_persona_if_configured(&mut app, AppActionContext::default());

        // Verify the original persona is still active (default not loaded)
        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "bob-student");
        assert_eq!(app.ui.user_display_name, "Bob");
    }

    #[test]
    fn test_persona_picker_default_label_format() {
        use crate::core::app::picker::PickerMode;
        use crate::core::config::{Config, Persona};

        // Create a config with test personas
        let mut config = Config {
            personas: vec![Persona {
                id: "alice-dev".to_string(),
                display_name: "Alice".to_string(),
                bio: Some("A developer persona".to_string()),
            }],
            ..Default::default()
        };
        config.set_default_persona(
            "test".to_string(),
            "test-model".to_string(),
            "alice-dev".to_string(),
        );

        // Create app with personas and defaults
        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        // Open persona picker
        app.open_persona_picker();

        // Verify picker is open and has the correct mode
        assert!(matches!(
            app.current_picker_mode(),
            Some(PickerMode::Persona)
        ));

        // Get the picker state and verify the default persona has asterisk at the end
        let picker_state = app.picker_state().expect("Picker should be open");
        let items = &picker_state.items;

        // Find the alice-dev persona item
        let alice_item = items
            .iter()
            .find(|item| item.id == "alice-dev")
            .expect("Alice persona should be in picker");

        // Verify the label ends with asterisk (indicating it's a default)
        assert!(
            alice_item.label.ends_with('*'),
            "Default persona label should end with asterisk, got: {}",
            alice_item.label
        );
        assert_eq!(alice_item.label, "Alice (alice-dev)*");
    }

    #[test]
    fn test_persona_picker_metadata_includes_character_name() {
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::core::config::{Config, Persona};

        let config = Config {
            personas: vec![Persona {
                id: "mentor".to_string(),
                display_name: "Mentor".to_string(),
                bio: Some("Guide {{char}} with wisdom, {{user}}.".to_string()),
            }],
            ..Default::default()
        };

        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Aria".to_string(),
                description: String::new(),
                personality: String::new(),
                scenario: String::new(),
                first_mes: String::new(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };
        app.session.set_character(character);

        app.open_persona_picker();

        let picker_state = app.picker_state().expect("Picker should be open");
        let mentor_item = picker_state
            .items
            .iter()
            .find(|item| item.id == "mentor")
            .expect("Mentor persona should be in picker");
        let metadata = mentor_item
            .metadata
            .as_ref()
            .expect("Mentor persona should have metadata");

        assert!(metadata.contains("Guide Aria with wisdom, Mentor."));
    }

    #[test]
    fn test_load_default_preset_if_configured() {
        use crate::core::config::{Config, Preset};

        let mut config = Config {
            presets: vec![
                Preset {
                    id: "focus".to_string(),
                    pre: "Focus on details.".to_string(),
                    post: String::new(),
                },
                Preset {
                    id: "summary".to_string(),
                    pre: String::new(),
                    post: "Summarize at the end.".to_string(),
                },
            ],
            ..Default::default()
        };

        config.set_default_preset(
            "test".to_string(),
            "test-model".to_string(),
            "focus".to_string(),
        );

        let mut app = create_test_app();
        app.preset_manager = crate::core::preset::PresetManager::load_presets(&config)
            .expect("Failed to load presets");

        assert!(app.preset_manager.get_active_preset().is_none());

        load_default_preset_if_configured(&mut app, AppActionContext::default());

        let active = app
            .preset_manager
            .get_active_preset()
            .expect("preset active");
        assert_eq!(active.id, "focus");
    }

    #[test]
    fn test_load_default_preset_respects_existing_active_preset() {
        use crate::core::config::{Config, Preset};

        let mut config = Config {
            presets: vec![
                Preset {
                    id: "focus".to_string(),
                    pre: "Focus on details.".to_string(),
                    post: String::new(),
                },
                Preset {
                    id: "summary".to_string(),
                    pre: String::new(),
                    post: "Summarize at the end.".to_string(),
                },
            ],
            ..Default::default()
        };

        config.set_default_preset(
            "test".to_string(),
            "test-model".to_string(),
            "focus".to_string(),
        );

        let mut app = create_test_app();
        app.preset_manager = crate::core::preset::PresetManager::load_presets(&config)
            .expect("Failed to load presets");
        app.preset_manager
            .set_active_preset("summary")
            .expect("activate preset");

        load_default_preset_if_configured(&mut app, AppActionContext::default());

        let active = app
            .preset_manager
            .get_active_preset()
            .expect("preset active");
        assert_eq!(active.id, "summary");
    }
}
