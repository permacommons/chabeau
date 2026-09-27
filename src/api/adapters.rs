//! Translation boundary between the neutral API model and provider wire formats.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::neutral::{
    ContentPart, FinishReason, ProviderError, Request, Role, StreamEvent, ToolCallDelta, Usage,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    #[default]
    OpenaiChatCompletions,
    AnthropicMessages,
}

impl AdapterKind {
    pub fn adapter(self) -> &'static dyn ProviderAdapter {
        match self {
            Self::OpenaiChatCompletions => &OpenAiAdapter,
            Self::AnthropicMessages => &AnthropicAdapter,
        }
    }
}

pub trait ProviderAdapter: Send + Sync {
    fn endpoint(&self) -> &'static str;
    fn encode_request(&self, request: &Request) -> Value;
    fn decode_event(&self, payload: &str) -> Vec<StreamEvent>;
}

struct OpenAiAdapter;
struct AnthropicAdapter;

fn reason(value: &str) -> FinishReason {
    match value {
        "stop" | "end_turn" | "stop_sequence" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "tool_use" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.into()),
    }
}

fn provider_error(value: &Value) -> ProviderError {
    let error = value.get("error").unwrap_or(value);
    ProviderError {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Provider returned an error")
            .to_string(),
        code: error.get("code").and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        }),
        kind: error.get("type").and_then(Value::as_str).map(str::to_owned),
        retryable: error
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| matches!(t, "overloaded_error" | "rate_limit_error")),
    }
}

impl ProviderAdapter for OpenAiAdapter {
    fn endpoint(&self) -> &'static str {
        "chat/completions"
    }
    fn encode_request(&self, request: &Request) -> Value {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                let text = m.content.iter().filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::ToolResult { content, .. } => Some(content.as_str()),
                    ContentPart::ToolCall { .. } => None,
                }).collect::<String>();
                let mut value = json!({"role": format!("{:?}", m.role).to_lowercase(), "content": text});
                if let Some(name) = &m.name { value["name"] = json!(name); }
                let calls: Vec<Value> = m.content.iter().filter_map(|part| match part {
                    ContentPart::ToolCall { id, name, arguments } => Some(json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments}})),
                    _ => None,
                }).collect();
                if !calls.is_empty() { value["tool_calls"] = json!(calls); }
                if let Some(ContentPart::ToolResult { tool_call_id, .. }) = m.content.iter().find(|part| matches!(part, ContentPart::ToolResult { .. })) { value["tool_call_id"] = json!(tool_call_id); }
                value
            })
            .collect();
        json!({"model":request.model,"messages":messages,"stream":true,"stream_options":{"include_usage":true},"tools":request.tools.iter().map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}})).collect::<Vec<_>>(),"max_tokens":request.max_tokens,"temperature":request.temperature,"stop":request.stop})
    }
    fn decode_event(&self, payload: &str) -> Vec<StreamEvent> {
        if payload == "[DONE]" {
            return vec![StreamEvent::Completed {
                finish_reason: None,
            }];
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return vec![StreamEvent::Failed(ProviderError {
                message: "Invalid JSON stream event".into(),
                code: None,
                kind: Some("decode_error".into()),
                retryable: false,
            })];
        };
        if v.get("error").is_some() {
            return vec![StreamEvent::Failed(provider_error(&v))];
        }
        let mut out = Vec::new();
        if let Some(usage) = v.get("usage") {
            out.push(StreamEvent::Usage(Usage {
                input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
                output_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
                total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            }));
        }
        for choice in v
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let delta = &choice["delta"];
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                out.push(StreamEvent::TextDelta(text.into()));
            }
            if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                out.push(StreamEvent::ReasoningDelta(text.into()));
            }
            for call in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push(StreamEvent::ToolCallDelta(ToolCallDelta {
                    index: call.get("index").and_then(Value::as_u64).unwrap_or(0) as u32,
                    id: call.get("id").and_then(Value::as_str).map(str::to_owned),
                    name: call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    arguments: call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }));
            }
            if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
                out.push(StreamEvent::Completed {
                    finish_reason: Some(reason(r)),
                });
            }
        }
        out
    }
}

