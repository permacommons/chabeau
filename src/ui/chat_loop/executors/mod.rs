use ratatui::prelude::Size;
use tokio_util::sync::CancellationToken;

use crate::core::app::{AppActionContext, AppActionDispatcher};

use super::AppHandle;

pub mod mcp_init;
pub mod mcp_tools;
pub mod model_loader;

#[derive(Clone)]
pub struct ExecutorContext {
    pub app: AppHandle,
    pub dispatcher: AppActionDispatcher,
    pub cancel_token: Option<CancellationToken>,
    pub term_size: Size,
}

impl ExecutorContext {
    pub async fn from_app(app: AppHandle, dispatcher: AppActionDispatcher) -> Self {
        let (cancel_token, term_size) = app
            .read(|app| {
                (
                    app.session.stream_cancel_token.clone(),
                    app.ui.last_term_size,
                )
            })
            .await;
        Self {
            app,
            dispatcher,
            cancel_token,
            term_size,
        }
    }

    pub fn action_context(&self) -> AppActionContext {
        AppActionContext {
            term_width: self.term_size.width,
            term_height: self.term_size.height,
        }
    }
}
