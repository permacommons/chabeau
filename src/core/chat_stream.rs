use futures_util::StreamExt;
use memchr::memchr;
use tokio::sync::mpsc;

use crate::api::{ChatMessage, ChatRequest, ChatResponse};
use crate::core::message::AppMessageKind;
use crate::utils::url::construct_api_url;

#[derive(Clone, Debug)]
pub enum StreamMessage {
    Chunk(String),
    Error(String),
    App {
        kind: AppMessageKind,
        content: String,
    },
    End,
}

fn extract_data_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim_start)
}

fn handle_data_payload(
    payload: &str,
    tx: &mpsc::UnboundedSender<(StreamMessage, u64)>,
    stream_id: u64,
) -> bool {
    if payload == "[DONE]" {
        let _ = tx.send((StreamMessage::End, stream_id));
        return true;
    }

    match serde_json::from_str::<ChatResponse>(payload) {
        Ok(response) => {
            if let Some(choice) = response.choices.first() {
                if let Some(content) = &choice.delta.content {
                    let _ = tx.send((StreamMessage::Chunk(content.clone()), stream_id));
                }
            }
            false
        }
        Err(_) => {
            if payload.trim().is_empty() {
                return false;
            }

            let formatted_error = format_api_error(payload);
            let _ = tx.send((StreamMessage::Error(formatted_error), stream_id));
            let _ = tx.send((StreamMessage::End, stream_id));
            true
        }
    }
}

fn process_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<(StreamMessage, u64)>,
    stream_id: u64,
) -> bool {
    extract_data_payload(line)
        .map(|payload| handle_data_payload(payload, tx, stream_id))
        .unwrap_or(false)
}

fn route_sse_frame(
    frame: SseFrame,
    tx: &mpsc::UnboundedSender<(StreamMessage, u64)>,
    stream_id: u64,
) -> bool {
    match frame {
        SseFrame::Data(line) => process_sse_line(&line, tx, stream_id),
        SseFrame::AppMessage { kind, content } => {
            if !content.trim().is_empty() {
                let _ = tx.send((StreamMessage::App { kind, content }, stream_id));
            }
            false
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SseFrame {
    Data(String),
    AppMessage {
        kind: AppMessageKind,
        content: String,
    },
}

pub trait SseFramer {
    fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame>;
    fn finish(&mut self) -> Vec<SseFrame>;
}

#[derive(Default)]
pub struct SimpleSseFramer {
    buffer: Vec<u8>,
    utf8_warning_emitted: bool,
}

impl SimpleSseFramer {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize_line(&mut self, bytes: &[u8]) -> Option<SseFrame> {
        if bytes.is_empty() {
            return None;
        }

        match std::str::from_utf8(bytes) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(SseFrame::Data(trimmed.to_string()))
                }
            }
            Err(err) => {
                if self.utf8_warning_emitted {
                    None
                } else {
                    self.utf8_warning_emitted = true;
                    Some(SseFrame::AppMessage {
                        kind: AppMessageKind::Warning,
                        content: format!(
                            "Received invalid UTF-8 in response stream: {err}. Bytes were ignored. Additional invalid UTF-8 warnings will be suppressed."
                        ),
                    })
                }
            }
        }
    }
}

impl SseFramer for SimpleSseFramer {
    fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut search_index = 0;

        while let Some(relative_pos) = memchr(b'\n', &self.buffer[search_index..]) {
            let newline_index = search_index + relative_pos;
            let mut line_end = newline_index;
            if line_end > search_index && self.buffer[line_end - 1] == b'\r' {
                line_end -= 1;
            }

            let line_bytes = self.buffer[search_index..line_end].to_vec();
            if let Some(line) = self.normalize_line(&line_bytes) {
                frames.push(line);
            }

            search_index = newline_index + 1;
        }

        if search_index > 0 {
            self.buffer.drain(..search_index);
        }

        frames
    }

    fn finish(&mut self) -> Vec<SseFrame> {
        if self.buffer.is_empty() {
            return Vec::new();
        }

        let mut line_end = self.buffer.len();
        while line_end > 0 && self.buffer[line_end - 1] == b'\r' {
            line_end -= 1;
        }

        let line_bytes = self.buffer[..line_end].to_vec();
        let line = self.normalize_line(&line_bytes);
        self.buffer.clear();
        line.into_iter().collect()
    }
}

fn extract_error_summary(value: &serde_json::Value) -> Option<String> {
    let summary = value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            value.get("error").and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.to_string()),
                serde_json::Value::Object(map) => map
                    .get("message")
                    .and_then(|message| message.as_str().map(str::to_owned)),
                _ => None,
            })
        })
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str().map(str::to_owned))
        });

    summary.map(|text| {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed.trim().to_string()
    })
}

