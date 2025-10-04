use futures_util::StreamExt;
use memchr::memchr;
use tokio::sync::mpsc;

use crate::api::{ChatMessage, ChatRequest, ChatResponse};
use crate::utils::url::construct_api_url;

#[derive(Clone, Debug)]
pub enum StreamMessage {
    Chunk(String),
    Error(String),
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
        }
        Err(e) => {
            eprintln!("Failed to parse JSON: {e} - Data: {payload}");
        }
    }

    false
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

fn format_api_error(error_text: &str) -> String {
    let trimmed = error_text.trim();

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        format!("API Error:\n```json\n{}\n```", trimmed)
    } else if trimmed.starts_with('<') && trimmed.ends_with('>') {
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
                            let mut buffer: Vec<u8> = Vec::new();

                            while let Some(chunk) = stream.next().await {
                                if cancel_token.is_cancelled() {
                                    return;
                                }

                                if let Ok(chunk_bytes) = chunk {
                                    buffer.extend_from_slice(&chunk_bytes);

                                    while let Some(newline_pos) = memchr(b'\n', &buffer) {
                                        let line_str = match std::str::from_utf8(&buffer[..newline_pos]) {
                                            Ok(s) => s.trim(),
                                            Err(e) => {
                                                eprintln!("Invalid UTF-8 in stream: {e}");
                                                buffer.drain(..=newline_pos);
                                                continue;
                                            }
                                        };

                                        let should_end = process_sse_line(
                                            line_str,
                                            &tx_clone,
                                            stream_id,
                                        );
                                        buffer.drain(..=newline_pos);
                                        if should_end {
                                            return;
                                        }
                                    }
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
}
