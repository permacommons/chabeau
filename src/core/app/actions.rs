use std::time::Instant;

use tokio::sync::mpsc;

use super::App;
use crate::api::ModelsResponse;
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
    SubmitMessage {
        message: String,
    },
    RetryLastMessage,
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
            app.conversation().set_status(message);
            if ctx.term_width > 0 && ctx.term_height > 0 {
                let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
                let mut conversation = app.conversation();
                let available_height =
                    conversation.calculate_available_height(ctx.term_height, input_area_height);
                conversation.update_scroll_position(available_height, ctx.term_width);
            }
            None
        }
        AppAction::ClearInput => {
            app.ui.clear_input();
            if ctx.term_width > 0 {
                app.ui.recompute_input_layout_after_edit(ctx.term_width);
            }
            None
        }
        AppAction::SubmitMessage { message } => {
            let params = prepare_stream_params_for_message(app, message, ctx);
            Some(AppCommand::SpawnStream(params))
        }
        AppAction::RetryLastMessage => prepare_retry_stream(app, ctx),
        AppAction::ModelPickerLoaded {
            default_model_for_provider,
            models_response,
        } => {
            if let Err(e) =
                app.complete_model_picker_request(default_model_for_provider, models_response)
            {
                app.conversation()
                    .set_status(format!("Model picker error: {}", e));
            }
            None
        }
        AppAction::ModelPickerLoadFailed { error } => {
            app.fail_model_picker_request();
            app.conversation()
                .set_status(format!("Model picker error: {}", error));
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
