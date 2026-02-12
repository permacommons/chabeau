use crate::core::config::data::McpServerConfig;
use std::collections::HashMap;

pub fn require_stdio_command(config: &McpServerConfig) -> Result<String, String> {
    config
        .command
        .clone()
        .ok_or_else(|| "MCP command is required for stdio transport.".to_string())
}

pub fn stdio_args(config: &McpServerConfig) -> Vec<String> {
    config.args.clone().unwrap_or_default()
}

pub fn stdio_env(config: &McpServerConfig) -> Option<HashMap<String, String>> {
    config.env.clone()
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

#[cfg(test)]
mod tests {
    use super::SseLineBuffer;

    #[test]
    fn sse_line_buffer_handles_chunk_boundaries() {
        let mut buffer = SseLineBuffer::default();
        assert_eq!(buffer.push(b"data: one\n\n"), vec!["data: one"]);
        assert_eq!(buffer.push(b"data: t"), Vec::<String>::new());
        assert_eq!(buffer.push(b"wo\n"), vec!["data: two"]);
        assert_eq!(buffer.finish(), Vec::<String>::new());
    }
}
