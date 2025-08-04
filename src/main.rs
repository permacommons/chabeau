mod api;
mod app;
mod auth;
mod commands;
mod logging;
mod message;
mod ui;

use clap::{Parser, Subcommand};
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
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};

use api::{ChatRequest, ChatResponse};
use app::App;
use auth::AuthManager;
use commands::{process_input, CommandResult};
use ui::ui;

async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();

    println!("🔗 Available Providers");
    println!("━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // Check built-in providers
    let builtin_providers = vec![
        ("openai", "OpenAI", "https://api.openai.com/v1"),
        ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
        ("poe", "Poe", "https://api.poe.com/v1"),
    ];

    for (name, display_name, url) in builtin_providers {
        let status = match auth_manager.get_token(name) {
            Ok(Some(_)) => "✅ configured",
            Ok(None) => "❌ not configured",
            Err(_) => "❓ error checking",
        };
        println!("  {} ({}) - {}", display_name, name, status);
        println!("    URL: {}", url);
        println!();
    }

    // Check for custom providers
    match auth_manager.list_custom_providers() {
        Ok(custom_providers) => {
            if custom_providers.is_empty() {
                println!("Custom providers: none configured");
            } else {
                println!("Custom providers:");
                for (name, url, has_token) in custom_providers {
                    let status = if has_token { "✅ configured" } else { "❌ not configured" };
                    println!("  {} - {}", name, status);
                    println!("    URL: {}", url);
                }
            }
        }
        Err(_) => {
            println!("Custom providers: error checking");
        }
    }
    println!();

    // Show which provider would be used by default
    match auth_manager.find_first_available_auth() {
        Some((provider, _)) => {
            println!("🎯 Default provider: {} ({})", provider.display_name, provider.name);
        }
        None => {
            println!("⚠️  No configured providers found");
            println!();
            println!("To configure authentication:");
            println!("  chabeau auth                    # Interactive setup");
            println!();
            println!("Or use environment variables:");
            println!("  export OPENAI_API_KEY=sk-...   # For OpenAI");
        }
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = "A terminal-based chat interface using OpenAI API")]
#[command(long_about = "Chabeau is a full-screen terminal chat interface that connects to various AI APIs \
for real-time conversations. It supports streaming responses and provides a clean, \
responsive interface with color-coded messages.\n\n\
Authentication:\n\
  Use 'chabeau auth' to set up API credentials securely in your system keyring.\n\
  Supports OpenAI, OpenRouter, Poe, and custom providers.\n\n\
