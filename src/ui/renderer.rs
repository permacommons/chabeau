//! Terminal UI rendering with Ratatui.
//!
//! This module composes the chat transcript, pickers, and input area using
//! Ratatui layout primitives. It caches wrapped lines for performance,
//! adapts styling when pickers are open, and projects mode-specific prompts
//! (compose, edit, streaming indicators) into the title bar.
//!
//! Scroll state and OSC hyperlink metadata are recomputed only when necessary
//! to keep redraws responsive.

use crate::core::app::ui_state::EditSelectTarget;
use crate::core::app::App;
use crate::core::message::ROLE_ASSISTANT;
use crate::core::text_wrapping::{TextWrapper, WrapConfig};
use crate::ui::osc_state::{compute_render_state, set_render_state, OscRenderState};
use crate::ui::span::SpanKind;
use crate::ui::title::build_main_title;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::borrow::Cow;

pub fn ui(f: &mut Frame, app: &mut App) {
    // Paint full-frame background based on theme to ensure readable contrast
    let bg_block = Block::default().style(Style::default().bg(app.ui.theme.background_color));
    f.render_widget(bg_block, f.area());

    // Calculate dynamic input area height based on content
    let input_area_height = app.ui.calculate_input_area_height(f.area().width);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(input_area_height + 2), // +2 for borders
        ])
        .split(f.area());

    // Use cached prewrapped lines in normal mode for faster redraws.
    // Otherwise, build lines with selection/highlight and prewrap on the fly.
    let (lines, span_metadata) = if !app.ui.in_edit_select_mode() && !app.ui.in_block_select_mode()
    {
        let lines = app.get_prewrapped_lines_cached(chunks[0].width).clone();
        let metadata = app
            .get_prewrapped_span_metadata_cached(chunks[0].width)
            .clone();
        (lines, metadata)
    } else if app.ui.in_edit_select_mode() {
        let highlight = Style::default();
        let layout = crate::utils::scroll::ScrollCalculator::build_layout_with_theme_and_selection_and_flags_and_width(
            &app.ui.messages,
            &app.ui.theme,
            app.ui.selected_edit_message_index(),
            highlight,
            app.ui.markdown_enabled,
            app.ui.syntax_enabled,
            Some(chunks[0].width as usize),
            Some(app.ui.user_display_name.clone()),
        );
        (layout.lines, layout.span_metadata)
    } else if app.ui.in_block_select_mode() {
        let highlight = Style::default().add_modifier(Modifier::BOLD);
        let layout = crate::utils::scroll::ScrollCalculator::build_layout_with_codeblock_highlight_and_flags_and_width(
            &app.ui.messages,
            &app.ui.theme,
            app.ui.selected_block_index(),
            highlight,
            app.ui.markdown_enabled,
            app.ui.syntax_enabled,
            Some(chunks[0].width as usize),
            Some(app.ui.user_display_name.clone()),
        );
        (layout.lines, layout.span_metadata)
    } else {
        unreachable!()
    };

    // Calculate scroll position using the prewrapped lines (exact render)
    let available_height = {
        // Delegate available-height computation to the conversation controller
        let conversation = app.conversation();
        conversation.calculate_available_height(f.area().height, input_area_height)
    };

    // Compute maximum scroll via UiState helper
    let max_offset = app
        .ui
        .calculate_max_scroll_offset(available_height, chunks[0].width);

    // Clamp the user-controlled scroll offset
    let scroll_offset = app.ui.scroll_offset.min(max_offset);

    // Create enhanced title with version, provider, model name and logging status
    let title_text = build_main_title(app, chunks[0].width);
    let block = Block::default().title(Span::styled(title_text, app.ui.theme.title_style));
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
        .style(Style::default().bg(app.ui.theme.background_color))
        .block(block)
        .scroll((scroll_offset, app.ui.horizontal_scroll_offset));

    f.render_widget(messages_paragraph, chunks[0]);

    if picker_active {
        set_render_state(OscRenderState::default());
    } else {
        let state = compute_render_state(
            inner_area,
            &lines,
            &span_metadata,
            scroll_offset as usize,
            app.ui.horizontal_scroll_offset,
        );
        set_render_state(state);
    }

    // Input area takes full width

    // Pulsing indicator rendered in the title for simplicity
    const STREAMING_FRAMES: [&str; 8] = ["○", "◔", "◑", "◕", "●", "◕", "◑", "◔"];
    const ROTATIONS_PER_SECOND: f32 = 0.5;

    let indicator = if app.ui.is_activity_indicator_visible() {
        let elapsed = app.ui.pulse_start.elapsed().as_secs_f32();
        let total_frames =
            (elapsed * ROTATIONS_PER_SECOND * STREAMING_FRAMES.len() as f32).floor() as usize;
        STREAMING_FRAMES[total_frames % STREAMING_FRAMES.len()]
    } else {
        ""
    };

    let base_title: Cow<'_, str> = if app.ui.in_edit_select_mode() {
        match app.ui.edit_select_target() {
            Some(EditSelectTarget::Assistant) => {
                Cow::Borrowed(
                    "Select assistant message (↑/↓ • Enter=Edit→Truncate • e=Edit in place • Del=Truncate • Esc=Cancel)",
                )
            }
            _ => {
                Cow::Borrowed(
                    "Select user message (↑/↓ • Enter=Edit→Truncate • e=Edit in place • Del=Truncate • Esc=Cancel)",
                )
            }
        }
    } else if app.ui.in_block_select_mode() {
        Cow::Borrowed("Select code block (↑/↓ • c=Copy • s=Save • Esc=Cancel)")
    } else if app.picker_session().is_some() {
        // Show specific prompt for picker mode with global shortcuts
        match app.current_picker_mode() {
            Some(crate::core::app::PickerMode::Model) => {
                Cow::Borrowed("Select a model (Esc=cancel • Ctrl+C=quit)")
            }
            Some(crate::core::app::PickerMode::Provider) => {
                Cow::Borrowed("Select a provider (Esc=cancel • Ctrl+C=quit)")
            }
            Some(crate::core::app::PickerMode::Theme) => {
                Cow::Borrowed("Select a theme (Esc=cancel • Ctrl+C=quit)")
            }
            Some(crate::core::app::PickerMode::Character) => {
                Cow::Borrowed("Select a character (Esc=cancel • Ctrl+C=quit)")
            }
            Some(crate::core::app::PickerMode::Persona) => {
                Cow::Borrowed("Select a persona (Esc=cancel • Ctrl+C=quit)")
            }
            Some(crate::core::app::PickerMode::Preset) => {
                Cow::Borrowed("Select a preset (Esc=cancel • Ctrl+C=quit)")
            }
            _ => Cow::Borrowed("Make a selection (Esc=cancel • Ctrl+C=quit)"),
        }
    } else if app.ui.file_prompt().is_some() {
        Cow::Borrowed("Specify new filename (Esc=Cancel • Alt+Enter=Overwrite)")
    } else if let Some(index) = app.ui.in_place_edit_index() {
        let mut title = String::from("Edit in place: Enter=Apply • Esc=Cancel (no send)");
        if app.ui.compose_mode {
            if let Some(message) = app.ui.messages.get(index) {
                if message.role == ROLE_ASSISTANT {
                    title.push_str(" • F4: toggle compose mode");
                }
            }
        }
        Cow::Owned(title)
    } else if app.ui.is_editing_assistant_message() {
        if app.ui.compose_mode {
            Cow::Owned(String::from("Edit message (F4: toggle compose mode)"))
        } else {
            Cow::Borrowed("Edit message")
        }
    } else if app.ui.compose_mode {
        Cow::Borrowed("Compose a message (F4=toggle compose mode, Enter=new line, Alt+Enter=send)")
    } else if app.ui.is_streaming {
        Cow::Borrowed("Type a new message (Esc=interrupt • Ctrl+R=retry)")
    } else {
        Cow::Borrowed("Type a new message (Alt+Enter=new line • Ctrl+C=quit • More: Type /help)")
    };
    // Build a styled title with theme styling on base title and indicator
    let input_title: Line = if indicator.is_empty() {
        Line::from(Span::styled(
            base_title.to_string(),
            app.ui.theme.input_title_style,
        ))
    } else {
        Line::from(vec![
            Span::styled(base_title.to_string(), app.ui.theme.input_title_style),
            Span::raw(" "), // 1 space before indicator
            Span::styled(
                indicator.to_string(),
                app.ui.theme.streaming_indicator_style,
            ),
            Span::raw("  "), // 2 spaces after indicator for a bit more padding
        ])
    };

    // Prepare optional bottom-left status message, shortened and left-aligned
    let status_bottom: Option<Line> = if let Some(status) = &app.ui.status {
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
                app.ui.theme.error_text_style
            } else {
                app.ui.theme.system_text_style
            };
            // Build a brief highlight effect: flash brighter then dim (same timing as success)
            let style = if let Some(set_at) = app.ui.status_set_at {
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

            // Compose a visual border with left-aligned status text and right filler
            // Account for one leading and one trailing space
            let status_len = text_raw.chars().count() + 2;
            let dash_count = inner_width.saturating_sub(status_len);
            let right_border = if dash_count > 0 {
                "─".repeat(dash_count)
            } else {
                String::new()
            };
            let line = Line::from(vec![
                Span::styled(" ", app.ui.theme.input_border_style),
                Span::styled(text_raw.clone(), style),
                Span::styled(" ", app.ui.theme.input_border_style),
                Span::styled(right_border, app.ui.theme.input_border_style),
            ]);
            Some(line)
        }
    } else {
        None
    };

    // Render border/title and the textarea inside
    let mut input_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(app.ui.theme.background_color))
        .border_style(app.ui.theme.input_border_style)
        .title(input_title);
    if let Some(bottom) = status_bottom {
        input_block = input_block.title_bottom(bottom);
    }

    let area = chunks[1];
    let inner = input_block.inner(area);
    f.render_widget(input_block, area);

    // Reserve a column for the focus indicator and render it
    let mut text_area = inner;
    let mut consumed_columns = 0;
    if inner.width > 0 {
        let indicator = if app.ui.is_input_focused() {
            "›"
        } else {
            "·"
        };
        let indicator_style = app
            .ui
            .theme
            .system_text_style
            .patch(Style::default().bg(app.ui.theme.background_color));
        let indicator_line = Line::from(Span::styled(indicator.to_string(), indicator_style));
        let indicator_paragraph = Paragraph::new(indicator_line)
            .style(Style::default().bg(app.ui.theme.background_color));
        let indicator_area = Rect {
            x: inner.x,
            y: inner.y,
            width: 1,
            height: inner.height,
        };
        f.render_widget(indicator_paragraph, indicator_area);
        consumed_columns += 1;

        if inner.width > 1 {
            let spacer_line = Line::from(Span::styled(" ".to_string(), indicator_style));
            let spacer_paragraph = Paragraph::new(spacer_line)
                .style(Style::default().bg(app.ui.theme.background_color));
            let spacer_area = Rect {
                x: inner.x.saturating_add(1),
                y: inner.y,
                width: 1,
                height: inner.height,
            };
            f.render_widget(spacer_paragraph, spacer_area);
            consumed_columns += 1;
        }
    }

    if consumed_columns > 0 {
        text_area.x = text_area.x.saturating_add(consumed_columns);
        text_area.width = text_area.width.saturating_sub(consumed_columns);
    }

    // Render wrapped input text with a one-column right margin
    // Wrap one character earlier to avoid cursor touching the border
    let available_width = text_area.width.saturating_sub(1);
    if available_width > 0 && text_area.height > 0 {
        let config = WrapConfig::new(available_width as usize);
        let wrapped_text = TextWrapper::wrap_text(app.ui.get_input_text(), &config);
        let paragraph = Paragraph::new(wrapped_text)
            .style(
                app.ui
                    .theme
                    .input_text_style
                    .patch(Style::default().bg(app.ui.theme.background_color)),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.ui.input_scroll_offset, 0));
        f.render_widget(paragraph, text_area);

        // Set cursor based on wrapped text and linear cursor position
        // Suppress cursor when picker is open (like Ctrl+B/Ctrl+P modes)
        if app.ui.is_input_active() && app.ui.is_input_focused() && app.picker_session().is_none() {
            let (line, col) = TextWrapper::calculate_cursor_position_in_wrapped_text(
                app.ui.get_input_text(),
                app.ui.get_input_cursor_position(),
                &config,
            );
            let visible_line = (line as u16).saturating_sub(app.ui.input_scroll_offset);
            if visible_line < text_area.height {
                let cursor_x = text_area.x.saturating_add(col as u16);
                let cursor_y = text_area.y.saturating_add(visible_line);
                f.set_cursor_position((cursor_x, cursor_y));
            }
        }
    } else if text_area.width > 0 && text_area.height > 0 {
        let blank = Paragraph::new("")
            .style(Style::default().bg(app.ui.theme.background_color))
            .wrap(Wrap { trim: false });
        f.render_widget(blank, text_area);
    }

    // Render modal picker or inspect overlay if present
    if app.picker_inspect_state().is_some() {
        let theme = app.ui.theme.clone();
        let inspect_title = app
            .picker_inspect_state()
            .map(|state| state.title.clone())
            .unwrap_or_else(|| "Inspect".to_string());
        let area = centered_rect(80, 80, f.area());

        f.render_widget(Clear, area);
        let modal_bg = Block::default().style(Style::default().bg(theme.background_color));
        f.render_widget(modal_bg, area);

        let modal_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.input_border_style)
            .title(Span::styled(inspect_title, theme.title_style));
        let content_area = modal_block.inner(area);
        f.render_widget(modal_block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(2), Constraint::Length(2)])
            .split(content_area);

        let body_area = inset_rect(chunks[0], 1, 1);
        let help_area = chunks[1];

        if let Some(inspect_state) = app.picker_inspect_state_mut() {
            let wrap_width = body_area.width as usize;
            let wrap_config = WrapConfig::new(wrap_width);
            let wrapped_text = TextWrapper::wrap_text(&inspect_state.content, &wrap_config);
            let total_lines =
                TextWrapper::count_wrapped_lines(&inspect_state.content, &wrap_config);
            let visible_height = body_area.height.max(1) as usize;
            let max_scroll = total_lines.saturating_sub(visible_height);
            let max_scroll_u16 = max_scroll.min(u16::MAX as usize) as u16;
            if inspect_state.scroll_offset > max_scroll_u16 {
                inspect_state.scroll_offset = max_scroll_u16;
            }

            let paragraph = Paragraph::new(wrapped_text)
                .style(
                    theme
                        .assistant_text_style
                        .patch(Style::default().bg(theme.background_color)),
                )
                .wrap(Wrap { trim: false })
                .scroll((inspect_state.scroll_offset, 0));
            f.render_widget(paragraph, body_area);
        }

        let help_lines = vec![
            Line::from(Span::styled(
                "Esc=Back to picker • ↑/↓=Scroll • PgUp/PgDn=Faster",
                theme.system_text_style,
            )),
            Line::from(Span::styled("Home/End=Jump", theme.system_text_style)),
        ];
        let help = Paragraph::new(help_lines).style(theme.system_text_style);
        f.render_widget(help, help_area);
    } else if let Some(picker) = app.picker_state() {
        let area = centered_rect(60, 60, f.area());

        // Clear any content under the modal
        f.render_widget(Clear, area);
        // Paint modal background consistent with theme
        let modal_bg = Block::default().style(Style::default().bg(app.ui.theme.background_color));
        f.render_widget(modal_bg, area);

        // Outer bordered block with title
        let modal_block = Block::default()
            .borders(Borders::ALL)
            .border_style(app.ui.theme.input_border_style)
            .title(Span::styled(&picker.title, app.ui.theme.title_style));
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
                    app.ui.theme.assistant_text_style,
                )))
            })
            .collect();

        let list = List::new(items)
            .style(Style::default().bg(app.ui.theme.background_color))
            .highlight_style(
                app.ui
                    .theme
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
            app.ui
                .theme
                .system_text_style
                .add_modifier(ratatui::style::Modifier::DIM),
        ));
        f.render_widget(metadata, metadata_area);

        // Generate help text for picker
        let help_text = generate_picker_help_text(app);
        let help = Paragraph::new(help_text.as_str()).style(app.ui.theme.system_text_style);
        f.render_widget(help, help_area);
    }
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
        Some(crate::core::app::PickerMode::Character) => app
            .character_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        Some(crate::core::app::PickerMode::Persona) => app
            .persona_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        Some(crate::core::app::PickerMode::Preset) => app
            .preset_picker_state()
            .map(|state| state.search_filter.as_str())
            .unwrap_or(""),
        _ => "",
    };

    let inspect_help = " • Ctrl+O=Inspect";
    let first_line = if search_filter.is_empty() {
        format!(
            "↑/↓=Navigate • F6=Sort • Type=Filter{}{}",
            inspect_help, del_help
        )
    } else {
        format!(
            "↑/↓=Navigate • Backspace=Clear • F6=Sort{}{}",
            inspect_help, del_help
        )
    };

    // Suppress persistent save option during env-only startup model selection
    let show_persist = !(app.session.startup_env_only
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
        App, CharacterPickerState, ModelPickerState, PickerData, PickerSession,
        ProviderPickerState, ThemePickerState,
    };
    use crate::ui::picker::PickerState;
    use crate::ui::theme::Theme;

    fn create_test_app() -> App {
        App::new_test_app(Theme::dark_default(), true, false)
    }

    fn set_model_picker(
        app: &mut App,
        search_filter: &str,
        items: Vec<crate::ui::picker::PickerItem>,
        selected: usize,
        has_dates: bool,
    ) {
        let picker_state = PickerState::new("Test".to_string(), items.clone(), selected);
        app.picker.picker_session = Some(PickerSession {
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
        app.picker.picker_session = Some(PickerSession {
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
        app.picker.picker_session = Some(PickerSession {
            state: picker_state,
            data: PickerData::Provider(ProviderPickerState {
                search_filter: search_filter.to_string(),
                all_items: items,
                before_provider: None,
            }),
        });
    }

    fn set_character_picker(
        app: &mut App,
        search_filter: &str,
        items: Vec<crate::ui::picker::PickerItem>,
        selected: usize,
    ) {
        let picker_state = PickerState::new("Test".to_string(), items.clone(), selected);
        app.picker.picker_session = Some(PickerSession {
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: search_filter.to_string(),
                all_items: items,
            }),
        });
    }

    fn find_segment<'a>(title: &'a str, prefix: &str) -> Option<&'a str> {
        title
            .split(" • ")
            .find(|segment| segment.starts_with(prefix))
    }

    fn find_title_with<F>(app: &App, mut width: u16, predicate: F) -> Option<(u16, String)>
    where
        F: Fn(&str) -> bool,
    {
        while width > 0 {
            width -= 1;
            let title = build_main_title(app, width);
            if predicate(&title) {
                return Some((width, title));
            }
        }
        None
    }

    #[test]
    fn title_shows_no_model_selected_during_transition() {
        let mut app = create_test_app();
        app.session.provider_display_name = "Cerebras".to_string();
        app.session.model = "foo-model".to_string();
        app.picker.in_provider_model_transition = true;

        let title = build_main_title(&app, 1000);
        assert!(title.contains("(no model selected)"));
        assert!(!title.contains("foo-model"));
        assert!(!title.contains("Preset:"));
    }

    #[test]
    fn title_shows_model_when_not_in_transition() {
        let mut app = create_test_app();
        app.session.provider_display_name = "Cerebras".to_string();
        app.session.model = "foo-model".to_string();
        app.picker.in_provider_model_transition = false;

        let title = build_main_title(&app, 1000);
        assert!(title.contains("foo-model"));
        assert!(!title.contains("(no model selected)"));
        assert!(!title.contains("Preset:"));
    }

    #[test]
    fn test_generate_picker_help_text_model_no_filter_no_default() {
        let mut app = create_test_app();
        // Create picker with non-default item
        let items = vec![crate::ui::picker::PickerItem {
            id: "test-model".to_string(),
            label: "Test Model".to_string(),
            metadata: None,
            inspect_metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter • Ctrl+O=Inspect"));
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
            inspect_metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "gpt", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • Backspace=Clear • F6=Sort • Ctrl+O=Inspect"));
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
            inspect_metadata: None,
            sort_key: None,
        }];
        set_provider_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
        assert!(help_text.contains(
            "↑/↓=Navigate • F6=Sort • Type=Filter • Ctrl+O=Inspect • Del=Remove default"
        ));
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
            inspect_metadata: None,
            sort_key: None,
        }];
        set_model_picker(&mut app, "", items, 0, false);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
        assert!(help_text.contains(
            "↑/↓=Navigate • F6=Sort • Type=Filter • Ctrl+O=Inspect • Del=Remove default"
        ));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
    }

    #[test]
    fn test_generate_picker_help_text_theme_picker() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "dark".to_string(),
            label: "Dark Theme".to_string(),
            metadata: None,
            inspect_metadata: None,
            sort_key: None,
        }];
        set_theme_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter • Ctrl+O=Inspect"));
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
            inspect_metadata: None,
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

    #[test]
    fn test_generate_picker_help_text_character_picker() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "alice".to_string(),
            label: "Alice".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            inspect_metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];
        set_character_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • F6=Sort • Type=Filter • Ctrl+O=Inspect"));
        assert!(help_text.contains("Enter=This session • Alt+Enter=As default"));
        assert!(!help_text.contains("Del=Remove default"));
    }

    #[test]
    fn test_generate_picker_help_text_character_with_filter() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "alice".to_string(),
            label: "Alice".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            inspect_metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];
        set_character_picker(&mut app, "ali", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("↑/↓=Navigate • Backspace=Clear • F6=Sort • Ctrl+O=Inspect"));
        assert!(!help_text.contains("Type=Filter"));
    }

    #[test]
    fn test_generate_picker_help_text_character_with_default_selected() {
        let mut app = create_test_app();
        let items = vec![crate::ui::picker::PickerItem {
            id: "alice".to_string(),
            label: "Alice*".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            inspect_metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];
        set_character_picker(&mut app, "", items, 0);

        let help_text = generate_picker_help_text(&app);

        assert!(help_text.contains("Del=Remove default"));
    }

    #[test]
    fn title_shows_character_name_when_active() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();
        app.session.provider_display_name = "OpenAI".to_string();
        app.session.model = "gpt-4".to_string();

        // Set an active character
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alice".to_string(),
                description: "A helpful assistant".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Helping users".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };
        app.session.active_character = Some(card);

        let title = build_main_title(&app, 1000);
        assert!(title.contains("Character: Alice"));
        assert!(title.contains("OpenAI"));
        assert!(title.contains("gpt-4"));
        assert!(!title.contains("Preset:"));
    }

    #[test]
    fn title_does_not_show_character_when_none() {
        let mut app = create_test_app();
        app.session.provider_display_name = "OpenAI".to_string();
        app.session.model = "gpt-4".to_string();
        app.session.active_character = None;

        let title = build_main_title(&app, 1000);
        assert!(!title.contains("Character:"));
        assert!(title.contains("OpenAI"));
        assert!(title.contains("gpt-4"));
        assert!(!title.contains("Preset:"));
    }

    #[test]
    fn title_shows_active_preset_when_set() {
        let mut app = create_test_app();
        app.session.provider_display_name = "OpenAI".to_string();
        app.session.model = "gpt-4".to_string();
        app.preset_manager
            .set_active_preset("short")
            .expect("preset to activate");

        let title = build_main_title(&app, 1000);
        assert!(title.contains("Preset: short"));
        assert!(title.contains("(gpt-4) • Preset: short"));
    }

    #[test]
    fn title_places_preset_after_character_when_active() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();
        app.session.provider_display_name = "OpenAI".to_string();
        app.session.model = "gpt-4".to_string();
        app.preset_manager
            .set_active_preset("short")
            .expect("preset to activate");

        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alice".to_string(),
                description: "A helpful assistant".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Helping users".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };
        app.session.active_character = Some(card);

        let title = build_main_title(&app, 1000);
        assert!(title.contains("Character: Alice • Preset: short"));
    }

    #[test]
    fn title_abbreviates_and_hides_fields_based_on_width() {
        use crate::character::card::{CharacterCard, CharacterData};

        let mut app = create_test_app();
        app.session.provider_display_name = "OpenAI".to_string();
        app.session.model = "gpt-4".to_string();
        app.preset_manager
            .set_active_preset("roleplay")
            .expect("preset to activate");

        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Jean-Luc Picard".to_string(),
                description: "A starship captain".to_string(),
                personality: "Decisive".to_string(),
                scenario: "Commanding the Enterprise".to_string(),
                first_mes: "Make it so.".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };
        app.session.active_character = Some(card);

        let wide_title = build_main_title(&app, 1000);
        assert!(matches!(
            find_segment(&wide_title, "Character: "),
            Some(segment) if segment.ends_with("Jean-Luc Picard")
        ));
        assert!(matches!(
            find_segment(&wide_title, "Preset: "),
            Some(segment) if segment.ends_with("roleplay")
        ));

        let (char_width, char_abbrev_title) = find_title_with(&app, 1000, |title| {
            matches!(
                find_segment(title, "Character: "),
                Some(segment) if segment.contains('…') && !segment.ends_with("Picard")
            )
        })
        .expect("character should abbreviate before hiding");
        assert_eq!(
            find_segment(&char_abbrev_title, "Preset: "),
            Some("Preset: roleplay")
        );

        let (preset_width, preset_abbrev_title) = find_title_with(&app, char_width, |title| {
            matches!(
                find_segment(title, "Preset: "),
                Some(segment) if segment.contains('…') && !segment.ends_with("roleplay")
            )
        })
        .expect("preset should abbreviate after character");
        assert!(find_segment(&preset_abbrev_title, "Character: ").is_some());

        let (char_hidden_width, char_hidden_title) = find_title_with(&app, preset_width, |title| {
            find_segment(title, "Character: ").is_none()
        })
        .expect("character should hide before preset");
        assert!(find_segment(&char_hidden_title, "Preset: ").is_some());

        let (_, preset_hidden_title) = find_title_with(&app, char_hidden_width, |title| {
            find_segment(title, "Preset: ").is_none()
        })
        .expect("preset should hide after character");
        assert!(find_segment(&preset_hidden_title, "Character: ").is_none());
        assert!(find_segment(&preset_hidden_title, "Preset: ").is_none());
        assert!(find_segment(&preset_hidden_title, "Logging: ").is_some());
    }
}
