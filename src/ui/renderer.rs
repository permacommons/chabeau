use crate::core::app::App;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::ui::osc_state::{compute_render_state, set_render_state, OscRenderState};
use crate::ui::span::SpanKind;
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
    let (lines, span_metadata) = if !app.in_edit_select_mode() && !app.in_block_select_mode() {
        let lines = app.get_prewrapped_lines_cached(chunks[0].width).clone();
        let metadata = app
            .get_prewrapped_span_metadata_cached(chunks[0].width)
            .clone();
        (lines, metadata)
    } else if app.in_edit_select_mode() {
        let highlight = Style::default();
        let layout = crate::utils::scroll::ScrollCalculator::build_layout_with_theme_and_selection_and_flags_and_width(
            &app.messages,
            &app.theme,
            app.selected_user_message_index(),
            highlight,
            app.markdown_enabled,
            app.syntax_enabled,
            Some(chunks[0].width as usize),
        );
        (layout.lines, layout.span_metadata)
    } else if app.in_block_select_mode() {
        let highlight = Style::default().add_modifier(Modifier::BOLD);
        let layout = crate::utils::scroll::ScrollCalculator::build_layout_with_codeblock_highlight_and_flags_and_width(
            &app.messages,
            &app.theme,
            app.selected_block_index(),
            highlight,
            app.markdown_enabled,
            app.syntax_enabled,
            Some(chunks[0].width as usize),
        );
        (layout.lines, layout.span_metadata)
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
    let title_text = build_main_title(app);
    let block = Block::default().title(Span::styled(title_text, app.theme.title_style));
    let inner_area = block.inner(chunks[0]);
    let picker_active = app.picker_state().is_some();

    let mut messages_lines = lines.clone();
    if picker_active {
        for (line, kinds) in messages_lines.iter_mut().zip(span_metadata.iter()) {
            for (span, kind) in line.spans.iter_mut().zip(kinds.iter()) {
                if matches!(kind, SpanKind::Link(_)) {
                    span.style = span.style.remove_modifier(
                        Modifier::UNDERLINED | Modifier::SLOW_BLINK | Modifier::RAPID_BLINK,
                    );
                }
            }
        }
    }

    let messages_paragraph = Paragraph::new(messages_lines)
        .style(Style::default().bg(app.theme.background_color))
        .block(block)
        .scroll((scroll_offset, app.horizontal_scroll_offset));

    f.render_widget(messages_paragraph, chunks[0]);

    if picker_active {
        set_render_state(OscRenderState::default());
    } else {
        let state = compute_render_state(
            inner_area,
            &lines,
            &span_metadata,
            scroll_offset as usize,
            app.horizontal_scroll_offset,
        );
        set_render_state(state);
    }

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

    let base_title = if app.in_edit_select_mode() {
        "Select user message (↑/↓ • Enter=Edit→Truncate • e=Edit in place • Del=Truncate • Esc=Cancel)"
    } else if app.in_block_select_mode() {
        "Select code block (↑/↓ • c=Copy • s=Save • Esc=Cancel)"
    } else if app.picker_session().is_some() {
        // Show specific prompt for picker mode with global shortcuts
        match app.current_picker_mode() {
            Some(crate::core::app::PickerMode::Model) => {
                "Select a model (Esc=cancel • Ctrl+C=quit)"
            }
            Some(crate::core::app::PickerMode::Provider) => {
                "Select a provider (Esc=cancel • Ctrl+C=quit)"
            }
            Some(crate::core::app::PickerMode::Theme) => {
                "Select a theme (Esc=cancel • Ctrl+C=quit)"
            }
            _ => "Make a selection (Esc=cancel • Ctrl+C=quit)",
        }
    } else if app.file_prompt().is_some() {
        "Specify new filename (Esc=Cancel • Alt+Enter=Overwrite)"
    } else if app.in_place_edit_index().is_some() {
        "Edit in place: Enter=Apply • Esc=Cancel (no send)"
    } else if app.compose_mode {
        "Compose a message (F4=toggle compose mode, Enter=new line, Alt+Enter=send)"
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
                    || s.contains("cancelled")
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
    // Suppress cursor when picker is open (like Ctrl+B/Ctrl+P modes)
    if app.is_input_active() && available_width > 0 && app.picker_session().is_none() {
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
    if let Some(picker) = app.picker_state() {
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
        // Split space to show list + metadata footer + help (2 lines)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // List area
                Constraint::Length(1), // Metadata footer
                Constraint::Length(2), // Help area (2 lines)
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

        // Generate help text for picker
        let help_text = generate_picker_help_text(app);
        let help = Paragraph::new(help_text.as_str()).style(app.theme.system_text_style);
        f.render_widget(help, help_area);
    }
}

fn build_main_title(app: &App) -> String {
    let model_display = if app.in_provider_model_transition || app.model.is_empty() {
        "no model selected".to_string()
    } else {
        app.model.clone()
    };
    let provider_display = if app.provider_display_name.trim().is_empty() {
        "(no provider selected)".to_string()
    } else {
        app.provider_display_name.clone()
    };
    format!(
        "Chabeau v{} - {} ({}) • Logging: {}",
        env!("CARGO_PKG_VERSION"),
        provider_display,
        model_display,
        app.get_logging_status()
    )
}

/// Generate help text for picker dialogs with appropriate shortcuts
fn generate_picker_help_text(app: &App) -> String {
    // Check if current selection is a default (has asterisk)
    let selected_is_default = app
        .picker_state()
        .and_then(|picker| picker.get_selected_item())
        .map(|item| item.label.ends_with('*'))
        .unwrap_or(false);

    let del_help = if selected_is_default {
        " • Del=Remove default"
    } else {
        ""
    };

    // Get the search filter for the current picker mode
    let search_filter = match app.current_picker_mode() {
        Some(crate::core::app::PickerMode::Model) => app
            .model_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        Some(crate::core::app::PickerMode::Theme) => app
            .theme_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        Some(crate::core::app::PickerMode::Provider) => app
            .provider_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        _ => "",
    };

    let first_line = if search_filter.is_empty() {
        format!("↑/↓=Navigate • F6=Sort • Type=Filter{}", del_help)
    } else {
        format!("↑/↓=Navigate • Backspace=Clear • F6=Sort{}", del_help)
    };

    // Suppress persistent save option during env-only startup model selection
    let show_persist = !(app.startup_env_only
        && app.current_picker_mode() == Some(crate::core::app::PickerMode::Model));
    if show_persist {
        format!("{}\nEnter=This session • Alt+Enter=As default", first_line)
    } else {
        format!("{}\nEnter=This session", first_line)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::{
        App, ModelPickerState, PickerData, PickerMode, PickerSession, ProviderPickerState,
        ThemePickerState,
    };
    use crate::ui::picker::PickerState;
    use crate::ui::theme::Theme;

    fn create_test_app() -> App {
        App::new_bench(Theme::dark_default(), true, false)
    }

    fn set_model_picker(
        app: &mut App,
        search_filter: &str,
        items: Vec<crate::ui::picker::PickerItem>,
        selected: usize,
        has_dates: bool,
    ) {
        let picker_state = PickerState::new("Test".to_string(), items.clone(), selected);
        app.picker_session = Some(PickerSession {
            mode: PickerMode::Model,
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: search_filter.to_string(),
                all_items: items,
                before_model: None,
                has_dates,
            }),
        });
    }

    fn set_theme_picker(
        app: &mut App,
        search_filter: &str,
        items: Vec<crate::ui::picker::PickerItem>,
        selected: usize,
    ) {
        let picker_state = PickerState::new("Test".to_string(), items.clone(), selected);
        app.picker_session = Some(PickerSession {
            mode: PickerMode::Theme,
            state: picker_state,
            data: PickerData::Theme(ThemePickerState {
                search_filter: search_filter.to_string(),
                all_items: items,
                before_theme: None,
                before_theme_id: None,
            }),
        });
    }

    fn set_provider_picker(
        app: &mut App,
        search_filter: &str,
        items: Vec<crate::ui::picker::PickerItem>,
        selected: usize,
    ) {
        let picker_state = PickerState::new("Test".to_string(), items.clone(), selected);
        app.picker_session = Some(PickerSession {
            mode: PickerMode::Provider,
            state: picker_state,
            data: PickerData::Provider(ProviderPickerState {
                search_filter: search_filter.to_string(),
                all_items: items,
                before_provider: None,
            }),
        });
    }

    #[test]
    fn title_shows_no_model_selected_during_transition() {
        let mut app = create_test_app();
        app.provider_display_name = "Cerebras".to_string();
        app.model = "foo-model".to_string();
        app.in_provider_model_transition = true;

        let title = build_main_title(&app);
        assert!(title.contains("(no model selected)"));
        assert!(!title.contains("foo-model"));
    }

    #[test]
    fn title_shows_model_when_not_in_transition() {
        let mut app = create_test_app();
        app.provider_display_name = "Cerebras".to_string();
        app.model = "foo-model".to_string();
        app.in_provider_model_transition = false;

        let title = build_main_title(&app);
        assert!(title.contains("foo-model"));
        assert!(!title.contains("(no model selected)"));
    }

    #[test]
    fn test_generate_picker_help_text_model_no_filter_no_default() {
        let mut app = create_test_app();
        // Create picker with non-default item
        let items = vec![crate::ui::picker::PickerItem {
            id: "test-model".to_string(),
            label: "Test Model".to_string(),
            metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
        assert!(!help_text.contains("Del=Remove default"));
    }

    #[test]
    fn test_generate_picker_help_text_model_with_filter() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "test-model".to_string(),
            label: "Test Model".to_string(),
            metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "gpt", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • Backspace=Clear • F6=Sort"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
        assert!(!help_text.contains("Type=Filter"));
    }

    #[test]
    fn test_generate_picker_help_text_with_default_selected() {
        let mut app = create_test_app();
        // Create picker with default item (has asterisk)
        let items = vec![crate::ui::picker::PickerItem {
            id: "default-provider".to_string(),
            label: "Default Provider*".to_string(), // Note: asterisk goes in label, not id
            metadata: None,
            sort_key: None,
        }];
        set_provider_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter • Del=Remove default"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
    }

    #[test]
    fn test_generate_picker_help_text_model_with_default_selected() {
        let mut app = create_test_app();
        // Create picker with default item (has asterisk)
        let items = vec![crate::ui::picker::PickerItem {
            id: "default-model".to_string(),
            label: "Default Model*".to_string(),
            metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter • Del=Remove default"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
    }

    #[test]
    fn test_generate_picker_help_text_theme_picker() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "dark".to_string(),
            label: "Dark Theme".to_string(),
            metadata: None,
            sort_key: None,
        }];
        set_theme_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
        assert!(!help_text.contains("Del=Remove default"));
    }

    #[test]
    fn test_generate_picker_help_text_theme_with_default_selected() {
        let mut app = create_test_app();
        // Default theme: asterisk on label
        let items = vec![crate::ui::picker::PickerItem {
            id: "dark".to_string(),
            label: "Dark Theme*".to_string(),
            metadata: None,
            sort_key: None,
        }];
        set_theme_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
    }

    #[test]
    fn test_generate_picker_help_text_no_picker() {
        let app = create_test_app();
        // No picker set

        let help_text = generate_picker_help_text(&app);

        // Should still generate basic help text
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
        assert!(!help_text.contains("Del=Remove default"));
    }
}
