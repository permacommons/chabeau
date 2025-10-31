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

/// Layout information for wrapped text.
#[derive(Debug, Clone)]
pub struct WrappedCursorLayout {
    position_map: Vec<(usize, usize)>,
    line_count: usize,
}

impl WrappedCursorLayout {
    /// Construct a layout from an existing cursor position map and the last line index
    /// that appeared during wrapping.
    fn new(position_map: Vec<(usize, usize)>, last_line: usize) -> Self {
        let line_count = last_line.saturating_add(1).max(1);
        Self {
            position_map,
            line_count,
        }
    }

    /// Total number of visual lines in the wrapped text.
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// Borrow the cursor position mapping.
    pub fn position_map(&self) -> &[(usize, usize)] {
        &self.position_map
    }

    /// Consume the layout and return the mapping vector.
    pub fn into_position_map(self) -> Vec<(usize, usize)> {
        self.position_map
    }

    /// Returns the index range for the requested visual line if it exists.
    /// The returned range is expressed in cursor indices (positions between characters)
    /// and is inclusive on both ends.
    pub fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let mut start = None;
        let mut end = None;

        for (idx, (mapped_line, _)) in self.position_map.iter().enumerate() {
            if *mapped_line == line {
                if start.is_none() {
                    start = Some(idx);
                }
                end = Some(idx);
            } else if *mapped_line > line {
                break;
            }
        }

        start.zip(end)
    }

    /// Clamp the requested cursor index into the valid range and return its coordinates.
    pub fn coordinates_for_index(&self, idx: usize) -> (usize, usize) {
        let clamped = idx.min(self.position_map.len().saturating_sub(1));
        self.position_map.get(clamped).copied().unwrap_or((0, 0))
    }

    /// Find the cursor index for the desired visual line and column, returning the closest
    /// match when the line is shorter than the requested column.
    pub fn find_index_on_line(&self, target_line: usize, desired_col: usize) -> Option<usize> {
        let mut candidate = None;
        let mut fallback = None;

        for (idx, (line, col)) in self.position_map.iter().enumerate() {
            if *line == target_line {
                fallback = Some(idx);
                if *col >= desired_col {
                    candidate = Some(idx);
                    break;
                }
            } else if *line > target_line {
                break;
            }
        }

        candidate.or(fallback)
    }
}

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
        Self::cursor_layout(text, config).line_count()
    }

    /// Calculate which line a cursor position would be on after wrapping
    pub fn calculate_cursor_line(text: &str, cursor_position: usize, config: &WrapConfig) -> usize {
        Self::cursor_layout(text, config)
            .coordinates_for_index(cursor_position)
            .0
    }

    /// Build a mapping of cursor positions to wrapped line/column coordinates
    pub fn cursor_position_map(text: &str, config: &WrapConfig) -> Vec<(usize, usize)> {
        Self::cursor_layout(text, config).into_position_map()
    }

    /// Compute the cursor layout for wrapped text, including the position map and total lines.
    pub fn cursor_layout(text: &str, config: &WrapConfig) -> WrappedCursorLayout {
        // Build a mapping for cursor "positions" (between characters), not just characters.
        // There are N+1 positions for N characters.
        let original_chars: Vec<char> = text.chars().collect();

        // Always include the origin so the map has at least one entry
        let mut pos_map: Vec<(usize, usize)> = vec![(0, 0); original_chars.len() + 1];

        if original_chars.is_empty() {
            return WrappedCursorLayout::new(pos_map, 0);
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

        let mut wrapped_idx = 0usize; // index into wrapped_coords for non-newline chars
        let mut last_line = 0usize;
        let mut line_starts: Vec<(usize, usize)> = vec![(0, 0)];

        for (idx, ch) in original_chars.iter().enumerate() {
            if *ch == '\n' {
                let next_line = last_line.saturating_add(1);
                pos_map[idx + 1] = (next_line, 0);
                line_starts.push((next_line, idx + 1));
                last_line = next_line;
            } else {
                let (line, col) = wrapped_coords
                    .get(wrapped_idx)
                    .copied()
                    .unwrap_or((last_line, 0));
                pos_map[idx + 1] = (line, col);
                if line > last_line {
                    line_starts.push((line, idx));
                }
                last_line = line;
                wrapped_idx += 1;
            }
        }

        for (line, start_idx) in line_starts.into_iter() {
            if start_idx < pos_map.len() {
                pos_map[start_idx] = (line, 0);
            }
        }

        WrappedCursorLayout::new(pos_map, last_line)
    }

    /// Calculate cursor position within wrapped text using a character-by-character mapping
    pub fn calculate_cursor_position_in_wrapped_text(
        text: &str,
        cursor_position: usize,
        config: &WrapConfig,
    ) -> (usize, usize) {
        Self::cursor_layout(text, config).coordinates_for_index(cursor_position)
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

        let expectations = [(0, (0usize, 0usize)), (1, (0, 2)), (2, (1, 0)), (3, (1, 2))];

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

        // Cursor at position 5 (before wrapping to "world") sits at the start of the next line
        assert_eq!(line, 1);
        assert_eq!(col, 0);
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
    fn test_cursor_position_map_soft_wrap_column_zero() {
        let config = WrapConfig::new(4);
        let text = "abcdefgh";
        let map = TextWrapper::cursor_position_map(text, &config);

        assert_eq!(map[0], (0, 0));
        assert_eq!(
            map[4],
            (1, 0),
            "start of wrapped line should be column zero"
        );
        assert_eq!(map[8], (1, 4), "cursor after final char stays on last line");
    }

    #[test]
    fn cursor_layout_reports_line_count_and_line_search() {
        let config = WrapConfig::new(10);
        let text = "hi\nthere";
        let layout = TextWrapper::cursor_layout(text, &config);

        assert_eq!(layout.line_count(), 2);
        assert_eq!(layout.find_index_on_line(0, 2), Some(2));

        let newline_index = text.chars().position(|c| c == '\n').unwrap();
        assert_eq!(layout.find_index_on_line(1, 0), Some(newline_index + 1));

        let end_index = text.chars().count();
        assert_eq!(layout.find_index_on_line(1, 10), Some(end_index));
        assert_eq!(layout.coordinates_for_index(end_index), (1, 5));
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
