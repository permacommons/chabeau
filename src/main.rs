mod api;
mod app;
mod commands;
mod logging;
mod message;
mod ui;

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::{
    error::Error,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};

use api::{ChatRequest, ChatResponse};
use app::App;
use commands::{process_input, CommandResult};
use ui::ui;

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = "A terminal-based chat interface using OpenAI API")]
#[command(long_about = "Chabeau is a full-screen terminal chat interface that connects to OpenAI's API \
for real-time conversations. It supports streaming responses and provides a clean, \
responsive interface with color-coded messages.\n\n\
Environment Variables:\n\
  OPENAI_API_KEY    Your OpenAI API key (required)\n\
  OPENAI_BASE_URL   Custom API base URL (optional, defaults to https://api.openai.com/v1)\n\n\
Controls:\n\
  Type              Enter your message in the input field\n\
  Enter             Send the message\n\
  Up/Down/Mouse     Scroll through chat history\n\
  Ctrl+C            Quit the application\n\
  Ctrl+R            Retry the last bot response\n\
  Backspace         Delete characters in the input field\n\n\
Commands:\n\
  /log <filename>   Enable logging to specified file\n\
  /log              Toggle logging pause/resume")]
struct Args {
    #[arg(short, long, default_value = "gpt-4o", help = "OpenAI model to use for chat")]
    model: String,

    #[arg(long, help = "Enable logging to specified file")]
    log: Option<String>,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // Create app first (this checks environment variables)
    let app = Arc::new(Mutex::new(match App::new(args.model, args.log) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }));

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel for streaming updates
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Special message to signal end of streaming
    const STREAM_END_MARKER: &str = "<<STREAM_END>>";

    // Main loop
    let result = loop {
        {
            let app_guard = app.lock().await;
            terminal.draw(|f| ui(f, &app_guard))?;
        }

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                            break Ok(());
                        }
                        KeyCode::Char('r') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                            // Retry the last bot response
                            let (should_retry, api_messages, client, model, api_key, base_url) = {
                                let mut app_guard = app.lock().await;

                                // If currently streaming, interrupt it first
                                if app_guard.is_streaming {
                                    app_guard.is_streaming = false;
                                    app_guard.stream_interrupted = true;
                                }

                                let terminal_height = terminal.size().unwrap_or_default().height;
                                let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title

                                if let Some(api_messages) = app_guard.prepare_retry(available_height) {
                                    // Set streaming state and reset pulse timer
                                    app_guard.is_streaming = true;
                                    app_guard.pulse_start = Instant::now();
                                    app_guard.stream_interrupted = false;

                                    (
                                        true,
                                        api_messages,
                                        app_guard.client.clone(),
                                        app_guard.model.clone(),
                                        app_guard.api_key.clone(),
                                        app_guard.base_url.clone(),
                                    )
                                } else {
                                    (false, Vec::new(), app_guard.client.clone(), String::new(), String::new(), String::new())
                                }
                            };

                            if !should_retry {
                                continue;
                            }

                            // Spawn the same API request logic as for Enter key
                            let tx_clone = tx.clone();
                            let app_clone = app.clone();
                            tokio::spawn(async move {
                                let request = ChatRequest {
                                    model,
                                    messages: api_messages,
                                    stream: true,
                                };

                                match client
                                    .post(&format!("{}/chat/completions", base_url))
                                    .header("Authorization", format!("Bearer {}", api_key))
                                    .header("Content-Type", "application/json")
                                    .json(&request)
                                    .send()
                                    .await
                                {
                                    Ok(response) => {
                                        if !response.status().is_success() {
                                            if let Ok(error_text) = response.text().await {
                                                eprintln!("API request failed: {}", error_text);
                                            }
                                            return;
                                        }

                                        let mut stream = response.bytes_stream();
                                        let mut buffer = String::new();

                                        while let Some(chunk) = stream.next().await {
                                            // Check if stream was interrupted
                                            {
                                                let app_guard = app_clone.lock().await;
                                                if app_guard.stream_interrupted || !app_guard.is_streaming {
                                                    // Stream was interrupted, stop processing
                                                    let _ = tx_clone.send(STREAM_END_MARKER.to_string());
                                                    return;
                                                }
                                            }

                                            if let Ok(chunk) = chunk {
                                                let chunk_str = String::from_utf8_lossy(&chunk);
                                                buffer.push_str(&chunk_str);

                                                // Process complete lines from buffer
                                                while let Some(newline_pos) = buffer.find('\n') {
                                                    let line = buffer[..newline_pos].trim().to_string();
                                                    buffer.drain(..=newline_pos);

                                                    if line.starts_with("data: ") {
                                                        let data = &line[6..];
                                                        if data == "[DONE]" {
                                                            // Signal end of streaming
                                                            let _ = tx_clone.send(STREAM_END_MARKER.to_string());
                                                            return;
                                                        }

                                                        match serde_json::from_str::<ChatResponse>(data) {
                                                            Ok(response) => {
                                                                if let Some(choice) = response.choices.first() {
                                                                    if let Some(content) = &choice.delta.content {
                                                                        let _ = tx_clone.send(content.clone());
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                eprintln!("Failed to parse JSON: {} - Data: {}", e, data);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error sending message: {}", e);
                                    }
                                }
                            });
                        }
                        KeyCode::Esc => {
                            let mut app_guard = app.lock().await;
                            if app_guard.is_streaming {
                                // Interrupt the stream
                                app_guard.is_streaming = false;
                                app_guard.stream_interrupted = true;
                            }
                        }
                        KeyCode::Enter => {
                            let (should_send_to_api, api_messages, client, model, api_key, base_url) = {
                                let mut app_guard = app.lock().await;
                                if app_guard.input.trim().is_empty() {
                                    continue;
                                }

                                // If currently streaming, interrupt it
                                if app_guard.is_streaming {
                                    app_guard.is_streaming = false;
                                    app_guard.stream_interrupted = true;
                                }

                                let input_text = app_guard.input.clone();
                                app_guard.input.clear();

                                // Process input for commands
                                match process_input(&mut app_guard, &input_text) {
                                    CommandResult::Continue => {
                                        // Command was processed, don't send to API
                                        continue;
                                    }
                                    CommandResult::ProcessAsMessage(message) => {
                                        // Re-enable auto-scroll when user sends a new message
                                        app_guard.auto_scroll = true;
                                        // Set streaming state and reset pulse timer
                                        app_guard.is_streaming = true;
                                        app_guard.pulse_start = Instant::now();
                                        app_guard.stream_interrupted = false;
                                        let api_messages = app_guard.add_user_message(message);
                                        (
                                            true,
                                            api_messages,
                                            app_guard.client.clone(),
                                            app_guard.model.clone(),
                                            app_guard.api_key.clone(),
                                            app_guard.base_url.clone(),
                                        )
                                    }
                                }
                            };

                            if !should_send_to_api {
                                continue;
                            }

                            let tx_clone = tx.clone();
                            let app_clone = app.clone();
                            tokio::spawn(async move {
                                let request = ChatRequest {
                                    model,
                                    messages: api_messages,
                                    stream: true,
                                };

                                match client
                                    .post(&format!("{}/chat/completions", base_url))
                                    .header("Authorization", format!("Bearer {}", api_key))
                                    .header("Content-Type", "application/json")
                                    .json(&request)
                                    .send()
                                    .await
                                {
                                    Ok(response) => {
                                        if !response.status().is_success() {
                                            if let Ok(error_text) = response.text().await {
                                                eprintln!("API request failed: {}", error_text);
                                            }
                                            return;
                                        }

                                        let mut stream = response.bytes_stream();
                                        let mut buffer = String::new();

                                        while let Some(chunk) = stream.next().await {
                                            // Check if stream was interrupted
                                            {
                                                let app_guard = app_clone.lock().await;
                                                if app_guard.stream_interrupted || !app_guard.is_streaming {
                                                    // Stream was interrupted, stop processing
                                                    let _ = tx_clone.send(STREAM_END_MARKER.to_string());
                                                    return;
                                                }
                                            }

                                            if let Ok(chunk) = chunk {
                                                let chunk_str = String::from_utf8_lossy(&chunk);
                                                buffer.push_str(&chunk_str);

                                                // Process complete lines from buffer
                                                while let Some(newline_pos) = buffer.find('\n') {
                                                    let line = buffer[..newline_pos].trim().to_string();
                                                    buffer.drain(..=newline_pos);

                                                    if line.starts_with("data: ") {
                                                        let data = &line[6..];
                                                        if data == "[DONE]" {
                                                            // Signal end of streaming
                                                            let _ = tx_clone.send(STREAM_END_MARKER.to_string());
                                                            return;
                                                        }

                                                        match serde_json::from_str::<ChatResponse>(data) {
                                                            Ok(response) => {
                                                                if let Some(choice) = response.choices.first() {
                                                                    if let Some(content) = &choice.delta.content {
                                                                        let _ = tx_clone.send(content.clone());
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                eprintln!("Failed to parse JSON: {} - Data: {}", e, data);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error sending message: {}", e);
                                    }
                                }
                            });
                        }
                        KeyCode::Char(c) => {
                            let mut app_guard = app.lock().await;
                            app_guard.input.push(c);
                        }
                        KeyCode::Backspace => {
                            let mut app_guard = app.lock().await;
                            app_guard.input.pop();
                        }
                        KeyCode::Up => {
                            let mut app_guard = app.lock().await;
                            // Disable auto-scroll when user manually scrolls
                            app_guard.auto_scroll = false;
                            app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            let mut app_guard = app.lock().await;
                            // Disable auto-scroll when user manually scrolls
                            app_guard.auto_scroll = false;
                            let terminal_height = terminal.size().unwrap_or_default().height;
                            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height);
                            app_guard.scroll_offset = (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            let mut app_guard = app.lock().await;
                            // Disable auto-scroll when user manually scrolls
                            app_guard.auto_scroll = false;
                            app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(3);
                        }
                        MouseEventKind::ScrollDown => {
                            let mut app_guard = app.lock().await;
                            // Disable auto-scroll when user manually scrolls
                            app_guard.auto_scroll = false;
                            let terminal_height = terminal.size().unwrap_or_default().height;
                            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height);
                            app_guard.scroll_offset = (app_guard.scroll_offset.saturating_add(3)).min(max_scroll);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Handle streaming updates - drain all available messages
        let mut received_any = false;
        while let Ok(content) = rx.try_recv() {
            if content == STREAM_END_MARKER {
                // End of streaming - clear the streaming state and finalize response
                let mut app_guard = app.lock().await;
                app_guard.finalize_response();
                app_guard.is_streaming = false;
                drop(app_guard);
                received_any = true;
            } else {
                let mut app_guard = app.lock().await;
                let terminal_height = terminal.size().unwrap_or_default().height;
                let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                app_guard.append_to_response(&content, available_height);
                drop(app_guard);
                received_any = true;
            }
        }
        if received_any {
            continue; // Force a redraw after processing all updates
        }
    };

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}