fn format_api_error(error_text: &str) -> String {
    let trimmed = error_text.trim();

    if trimmed.is_empty() {
        return "API Error:\n```\n<empty>\n```".to_string();
    }

    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Ok(pretty_json) = serde_json::to_string_pretty(&json_value) {
            if let Some(summary) = extract_error_summary(&json_value) {
                if !summary.is_empty() {
                    return format!("API Error: {}\n```json\n{}\n```", summary, pretty_json);
                }
            }
            return format!("API Error:\n```json\n{}\n```", pretty_json);
        }
    }

    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        format!("API Error:\n```xml\n{}\n```", trimmed)
    } else {
        format!("API Error:\n```\n{}\n```", trimmed)
    }
}

pub struct StreamParams {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub provider_name: String,
    pub model: String,
    pub api_messages: Vec<ChatMessage>,
    pub cancel_token: tokio_util::sync::CancellationToken,
    pub stream_id: u64,
}

#[derive(Clone)]
pub struct ChatStreamService {
    tx: mpsc::UnboundedSender<(StreamMessage, u64)>,
}

impl ChatStreamService {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<(StreamMessage, u64)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    pub fn spawn_stream(&self, params: StreamParams) {
        let tx_clone = self.tx.clone();
        tokio::spawn(async move {
            let StreamParams {
                client,
                base_url,
                api_key,
                provider_name,
                model,
                api_messages,
                cancel_token,
                stream_id,
            } = params;

            let request = ChatRequest {
                model,
                messages: api_messages,
                stream: true,
            };

            tokio::select! {
                _ = async {
                    let chat_url = construct_api_url(&base_url, "chat/completions");
                    let http_request = client
                        .post(chat_url)
                        .header("Content-Type", "application/json");

                    let http_request = crate::utils::auth::add_auth_headers(
                        http_request,
                        &provider_name,
                        &api_key,
                    );

                    match http_request
                        .json(&request)
                        .send()
                        .await
                    {
                        Ok(response) => {
                            if !response.status().is_success() {
                                let error_text = response
                                    .text()
                                    .await
                                    .unwrap_or_else(|_| "<no body>".to_string());
                                let formatted_error = format_api_error(&error_text);
                                let _ = tx_clone
                                    .send((StreamMessage::Error(formatted_error), stream_id));
                                let _ = tx_clone.send((StreamMessage::End, stream_id));
                                return;
                            }

                            let mut stream = response.bytes_stream();
                            let mut framer = SimpleSseFramer::new();

                            while let Some(chunk) = stream.next().await {
                                if cancel_token.is_cancelled() {
                                    return;
                                }

                                if let Ok(chunk_bytes) = chunk {
                                    for frame in framer.push(&chunk_bytes) {
                                        if route_sse_frame(frame, &tx_clone, stream_id) {
                                            return;
                                        }
                                    }
                                }
                            }

                            for frame in framer.finish() {
                                if route_sse_frame(frame, &tx_clone, stream_id) {
                                    return;
                                }
                            }

                            let _ = tx_clone.send((StreamMessage::End, stream_id));
                        }
                        Err(e) => {
                            let formatted_error = format_api_error(&e.to_string());
                            let _ = tx_clone
                                .send((StreamMessage::Error(formatted_error), stream_id));
                            let _ = tx_clone.send((StreamMessage::End, stream_id));
                        }
                    }
                } => {}
                _ = cancel_token.cancelled() => {}
            }
        });
    }

