use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
}

#[derive(Deserialize)]
pub struct ChatResponseDelta {
    pub content: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatResponseChoice {
    pub delta: ChatResponseDelta,
}

#[derive(Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatResponseChoice>,
}
