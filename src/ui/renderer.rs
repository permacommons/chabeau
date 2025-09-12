use crate::core::app::App;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub fn ui(f: &mut Frame, app: &mut App) {
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

    // Use cached prewrapped lines in normal mode for faster redraws.
    // Otherwise, build lines with selection/highlight and prewrap on the fly.
    let lines = if !app.edit_select_mode && !app.block_select_mode {
        app.get_prewrapped_lines_cached(chunks[0].width).clone()
    } else if app.edit_select_mode {
        let highlight = app
            .theme
            .streaming_indicator_style
            .add_modifier(Modifier::REVERSED);
        let built = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_selection_and_flags(
            &app.messages,
            &app.theme,
            app.selected_user_message_index,
            highlight,
            app.markdown_enabled,
            app.syntax_enabled,
        );
        crate::utils::scroll::ScrollCalculator::prewrap_lines(&built, chunks[0].width)
    } else if app.block_select_mode {
        let highlight = app
            .theme
            .streaming_indicator_style
            .add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let built = crate::utils::scroll::ScrollCalculator::build_display_lines_with_codeblock_highlight_and_flags(
            &app.messages,
            &app.theme,
            app.selected_block_index,
            highlight,
            app.markdown_enabled,
            app.syntax_enabled,
        );
        crate::utils::scroll::ScrollCalculator::prewrap_lines(&built, chunks[0].width)
    } else {
        unreachable!()
    };

    // Calculate scroll position using the prewrapped lines (exact render)
    let available_height = chunks[0].height.saturating_sub(1); // Account for title
    let total_wrapped_lines = lines.len() as u16;

    // Always use the app's scroll_offset, but ensure it's within bounds
    let max_offset = if total_wrapped_lines > available_height {
        total_wrapped_lines.saturating_sub(available_height)
    } else {
        0
    };
    // Clamp the user-controlled scroll offset
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
        .scroll((scroll_offset, app.horizontal_scroll_offset));

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

    let base_title = if app.edit_select_mode {
        "Select user message (↑/↓ • Enter=Edit→Truncate • e=Edit in place • Del=Truncate • Esc=Cancel)"
    } else if app.block_select_mode {
        "Select code block (↑/↓ • c=Copy • s=Save • Esc=Cancel)"
    } else if app.file_prompt.is_some() {
        "Specify new filename (Esc=Cancel • Alt+Enter=Overwrite)"
    } else if app.in_place_edit_index.is_some() {
        "Edit in place: Enter=Apply • Esc=Cancel (no send)"
    } else if app.is_streaming {
        "Type a new message (Esc=interrupt • Ctrl+R=retry)"
    } else {
        "Type a new message (Alt+Enter=new line • Ctrl+C=quit • More: Type /help)"
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

    // Prepare optional bottom-right status message, shortened and right-aligned
    let status_bottom: Option<Line> = if let Some(status) = &app.status {
        // Limit to available width minus borders and a small margin
        let input_area_width = chunks[1].width;
        let inner_width = input_area_width.saturating_sub(2) as usize; // exclude borders
        if inner_width < 8 {
            None
        } else {
            // Leave one space on both sides of the status text
            let max_chars = inner_width.saturating_sub(2);
            let text_raw = if status.chars().count() > max_chars {
                // Truncate and add ellipsis
                let mut s = String::new();
                for (i, ch) in status.chars().enumerate() {
                    if i + 1 >= max_chars {
                        break;
                    }
                    s.push(ch);
                }
                s.push('…');
                s
            } else {
                status.clone()
            };
            // Determine if this is an error status to use error color
            let is_error = {
                let s = text_raw.to_ascii_lowercase();
                s.contains("error")
                    || s.contains("exists")
                    || s.contains("failed")
                    || s.contains("denied")
            };
            let base_style = if is_error {
                app.theme.error_text_style
            } else {
                app.theme.system_text_style
            };
            // Build a brief highlight effect: flash brighter then dim (same timing as success)
            let style = if let Some(set_at) = app.status_set_at {
                let ms = set_at.elapsed().as_millis() as u64;
                if ms < 300 {
                    // brief highlight: bold the base style
                    base_style.add_modifier(Modifier::BOLD)
                } else if ms < 900 {
                    base_style.add_modifier(Modifier::DIM)
                } else {
                    base_style
                }
            } else {
                base_style
            };

            // Compose a visual border with left filler and right-aligned status text
            // Account for one leading and one trailing space
            let status_len = text_raw.chars().count() + 2;
            let dash_count = inner_width.saturating_sub(status_len);
            let left_border = if dash_count > 0 {
                "─".repeat(dash_count)
            } else {
                String::new()
            };
            let line = Line::from(vec![
                Span::styled(left_border, app.theme.input_border_style),
                Span::styled(" ", app.theme.input_border_style),
                Span::styled(text_raw.clone(), style),
                Span::styled(" ", app.theme.input_border_style),
            ]);
            Some(line)
        }
    } else {
        None
    };

    // Render border/title and the textarea inside
    let mut input_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(app.theme.background_color))
        .border_style(app.theme.input_border_style)
        .title(input_title);
    if let Some(bottom) = status_bottom {
        input_block = input_block.title_bottom(bottom);
    }

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
        // Split space to show list + metadata footer + help
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // List area
                Constraint::Length(1), // Metadata footer
                Constraint::Length(1), // Help area
            ])
            .split(content_area);
        // Add an extra inset for whitespace inside list area
        let list_area = inset_rect(chunks[0], 1, 1);
        let metadata_area = chunks[1];
        let help_area = chunks[2];

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

        // Render metadata footer for selected item
        let metadata_text = picker
            .get_selected_metadata()
            .unwrap_or("No metadata available");
        let metadata = Paragraph::new(Span::styled(
            metadata_text,
            app.theme
                .system_text_style
                .add_modifier(ratatui::style::Modifier::DIM),
        ));
        f.render_widget(metadata, metadata_area);

        // Render in-modal help aligned with theme
        let help_text = match app.picker_mode {
            Some(crate::core::app::PickerMode::Model) => {
                if app.model_search_filter.is_empty() {
                    "↑/↓/Home/End to navigate • F2 to sort • type to filter • Enter to apply • Esc to cancel"
                } else {
                    "↑/↓/Home/End to navigate • Backspace to clear • F2 to sort • Enter to apply • Esc to cancel"
                }
            }
            Some(crate::core::app::PickerMode::Theme) => {
                if app.theme_search_filter.is_empty() {
                    "↑/↓/Home/End to navigate • F2 to sort • type to filter • Enter to apply • Esc to cancel"
                } else {
                    "↑/↓/Home/End to navigate • Backspace to clear • F2 to sort • Enter to apply • Esc to cancel"
                }
            }
            _ => "↑/↓ to navigate • Enter to apply • Esc to cancel",
        };
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
