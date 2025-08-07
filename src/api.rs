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

#[derive(Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub created: Option<u64>,
    pub created_at: Option<String>,
    pub owned_by: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}
