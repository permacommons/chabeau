use crate::api::ChatMessage;
use rust_mcp_schema::{
    CreateMessageRequest, CreateMessageRequestParams, MessageMeta, Role, SamplingMessageContent,
    SamplingMessageContentBlock, TextContent,
};
use std::time::Duration;

pub fn summarize_sampling_request(request: &CreateMessageRequest) -> String {
    let message_count = request.params.messages.len();
    let max_tokens = request.params.max_tokens;
    let system_prompt = request
        .params
        .system_prompt
        .as_ref()
        .map(|prompt| format!("system prompt: {}", summarize_prompt(prompt)))
        .unwrap_or_else(|| "system prompt: none".to_string());
    format!("messages: {message_count}, maxTokens: {max_tokens}, {system_prompt}")
}

pub fn serialize_sampling_params(request: &CreateMessageRequest) -> String {
    serde_json::to_string(&request.params).unwrap_or_else(|_| "{}".to_string())
}

pub fn build_sampling_messages(request: &CreateMessageRequest) -> Result<Vec<ChatMessage>, String> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request.params.system_prompt.as_ref() {
        if !system_prompt.trim().is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: system_prompt.clone(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    for message in &request.params.messages {
        let content = sampling_content_to_text(&message.content)?;
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        messages.push(ChatMessage {
            role: role.to_string(),
            content,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
    }

    Ok(messages)
}

pub fn sampling_timeout_for_request(request: &CreateMessageRequest) -> Option<Duration> {
    sampling_timeout_from_params(&request.params)
}

fn sampling_timeout_from_params(params: &CreateMessageRequestParams) -> Option<Duration> {
    let meta = params.meta.as_ref()?;
    sampling_timeout_from_meta(meta)
}

fn sampling_timeout_from_meta(meta: &MessageMeta) -> Option<Duration> {
    let extra = meta.extra.as_ref()?;
    let mut timeout_ms: Option<u64> = None;
    let mut timeout_secs: Option<f64> = None;

    for (key, value) in extra {
        let key = key.to_ascii_lowercase();
        if key.contains("timeoutms") {
            if let Some(ms) = parse_positive_number(value) {
                timeout_ms = Some(ms);
            }
        } else if key.contains("timeout") {
            timeout_secs = parse_positive_float(value);
        }
    }

    if let Some(ms) = timeout_ms {
        return Some(Duration::from_millis(ms));
    }
    let secs = timeout_secs?;
    Some(Duration::from_millis((secs * 1000.0).ceil() as u64))
}

fn parse_positive_number(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_positive_float(value: &serde_json::Value) -> Option<f64> {
    let number = match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    if number > 0.0 {
        Some(number)
    } else {
        None
    }
}

pub fn map_finish_reason(reason: Option<String>) -> Option<String> {
    let reason = reason?;
    let mapped = match reason.as_str() {
        "stop" => "endTurn",
        "length" => "maxTokens",
        "tool_calls" => "toolUse",
        "content_filter" => "stopSequence",
        _ => return Some(reason),
    };
    Some(mapped.to_string())
}

fn sampling_content_to_text(content: &SamplingMessageContent) -> Result<String, String> {
    match content {
        SamplingMessageContent::TextContent(text) => Ok(text.text.clone()),
        SamplingMessageContent::SamplingMessageContentBlock(blocks) => {
            let mut parts = Vec::new();
            for block in blocks {
                parts.push(sampling_block_to_text(block)?);
            }
            Ok(parts.join("\n"))
        }
        _ => Err("Sampling content must be text-only for this client.".to_string()),
    }
}

fn sampling_block_to_text(block: &SamplingMessageContentBlock) -> Result<String, String> {
    match block {
        SamplingMessageContentBlock::TextContent(TextContent { text, .. }) => Ok(text.clone()),
        _ => Err("Sampling content blocks must be text-only for this client.".to_string()),
    }
}

fn summarize_prompt(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.len() <= 48 {
        trimmed.to_string()
    } else {
        let mut truncated = trimmed.chars().take(48).collect::<String>();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_mcp_schema::{
        CreateMessageRequest, CreateMessageRequestParams, ImageContent, MessageMeta, RequestId,
        Role, SamplingMessage, SamplingMessageContent, TextContent,
    };
    use serde_json::json;

    fn base_request(messages: Vec<SamplingMessage>) -> CreateMessageRequest {
        let params = CreateMessageRequestParams {
            include_context: None,
            max_tokens: 16,
            messages,
            meta: None,
            metadata: None,
            model_preferences: None,
            stop_sequences: Vec::new(),
            system_prompt: Some("System prompt".to_string()),
            task: None,
            temperature: None,
            tool_choice: None,
            tools: Vec::new(),
        };
        CreateMessageRequest::new(RequestId::Integer(1), params)
    }

    #[test]
    fn build_sampling_messages_includes_system_prompt() {
        let message = SamplingMessage {
            role: Role::User,
            content: SamplingMessageContent::from(TextContent::new(
                "Hello".to_string(),
                None,
                None,
            )),
            meta: None,
        };
        let request = base_request(vec![message]);
        let messages = build_sampling_messages(&request).expect("messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, "System prompt");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "Hello");
    }

    #[test]
    fn build_sampling_messages_rejects_image_content() {
        let image = ImageContent::new("data".to_string(), "image/png".to_string(), None, None);
        let message = SamplingMessage {
            role: Role::User,
            content: SamplingMessageContent::from(image),
            meta: None,
        };
        let request = base_request(vec![message]);
        let err = build_sampling_messages(&request)
            .err()
            .expect("should reject image");
        assert!(err.contains("text-only"));
    }

    #[test]
    fn sampling_timeout_prefers_timeout_ms() {
        let mut request = base_request(Vec::new());
        request.params.meta = Some(MessageMeta {
            progress_token: None,
            extra: Some(serde_json::Map::from_iter([(
                "server/timeoutMs".to_string(),
                json!(120_000),
            )])),
        });

        let timeout = sampling_timeout_for_request(&request).expect("timeout");
        assert_eq!(timeout.as_millis(), 120_000);
    }

    #[test]
    fn sampling_timeout_accepts_timeout_seconds() {
        let mut request = base_request(Vec::new());
        request.params.meta = Some(MessageMeta {
            progress_token: None,
            extra: Some(serde_json::Map::from_iter([(
                "timeout".to_string(),
                json!(90),
            )])),
        });

        let timeout = sampling_timeout_for_request(&request).expect("timeout");
        assert_eq!(timeout.as_secs(), 90);
    }

    #[test]
    fn sampling_timeout_ignores_invalid_values() {
        let mut request = base_request(Vec::new());
        request.params.meta = Some(MessageMeta {
            progress_token: None,
            extra: Some(serde_json::Map::from_iter([(
                "timeoutMs".to_string(),
                json!("nope"),
            )])),
        });

        assert!(sampling_timeout_for_request(&request).is_none());
    }
}
