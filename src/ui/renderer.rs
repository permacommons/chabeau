use crate::core::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use std::time::Instant;

// Constants for streaming indicator layout
const BORDER_WIDTH: usize = 2;
const INDICATOR_SPACING: usize = 3; // Space for gap + indicator + padding
const ELLIPSIS_LENGTH: usize = 3;

/// Creates input text with a pulsing streaming indicator positioned in the top-right corner
fn create_input_with_streaming_indicator(input: &str, pulse_start: Instant, terminal_width: u16) -> String {
    // Calculate pulse animation (0.0 to 1.0 over 1 second)
    let elapsed = pulse_start.elapsed().as_millis() as f32 / 1000.0;
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

    let input_lines: Vec<&str> = input.split('\n').collect();
    let inner_width = terminal_width.saturating_sub(BORDER_WIDTH as u16) as usize;

    if input_lines.len() == 1 {
        create_single_line_with_indicator(input, symbol, inner_width)
    } else {
        create_multiline_with_indicator(&input_lines, symbol, inner_width)
    }
}

/// Creates a single line with the streaming indicator at the end
fn create_single_line_with_indicator(input: &str, symbol: &str, inner_width: usize) -> String {
    let mut result = vec![' '; inner_width];
    let input_chars: Vec<char> = input.chars().collect();
    let max_input_len = inner_width.saturating_sub(INDICATOR_SPACING);

    // Copy input characters to the beginning
    for (i, &ch) in input_chars.iter().take(max_input_len).enumerate() {
        result[i] = ch;
    }

    // Add ellipsis if input was truncated
    if input_chars.len() > max_input_len && max_input_len >= ELLIPSIS_LENGTH {
        for i in 0..ELLIPSIS_LENGTH {
            result[max_input_len - ELLIPSIS_LENGTH + i] = '.';
        }
    }

    // Place the indicator with padding from the right border
    if inner_width > 1 {
        if let Some(symbol_char) = symbol.chars().next() {
            result[inner_width - 2] = symbol_char;
        }
    }

    result.into_iter().collect()
}

/// Creates multi-line input with the streaming indicator on the first line only
fn create_multiline_with_indicator(input_lines: &[&str], symbol: &str, inner_width: usize) -> String {
    let mut modified_lines = Vec::new();

    for (line_idx, line) in input_lines.iter().enumerate() {
        if line_idx == 0 {
            // First line gets the indicator
            modified_lines.push(add_indicator_to_line(line, symbol, inner_width));
        } else {
            // Other lines remain unchanged
            modified_lines.push(line.to_string());
        }
    }

    modified_lines.join("\n")
}

/// Adds the streaming indicator to a single line, handling padding and truncation
fn add_indicator_to_line(line: &str, symbol: &str, inner_width: usize) -> String {
    let line_chars: Vec<char> = line.chars().collect();
    let available_space = inner_width.saturating_sub(INDICATOR_SPACING);

    if line_chars.len() <= available_space {
        // Line fits - pad to full width and add indicator
        let mut padded_line = String::from(line);
        let spaces_needed = inner_width.saturating_sub(line_chars.len()).saturating_sub(2);

        for _ in 0..spaces_needed {
            padded_line.push(' ');
        }

        padded_line.push_str(symbol);
        padded_line
    } else {
        // Line is too long - truncate with ellipsis and add indicator
        let mut truncated_line = String::new();
        let truncate_at = available_space.saturating_sub(ELLIPSIS_LENGTH);

        for (i, &ch) in line_chars.iter().enumerate() {
            if i >= truncate_at {
                break;
            }
            truncated_line.push(ch);
        }

        truncated_line.push_str("...");

        // Add spaces to fill remaining width
        let spaces_needed = inner_width.saturating_sub(truncated_line.chars().count()).saturating_sub(1);
        for _ in 0..spaces_needed {
            truncated_line.push(' ');
        }

        truncated_line.push_str(symbol);
        truncated_line
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
        "Type message (Esc to interrupt, Ctrl+R to retry)"
    } else {
        "Type message (Alt+Enter for new line, /help for help, Ctrl+C to quit)"
    };

    // Create input text with streaming indicator if needed
    let input_text = if app.is_streaming {
        create_input_with_streaming_indicator(&app.input, app.pulse_start, f.area().width)
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

        for (chars_processed, &ch) in input_chars.iter().enumerate() {
            if chars_processed >= cursor_position {
                break;
            }

            if ch == '\n' {
                current_line += 1;
                current_col = 0;
            } else {
                current_col += 1;
            }
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
