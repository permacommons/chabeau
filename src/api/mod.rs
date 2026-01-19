use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatToolDefinition>>,
}

#[derive(Deserialize)]
pub struct ChatResponseDelta {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCallDelta>>,
}

#[derive(Deserialize)]
pub struct ChatResponseChoice {
    pub delta: ChatResponseDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatResponseChoice>,
}

#[derive(Deserialize)]
pub struct ChatToolCallFunctionDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatToolCallDelta {
    pub index: Option<u32>,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<ChatToolCallFunctionDelta>,
}

#[derive(Serialize, Clone)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatToolCallFunction,
}

#[derive(Serialize, Clone)]
pub struct ChatToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize, Clone)]
pub struct ChatToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatToolFunction,
}

#[derive(Serialize, Clone)]
pub struct ChatToolFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
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

pub mod models;
