//! Input utilities for terminal applications
//!
//! This module provides utilities for handling user input, including text sanitization
//! and masked input functionality.

/// Sanitize text input to prevent TUI corruption
///
/// This function:
/// - Converts tabs to 4 spaces
/// - Converts carriage returns to newlines
/// - Filters out control characters except newlines
///
/// This is used by both the chat loop and masked input to ensure consistent
/// text handling across the application.
pub fn sanitize_text_input(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());

    for c in text.chars() {
        match c {
            '\t' => sanitized.push_str("    "),
            '\r' => sanitized.push('\n'),
            '\n' => sanitized.push(c),
            _ if !c.is_control() => sanitized.push(c),
            _ => {}
        }
    }

    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_text_input_basic() {
        let input = "hello world";
        let result = sanitize_text_input(input);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_sanitize_text_input_tabs() {
        let input = "hello\tworld";
        let result = sanitize_text_input(input);
        assert_eq!(result, "hello    world");
    }

    #[test]
    fn test_sanitize_text_input_carriage_returns() {
        let input = "hello\rworld";
        let result = sanitize_text_input(input);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_sanitize_text_input_mixed_control_chars() {
        let input = "hello\x07\tworld\r\ntest";
        let result = sanitize_text_input(input);
        assert_eq!(result, "hello    world\n\ntest");
    }

    #[test]
    fn test_sanitize_text_input_preserves_newlines() {
        let input = "line1\nline2\nline3";
        let result = sanitize_text_input(input);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_sanitize_text_input_filters_control_chars() {
        let input = "hello\x01\x02world\x03";
        let result = sanitize_text_input(input);
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn test_sanitize_text_input_long_mixed_input() {
        let input = "start\thello\x07middle\rend\n".repeat(256) + "tail\x00\t\rline";
        let result = sanitize_text_input(&input);
        let expected = "start    hellomiddle\nend\n".repeat(256) + "tail    \nline";
        assert_eq!(result, expected);
    }
}
