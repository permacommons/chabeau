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

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
        let mut current_line_width = 0;

        // Split text by explicit newlines first
        for (line_idx, line) in text.split('\n').enumerate() {
            if line_idx > 0 {
                result.push('\n');
                current_line_width = 0;
            }

            let mut current_word = String::new();
            let mut current_word_width = 0;

            for grapheme in line.graphemes(true) {
                let is_whitespace = grapheme.chars().all(|c| c.is_whitespace());
                let grapheme_width = UnicodeWidthStr::width(grapheme);

                if is_whitespace {
                    if !current_word.is_empty() {
                        Self::flush_word(
                            &mut result,
                            &mut current_line_width,
                            &current_word,
                            current_word_width,
                            config.width,
                        );
                        current_word.clear();
                        current_word_width = 0;
                    }

                    if current_line_width + grapheme_width > config.width {
                        result.push('\n');
                        current_line_width = 0;
                    } else {
                        result.push_str(grapheme);
                        current_line_width += grapheme_width;
                    }
                } else {
                    current_word.push_str(grapheme);
                    current_word_width += grapheme_width;
                }
            }

            if !current_word.is_empty() {
                Self::flush_word(
                    &mut result,
                    &mut current_line_width,
                    &current_word,
                    current_word_width,
                    config.width,
                );
            }
        }

        result
    }

    fn flush_word(
        result: &mut String,
        current_line_width: &mut usize,
        word: &str,
        word_width: usize,
        width: usize,
    ) {
        if *current_line_width > 0 && *current_line_width + word_width > width {
            result.push('\n');
            *current_line_width = 0;
        }

        if word_width > width {
            Self::handle_long_word(result, word, current_line_width, width);
        } else {
            result.push_str(word);
            *current_line_width += word_width;
        }
    }

    /// Handle words that are longer than the line width by breaking them
    fn handle_long_word(
        result: &mut String,
        word: &str,
        current_line_width: &mut usize,
        width: usize,
    ) {
        let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(word, true).collect();
        let mut idx = 0;

        while idx < graphemes.len() {
            if *current_line_width >= width {
                result.push('\n');
                *current_line_width = 0;
            }

            let mut chunk = String::new();
            let mut advanced = false;

            while idx < graphemes.len() {
                let grapheme = graphemes[idx];
                let grapheme_width = UnicodeWidthStr::width(grapheme);

                if grapheme_width > width {
                    if *current_line_width != 0 {
                        result.push('\n');
                        *current_line_width = 0;
                    }

                    result.push_str(grapheme);
                    *current_line_width = grapheme_width;
                    idx += 1;
                    advanced = true;
                    break;
                }

                if *current_line_width + grapheme_width > width {
                    if chunk.is_empty() {
                        result.push('\n');
                        *current_line_width = 0;
                        continue;
                    }
                    break;
                }

                chunk.push_str(grapheme);
                *current_line_width += grapheme_width;
                idx += 1;
                advanced = true;

                if *current_line_width == width {
                    break;
                }
            }

            if !chunk.is_empty() {
                result.push_str(&chunk);
            }

            if idx < graphemes.len() {
                result.push('\n');
                *current_line_width = 0;
            } else if !advanced {
                break;
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

    /// Calculate cursor position within wrapped text using a character-by-character mapping
    pub fn calculate_cursor_position_in_wrapped_text(
        text: &str,
        cursor_position: usize,
        config: &WrapConfig,
    ) -> (usize, usize) {
        let cursor_position = cursor_position.min(text.chars().count());

        // Build a mapping for cursor "positions" (between characters), not just characters.
        // There are N+1 positions for N characters.
        let original_chars: Vec<char> = text.chars().collect();

        // Early exit for empty text
        if original_chars.is_empty() {
            return (0, 0);
        }

        // Wrap the full text once and collect coordinates for each visible (non-newline) character
        let wrapped_text = Self::wrap_text(text, config);
        let wrapped_lines: Vec<&str> = wrapped_text.split('\n').collect();
        let mut wrapped_coords: Vec<(usize, usize)> = Vec::new();
        for (line_idx, line) in wrapped_lines.iter().enumerate() {
            let mut col = 0usize;
            for grapheme in line.graphemes(true) {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                col += grapheme_width;
                let grapheme_char_count = grapheme.chars().count();
                for _ in 0..grapheme_char_count {
                    wrapped_coords.push((line_idx, col));
                }
            }
        }

        // Build the position map of length original_chars.len() + 1
        let mut pos_map: Vec<(usize, usize)> = Vec::with_capacity(original_chars.len() + 1);
        // Position 0 is always the start
        pos_map.push((0, 0));

        let mut wrapped_idx = 0usize; // index into wrapped_coords for non-newline chars
        let mut current_line = 0usize;

        for ch in original_chars.iter() {
            if *ch == '\n' {
                // After a newline, cursor moves to start of next line
                current_line = current_line.saturating_add(1);
                pos_map.push((current_line, 0));
            } else {
                // Map to the coordinate immediately AFTER this character
                if wrapped_idx < wrapped_coords.len() {
                    let (l, c) = wrapped_coords[wrapped_idx];
                    current_line = l;
                    pos_map.push((current_line, c));
                    wrapped_idx += 1;
                } else if let Some(last_line) = wrapped_lines.last() {
                    // Fallback to end of last line if somehow we ran out
                    let last_width = (*last_line)
                        .graphemes(true)
                        .map(UnicodeWidthStr::width)
                        .sum();
                    pos_map.push((wrapped_lines.len() - 1, last_width));
                } else {
                    pos_map.push((0, 0));
                }
            }
        }

        // Clamp and return
        let idx = cursor_position.min(pos_map.len().saturating_sub(1));
        pos_map[idx]
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
        assert!(lines.iter().all(|line| UnicodeWidthStr::width(*line) <= 5));
    }

    #[test]
    fn test_wrap_with_double_width_emoji() {
        let config = WrapConfig::new(4);
        let text = "😀😀😀";
        let wrapped = TextWrapper::wrap_text(text, &config);

        let lines: Vec<&str> = wrapped.split('\n').collect();
        assert_eq!(lines, vec!["😀😀", "😀"]);
        assert_eq!(UnicodeWidthStr::width(lines[0]), 4);
        assert_eq!(UnicodeWidthStr::width(lines[1]), 2);
    }

    #[test]
    fn test_cursor_mapping_with_double_width_emoji() {
        let config = WrapConfig::new(4);
        let text = "😀😀😀";

        let expectations = [(0, (0usize, 0usize)), (1, (0, 2)), (2, (0, 4)), (3, (1, 2))];

        for (cursor, expected) in expectations {
            let (line, col) =
                TextWrapper::calculate_cursor_position_in_wrapped_text(text, cursor, &config);
            assert_eq!((line, col), expected);
        }
    }

    #[test]
    fn test_cursor_position_calculation() {
        let config = WrapConfig::new(5);
        let text = "hello world";
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(text, 5, &config);

        // Cursor at position 5 (after "hello") is at end of first line
        assert_eq!(line, 0);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_cursor_position_with_multiple_spaces_and_newlines() {
        // Ensure spaces are preserved and cursor maps correctly across wraps and newlines
        let config = WrapConfig::new(6);
        let text = "ab   cd ef\nxyz";
        // Position after 'ab   ' (5 chars). Ensure mapping is within width and non-negative.
        let (l1, c1) = TextWrapper::calculate_cursor_position_in_wrapped_text(text, 5, &config);
        assert!(c1 <= 6, "col should be within width, got {}", c1);
        assert!(l1 <= 2, "line should be within expected range, got {}", l1);

        // Position 8 (crossing the first wrap boundary possibly before 'ef')
        let (l2, c2) = TextWrapper::calculate_cursor_position_in_wrapped_text(text, 8, &config);
        // Just assert mapping is within reasonable bounds (not panicking) and col < width
        assert!(c2 <= 6, "col should be within width, got {}", c2);
        assert!(l2 <= 2, "line should be within expected range, got {}", l2);

        // After newline into 'xyz'
        let pos_xyz = text.find('x').unwrap();
        let (l3, c3) =
            TextWrapper::calculate_cursor_position_in_wrapped_text(text, pos_xyz, &config);
        assert!(
            l3 >= 1,
            "cursor should move to next visual line after newline"
        );
        assert_eq!(c3, 0);
    }

    #[test]
    fn test_extra_padding() {
        // Test to verify that wrapping doesn't add extra whitespace
        let config = WrapConfig::new(18);
        let text = "word1 word2 word3 word4";
        let wrapped = TextWrapper::wrap_text(text, &config);

        // Count spaces in original vs wrapped
        let original_spaces = text.chars().filter(|&c| c == ' ').count();
        let wrapped_spaces = wrapped.chars().filter(|&c| c == ' ').count();

        // The wrapped text should not have MORE spaces than the original
        assert_eq!(
            wrapped_spaces, original_spaces,
            "Wrapped text has extra spaces!"
        );
    }
}
