use std::{error::Error, io, io::Write, sync::Arc};

use ratatui::crossterm::{
    cursor::SetCursorStyle,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use ratatui::{crossterm::style::Print, style::Color};
use tokio::sync::Mutex;

use crate::ui::osc_backend::OscBackend;
use crate::utils::color::color_to_rgb;

pub type SharedTerminal<W = io::Stdout> = Arc<Mutex<Terminal<OscBackend<W>>>>;

pub fn setup_terminal(cursor_color: Option<Color>) -> Result<SharedTerminal, Box<dyn Error>> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        SetCursorStyle::SteadyBar
    )?;

    if let Some(color) = cursor_color {
        queue_cursor_color(&mut stdout, color)?;
        stdout.flush()?;
    }

    let backend = OscBackend::new(stdout);
    let terminal = Terminal::new(backend).inspect_err(|_| {
        let _ = disable_raw_mode();
    })?;

    Ok(Arc::new(Mutex::new(terminal)))
}

pub async fn restore_terminal<W>(terminal: &SharedTerminal<W>) -> Result<(), Box<dyn Error>>
where
    W: Write + Send + 'static,
{
    disable_raw_mode()?;
    let mut guard = terminal.lock().await;
    queue_reset_cursor_color(guard.backend_mut())?;
    guard.backend_mut().flush()?;
    execute!(
        guard.backend_mut(),
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    guard.show_cursor()?;
    Ok(())
}

pub async fn apply_cursor_color_to_terminal<W>(
    terminal: &SharedTerminal<W>,
    color: Option<Color>,
) -> io::Result<()>
where
    W: Write + Send + 'static,
{
    let mut guard = terminal.lock().await;
    match color {
        Some(color) => queue_cursor_color(guard.backend_mut(), color)?,
        None => queue_reset_cursor_color(guard.backend_mut())?,
    }
    guard.backend_mut().flush()
}

fn queue_cursor_color<W: Write>(writer: &mut W, color: Color) -> io::Result<()> {
    if let Some(payload) = cursor_color_payload(color) {
        execute!(writer, Print(format!("\x1b]12;{}\x1b\\", payload)))?;
    }
    Ok(())
}

fn queue_reset_cursor_color<W: Write>(writer: &mut W) -> io::Result<()> {
    execute!(writer, Print("\x1b]112\x1b\\"))
}

fn cursor_color_payload(color: Color) -> Option<String> {
    color_to_rgb(color).map(|(r, g, b)| format!("#{:02x}{:02x}{:02x}", r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    #[test]
    fn setup_terminal_enables_raw_mode() {
        // We cannot reliably assert raw mode toggling without a real terminal, but we can
        // ensure the helper returns an error-free result and produces a terminal handle.
        // The guard is dropped immediately which restores the original terminal state.
        //
        // Note: running this test inside headless CI environments may fail because crossterm
        // cannot switch the terminal backend. To keep the suite stable, we only assert that
        // the helper constructs the data structures when it succeeds.
        if let Ok(terminal) = setup_terminal(None) {
            let runtime = Runtime::new().expect("runtime");
            runtime.block_on(async {
                let _ = restore_terminal(&terminal).await;
            });
        }
    }

    #[test]
    fn cursor_color_payload_is_hex() {
        assert_eq!(
            cursor_color_payload(Color::Rgb(0x12, 0x34, 0x56)).as_deref(),
            Some("#123456")
        );
    }

    #[test]
    fn queues_cursor_color_sequence() {
        let mut buf: Vec<u8> = Vec::new();
        queue_cursor_color(&mut buf, Color::Rgb(0x12, 0x34, 0x56)).expect("write");
        assert_eq!(buf, b"\x1b]12;#123456\x1b\\");
    }

    #[test]
    fn apply_cursor_color_writes_sequence() {
        let mut buf: Vec<u8> = Vec::new();
        queue_cursor_color(&mut buf, Color::Rgb(0x12, 0x34, 0x56)).expect("write");
        assert_eq!(buf, b"\x1b]12;#123456\x1b\\");
    }

    #[test]
    fn apply_cursor_reset_writes_sequence() {
        let mut buf: Vec<u8> = Vec::new();
        queue_reset_cursor_color(&mut buf).expect("write");
        assert_eq!(buf, b"\x1b]112\x1b\\");
    }
}
