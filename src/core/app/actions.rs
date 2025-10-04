use std::sync::Arc;

use tokio::sync::Mutex;

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

#[derive(Clone)]
pub struct AppActionDispatcher {
    app: Arc<Mutex<App>>,
}

impl AppActionDispatcher {
    pub fn new(app: Arc<Mutex<App>>) -> Self {
        Self { app }
    }

    pub async fn dispatch_many<I>(&self, actions: I, ctx: &AppActionContext)
    where
        I: IntoIterator<Item = AppAction>,
    {
        let mut app_guard = self.app.lock().await;
        for action in actions.into_iter() {
            apply_action(&mut app_guard, action, ctx);
        }
    }

    pub async fn current_stream_id(&self) -> u64 {
        let app_guard = self.app.lock().await;
        app_guard.session.current_stream_id
    }
}

pub(crate) fn apply_action(app: &mut App, action: AppAction, ctx: &AppActionContext) {
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

fn append_response_chunk(app: &mut App, chunk: &str, ctx: &AppActionContext) {
    if chunk.is_empty() {
        return;
    }

    let input_area_height = app.ui.calculate_input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.append_to_response(chunk, available_height, ctx.term_width);
}
