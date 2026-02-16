//! Shared single-line terminal editor for interactive CLI prompts.

use crate::utils::input::sanitize_text_input;
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::fmt;
use std::io::{self, Write};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEditorState {
    pub text: String,
    pub cursor: usize,
    pub reveal_mask_tail: bool,
}

impl LineEditorState {
    pub fn with_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            reveal_mask_tail: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskMode {
    None,
    Hidden,
    RevealTail { tail_chars: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEditorOptions {
    pub initial_text: String,
    pub allow_cancel: bool,
    pub mask_mode: MaskMode,
}

impl Default for LineEditorOptions {
    fn default() -> Self {
        Self {
            initial_text: String::new(),
            allow_cancel: true,
            mask_mode: MaskMode::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEditAction {
    Insert(char),
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveStart,
    MoveEnd,
    DeleteToEnd,
    DeleteWord,
    ClearAll,
    ToggleMaskReveal,
    Paste(String),
    Submit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEditOutcome {
    Continue { redraw: bool },
    Submit(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct LineEditorError {
    message: String,
}

impl LineEditorError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LineEditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LineEditorError {}

pub fn prompt_line_editor(
    prompt: &str,
    options: &LineEditorOptions,
) -> Result<String, LineEditorError> {
    enable_raw_mode().map_err(|err| LineEditorError::new(err.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, event::EnableBracketedPaste)
        .map_err(|err| LineEditorError::new(err.to_string()))?;

    let result = (|| -> Result<String, LineEditorError> {
        let mut state = LineEditorState::with_text(options.initial_text.clone());
        let mut needs_redraw = true;

        loop {
            if needs_redraw {
                redraw_line(prompt, &state, options)
                    .map_err(|err| LineEditorError::new(err.to_string()))?;
                needs_redraw = false;
            }

            if event::poll(Duration::from_millis(100))
                .map_err(|err| LineEditorError::new(err.to_string()))?
            {
                match event::read().map_err(|err| LineEditorError::new(err.to_string()))? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if let Some(action) = map_key_event_to_action(&key, options) {
                            match apply_line_edit_action(&mut state, action, options) {
                                LineEditOutcome::Continue { redraw } => needs_redraw = redraw,
                                LineEditOutcome::Submit(value) => break Ok(value),
                                LineEditOutcome::Cancelled => {
                                    break Err(LineEditorError::new("Cancelled by user"));
                                }
                            }
                        }
                    }
                    Event::Paste(text) => {
                        let sanitized = sanitize_text_input(&text);
                        match apply_line_edit_action(
                            &mut state,
                            LineEditAction::Paste(sanitized),
                            options,
                        ) {
                            LineEditOutcome::Continue { redraw } => needs_redraw = redraw,
                            LineEditOutcome::Submit(value) => break Ok(value),
                            LineEditOutcome::Cancelled => {
                                break Err(LineEditorError::new("Cancelled by user"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    })();

    let disable_raw_result =
        disable_raw_mode().map_err(|err| LineEditorError::new(err.to_string()));
    let disable_paste_result = execute!(stdout, event::DisableBracketedPaste)
        .map_err(|err| LineEditorError::new(err.to_string()));
    println!();

    let mut final_result = result;
    if let Err(err) = disable_raw_result {
        if final_result.is_ok() {
            final_result = Err(err);
        }
    }
    if let Err(err) = disable_paste_result {
        if final_result.is_ok() {
            final_result = Err(err);
        }
    }
    final_result
}

fn redraw_line(
    prompt: &str,
    state: &LineEditorState,
    options: &LineEditorOptions,
) -> io::Result<()> {
    let display_text = display_text(state, options);
    let prefix = display_prefix_up_to_cursor(state, options);
    let prompt_width = UnicodeWidthStr::width(prompt);
    let prefix_width = UnicodeWidthStr::width(prefix.as_str());

    print!("\r\x1b[K{}{}", prompt, display_text);

    let cursor_columns = prompt_width + prefix_width;
    if cursor_columns > 0 {
        print!("\r\x1b[{}C", cursor_columns);
    } else {
        print!("\r");
    }

    io::stdout().flush()
}

fn display_text(state: &LineEditorState, options: &LineEditorOptions) -> String {
    match &options.mask_mode {
        MaskMode::None => state.text.clone(),
        MaskMode::Hidden => "*".repeat(state.text.chars().count()),
        MaskMode::RevealTail { tail_chars } => {
            let text_len = state.text.chars().count();
            if state.reveal_mask_tail && text_len >= *tail_chars {
                let visible_start = text_len - tail_chars;
                let visible_tail = state.text.chars().skip(visible_start).collect::<String>();
                format!("{}{}", "*".repeat(visible_start), visible_tail)
            } else {
                "*".repeat(text_len)
            }
        }
    }
}

fn display_prefix_up_to_cursor(state: &LineEditorState, options: &LineEditorOptions) -> String {
    let display = display_text(state, options);
    display.chars().take(state.cursor).collect()
}

pub fn map_key_event_to_action(
    key: &event::KeyEvent,
    options: &LineEditorOptions,
) -> Option<LineEditAction> {
    match key.code {
        KeyCode::Enter => Some(LineEditAction::Submit),
        KeyCode::Esc if options.allow_cancel => Some(LineEditAction::Cancel),
        KeyCode::Backspace => Some(LineEditAction::Backspace),
        KeyCode::Delete => Some(LineEditAction::Delete),
        KeyCode::Left => Some(LineEditAction::MoveLeft),
        KeyCode::Right => Some(LineEditAction::MoveRight),
        KeyCode::Home => Some(LineEditAction::MoveStart),
        KeyCode::End => Some(LineEditAction::MoveEnd),
        KeyCode::F(2) if matches!(options.mask_mode, MaskMode::RevealTail { .. }) => {
            Some(LineEditAction::ToggleMaskReveal)
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LineEditAction::MoveStart)
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LineEditAction::MoveEnd)
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LineEditAction::DeleteToEnd)
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LineEditAction::DeleteWord)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LineEditAction::ClearAll)
        }
        KeyCode::Char('c')
            if key.modifiers.contains(KeyModifiers::CONTROL) && options.allow_cancel =>
        {
            Some(LineEditAction::Cancel)
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if c == '\n' || c == '\r' {
                Some(LineEditAction::Submit)
            } else {
                Some(LineEditAction::Insert(c))
            }
        }
        _ => None,
    }
}

pub fn apply_line_edit_action(
    state: &mut LineEditorState,
    action: LineEditAction,
    options: &LineEditorOptions,
) -> LineEditOutcome {
    match action {
        LineEditAction::Insert(c) => {
            insert_char_at_cursor(&mut state.text, state.cursor, c);
            state.cursor += 1;
            state.reveal_mask_tail = false;
            LineEditOutcome::Continue { redraw: true }
        }
        LineEditAction::Backspace => {
            if state.cursor == 0 {
                LineEditOutcome::Continue { redraw: false }
            } else {
                let removed = remove_char_before_cursor(&mut state.text, state.cursor);
                if removed {
                    state.cursor = state.cursor.saturating_sub(1);
                    state.reveal_mask_tail = false;
                    LineEditOutcome::Continue { redraw: true }
                } else {
                    LineEditOutcome::Continue { redraw: false }
                }
            }
        }
        LineEditAction::Delete => {
            let removed = remove_char_at_cursor(&mut state.text, state.cursor);
            if removed {
                state.reveal_mask_tail = false;
                LineEditOutcome::Continue { redraw: true }
            } else {
                LineEditOutcome::Continue { redraw: false }
            }
        }
        LineEditAction::MoveLeft => {
            if state.cursor > 0 {
                state.cursor -= 1;
                LineEditOutcome::Continue { redraw: true }
            } else {
                LineEditOutcome::Continue { redraw: false }
            }
        }
        LineEditAction::MoveRight => {
            let len = state.text.chars().count();
            if state.cursor < len {
                state.cursor += 1;
                LineEditOutcome::Continue { redraw: true }
            } else {
                LineEditOutcome::Continue { redraw: false }
            }
        }
        LineEditAction::MoveStart => {
            if state.cursor == 0 {
                LineEditOutcome::Continue { redraw: false }
            } else {
                state.cursor = 0;
                LineEditOutcome::Continue { redraw: true }
            }
        }
        LineEditAction::MoveEnd => {
            let end = state.text.chars().count();
            if state.cursor == end {
                LineEditOutcome::Continue { redraw: false }
            } else {
                state.cursor = end;
                LineEditOutcome::Continue { redraw: true }
            }
        }
        LineEditAction::DeleteToEnd => {
            let byte_idx = char_to_byte_index(&state.text, state.cursor);
            if byte_idx >= state.text.len() {
                LineEditOutcome::Continue { redraw: false }
            } else {
                state.text.truncate(byte_idx);
                state.reveal_mask_tail = false;
                LineEditOutcome::Continue { redraw: true }
            }
        }
        LineEditAction::DeleteWord => {
            if state.cursor == 0 {
                LineEditOutcome::Continue { redraw: false }
            } else {
                let new_cursor = delete_word_before_cursor(&mut state.text, state.cursor);
                state.cursor = new_cursor;
                state.reveal_mask_tail = false;
                LineEditOutcome::Continue { redraw: true }
            }
        }
        LineEditAction::ClearAll => {
            if state.text.is_empty() {
                LineEditOutcome::Continue { redraw: false }
            } else {
                state.text.clear();
                state.cursor = 0;
                state.reveal_mask_tail = false;
                LineEditOutcome::Continue { redraw: true }
            }
        }
        LineEditAction::ToggleMaskReveal => {
            if matches!(options.mask_mode, MaskMode::RevealTail { .. }) {
                state.reveal_mask_tail = !state.reveal_mask_tail;
                LineEditOutcome::Continue { redraw: true }
            } else {
                LineEditOutcome::Continue { redraw: false }
            }
        }
        LineEditAction::Paste(text) => {
            let before_newline = text.split('\n').next().unwrap_or("");
            if !before_newline.is_empty() {
                insert_str_at_cursor(&mut state.text, state.cursor, before_newline);
                state.cursor += before_newline.chars().count();
                state.reveal_mask_tail = false;
            }
            if text.contains('\n') {
                LineEditOutcome::Submit(state.text.clone())
            } else {
                LineEditOutcome::Continue {
                    redraw: !before_newline.is_empty(),
                }
            }
        }
        LineEditAction::Submit => LineEditOutcome::Submit(state.text.clone()),
        LineEditAction::Cancel => LineEditOutcome::Cancelled,
    }
}

fn insert_char_at_cursor(input: &mut String, cursor: usize, c: char) {
    let byte_idx = char_to_byte_index(input, cursor);
    input.insert(byte_idx, c);
}

fn insert_str_at_cursor(input: &mut String, cursor: usize, text: &str) {
    let byte_idx = char_to_byte_index(input, cursor);
    input.insert_str(byte_idx, text);
}

fn remove_char_before_cursor(input: &mut String, cursor: usize) -> bool {
    if cursor == 0 {
        return false;
    }
    let end = char_to_byte_index(input, cursor);
    let start = char_to_byte_index(input, cursor - 1);
    input.replace_range(start..end, "");
    true
}

fn remove_char_at_cursor(input: &mut String, cursor: usize) -> bool {
    let start = char_to_byte_index(input, cursor);
    if start >= input.len() {
        return false;
    }
    let end = char_to_byte_index(input, cursor + 1);
    input.replace_range(start..end, "");
    true
}

fn delete_word_before_cursor(input: &mut String, cursor: usize) -> usize {
    let mut chars: Vec<char> = input.chars().collect();
    let mut idx = cursor.min(chars.len());
    while idx > 0 && chars[idx - 1] == ' ' {
        idx -= 1;
    }
    while idx > 0 && chars[idx - 1] != ' ' {
        idx -= 1;
    }
    chars.drain(idx..cursor.min(chars.len()));
    *input = chars.into_iter().collect();
    idx
}

fn char_to_byte_index(input: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    input
        .char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(input.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn default_options() -> LineEditorOptions {
        LineEditorOptions::default()
    }

    #[test]
    fn insert_and_move_cursor() {
        let mut state = LineEditorState::with_text(String::new());
        let options = default_options();
        assert_eq!(
            apply_line_edit_action(&mut state, LineEditAction::Insert('a'), &options),
            LineEditOutcome::Continue { redraw: true }
        );
        assert_eq!(state.text, "a");
        assert_eq!(state.cursor, 1);

        apply_line_edit_action(&mut state, LineEditAction::MoveLeft, &options);
        apply_line_edit_action(&mut state, LineEditAction::Insert('b'), &options);
        assert_eq!(state.text, "ba");
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn ctrl_k_deletes_to_end() {
        let mut state = LineEditorState::with_text("hello world".to_string());
        state.cursor = 6;
        let options = default_options();
        assert_eq!(
            apply_line_edit_action(&mut state, LineEditAction::DeleteToEnd, &options),
            LineEditOutcome::Continue { redraw: true }
        );
        assert_eq!(state.text, "hello ");
        assert_eq!(state.cursor, 6);
    }

    #[test]
    fn paste_newline_submits() {
        let mut state = LineEditorState::with_text(String::new());
        let options = default_options();
        let outcome = apply_line_edit_action(
            &mut state,
            LineEditAction::Paste("token\nextra".to_string()),
            &options,
        );
        assert_eq!(outcome, LineEditOutcome::Submit("token".to_string()));
    }

    #[test]
    fn ctrl_a_and_ctrl_e_map_to_start_end() {
        let options = default_options();
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let ctrl_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert_eq!(
            map_key_event_to_action(&ctrl_a, &options),
            Some(LineEditAction::MoveStart)
        );
        assert_eq!(
            map_key_event_to_action(&ctrl_e, &options),
            Some(LineEditAction::MoveEnd)
        );
    }

    #[test]
    fn f2_maps_to_toggle_only_with_reveal_tail_mask() {
        let plain = default_options();
        let masked = LineEditorOptions {
            initial_text: String::new(),
            allow_cancel: true,
            mask_mode: MaskMode::RevealTail { tail_chars: 4 },
        };
        let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        assert_eq!(map_key_event_to_action(&f2, &plain), None);
        assert_eq!(
            map_key_event_to_action(&f2, &masked),
            Some(LineEditAction::ToggleMaskReveal)
        );
    }

    #[test]
    fn masked_display_can_reveal_tail() {
        let mut state = LineEditorState::with_text("abcdefgh".to_string());
        let options = LineEditorOptions {
            initial_text: String::new(),
            allow_cancel: true,
            mask_mode: MaskMode::RevealTail { tail_chars: 4 },
        };
        assert_eq!(display_text(&state, &options), "********");
        state.reveal_mask_tail = true;
        assert_eq!(display_text(&state, &options), "****efgh");
    }
}
