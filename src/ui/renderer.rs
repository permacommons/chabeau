use crate::core::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

// Constants for input area layout
const INDICATOR_SPACE: u16 = 4; // Space reserved for streaming indicator + margin

/// Handles rendering of the input area with proper text wrapping and indicator positioning
struct InputAreaRenderer {
    area: ratatui::layout::Rect,
    indicator_space: u16,
}

impl InputAreaRenderer {
    fn new(area: ratatui::layout::Rect) -> Self {
        Self {
            area,
            indicator_space: INDICATOR_SPACE,
        }
    }

    /// Get the area for the text content (inside border, reserving space for indicator)
    fn text_area(&self) -> ratatui::layout::Rect {
        ratatui::layout::Rect {
            x: self.area.x + 1, // Inside left border
            y: self.area.y + 1, // Inside top border
            width: self.area.width.saturating_sub(2 + self.indicator_space), // Reserve space for borders + indicator
            height: self.area.height.saturating_sub(2), // Inside borders
        }
    }

    /// Get the area for the streaming indicator
    fn indicator_area(&self) -> ratatui::layout::Rect {
        ratatui::layout::Rect {
            x: self.area.x + self.area.width.saturating_sub(3),
            y: self.area.y + 1,
            width: 1,
            height: 1,
        }
    }

    /// Get the effective text width for cursor positioning
    fn text_width(&self) -> usize {
        self.area.width.saturating_sub(2 + self.indicator_space) as usize
    }

    /// Render the complete input area
    fn render(&self, f: &mut Frame, app: &App, input_style: Style, input_title: &str) {
        // Render the border and title
        let border_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Reset))
            .title(input_title);

        f.render_widget(border_block, self.area);

        // Render the text in the reserved area
        let input_text = Paragraph::new(app.input.as_str())
            .style(input_style)
            .wrap(Wrap { trim: false })
            .scroll((app.input_scroll_offset, 0));

        f.render_widget(input_text, self.text_area());

        // Render streaming indicator if needed
        if app.is_streaming {
            self.render_streaming_indicator(f, app);
        }
    }

    /// Render the streaming indicator
    fn render_streaming_indicator(&self, f: &mut Frame, app: &App) {
        // Calculate pulse animation
        let elapsed = app.pulse_start.elapsed().as_millis() as f32 / 1000.0;
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

        let indicator_paragraph = Paragraph::new(symbol)
            .style(Style::default().fg(Color::Cyan));

        f.render_widget(indicator_paragraph, self.indicator_area());
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

    let input_renderer = InputAreaRenderer::new(chunks[1]);
    input_renderer.render(f, app, input_style, input_title);

    // Set cursor position for multi-line input with scrolling support
    if app.input_mode {
        let text_width = input_renderer.text_width();

        // Calculate cursor position using simple character counting with proper wrapping
        let cursor_position = app.input_cursor_position.min(app.input.chars().count());
        let mut line = 0u16;
        let mut col = 0usize;

        for (i, ch) in app.input.chars().enumerate() {
            if i >= cursor_position {
                break;
            }

            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                if col >= text_width {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
            }
        }

        // Calculate visible position accounting for scroll
        let visible_line = line.saturating_sub(app.input_scroll_offset);

        // Only show cursor if it's within the visible area
        if visible_line < input_area_height {
            let text_area = input_renderer.text_area();
            let cursor_x = text_area.x + (col as u16).min(text_width as u16);
            let cursor_y = text_area.y + visible_line;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