Environment Variables (fallback if no auth configured):\n\
  OPENAI_API_KEY    Your OpenAI API key\n\
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
    #[command(subcommand)]
    command: Option<Commands>,

    /// Model to use for chat
    #[arg(short = 'm', long, default_value = "gpt-4o", global = true)]
    model: String,

    /// Enable logging to specified file
    #[arg(short = 'l', long, global = true)]
    log: Option<String>,

    /// Provider to use (openai, openrouter, poe, or custom provider name)
    #[arg(short = 'p', long, global = true)]
    provider: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Set up authentication for API providers
    Auth,
    /// Remove authentication for API providers
    Deauth,
    /// Start the chat interface (default)
    Chat,
    /// List available providers and their authentication status
    Providers,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    match args.command.unwrap_or(Commands::Chat) {
        Commands::Auth => {
            let auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_auth() {
                eprintln!("❌ Authentication failed: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        Commands::Deauth => {
            let auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_deauth(args.provider) {
                eprintln!("❌ Deauthentication failed: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        Commands::Chat => {
            run_chat(args.model, args.log, args.provider).await
        }
        Commands::Providers => {
            list_providers().await
        }
    }
}

async fn run_chat(model: String, log: Option<String>, provider: Option<String>) -> Result<(), Box<dyn Error>> {
    // Create app with authentication
    let app = Arc::new(Mutex::new(match App::new_with_auth(model, log, provider) {
        Ok(app) => app,
        Err(e) => {
            // Check if this is an authentication error
            let error_msg = e.to_string();
            if error_msg.contains("No authentication") || error_msg.contains("OPENAI_API_KEY") {
                eprintln!("{}", error_msg);
                eprintln!();
                eprintln!("💡 Quick fixes:");
                eprintln!("  • chabeau auth                    # Interactive setup");
                eprintln!("  • chabeau providers               # Check provider status");
                eprintln!("  • export OPENAI_API_KEY=sk-...   # Use environment variable");
                std::process::exit(2); // Authentication error
            } else {
                eprintln!("❌ Error: {}", e);
                std::process::exit(1); // General error
            }
        }
    }));

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel for streaming updates with stream ID
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, u64)>();

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
                            // Retry the last bot response with debounce protection
                            let (should_retry, api_messages, client, model, api_key, base_url, cancel_token, stream_id) = {
                                let mut app_guard = app.lock().await;

                                // Check debounce at the event level to prevent any processing
                                let now = std::time::Instant::now();
                                if now.duration_since(app_guard.last_retry_time).as_millis() < 200 {
                                    // Too soon since last retry, ignore completely
                                    continue;
                                }

                                let terminal_size = terminal.size().unwrap_or_default();
                                let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title

                                if let Some(api_messages) = app_guard.prepare_retry(available_height, terminal_size.width) {
                                    // Start new stream (this will cancel any existing stream)
                                    let (cancel_token, stream_id) = app_guard.start_new_stream();

                                    (
                                        true,
                                        api_messages,
                                        app_guard.client.clone(),
                                        app_guard.model.clone(),
                                        app_guard.api_key.clone(),
                                        app_guard.base_url.clone(),
                                        cancel_token,
                                        stream_id,
                                    )
                                } else {
                                    (false, Vec::new(), app_guard.client.clone(), String::new(), String::new(), String::new(), tokio_util::sync::CancellationToken::new(), 0)
                                }
                            };

                            if !should_retry {
                                continue;
                            }

                            // Spawn the same API request logic as for Enter key
                            let tx_clone = tx.clone();
                            tokio::spawn(async move {
                                let request = ChatRequest {
                                    model,
                                    messages: api_messages,
                                    stream: true,
                                };

                                // Use tokio::select! to race between the HTTP request and cancellation
                                tokio::select! {
                                    _ = async {
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
                                                    // Check for cancellation before processing each chunk
                                                    if cancel_token.is_cancelled() {
                                                        return;
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
                                                                    let _ = tx_clone.send((STREAM_END_MARKER.to_string(), stream_id));
                                                                    return;
                                                                }

                                                                match serde_json::from_str::<ChatResponse>(data) {
                                                                    Ok(response) => {
                                                                        if let Some(choice) = response.choices.first() {
                                                                            if let Some(content) = &choice.delta.content {
                                                                                let _ = tx_clone.send((content.clone(), stream_id));
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
                                    } => {
                                        // HTTP request completed normally
                                    }
                                    _ = cancel_token.cancelled() => {
                                        // Stream was cancelled, clean up
                                        return;
                                    }
                                }
                            });
                        }
                        KeyCode::Esc => {
                            let mut app_guard = app.lock().await;
                            if app_guard.is_streaming {
                                // Use the new cancellation mechanism
                                app_guard.cancel_current_stream();
                            }
                        }
                        KeyCode::Enter => {
                            let (should_send_to_api, api_messages, client, model, api_key, base_url, cancel_token, stream_id) = {
                                let mut app_guard = app.lock().await;
                                if app_guard.input.trim().is_empty() {
                                    continue;
                                }

                                let input_text = app_guard.input.clone();
                                app_guard.input.clear();

                                // Process input for commands
                                match process_input(&mut app_guard, &input_text) {
                                    CommandResult::Continue => {
                                        // Command was processed, don't send to API
                                        // Update scroll position to ensure latest messages are visible
                                        let terminal_size = terminal.size().unwrap_or_default();
                                        let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                                        app_guard.update_scroll_position(available_height, terminal_size.width);
                                        continue;
                                    }
                                    CommandResult::ProcessAsMessage(message) => {
                                        // Re-enable auto-scroll when user sends a new message
                                        app_guard.auto_scroll = true;

                                        // Start new stream (this will cancel any existing stream)
                                        let (cancel_token, stream_id) = app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);

                                        // Update scroll position to ensure latest messages are visible
                                        let terminal_size = terminal.size().unwrap_or_default();
                                        let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                                        app_guard.update_scroll_position(available_height, terminal_size.width);

                                        (
                                            true,
                                            api_messages,
                                            app_guard.client.clone(),
                                            app_guard.model.clone(),
                                            app_guard.api_key.clone(),
                                            app_guard.base_url.clone(),
                                            cancel_token,
                                            stream_id,
                                        )
                                    }
                                }
                            };

                            if !should_send_to_api {
                                continue;
                            }

                            let tx_clone = tx.clone();
                            tokio::spawn(async move {
                                let request = ChatRequest {
                                    model,
                                    messages: api_messages,
                                    stream: true,
                                };

                                // Use tokio::select! to race between the HTTP request and cancellation
                                tokio::select! {
                                    _ = async {
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
                                                    // Check for cancellation before processing each chunk
                                                    if cancel_token.is_cancelled() {
                                                        return;
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
                                                                    let _ = tx_clone.send((STREAM_END_MARKER.to_string(), stream_id));
                                                                    return;
                                                                }

                                                                match serde_json::from_str::<ChatResponse>(data) {
                                                                    Ok(response) => {
                                                                        if let Some(choice) = response.choices.first() {
                                                                            if let Some(content) = &choice.delta.content {
                                                                                let _ = tx_clone.send((content.clone(), stream_id));
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
                                    } => {
                                        // HTTP request completed normally
                                    }
                                    _ = cancel_token.cancelled() => {
                                        // Stream was cancelled, clean up
                                        return;
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
                            let terminal_size = terminal.size().unwrap_or_default();
                            let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height, terminal_size.width);
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
                            let terminal_size = terminal.size().unwrap_or_default();
                            let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height, terminal_size.width);
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
        while let Ok((content, msg_stream_id)) = rx.try_recv() {
            let mut app_guard = app.lock().await;

            // Only process messages from the current stream
            if msg_stream_id != app_guard.current_stream_id {
                // This message is from an old stream, ignore it
                drop(app_guard);
                continue;
            }

            if content == STREAM_END_MARKER {
                // End of streaming - clear the streaming state and finalize response
                app_guard.finalize_response();
                app_guard.is_streaming = false;
                drop(app_guard);
                received_any = true;
            } else {
                let terminal_size = terminal.size().unwrap_or_default();
                let available_height = terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                app_guard.append_to_response(&content, available_height, terminal_size.width);
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
