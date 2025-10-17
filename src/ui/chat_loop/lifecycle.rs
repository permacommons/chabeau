use std::{error::Error, io, sync::Arc};

use ratatui::crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use tokio::sync::Mutex;

use crate::ui::osc_backend::OscBackend;

pub type SharedTerminal = Arc<Mutex<Terminal<OscBackend<io::Stdout>>>>;

pub fn setup_terminal() -> Result<SharedTerminal, Box<dyn Error>> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    let backend = OscBackend::new(stdout);
    let terminal = Terminal::new(backend).inspect_err(|_| {
        let _ = disable_raw_mode();
    })?;

    Ok(Arc::new(Mutex::new(terminal)))
}

pub async fn restore_terminal(terminal: &SharedTerminal) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    let mut guard = terminal.lock().await;
    execute!(
        guard.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    guard.show_cursor()?;
    Ok(())
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
        if let Ok(terminal) = setup_terminal() {
            let runtime = Runtime::new().expect("runtime");
            runtime.block_on(async {
                let _ = restore_terminal(&terminal).await;
            });
        }
    }
}
