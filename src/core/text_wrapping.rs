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

use unicode_width::UnicodeWidthChar;

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
    /// Wrap text at word boundaries while preserving all original spacing.
    pub fn wrap_text(text: &str, config: &WrapConfig) -> String {
        wrap_with_layout(text, config).0
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
        wrap_with_layout(text, config).1
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

#[derive(Debug)]
struct WordSegment {
    chars: Vec<(char, usize, usize)>,
    total_width: usize,
}

#[derive(Debug)]
struct SpaceSegment {
    chars: Vec<(char, usize, usize)>,
    total_width: usize,
}

#[derive(Debug)]
enum Segment {
    Word(WordSegment),
    Spaces(SpaceSegment),
    Newline { idx: usize },
}

fn wrap_with_layout(text: &str, config: &WrapConfig) -> (String, WrappedCursorLayout) {
    let char_count = text.chars().count();
    let mut builder = LayoutBuilder::new(config.width, char_count);

    let mut segments: Vec<Segment> = Vec::new();
    let mut iter = text.chars().enumerate().peekable();

    while let Some((idx, ch)) = iter.next() {
        if ch == '\n' {
            segments.push(Segment::Newline { idx });
            continue;
        }

        let width = UnicodeWidthChar::width(ch).unwrap_or(0);

        if ch.is_whitespace() {
            let mut chars = vec![(ch, idx, width)];
            while let Some(&(_, next_ch)) = iter.peek() {
                if next_ch == '\n' || !next_ch.is_whitespace() {
                    break;
                }
                let (next_idx, next_char) = iter.next().unwrap();
                let next_width = UnicodeWidthChar::width(next_char).unwrap_or(0);
                chars.push((next_char, next_idx, next_width));
            }
            let total_width = chars.iter().map(|(_, _, w)| *w).sum();
            segments.push(Segment::Spaces(SpaceSegment { chars, total_width }));
            continue;
        }

        let mut chars = vec![(ch, idx, width)];
        while let Some(&(_, next_ch)) = iter.peek() {
            if next_ch == '\n' || next_ch.is_whitespace() {
                break;
            }
            let (next_idx, next_char) = iter.next().unwrap();
            let next_width = UnicodeWidthChar::width(next_char).unwrap_or(0);
            chars.push((next_char, next_idx, next_width));
        }
        let total_width = chars.iter().map(|(_, _, w)| *w).sum();
        segments.push(Segment::Word(WordSegment { chars, total_width }));
    }

    for (i, segment) in segments.iter().enumerate() {
        match segment {
            Segment::Word(word) => {
                if builder.allow_wrap && builder.width > 0 && word.total_width > builder.width {
                    builder.handle_long_word(&word.chars);
                } else {
                    builder.handle_word(&word.chars, word.total_width);
                }
            }
            Segment::Spaces(spaces) => {
                let next_word_width = segments.get(i + 1).and_then(|next| match next {
                    Segment::Word(word) => Some(word.total_width),
                    _ => None,
                });
                builder.handle_space_segment(spaces, next_word_width);
            }
            Segment::Newline { idx } => builder.handle_newline(*idx),
        }
    }

    builder.finalize()
}

#[derive(Debug)]
struct LayoutBuilder {
    width: usize,
    allow_wrap: bool,
    wrapped: String,
    position_map: Vec<(usize, usize)>,
    current_line: usize,
    current_col: usize,
}

impl LayoutBuilder {
    fn new(width: usize, char_count: usize) -> Self {
        let allow_wrap = width > 0;
        let mut position_map = vec![(0, 0); char_count + 1];
        if char_count == 0 {
            position_map[0] = (0, 0);
        }
        Self {
            width,
            allow_wrap,
            wrapped: String::new(),
            position_map,
            current_line: 0,
            current_col: 0,
        }
    }

    fn handle_word(&mut self, word: &[(char, usize, usize)], total_width: usize) {
        if self.should_wrap_word(total_width) {
            if let Some(&(_, next_idx, _)) = word.first() {
                self.push_soft_break(next_idx);
            }
        }

        for &(ch, idx, width) in word {
            self.push_text_char(ch, idx, width);
        }
    }

    fn handle_long_word(&mut self, word: &[(char, usize, usize)]) {
        for &(ch, idx, width) in word {
            if self.should_wrap_char(width) {
                self.push_soft_break(idx);
            }
            self.push_text_char(ch, idx, width);
        }
    }

    fn handle_space_segment(&mut self, spaces: &SpaceSegment, next_word_width: Option<usize>) {
        if self.allow_wrap && spaces.chars.len() == 1 && spaces.chars[0].0 == ' ' {
            if let Some(next_width) = next_word_width {
                if next_width > 0
                    && self.current_col > 0
                    && self
                        .current_col
                        .saturating_add(spaces.total_width)
                        .saturating_add(next_width)
                        > self.width
                {
                    let (_, idx, _) = spaces.chars[0];
                    self.push_soft_break(idx + 1);
                    return;
                }
            }
        }

        for &(ch, idx, width) in &spaces.chars {
            self.handle_whitespace_char(ch, idx, width);
        }
    }

    fn handle_whitespace_char(&mut self, ch: char, idx: usize, width: usize) {
        if self.allow_wrap && width > 0 && self.current_col.saturating_add(width) > self.width {
            self.push_soft_break(idx);
        }
        self.push_text_char(ch, idx, width);
    }

    fn handle_newline(&mut self, idx: usize) {
        self.wrapped.push('\n');
        self.current_line = self.current_line.saturating_add(1);
        self.current_col = 0;
        self.position_map[idx + 1] = (self.current_line, 0);
    }

    fn push_text_char(&mut self, ch: char, idx: usize, width: usize) {
        self.wrapped.push(ch);
        self.current_col = self.current_col.saturating_add(width);
        self.position_map[idx + 1] = (self.current_line, self.current_col);
    }

    fn push_soft_break(&mut self, next_index: usize) {
        self.wrapped.push('\n');
        self.current_line = self.current_line.saturating_add(1);
        self.current_col = 0;
        if next_index < self.position_map.len() {
            self.position_map[next_index] = (self.current_line, 0);
        }
    }

    fn should_wrap_word(&self, word_width: usize) -> bool {
        self.allow_wrap
            && word_width > 0
            && self.current_col > 0
            && self.current_col.saturating_add(word_width) > self.width
    }

    fn should_wrap_char(&self, char_width: usize) -> bool {
        self.allow_wrap
            && char_width > 0
            && self.current_col > 0
            && self.current_col.saturating_add(char_width) > self.width
    }

    fn finalize(self) -> (String, WrappedCursorLayout) {
        let last_line = self.current_line;
        let layout = WrappedCursorLayout::new(self.position_map, last_line);
        (self.wrapped, layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

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
    fn single_space_elided_when_wrap_occurs() {
        let config = WrapConfig::new(5);
        let text = "hello world";
        let wrapped = TextWrapper::wrap_text(text, &config);

        assert_eq!(wrapped, "hello\nworld");
    }

    #[test]
    fn elided_space_maps_cursor_to_next_line_start() {
        let config = WrapConfig::new(5);
        let text = "hello world";
        let space_index = text.chars().position(|c| c == ' ').unwrap();
        let layout = TextWrapper::cursor_layout(text, &config);

        assert_eq!(layout.coordinates_for_index(space_index), (0, 5));
        assert_eq!(layout.coordinates_for_index(space_index + 1), (1, 0));
    }

    #[test]
    fn multiple_spaces_preserved_across_wrap() {
        let config = WrapConfig::new(4);
        let text = "foo  bar";
        let wrapped = TextWrapper::wrap_text(text, &config);

        let original_spaces = text.chars().filter(|&c| c == ' ').count();
        let wrapped_spaces = wrapped.chars().filter(|&c| c == ' ').count();

        assert_eq!(wrapped_spaces, original_spaces);
        assert!(wrapped.contains("\n "));
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
        let space_index = text.chars().position(|c| c == ' ').unwrap();
        let before_space =
            TextWrapper::calculate_cursor_position_in_wrapped_text(text, space_index, &config);
        assert_eq!(before_space, (0, 5));

        let after_space =
            TextWrapper::calculate_cursor_position_in_wrapped_text(text, space_index + 1, &config);

        // After consuming the separator the cursor lands at the start of the next visual line.
        assert_eq!(after_space, (1, 0));
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
    fn cursor_layout_tracks_consecutive_blank_lines() {
        let config = WrapConfig::new(20);
        let text = "first line\n\nsecond line";
        let (_, layout) = super::wrap_with_layout(text, &config);

        assert_eq!(layout.line_count(), 3);

        let first_newline = text.find('\n').unwrap();
        let blank_line_start = first_newline + 1;
        assert_eq!(layout.coordinates_for_index(blank_line_start), (1, 0));

        let (start, end) = layout
            .line_bounds(1)
            .expect("blank line should have bounds");
        assert_eq!(start, blank_line_start);
        assert_eq!(end, blank_line_start);

        // After the second newline we move to the third line at column zero.
        assert_eq!(layout.coordinates_for_index(blank_line_start + 1), (2, 0));
    }

    #[test]
    fn position_map_lines_are_monotonic() {
        let config = WrapConfig::new(8);
        let text = "alpha beta\n\n\nlonger paragraph that wraps across multiple words";
        let layout = TextWrapper::cursor_layout(text, &config);

        let mut last_line = 0usize;
        for &(line, _) in layout.position_map() {
            assert!(
                line >= last_line,
                "visual lines should not decrease ({} -> {})",
                last_line,
                line
            );
            last_line = line;
        }
        assert!(layout.line_count() >= 4);
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

        // The wrapped text should not introduce additional spaces compared to the source
        assert!(
            wrapped_spaces <= original_spaces,
            "Wrapped text has extra spaces!"
        );
    }
}
