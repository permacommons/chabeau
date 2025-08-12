use crate::core::app::App;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap, Widget},
    Frame,
};

// Constants for input area layout
const INDICATOR_SPACE: u16 = 4; // Space reserved for streaming indicator + margin

/// A custom input widget that handles text wrapping, cursor positioning, and streaming indicator
struct InputWidget<'a> {
    text: &'a str,
    cursor_position: usize,
    scroll_offset: u16,
    style: Style,
    block: Option<Block<'a>>,
    is_streaming: bool,
    pulse_start: std::time::Instant,
}

impl<'a> InputWidget<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor_position: 0,
            scroll_offset: 0,
            style: Style::default(),
            block: None,
            is_streaming: false,
            pulse_start: std::time::Instant::now(),
        }
    }

    fn cursor_position(mut self, position: usize) -> Self {
        self.cursor_position = position;
        self
    }

    fn scroll(mut self, offset: u16) -> Self {
        self.scroll_offset = offset;
        self
    }

    fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    fn streaming(mut self, is_streaming: bool, pulse_start: std::time::Instant) -> Self {
        self.is_streaming = is_streaming;
        self.pulse_start = pulse_start;
        self
    }
}

impl<'a> Widget for InputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render the block (border and title) first
        let inner_area = if let Some(ref block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        // Reserve space for streaming indicator
        let text_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width.saturating_sub(INDICATOR_SPACE),
            height: inner_area.height,
        };

        // Pre-process text to insert line breaks at character boundaries
        let wrapped_text = self.wrap_text_at_boundaries(self.text, text_area.width as usize);

        // Render the pre-wrapped text without additional wrapping
        let paragraph = Paragraph::new(wrapped_text.as_str())
            .style(self.style)
            .scroll((self.scroll_offset, 0));

        paragraph.render(text_area, buf);

        // Render streaming indicator if needed
        if self.is_streaming {
            self.render_streaming_indicator(area, buf);
        }

        // Set cursor position if we're in input mode
        // Note: This is handled by the caller since Widget trait doesn't have access to Frame
    }
}

impl<'a> InputWidget<'a> {
    /// Pre-process text to insert line breaks at word boundaries while preserving spacing
    fn wrap_text_at_boundaries(&self, text: &str, width: usize) -> String {
        if width == 0 {
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
                        if current_line_len > 0 && current_line_len + word_len > width {
                            // Need to wrap before this word
                            result.push('\n');
                            current_line_len = 0;
                        }

                        // Handle very long words
                        if word_len > width {
                            let mut remaining_word = current_word.as_str();
                            while !remaining_word.is_empty() {
                                let chars_to_take = width.saturating_sub(current_line_len);
                                if chars_to_take == 0 {
                                    result.push('\n');
                                    current_line_len = 0;
                                    continue;
                                }

                                let word_chars: Vec<char> = remaining_word.chars().collect();
                                let chunk: String = word_chars.iter().take(chars_to_take).collect();
                                result.push_str(&chunk);
                                current_line_len += chunk.chars().count();

                                remaining_word = &remaining_word[chunk.len()..];

                                if !remaining_word.is_empty() {
                                    result.push('\n');
                                    current_line_len = 0;
                                }
                            }
                        } else {
                            // Normal word that fits
                            result.push_str(&current_word);
                            current_line_len += word_len;
                        }

                        current_word.clear();
                    }

                    // Add the whitespace character if it fits
                    if current_line_len < width {
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
                if current_line_len > 0 && current_line_len + word_len > width {
                    // Need to wrap before this word
                    result.push('\n');
                    current_line_len = 0;
                }

                // Handle very long words
                if word_len > width {
                    let mut remaining_word = current_word.as_str();
                    while !remaining_word.is_empty() {
                        let chars_to_take = width.saturating_sub(current_line_len);
                        if chars_to_take == 0 {
                            result.push('\n');
                            current_line_len = 0;
                            continue;
                        }

                        let word_chars: Vec<char> = remaining_word.chars().collect();
                        let chunk: String = word_chars.iter().take(chars_to_take).collect();
                        result.push_str(&chunk);
                        current_line_len += chunk.chars().count();

                        remaining_word = &remaining_word[chunk.len()..];

                        if !remaining_word.is_empty() {
                            result.push('\n');
                            current_line_len = 0;
                        }
                    }
                } else {
                    // Normal word that fits
                    result.push_str(&current_word);
                    current_line_len += word_len;
                }
            }
        }

