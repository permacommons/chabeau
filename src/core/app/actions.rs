use tokio::sync::mpsc;

use super::App;

#[derive(Debug, Clone)]
pub enum AppAction {
    AppendResponseChunk { content: String },
    StreamErrored { message: String },
    StreamCompleted,
}

#[derive(Debug, Clone, Copy)]
pub struct AppActionContext {
    pub term_width: u16,
    pub term_height: u16,
}

#[derive(Debug, Clone)]
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

pub fn apply_actions(app: &mut App, envelopes: impl IntoIterator<Item = AppActionEnvelope>) {
    for envelope in envelopes {
        apply_action(app, envelope.action, envelope.context);
    }
}

pub fn apply_action(app: &mut App, action: AppAction, ctx: AppActionContext) {
    match action {
        AppAction::AppendResponseChunk { content } => {
            append_response_chunk(app, &content, ctx);
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
        }
        AppAction::StreamCompleted => {
            {
                let mut conversation = app.conversation();
                conversation.finalize_response();
            }
            app.ui.end_streaming();
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
