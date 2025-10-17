use std::time::Instant;

use super::App;
use crate::api::ChatMessage;
use crate::core::chat_stream::StreamParams;
use tokio_util::sync::CancellationToken;

impl App {
    pub fn is_current_stream(&self, stream_id: u64) -> bool {
        self.session.current_stream_id == stream_id
    }

    pub fn end_streaming(&mut self) {
        self.ui.end_streaming();
    }

    pub fn cancel_current_stream(&mut self) {
        self.conversation().cancel_current_stream();
    }

    pub fn enable_auto_scroll(&mut self) {
        self.ui.auto_scroll = true;
    }

    pub fn build_stream_params(
        &self,
        api_messages: Vec<ChatMessage>,
        cancel_token: CancellationToken,
        stream_id: u64,
    ) -> StreamParams {
        StreamParams {
            client: self.session.client.clone(),
            base_url: self.session.base_url.clone(),
            api_key: self.session.api_key.clone(),
            provider_name: self.session.provider_name.clone(),
            model: self.session.model.clone(),
            api_messages,
            cancel_token,
            stream_id,
        }
    }

    pub fn last_retry_time(&self) -> Instant {
        self.session.last_retry_time
    }

    pub fn update_last_retry_time(&mut self, instant: Instant) {
        self.session.last_retry_time = instant;
    }
}
