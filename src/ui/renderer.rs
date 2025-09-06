use crate::core::app::App;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub fn ui(f: &mut Frame, app: &App) {
    // Paint full-frame background based on theme to ensure readable contrast
    let bg_block = Block::default().style(Style::default().bg(app.theme.background_color));
    f.render_widget(bg_block, f.area());

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
    let title_text = format!(
        "Chabeau v{} - {} ({}) • Logging: {}",
        env!("CARGO_PKG_VERSION"),
        app.provider_display_name,
        app.model,
        app.get_logging_status()
    );
    let messages_paragraph = Paragraph::new(lines)
        .style(Style::default().bg(app.theme.background_color))
        .block(Block::default().title(Span::styled(title_text, app.theme.title_style)))
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
    // Build a styled title with theme styling on base title and indicator
    let input_title: Line = if indicator.is_empty() {
        Line::from(Span::styled(
            base_title.to_string(),
            app.theme.input_title_style,
        ))
    } else {
        Line::from(vec![
            Span::styled(base_title.to_string(), app.theme.input_title_style),
            Span::raw(" "), // 1 space before indicator
            Span::styled(indicator.to_string(), app.theme.streaming_indicator_style),
            Span::raw("  "), // 2 spaces after indicator for a bit more padding
        ])
    };

    // Render border/title and the textarea inside
    let input_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(app.theme.background_color))
        .border_style(app.theme.input_border_style)
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
        .style(
            app.theme
                .input_text_style
                .patch(Style::default().bg(app.theme.background_color)),
        )
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

    // Render modal picker overlay if present
    if let Some(picker) = &app.picker {
        let area = centered_rect(60, 60, f.area());

        // Clear any content under the modal
        f.render_widget(Clear, area);
        // Paint modal background consistent with theme
        let modal_bg = Block::default().style(Style::default().bg(app.theme.background_color));
        f.render_widget(modal_bg, area);

        // Outer bordered block with title
        let modal_block = Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.input_border_style)
            .title(Span::styled(&picker.title, app.theme.title_style));
        let content_area = modal_block.inner(area); // create padding space inside borders
        f.render_widget(modal_block, area);
        // Split space to show list + 1-line help
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(content_area);
        // Add an extra inset for whitespace inside list area
        let list_area = inset_rect(chunks[0], 1, 1);
        let help_area = chunks[1];

        // Items styled to match theme; we render inside inner content area to create whitespace
        let items: Vec<ListItem> = picker
            .items
            .iter()
            .map(|it| {
                ListItem::new(Line::from(Span::styled(
                    it.label.clone(),
                    app.theme.assistant_text_style,
                )))
            })
            .collect();

        let list = List::new(items)
            .style(Style::default().bg(app.theme.background_color))
            .highlight_style(
                app.theme
                    .streaming_indicator_style
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            )
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, list_area, &mut make_list_state(picker.selected));

        // Render in-modal help aligned with theme
        let help_text = "↑/↓ to navigate • Enter to apply • Esc to cancel";
        let help = Paragraph::new(Span::styled(help_text, app.theme.system_text_style));
        f.render_widget(help, help_area);
    }
}

fn make_list_state(selected: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));
    state
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1]);

    horizontal[1]
}

fn inset_rect(r: Rect, dx: u16, dy: u16) -> Rect {
    let nx = r.x.saturating_add(dx);
    let ny = r.y.saturating_add(dy);
    let nw = r.width.saturating_sub(dx.saturating_mul(2));
    let nh = r.height.saturating_sub(dy.saturating_mul(2));
    Rect {
        x: nx,
        y: ny,
        width: nw,
        height: nh,
    }
}
