use crate::core::app::App;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
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

    // Pulsing indicator rendered in the title for simplicity
    let indicator = if app.is_streaming {
        let elapsed = app.pulse_start.elapsed().as_millis() as f32 / 1000.0;
        let phase = (elapsed * 2.0) % 2.0;
        if phase < 0.33 {
            "○"
        } else if phase < 0.66 {
            "◐"
        } else {
            "●"
        }
    } else {
        ""
    };

    let base_title = if app.is_streaming {
        "Type message (Esc to interrupt, Ctrl+R to retry)"
    } else {
        "Type message (Alt+Enter for new line, /help for help, Ctrl+C to quit)"
    };
    // Build a styled title with a distinct-colored indicator and a touch more right padding
    let input_title: Line = if indicator.is_empty() {
        Line::from(base_title.to_string())
    } else {
        Line::from(vec![
            Span::raw(base_title.to_string()),
            Span::raw(" "), // 1 space before indicator
            Span::styled(
                indicator.to_string(),
                // Match assistant output color for now
                Style::default().fg(Color::White),
            ),
            Span::raw("  "), // 2 spaces after indicator for a bit more padding
        ])
    };

    // Render border/title and the textarea inside
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Reset))
        .title(input_title);

    let area = chunks[1];
    let inner = input_block.inner(area);
    f.render_widget(input_block, area);

    // Render wrapped input text with a one-column right margin
    // Wrap one character earlier to avoid cursor touching the border
    let available_width = inner.width.saturating_sub(1);
    let config = WrapConfig::new(available_width as usize);
    let wrapped_text = TextWrapper::wrap_text(app.get_input_text(), &config);
    let paragraph = Paragraph::new(wrapped_text)
        .wrap(Wrap { trim: false })
        .scroll((app.input_scroll_offset, 0));
    f.render_widget(paragraph, inner);

    // Set cursor based on wrapped text and linear cursor position
    if app.input_mode && available_width > 0 {
        let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
            app.get_input_text(),
            app.input_cursor_position,
            &config,
        );
        let visible_line = (line as u16).saturating_sub(app.input_scroll_offset);
        if visible_line < inner.height {
            let cursor_x = inner.x.saturating_add(col as u16);
            let cursor_y = inner.y.saturating_add(visible_line);
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
