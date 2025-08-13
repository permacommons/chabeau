//! Text wrapping utilities for input handling
//!
//! This module provides consistent word-wrapping logic that preserves spacing
//! and handles long words appropriately for terminal display.
//!
//! ## Why We Can't Use Ratatui's Built-in Wrapping
//!
//! Ratatui's `Paragraph` widget with `Wrap { trim: false }` cannot be used for
//! interactive text input because:
//!
//! 1. **No cursor position calculation**: Ratatui doesn't expose where text wraps,
//!    making it impossible to calculate where the cursor should appear.
//!
//! 2. **No line count access**: We need to know how many lines text will wrap to
//!    for input area sizing and scroll calculations.
//!
//! 3. **Spacing preservation**: Multiple consecutive spaces must be preserved
//!    exactly as typed by the user.
//!
//! Our solution pre-processes text with explicit line breaks at the correct
//! positions, then renders it with ratatui's `Paragraph` without wrapping enabled.
//! This ensures perfect alignment between our cursor calculations and the rendered text.

/// Configuration for text wrapping behavior
#[derive(Debug, Clone)]
pub struct WrapConfig {
    /// Maximum width for text lines
    pub width: usize,
}

impl WrapConfig {
    pub fn new(width: usize) -> Self {
        Self { width }
    }
}

/// Text wrapping engine that handles word boundaries while preserving spacing
pub struct TextWrapper;

impl TextWrapper {
    /// Wrap text at word boundaries while preserving all original spacing
    pub fn wrap_text(text: &str, config: &WrapConfig) -> String {
        if config.width == 0 {
            return text.to_string();
        }

        let mut result = String::new();
        let mut current_line_len = 0;

        // Split text by explicit newlines first
        for (line_idx, line) in text.split('\n').enumerate() {
            if line_idx > 0 {
                result.push('\n');
                current_line_len = 0;
            }

            // Process each character, keeping track of words and spaces
            let mut chars = line.chars().peekable();
            let mut current_word = String::new();

            while let Some(ch) = chars.next() {
                if ch.is_whitespace() {
                    // Flush current word if we have one
                    if !current_word.is_empty() {
                        let word_len = current_word.chars().count();

                        // Check if word fits on current line
                        if current_line_len > 0 && current_line_len + word_len > config.width {
                            // Need to wrap before this word
                            result.push('\n');
                            current_line_len = 0;
                        }

                        // Handle very long words
                        if word_len > config.width {
                            Self::handle_long_word(
                                &mut result,
                                &current_word,
                                &mut current_line_len,
                                config.width,
                            );
                        } else {
                            // Normal word that fits
                            result.push_str(&current_word);
                            current_line_len += word_len;
                        }

                        current_word.clear();
                    }

                    // Add the whitespace character if it fits
                    if current_line_len < config.width {
                        result.push(ch);
                        current_line_len += 1;
                    } else {
                        // Whitespace would exceed line, wrap and skip it
                        result.push('\n');
                        current_line_len = 0;
                        // Don't add the space at the beginning of a new line
                    }
                } else {
                    // Regular character - add to current word
                    current_word.push(ch);
                }
            }

            // Flush any remaining word
            if !current_word.is_empty() {
                let word_len = current_word.chars().count();

                // Check if word fits on current line
                if current_line_len > 0 && current_line_len + word_len > config.width {
                    // Need to wrap before this word
                    result.push('\n');
                    current_line_len = 0;
                }

                // Handle very long words
                if word_len > config.width {
                    Self::handle_long_word(
                        &mut result,
                        &current_word,
                        &mut current_line_len,
                        config.width,
                    );
                } else {
                    // Normal word that fits
                    result.push_str(&current_word);
                    current_line_len += word_len;
                }
            }
        }

        result
    }

