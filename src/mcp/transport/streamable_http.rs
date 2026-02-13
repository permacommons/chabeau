use futures_util::StreamExt;
use rust_mcp_schema::schema_utils::ServerMessage;

use super::{list_fetch_from_response, ListFetch};

pub fn fetch_list<T>(
    response: Result<ServerMessage, String>,
    parse: impl FnOnce(ServerMessage) -> Result<T, String>,
) -> ListFetch<T> {
    list_fetch_from_response(response, parse)
}

#[derive(Default)]
pub struct SseLineBuffer {
    buffer: Vec<u8>,
}

impl SseLineBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        self.drain_lines(false)
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.drain_lines(true)
    }

    fn drain_lines(&mut self, flush: bool) -> Vec<String> {
        let mut lines = Vec::new();
        let mut search_index = 0;

        while let Some(relative_pos) = self.buffer[search_index..].iter().position(|b| *b == b'\n')
        {
            let newline_index = search_index + relative_pos;
            let mut line_end = newline_index;
            if line_end > search_index && self.buffer[line_end - 1] == b'\r' {
                line_end -= 1;
            }

            let line_bytes = &self.buffer[search_index..line_end];
            if let Ok(text) = std::str::from_utf8(line_bytes) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }

            search_index = newline_index + 1;
        }

        if flush {
            if let Ok(text) = std::str::from_utf8(&self.buffer[search_index..]) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
            self.buffer.clear();
        } else if search_index > 0 {
            self.buffer.drain(..search_index);
        }

        lines
    }
}

pub fn is_event_stream_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream"))
}

pub fn sse_data_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

pub async fn next_sse_server_message(
    response: reqwest::Response,
    mut on_message: impl FnMut(&ServerMessage),
) -> Result<ServerMessage, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = SseLineBuffer::default();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| err.to_string())?;
        for line in buffer.push(&chunk) {
            if let Some(message) = decode_sse_line(&line)? {
                on_message(&message);
                if matches!(
                    message,
                    ServerMessage::Response(_) | ServerMessage::Error(_)
                ) {
                    return Ok(message);
                }
            }
        }
    }

    for line in buffer.finish() {
        if let Some(message) = decode_sse_line(&line)? {
            on_message(&message);
            if matches!(
                message,
                ServerMessage::Response(_) | ServerMessage::Error(_)
            ) {
                return Ok(message);
            }
        }
    }

    Err("Empty event-stream response.".to_string())
}

fn decode_sse_line(line: &str) -> Result<Option<ServerMessage>, String> {
    let Some(payload) = sse_data_payload(line) else {
        return Ok(None);
    };

    if payload.is_empty() {
        return Ok(None);
    }

    serde_json::from_str::<ServerMessage>(payload)
        .map(Some)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_buffer_handles_partial_lines() {
        let mut buffer = SseLineBuffer::default();
        assert!(buffer.push(b"data: one").is_empty());
        assert_eq!(buffer.push(b"\n\n"), vec!["data: one"]);
        assert!(buffer.finish().is_empty());
    }

    #[test]
    fn detects_event_stream_content_type() {
        assert!(is_event_stream_content_type(
            "text/event-stream; charset=utf-8"
        ));
        assert!(!is_event_stream_content_type("application/json"));
    }

    #[test]
    fn extracts_sse_payload() {
        assert_eq!(sse_data_payload("data: {\"id\":1}"), Some("{\"id\":1}"));
        assert_eq!(sse_data_payload("event: ping"), None);
    }
}