impl ProviderAdapter for AnthropicAdapter {
    fn endpoint(&self) -> &'static str {
        "messages"
    }
    fn encode_request(&self, request: &Request) -> Value {
        let mut system = Vec::new();
        let mut messages = Vec::new();
        for m in &request.messages {
            let content: Vec<Value> = m.content.iter().map(|part| match part {
                ContentPart::Text { text } => json!({"type":"text","text":text}),
                ContentPart::ToolCall { id, name, arguments } => json!({"type":"tool_use","id":id,"name":name,"input":serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| json!({"raw":arguments}))}),
                ContentPart::ToolResult { tool_call_id, content, is_error } => json!({"type":"tool_result","tool_use_id":tool_call_id,"content":content,"is_error":is_error}),
            }).collect();
            if matches!(m.role, Role::System) {
                system.push(
                    m.content
                        .iter()
                        .filter_map(|p| match p {
                            ContentPart::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<String>(),
                );
            } else {
                messages.push(json!({"role":if matches!(m.role, Role::Assistant) {"assistant"} else {"user"},"content":content}));
            }
        }
        json!({"model":request.model,"messages":messages,"system":system.join("\n\n"),"stream":true,"max_tokens":request.max_tokens.unwrap_or(4096),"temperature":request.temperature,"stop_sequences":request.stop,"tools":request.tools.iter().map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.input_schema})).collect::<Vec<_>>()})
    }
    fn decode_event(&self, payload: &str) -> Vec<StreamEvent> {
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return vec![StreamEvent::Failed(ProviderError {
                message: "Invalid JSON stream event".into(),
                code: None,
                kind: Some("decode_error".into()),
                retryable: false,
            })];
        };
        match v.get("type").and_then(Value::as_str).unwrap_or_default() {
            "content_block_start"
                if v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") =>
            {
                vec![StreamEvent::ToolCallDelta(ToolCallDelta {
                    index: v.get("index").and_then(Value::as_u64).unwrap_or(0) as u32,
                    id: v
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    name: v
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    arguments: None,
                })]
            }
            "content_block_delta" => match v.pointer("/delta/type").and_then(Value::as_str) {
                Some("text_delta") => v
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .map(|s| vec![StreamEvent::TextDelta(s.into())])
                    .unwrap_or_default(),
                Some("input_json_delta") => v
                    .pointer("/delta/partial_json")
                    .and_then(Value::as_str)
                    .map(|s| {
                        vec![StreamEvent::ToolCallDelta(ToolCallDelta {
                            index: v.get("index").and_then(Value::as_u64).unwrap_or(0) as u32,
                            id: None,
                            name: None,
                            arguments: Some(s.into()),
                        })]
                    })
                    .unwrap_or_default(),
                Some("thinking_delta") => v
                    .pointer("/delta/thinking")
                    .and_then(Value::as_str)
                    .map(|s| vec![StreamEvent::ReasoningDelta(s.into())])
                    .unwrap_or_default(),
                _ => vec![StreamEvent::Status {
                    status: "non_text_content".into(),
                    message: None,
                }],
            },
            "message_start" => v
                .pointer("/message/usage")
                .map(|u| {
                    vec![StreamEvent::Usage(Usage {
                        input_tokens: u.get("input_tokens").and_then(Value::as_u64),
                        output_tokens: u.get("output_tokens").and_then(Value::as_u64),
                        total_tokens: None,
                    })]
                })
                .unwrap_or_default(),
            "message_delta" => {
                let mut out = Vec::new();
                if let Some(u) = v.get("usage") {
                    out.push(StreamEvent::Usage(Usage {
                        input_tokens: None,
                        output_tokens: u.get("output_tokens").and_then(Value::as_u64),
                        total_tokens: None,
                    }));
                }
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    out.push(StreamEvent::Completed {
                        finish_reason: Some(reason(r)),
                    });
                }
                out
            }
            "message_stop" => vec![StreamEvent::Completed {
                finish_reason: None,
            }],
            "error" => vec![StreamEvent::Failed(provider_error(&v))],
            "ping" => vec![StreamEvent::Status {
                status: "ping".into(),
                message: None,
            }],
            other => vec![StreamEvent::Warning(format!(
                "Ignored provider event: {other}"
            ))],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_fixture(kind: AdapterKind, fixture: &str) -> Vec<StreamEvent> {
        fixture
            .lines()
            .flat_map(|line| kind.adapter().decode_event(line))
            .collect()
    }

    #[test]
    fn openai_fixture_decodes_fragmented_text_tools_usage_and_completion() {
        let events = decode_fixture(
            AdapterKind::OpenaiChatCompletions,
            include_str!("fixtures/openai_stream.jsonl"),
        );
        assert!(matches!(&events[0], StreamEvent::TextDelta(text) if text == "Hel"));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, StreamEvent::ToolCallDelta(_)))
                .count(),
            2
        );
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Usage(Usage {
                total_tokens: Some(13),
                ..
            })
        )));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: Some(FinishReason::ToolCalls)
            })
        ));
    }

    #[test]
    fn anthropic_fixture_decodes_non_text_usage_and_completion_without_done() {
        let events = decode_fixture(
            AdapterKind::AnthropicMessages,
            include_str!("fixtures/anthropic_stream.jsonl"),
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::Status { status, .. } if status == "ping")));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ReasoningDelta(text) if text == "checking")));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Usage(_))));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: Some(FinishReason::Stop)
            })
        ));
    }

    #[test]
    fn provider_error_fixture_is_normalized() {
        let events = decode_fixture(
            AdapterKind::AnthropicMessages,
            include_str!("fixtures/provider_error.json"),
        );
        assert!(
            matches!(&events[0], StreamEvent::Failed(error) if error.message == "Try again" && error.retryable)
        );
    }
}
