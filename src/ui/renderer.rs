use crate::core::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

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
        app.provider_name,
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
        "Type your message (Alt+Enter for new line, Esc to interrupt, Ctrl+R to retry, /help for help, Ctrl+C to quit)"
    } else if app.can_retry() {
        "Type your message (Alt+Enter for new line, Ctrl+R to retry, /help for help, Ctrl+C to quit)"
    } else {
        "Type your message (Alt+Enter for new line, /help for help, Ctrl+C to quit)"
    };

    // Create input text with streaming indicator if needed
    let input_text = if app.is_streaming {
        // Calculate pulse animation (0.0 to 1.0 over 1 second)
        let elapsed = app.pulse_start.elapsed().as_millis() as f32 / 1000.0;
        let pulse_phase = (elapsed * 2.0) % 2.0; // 2 cycles per second
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

        // Calculate available width inside the input box (account for borders)
        let inner_width = chunks[1].width.saturating_sub(2) as usize; // Remove left and right borders

        // Build a string that's exactly inner_width characters long
        // with the indicator ALWAYS at the last position
        let mut result = vec![' '; inner_width]; // Start with all spaces

        // Convert input to chars and place them at the beginning
        let input_chars: Vec<char> = app.input.chars().collect();
        let max_input_len = inner_width.saturating_sub(3); // Reserve space for gap + indicator + padding

        // Copy input characters to the beginning of result
        for (i, &ch) in input_chars.iter().take(max_input_len).enumerate() {
            result[i] = ch;
        }

        // If input was too long, add ellipsis
        if input_chars.len() > max_input_len && max_input_len >= 3 {
            result[max_input_len - 3] = '.';
            result[max_input_len - 2] = '.';
            result[max_input_len - 1] = '.';
        }

        // Place the indicator with one space padding from the right border
        if inner_width > 1 {
            // Get the first character of the symbol (should be just one)
            if let Some(symbol_char) = symbol.chars().next() {
                result[inner_width - 2] = symbol_char; // -2 instead of -1 for padding
            }
        }

        // Convert back to string
        result.into_iter().collect()
    } else {
        app.input.clone()
    };

    let input = Paragraph::new(input_text.as_str())
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Reset)) // Explicitly reset to system default
                .title(input_title),
        )
        .wrap(Wrap { trim: false }) // Don't trim whitespace to preserve newlines
        .scroll((app.input_scroll_offset, 0)); // Apply input scrolling

    f.render_widget(input, chunks[1]);

    // Set cursor position for multi-line input with scrolling support
    if app.input_mode {
        // Calculate the position of the cursor within the input text
        let input_chars: Vec<char> = app.input.chars().collect();
        let cursor_position = app.input_cursor_position.min(input_chars.len());

        // Find which line and column the cursor is on
        let mut current_line = 0u16;
        let mut current_col = 0usize;
        let mut chars_processed = 0;

        for (_, &ch) in input_chars.iter().enumerate() {
            if chars_processed >= cursor_position {
                break;
            }

            if ch == '\n' {
                current_line += 1;
                current_col = 0;
            } else {
                current_col += 1;
            }

            chars_processed += 1;
        }

        // Calculate the visible cursor position accounting for scroll offset
        let visible_cursor_line = current_line.saturating_sub(app.input_scroll_offset);

        // Only show cursor if it's within the visible area
        if visible_cursor_line < input_area_height {
            // Calculate the x position within the current line
            let max_cursor_x = if app.is_streaming {
                chunks[1].width.saturating_sub(6) // Leave space for indicator
            } else {
                chunks[1].width.saturating_sub(2) // Just account for borders
            };

            let cursor_x = (current_col as u16 + 1).min(max_cursor_x);
            let cursor_y = chunks[1].y + 1 + visible_cursor_line;

            f.set_cursor_position((chunks[1].x + cursor_x, cursor_y));
        }
    }
}