        result
    }

    /// Calculate cursor position using word wrapping logic that matches text rendering
    fn calculate_cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let inner_area = if self.block.is_some() {
            Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(2),
            }
        } else {
            area
        };

        let text_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width.saturating_sub(INDICATOR_SPACE),
            height: inner_area.height,
        };

        if text_area.width == 0 {
            return None;
        }

        let cursor_position = self.cursor_position.min(self.text.chars().count());

        // Count characters in original text up to cursor position
        let text_before_cursor: String = self.text.chars().take(cursor_position).collect();

        // Get the wrapped version of text before cursor
        let wrapped_before_cursor = self.wrap_text_at_boundaries(&text_before_cursor, text_area.width as usize);

        // Count lines and find column position in wrapped text
        let lines: Vec<&str> = wrapped_before_cursor.split('\n').collect();
        let line = (lines.len() as u16).saturating_sub(1);
        let col = if let Some(last_line) = lines.last() {
            last_line.chars().count() as u16
        } else {
            0
        };

        // Apply scroll offset
        let visible_line = line.saturating_sub(self.scroll_offset);

        // Only return position if cursor is within visible area
        if visible_line < text_area.height {
            Some((text_area.x + col, text_area.y + visible_line))
        } else {
            None
        }
    }

    /// Render the streaming indicator
    fn render_streaming_indicator(&self, area: Rect, buf: &mut Buffer) {
        // Calculate pulse animation
        let elapsed = self.pulse_start.elapsed().as_millis() as f32 / 1000.0;
        let pulse_phase = (elapsed * 2.0) % 2.0;
        let pulse_intensity = if pulse_phase < 1.0 {
            pulse_phase
        } else {
            2.0 - pulse_phase
        };

        // Choose symbol based on pulse intensity
        let symbol = if pulse_intensity < 0.33 {
            "○"
        } else if pulse_intensity < 0.66 {
            "◐"
        } else {
            "●"
        };

        // Position indicator in top-right of the widget
        let indicator_x = area.x + area.width.saturating_sub(3);
        let indicator_y = area.y + 1;

        if indicator_x < area.x + area.width && indicator_y < area.y + area.height {
            buf[(indicator_x, indicator_y)]
                .set_symbol(symbol)
                .set_style(Style::default().fg(Color::Cyan));
        }
    }
}

pub fn ui(f: &mut Frame, app: &App) {
    // Calculate dynamic input area height based on content
    let input_area_height = app.calculate_input_area_height(f.area().width);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(input_area_height + 2), // +2 for borders
        ])
        .split(f.area());

    // Use the shared method to build display lines
    let lines = app.build_display_lines();

    // Calculate scroll position using wrapped line count
    let available_height = chunks[0].height.saturating_sub(1); // Account for title
    let total_wrapped_lines = app.calculate_wrapped_line_count(chunks[0].width);

    // Always use the app's scroll_offset, but ensure it's within bounds
    let max_offset = if total_wrapped_lines > available_height {
        total_wrapped_lines.saturating_sub(available_height)
    } else {
        0
    };
    let scroll_offset = app.scroll_offset.min(max_offset);

    // Create enhanced title with version, provider, model name and logging status
    let title = format!(
        "Chabeau v{} - {} ({}) • Logging: {}",
        env!("CARGO_PKG_VERSION"),
        app.provider_display_name,
        app.model,
        app.get_logging_status()
    );

    let messages_paragraph = Paragraph::new(lines)
        .block(Block::default().title(title))
        .wrap(Wrap { trim: true })
        .scroll((scroll_offset, 0));

    f.render_widget(messages_paragraph, chunks[0]);

    // Input area takes full width
    let input_style = if app.input_mode {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let input_title = if app.is_streaming {
        "Type message (Esc to interrupt, Ctrl+R to retry)"
    } else {
        "Type message (Alt+Enter for new line, /help for help, Ctrl+C to quit)"
    };

    // Create and render the custom input widget
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Reset))
        .title(input_title);

    let input_widget = InputWidget::new(&app.input)
        .cursor_position(app.input_cursor_position)
        .scroll(app.input_scroll_offset)
        .style(input_style)
        .block(input_block)
        .streaming(app.is_streaming, app.pulse_start);

    f.render_widget(input_widget, chunks[1]);

    // Set cursor position using the widget's calculation
    if app.input_mode {
        let widget_for_cursor = InputWidget::new(&app.input)
            .cursor_position(app.input_cursor_position)
            .scroll(app.input_scroll_offset)
            .block(Block::default().borders(Borders::ALL));

        if let Some((cursor_x, cursor_y)) = widget_for_cursor.calculate_cursor_position(chunks[1]) {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
