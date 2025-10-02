use futures_util::StreamExt;
use memchr::memchr;
use tokio::sync::mpsc;

use crate::api::{ChatRequest, ChatResponse};
use crate::utils::url::construct_api_url;

#[derive(Clone, Debug)]
pub enum StreamMessage {
    Chunk(String),
    Error(String),
    End,
}

fn format_api_error(error_text: &str) -> String {
    let trimmed = error_text.trim();

    // Check if it looks like JSON
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        format!("API Error:\n```json\n{}\n```", trimmed)
    }
    // Check if it looks like XML
    else if trimmed.starts_with('<') && trimmed.ends_with('>') {
        format!("API Error:\n```xml\n{}\n```", trimmed)
    }
    // For plain text errors, still wrap in code fences for consistency
    else {
        format!("API Error:\n```\n{}\n```", trimmed)
    }
}

pub struct StreamParams {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub provider_name: String,
    pub model: String,
    pub api_messages: Vec<crate::api::ChatMessage>,
    pub cancel_token: tokio_util::sync::CancellationToken,
    pub stream_id: u64,
}

#[derive(Clone)]
pub struct StreamDispatcher {
    tx: mpsc::UnboundedSender<(StreamMessage, u64)>,
}

impl StreamDispatcher {
    pub fn new(tx: mpsc::UnboundedSender<(StreamMessage, u64)>) -> Self {
        Self { tx }
    }

    pub fn spawn(&self, params: StreamParams) {
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

                    let http_request = crate::utils::auth::add_auth_headers(http_request, &provider_name, &api_key);

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

                                        if let Some(data) = line_str.strip_prefix("data: ") {
                                            if data == "[DONE]" {
                                                let _ = tx_clone
                                                    .send((StreamMessage::End, stream_id));
                                                return;
                                            }

                                            match serde_json::from_str::<ChatResponse>(data) {
                                                Ok(response) => {
                                                    if let Some(choice) = response.choices.first() {
                                                        if let Some(content) = &choice.delta.content {
                                                            let _ = tx_clone.send((
                                                                StreamMessage::Chunk(
                                                                    content.clone(),
                                                                ),
                                                                stream_id,
                                                            ));
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!("Failed to parse JSON: {e} - Data: {data}");
                                                }
                                            }
                                        }
                                        buffer.drain(..=newline_pos);
                                    }
                                }
                            }

                            // Stream ended naturally (connection closed) - send end marker
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
}

#[cfg(test)]
impl StreamDispatcher {
    pub fn send_for_test(&self, message: StreamMessage, stream_id: u64) {
        let _ = self.tx.send((message, stream_id));
    }
}
