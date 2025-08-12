use crate::core::app::App;
use crate::core::constants::INDICATOR_SPACE;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap, Widget},
    Frame,
};

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
        let config = WrapConfig::new(width);
        TextWrapper::wrap_text(text, &config)
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

        // Use TextWrapper to calculate cursor position
        let config = WrapConfig::new(text_area.width as usize);
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(self.text, cursor_position, &config);

        // Apply scroll offset
        let visible_line = (line as u16).saturating_sub(self.scroll_offset);

        // Only return position if cursor is within visible area
        if visible_line < text_area.height {
            Some((text_area.x + col as u16, text_area.y + visible_line))
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