    /// Handle words that are longer than the line width by breaking them
    fn handle_long_word(
        result: &mut String,
        word: &str,
        current_line_len: &mut usize,
        width: usize,
    ) {
        let mut remaining_word = word;
        while !remaining_word.is_empty() {
            let chars_to_take = width.saturating_sub(*current_line_len);
            if chars_to_take == 0 {
                result.push('\n');
                *current_line_len = 0;
                continue;
            }

            let word_chars: Vec<char> = remaining_word.chars().collect();
            let chunk: String = word_chars.iter().take(chars_to_take).collect();
            result.push_str(&chunk);
            *current_line_len += chunk.chars().count();

            remaining_word = &remaining_word[chunk.len()..];

            if !remaining_word.is_empty() {
                result.push('\n');
                *current_line_len = 0;
            }
        }
    }

    /// Count the number of lines that text would wrap to
    pub fn count_wrapped_lines(text: &str, config: &WrapConfig) -> usize {
        if text.is_empty() {
            return 1;
        }

        let wrapped_text = Self::wrap_text(text, config);
        let lines: Vec<&str> = wrapped_text.split('\n').collect();
        lines.len().max(1)
    }

    /// Calculate which line a cursor position would be on after wrapping
    pub fn calculate_cursor_line(text: &str, cursor_position: usize, config: &WrapConfig) -> usize {
        let cursor_position = cursor_position.min(text.chars().count());

        // Get text before cursor
        let text_before_cursor: String = text.chars().take(cursor_position).collect();

        // Get the wrapped version of text before cursor
        let wrapped_before_cursor = Self::wrap_text(&text_before_cursor, config);

        // Count lines in wrapped text
        let lines: Vec<&str> = wrapped_before_cursor.split('\n').collect();
        lines.len().saturating_sub(1)
    }

    /// Calculate cursor position within wrapped text
    pub fn calculate_cursor_position_in_wrapped_text(
        text: &str,
        cursor_position: usize,
        config: &WrapConfig,
    ) -> (usize, usize) {
        let cursor_position = cursor_position.min(text.chars().count());

        // Get text before cursor
        let text_before_cursor: String = text.chars().take(cursor_position).collect();

        // Get the wrapped version of text before cursor
        let wrapped_before_cursor = Self::wrap_text(&text_before_cursor, config);

        // Count lines and find column position in wrapped text
        let lines: Vec<&str> = wrapped_before_cursor.split('\n').collect();
        let line = lines.len().saturating_sub(1);
        let col = if let Some(last_line) = lines.last() {
            last_line.chars().count()
        } else {
            0
        };

        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_wrapping() {
        let config = WrapConfig::new(10);
        let text = "hello world this is a test";
        let wrapped = TextWrapper::wrap_text(text, &config);

        // Should wrap at word boundaries
        assert!(wrapped.contains('\n'));
        assert!(!wrapped.contains("hello world this")); // Should wrap before "this"
    }

    #[test]
    fn test_preserve_multiple_spaces() {
        let config = WrapConfig::new(20);
        let text = "hello    world";
        let wrapped = TextWrapper::wrap_text(text, &config);

        // Should preserve multiple spaces
        assert_eq!(wrapped, "hello    world");
    }

    #[test]
    fn test_long_word_breaking() {
        let config = WrapConfig::new(5);
        let text = "superlongword";
        let wrapped = TextWrapper::wrap_text(text, &config);

        // Should break long words
        assert!(wrapped.contains('\n'));
        let lines: Vec<&str> = wrapped.split('\n').collect();
        assert!(lines.iter().all(|line| line.chars().count() <= 5));
    }

    #[test]
    fn test_cursor_position_calculation() {
        let config = WrapConfig::new(5);
        let text = "hello world";
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(text, 6, &config);

        // With width 5, "hello" (5 chars) fills first line, space wraps to second line
        // Cursor at position 6 (after "hello ") should be at start of second line
        assert_eq!(line, 1);
        assert_eq!(col, 0); // Start of second line after wrapping
    }
}