    #[cfg(test)]
    pub fn send_for_test(&self, message: StreamMessage, stream_id: u64) {
        let _ = self.tx.send((message, stream_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_sse_framer_handles_crlf_and_blank_lines() {
        let mut framer = SimpleSseFramer::new();
        let frames = framer.push(b"data: hello\r\nid:1\r\n\r\n");
        assert_eq!(
            frames,
            vec![
                SseFrame::Data("data: hello".to_string()),
                SseFrame::Data("id:1".to_string()),
            ]
        );

        let trailing = framer.finish();
        assert!(trailing.is_empty());
    }

    #[test]
    fn simple_sse_framer_flushes_end_of_stream() {
        let mut framer = SimpleSseFramer::new();
        assert!(framer.push(b"data: partial").is_empty());

        let frames = framer.finish();
        assert_eq!(frames, vec![SseFrame::Data("data: partial".to_string())]);
    }

    #[test]
    fn simple_sse_framer_reports_invalid_utf8() {
        let mut framer = SimpleSseFramer::new();
        let mut bytes = b"data:".to_vec();
        bytes.extend_from_slice(&[0xF0, 0x28, 0x8C, 0x28]);
        bytes.extend_from_slice(b"\n");

        let frames = framer.push(&bytes);
        match frames.as_slice() {
            [SseFrame::AppMessage { kind, content }] => {
                assert_eq!(*kind, AppMessageKind::Warning);
                assert!(content.contains("Received invalid UTF-8 in response stream"));
                assert!(content.contains("Bytes were ignored."));
                assert!(content.contains("Additional invalid UTF-8 warnings will be suppressed."));
            }
            other => panic!("unexpected frames: {other:?}"),
        }

        assert!(framer.finish().is_empty());
    }

    #[test]
    fn simple_sse_framer_suppresses_repeated_invalid_utf8() {
        let mut framer = SimpleSseFramer::new();

        let mut bytes = b"data:".to_vec();
        bytes.extend_from_slice(&[0xF0, 0x28, 0x8C, 0x28]);
        bytes.extend_from_slice(b"\n");
        assert_eq!(framer.push(&bytes).len(), 1);

        let mut second_bytes = b"data:".to_vec();
        second_bytes.extend_from_slice(&[0xF0, 0x28, 0x8C, 0x28]);
        second_bytes.extend_from_slice(b"\n");
        assert!(framer.push(&second_bytes).is_empty());

        assert!(framer.finish().is_empty());
    }

    #[test]
    fn process_sse_line_handles_spacing_variants() {
        let (service, mut rx) = ChatStreamService::new();
        let variants = [
            (
                r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
                "Hello",
                "data: [DONE]",
            ),
            (
                r#"data:{"choices":[{"delta":{"content":"World"}}]}"#,
                "World",
                "data:[DONE]",
            ),
        ];

        for (index, (chunk_line, expected_chunk, done_line)) in variants.iter().enumerate() {
            let stream_id = (index + 1) as u64;

            assert!(!process_sse_line(chunk_line, &service.tx, stream_id));
            let (message, received_id) = rx.try_recv().expect("expected chunk message");
            assert_eq!(received_id, stream_id);
            match message {
                StreamMessage::Chunk(content) => assert_eq!(content, *expected_chunk),
                other => panic!("expected chunk message, got {:?}", other),
            }

            assert!(process_sse_line(done_line, &service.tx, stream_id));
            let (message, received_id) = rx.try_recv().expect("expected end message");
            assert_eq!(received_id, stream_id);
            assert!(matches!(message, StreamMessage::End));
        }

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn process_sse_line_routes_stream_errors() {
        let (service, mut rx) = ChatStreamService::new();
        let error_line = r#"data: {"error":{"message":"internal server error"}}"#;
        let stream_id = 99;

        assert!(process_sse_line(error_line, &service.tx, stream_id));

        let (message, received_id) = rx.try_recv().expect("expected error message");
        assert_eq!(received_id, stream_id);
        match message {
            StreamMessage::Error(text) => {
                let expected = r#"API Error: internal server error
```json
{
  "error": {
    "message": "internal server error"
  }
}
```"#;
                assert_eq!(text, expected);
            }
            other => panic!("expected error message, got {:?}", other),
        }

        let (message, received_id) = rx.try_recv().expect("expected end message");
        assert_eq!(received_id, stream_id);
        assert!(matches!(message, StreamMessage::End));

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn format_api_error_prettifies_json_with_summary() {
        let raw = r#"{"error":{"message":"model overloaded","type":"invalid_request_error"}}"#;
        let formatted = format_api_error(raw);

        let expected = r#"API Error: model overloaded
```json
{
  "error": {
    "message": "model overloaded",
    "type": "invalid_request_error"
  }
}
```"#;
        assert_eq!(formatted, expected);
    }

    #[test]
    fn format_api_error_handles_json_without_summary() {
        let raw = r#"{"status":"failed"}"#;
        let formatted = format_api_error(raw);

        let expected = r#"API Error:
```json
{
  "status": "failed"
}
```"#;
        assert_eq!(formatted, expected);
    }

    #[test]
    fn format_api_error_handles_xml_and_plaintext() {
        let xml = "<error>bad</error>";
        let plain = "api failure";

        let formatted_xml = format_api_error(xml);
        let formatted_plain = format_api_error(plain);

        assert_eq!(formatted_xml, "API Error:\n```xml\n<error>bad</error>\n```");
        assert_eq!(formatted_plain, "API Error:\n```\napi failure\n```");
    }
}
