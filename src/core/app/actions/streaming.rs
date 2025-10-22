use std::time::Instant;

use super::{App, AppAction, AppActionContext, AppCommand};
use crate::core::chat_stream::StreamParams;
use crate::core::message::AppMessageKind;

pub(super) fn handle_streaming_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::AppendResponseChunk { content, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            append_response_chunk(app, &content, ctx);
            None
        }
        AppAction::StreamAppMessage {
            kind,
            message,
            stream_id,
        } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            append_stream_app_message(app, kind, message, ctx);
            None
        }
        AppAction::StreamErrored { message, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            handle_stream_error(app, message, ctx);
            None
        }
        AppAction::StreamCompleted { stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            finalize_stream(app);
            None
        }
        AppAction::CancelStreaming => {
            app.cancel_current_stream();
            None
        }
        AppAction::SubmitMessage { message } => Some(spawn_stream_for_message(app, message, ctx)),
        AppAction::RefineLastMessage { params } => refine_last_message(app, params, ctx),
        AppAction::RetryLastMessage => retry_last_message(app, ctx),
        _ => unreachable!("non-streaming action routed to streaming handler"),
    }
}

pub(super) fn spawn_stream_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> AppCommand {
    let params = prepare_stream_params_for_message(app, message, ctx);
    AppCommand::SpawnStream(params)
}

pub(super) fn retry_last_message(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    prepare_retry_stream(app, ctx)
}

pub(super) fn refine_last_message(
    app: &mut App,
    params: crate::commands::RefineParams,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let crate::commands::RefineParams {
        prompt,
        instructions,
        prefix,
    } = params;
    prepare_refine_stream(app, prompt, instructions, prefix, ctx)
}

fn append_response_chunk(app: &mut App, chunk: &str, ctx: AppActionContext) {
    if chunk.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.append_to_response(chunk, available_height, ctx.term_width);
}

fn append_stream_app_message(
    app: &mut App,
    kind: AppMessageKind,
    message: String,
    ctx: AppActionContext,
) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    conversation.add_app_message(kind, trimmed.to_string());
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn handle_stream_error(app: &mut App, message: String, ctx: AppActionContext) {
    let error_message = message.trim().to_string();
    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        conversation.add_app_message(AppMessageKind::Error, error_message);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
    app.end_streaming();
}

fn finalize_stream(app: &mut App) {
    {
        let mut conversation = app.conversation();
        conversation.finalize_response();
    }
    app.end_streaming();
}

fn prepare_stream_params_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> StreamParams {
    let term_width = ctx.term_width.max(1);
    let term_height = ctx.term_height.max(1);
    app.enable_auto_scroll();
    let input_area_height = app.input_area_height(term_width);
    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(message);
        let available_height =
            conversation.calculate_available_height(term_height, input_area_height);
        conversation.update_scroll_position(available_height, term_width);
        (cancel_token, stream_id, api_messages)
    };

    app.build_stream_params(api_messages, cancel_token, stream_id)
}

fn prepare_retry_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let now = Instant::now();
    if now.duration_since(app.last_retry_time()).as_millis() < 200 {
        return None;
    }

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
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
        app.update_last_retry_time(now);
        Some(AppCommand::SpawnStream(app.build_stream_params(
            api_messages,
            cancel_token,
            stream_id,
        )))
    } else {
        None
    }
}

fn prepare_refine_stream(
    app: &mut App,
    prompt: String,
    instructions: String,
    prefix: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let maybe_params = {
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation
            .prepare_refine(prompt, instructions, prefix, available_height, ctx.term_width)
            .map(|api_messages| {
                let (cancel_token, stream_id) = conversation.start_new_stream();
                (api_messages, cancel_token, stream_id)
            })
    };

    if let Some((api_messages, cancel_token, stream_id)) = maybe_params {
        app.update_last_retry_time(Instant::now());
        Some(AppCommand::SpawnStream(app.build_stream_params(
            api_messages,
            cancel_token,
            stream_id,
        )))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::{
        AppMessageKind, ROLE_APP_ERROR, ROLE_APP_WARNING, ROLE_ASSISTANT, ROLE_USER,
    };
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn stream_app_message_adds_trimmed_content_and_keeps_stream_alive() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = handle_streaming_action(
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

        assert!(app.ui.is_streaming);

        let result = handle_streaming_action(
            &mut app,
            AppAction::StreamAppMessage {
                kind: AppMessageKind::Warning,
                message: "  invalid utf8  ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());

        let last_message = app
            .ui
            .messages
            .back()
            .expect("expected trailing app message");
        assert_eq!(last_message.role, ROLE_APP_WARNING);
        assert_eq!(last_message.content, "invalid utf8");

        assert!(app.ui.is_streaming);
    }

    #[test]
    fn stream_errored_drops_empty_assistant_placeholder() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = handle_streaming_action(
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

        let result = handle_streaming_action(
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
}
