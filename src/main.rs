mod api;
mod app;
mod auth;
mod commands;
mod config;
mod logging;
mod message;
mod scroll;
mod ui;

use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::fs;
use std::process::Command;
use std::{error::Error, io, sync::Arc, time::Duration};
use tempfile::NamedTempFile;
use tokio::sync::{mpsc, Mutex};

use api::{ChatRequest, ChatResponse, ModelsResponse};
use app::App;
use auth::AuthManager;
use commands::{process_input, CommandResult};
use ui::ui;

async fn list_models(provider: Option<String>) -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();

    let (api_key, base_url, provider_name) = if let Some(provider_name) = provider {
        // User specified a provider
        if let Some((base_url, api_key)) = auth_manager.get_auth_for_provider(&provider_name)? {
            (api_key, base_url, provider_name)
        } else {
            return Err(format!("No authentication found for provider '{provider_name}'. Run 'chabeau auth' to set up authentication.").into());
        }
    } else {
        // Try to find any available authentication
        if let Some((provider, api_key)) = auth_manager.find_first_available_auth() {
            (api_key, provider.base_url, provider.display_name)
        } else {
            // Fall back to environment variables
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                "❌ No authentication configured and OPENAI_API_KEY environment variable not set

Please either:
1. Run 'chabeau auth' to set up authentication, or
2. Set environment variables:
   export OPENAI_API_KEY=\"your-api-key-here\"
   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional"
            })?;

            let base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

            (api_key, base_url, "Environment Variables".to_string())
        }
    };

    println!("🤖 Available Models for {provider_name}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let client = reqwest::Client::new();

    match client
        .get(format!("{base_url}/models"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return Err(
                    format!("API request failed with status {status}: {error_text}").into(),
                );
            }

            match response.json::<ModelsResponse>().await {
                Ok(models_response) => {
                    if models_response.data.is_empty() {
                        println!("No models found for this provider.");
                    } else {
                        println!(
                            "Found {} models (sorted newest first):",
                            models_response.data.len()
                        );
                        println!();

                        // Sort models by creation date (newest first), then by ID for consistent display
                        let mut models = models_response.data;
                        models.sort_by(|a, b| {
                            // First sort by creation date (newest first)
                            match (a.created, b.created) {
                                (Some(a_created), Some(b_created)) => b_created.cmp(&a_created),
                                (Some(_), None) => std::cmp::Ordering::Less, // Models with dates come first
                                (None, Some(_)) => std::cmp::Ordering::Greater, // Models without dates come last
                                (None, None) => a.id.cmp(&b.id), // Fall back to ID sorting
                            }
                        });

                        for model in models {
                            println!("  • {}", model.id);
                            if let Some(owned_by) = &model.owned_by {
                                if !owned_by.is_empty() && owned_by != "system" {
                                    println!("    Owner: {owned_by}");
                                }
                            }
                            if let Some(created) = model.created {
                                if created > 0 {
                                    // Convert Unix timestamp to human-readable date
                                    // Some APIs return timestamps in milliseconds, others in seconds
                                    let timestamp_secs = if created > 10_000_000_000 {
                                        // Likely milliseconds, convert to seconds
                                        created / 1000
                                    } else {
                                        // Already in seconds
                                        created
                                    };

                                    let datetime =
                                        DateTime::<Utc>::from_timestamp(timestamp_secs as i64, 0);
                                    if let Some(dt) = datetime {
                                        println!(
                                            "    Created: {}",
                                            dt.format("%Y-%m-%d %H:%M:%S UTC")
                                        );
                                    }
                                }
                            }
                            println!();
                        }
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to parse models response: {e}").into());
                }
            }
        }
        Err(e) => {
            return Err(format!("Failed to fetch models: {e}").into());
        }
    }

    Ok(())
}

async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new();
    let config = config::Config::load()?;

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
        println!("  {display_name} ({name}) - {status}");
        println!("    URL: {url}");
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
                    let status = if has_token {
                        "✅ configured"
                    } else {
                        "❌ not configured"
                    };
                    println!("  {name} - {status}");
                    println!("    URL: {url}");
                }
            }
        }
        Err(_) => {
            println!("Custom providers: error checking");
        }
    }
    println!();

    // Show which provider would be used by default
    if let Some(default_provider) = &config.default_provider {
        println!("🎯 Default provider: {default_provider} (from config)");
    } else {
        match auth_manager.find_first_available_auth() {
            Some((provider, _)) => {
                println!(
                    "🎯 Default provider: {} ({})",
                    provider.display_name, provider.name
                );
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
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "chabeau")]
#[command(about = "A terminal-based chat interface using OpenAI API")]
#[command(
    long_about = "Chabeau is a full-screen terminal chat interface that connects to various AI APIs \
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
  Ctrl+E            Open external editor (requires EDITOR env var)\n\
  Backspace         Delete characters in the input field\n\n\
Commands:\n\
  /help             Show extended help with keyboard shortcuts\n\
  /log <filename>   Enable logging to specified file\n\
  /log              Toggle logging pause/resume"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Model to use for chat, or list available models if no model specified
    #[arg(short = 'm', long, global = true, value_name = "MODEL", num_args = 0..=1, default_missing_value = "")]
    model: Option<String>,

    /// Enable logging to specified file
    #[arg(short = 'l', long, global = true)]
    log: Option<String>,

    /// Provider to use, or list available providers if no provider specified
    #[arg(short = 'p', long, global = true, value_name = "PROVIDER", num_args = 0..=1, default_missing_value = "")]
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
    /// Set configuration values
    Set {
        /// Configuration key to set
        key: String,
        /// Value to set for the key
        value: Option<String>,
    },
    /// Unset configuration values
    Unset {
        /// Configuration key to unset
        key: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    match args.command.unwrap_or(Commands::Chat) {
        Commands::Auth => {
            let auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_auth() {
                eprintln!("❌ Authentication failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Commands::Deauth => {
            let auth_manager = AuthManager::new();
            if let Err(e) = auth_manager.interactive_deauth(args.provider) {
                eprintln!("❌ Deauthentication failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Commands::Set { key, value } => {
            let mut config = config::Config::load()?;
            match key.as_str() {
                "default-provider" => {
                    if let Some(val) = value {
                        config.default_provider = Some(val.clone());
                        config.save()?;
                        println!("✅ Set default-provider to: {}", val);
                    } else {
                        config.print_all();
                    }
                }
                _ => {
                    eprintln!("❌ Unknown config key: {}", key);
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Commands::Unset { key } => {
            let mut config = config::Config::load()?;
            match key.as_str() {
                "default-provider" => {
                    config.default_provider = None;
                    config.save()?;
                    println!("✅ Unset default-provider");
                }
                _ => {
                    eprintln!("❌ Unknown config key: {}", key);
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Commands::Chat => {
            // Check if -p was provided without a provider name (empty string)
            match args.provider.as_deref() {
                Some("") => {
                    // -p was provided without a value, list available providers
                    list_providers().await
                }
                _ => {
                    // Normal flow: check -m flag behavior
                    let provider_for_operations = if args.provider.as_deref() == Some("") {
                        None // Don't pass empty string provider to other operations
                    } else {
                        args.provider
                    };

                    match args.model.as_deref() {
                        Some("") => {
                            // -m was provided without a value, list available models
                            list_models(provider_for_operations).await
                        }
                        Some(model) => {
                            // -m was provided with a value, use it for chat
                            run_chat(model.to_string(), args.log, provider_for_operations).await
                        }
                        None => {
                            // -m was not provided, use default model for chat
                            run_chat("gpt-4o".to_string(), args.log, provider_for_operations).await
                        }
                    }
                }
            }
        }
    }
}

async fn handle_external_editor(app: &mut App) -> Result<Option<String>, Box<dyn Error>> {
    // Check if EDITOR environment variable is set
    let editor = match std::env::var("EDITOR") {
        Ok(editor) if !editor.trim().is_empty() => editor,
        _ => {
            app.add_system_message("No EDITOR environment variable set. Please set EDITOR to your preferred text editor (e.g., export EDITOR=nano).".to_string());
            return Ok(None);
        }
    };

    // Create a temporary file
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    // Write current input to the temp file if there's any
    if !app.input.is_empty() {
        fs::write(&temp_path, &app.input)?;
    }

    // We need to temporarily exit raw mode to allow the editor to run
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    // Run the editor
    let mut command = Command::new(&editor);
    command.arg(&temp_path);

    let status = command.status()?;

    // Restore terminal mode
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    if !status.success() {
        app.add_system_message(format!("Editor exited with non-zero status: {status}"));
        return Ok(None);
    }

    // Read the file content
    let content = fs::read_to_string(&temp_path)?;

    // Check if file has content (not zero bytes and not just whitespace)
    if content.trim().is_empty() {
        app.add_system_message(
            "Editor file was empty or contained only whitespace - no message sent.".to_string(),
        );
        Ok(None)
    } else {
        // Clear the input and return the content to be sent immediately
        app.input.clear();
        let message = content.trim_end().to_string(); // Remove trailing newlines but preserve internal formatting
        Ok(Some(message))
    }

    // Temp file will be automatically cleaned up when it goes out of scope
}

async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Create app with authentication
    let app = Arc::new(Mutex::new(match App::new_with_auth(model, log, provider) {
        Ok(app) => app,
        Err(e) => {
            // Check if this is an authentication error
            let error_msg = e.to_string();
            if error_msg.contains("No authentication") || error_msg.contains("OPENAI_API_KEY") {
                eprintln!("{error_msg}");
                eprintln!();
                eprintln!("💡 Quick fixes:");
                eprintln!("  • chabeau auth                    # Interactive setup");
                eprintln!("  • chabeau -p                      # Check provider status");
                eprintln!("  • export OPENAI_API_KEY=sk-...   # Use environment variable");
                std::process::exit(2); // Authentication error
            } else {
                eprintln!("❌ Error: {e}");
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
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            break Ok(());
                        }
                        KeyCode::Char('e')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Handle CTRL+E for external editor
                            let editor_result = {
                                let mut app_guard = app.lock().await;
                                handle_external_editor(&mut app_guard).await
                            };

                            // Force a full redraw after editor
                            terminal.clear()?;

                            match editor_result {
                                Ok(Some(message)) => {
                                    // Editor returned content, send it immediately
                                    let (
                                        api_messages,
                                        client,
                                        model,
                                        api_key,
                                        base_url,
                                        cancel_token,
                                        stream_id,
                                    ) = {
                                        let mut app_guard = app.lock().await;

                                        // Re-enable auto-scroll when user sends a new message
                                        app_guard.auto_scroll = true;

                                        // Start new stream (this will cancel any existing stream)
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);

                                        // Update scroll position to ensure latest messages are visible
                                        let terminal_size = terminal.size().unwrap_or_default();
                                        let available_height = terminal_size
                                            .height
                                            .saturating_sub(3)
                                            .saturating_sub(1); // 3 for input area, 1 for title
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal_size.width,
                                        );

                                        (
                                            api_messages,
                                            app_guard.client.clone(),
                                            app_guard.model.clone(),
                                            app_guard.api_key.clone(),
                                            app_guard.base_url.clone(),
                                            cancel_token,
                                            stream_id,
                                        )
                                    };

                                    // Send the message to API
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
                                                    .post(format!("{base_url}/chat/completions"))
                                                    .header("Authorization", format!("Bearer {api_key}"))
                                                    .header("Content-Type", "application/json")
                                                    .json(&request)
                                                    .send()
                                                    .await
                                                {
                                                    Ok(response) => {
                                                        if !response.status().is_success() {
                                                            if let Ok(error_text) = response.text().await {
                                                                eprintln!("API request failed: {error_text}");
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

                                                                    if let Some(data) = line.strip_prefix("data: ") {
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
                                                                                eprintln!("Failed to parse JSON: {e} - Data: {data}");
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!("Error sending message: {e}");
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
                                Ok(None) => {
                                    // Editor returned no content or user cancelled
                                    let mut app_guard = app.lock().await;
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let available_height =
                                        terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                                    app_guard.update_scroll_position(
                                        available_height,
                                        terminal_size.width,
                                    );
                                }
                                Err(e) => {
                                    let mut app_guard = app.lock().await;
                                    app_guard.add_system_message(format!("Editor error: {e}"));

                                    // Update scroll position to show the new system message
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let available_height =
                                        terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                                    app_guard.update_scroll_position(
                                        available_height,
                                        terminal_size.width,
                                    );
                                }
                            }
                        }
                        KeyCode::Char('r')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Retry the last bot response with debounce protection
                            let (
                                should_retry,
                                api_messages,
                                client,
                                model,
                                api_key,
                                base_url,
                                cancel_token,
                                stream_id,
                            ) = {
                                let mut app_guard = app.lock().await;

                                // Check debounce at the event level to prevent any processing
                                let now = std::time::Instant::now();
                                if now.duration_since(app_guard.last_retry_time).as_millis() < 200 {
                                    // Too soon since last retry, ignore completely
                                    continue;
                                }

                                let terminal_size = terminal.size().unwrap_or_default();
                                let available_height =
                                    terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title

                                if let Some(api_messages) =
                                    app_guard.prepare_retry(available_height, terminal_size.width)
                                {
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
                                    (
                                        false,
                                        Vec::new(),
                                        app_guard.client.clone(),
                                        String::new(),
                                        String::new(),
                                        String::new(),
                                        tokio_util::sync::CancellationToken::new(),
                                        0,
                                    )
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
                                            .post(format!("{base_url}/chat/completions"))
                                            .header("Authorization", format!("Bearer {api_key}"))
                                            .header("Content-Type", "application/json")
                                            .json(&request)
                                            .send()
                                            .await
                                        {
                                            Ok(response) => {
                                                if !response.status().is_success() {
                                                    if let Ok(error_text) = response.text().await {
                                                        eprintln!("API request failed: {error_text}");
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

                                                            if let Some(data) = line.strip_prefix("data: ") {
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
                                                                        eprintln!("Failed to parse JSON: {e} - Data: {data}");
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Error sending message: {e}");
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
                            let (
                                should_send_to_api,
                                api_messages,
                                client,
                                model,
                                api_key,
                                base_url,
                                cancel_token,
                                stream_id,
                            ) = {
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
                                        let available_height = terminal_size
                                            .height
                                            .saturating_sub(3)
                                            .saturating_sub(1); // 3 for input area, 1 for title
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal_size.width,
                                        );
                                        continue;
                                    }
                                    CommandResult::ProcessAsMessage(message) => {
                                        // Re-enable auto-scroll when user sends a new message
                                        app_guard.auto_scroll = true;

                                        // Start new stream (this will cancel any existing stream)
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);

                                        // Update scroll position to ensure latest messages are visible
                                        let terminal_size = terminal.size().unwrap_or_default();
                                        let available_height = terminal_size
                                            .height
                                            .saturating_sub(3)
                                            .saturating_sub(1); // 3 for input area, 1 for title
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal_size.width,
                                        );

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
                                            .post(format!("{base_url}/chat/completions"))
                                            .header("Authorization", format!("Bearer {api_key}"))
                                            .header("Content-Type", "application/json")
                                            .json(&request)
                                            .send()
                                            .await
                                        {
                                            Ok(response) => {
                                                if !response.status().is_success() {
                                                    if let Ok(error_text) = response.text().await {
                                                        eprintln!("API request failed: {error_text}");
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

                                                            if let Some(data) = line.strip_prefix("data: ") {
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
                                                                        eprintln!("Failed to parse JSON: {e} - Data: {data}");
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Error sending message: {e}");
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
                            let available_height =
                                terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard
                                .calculate_max_scroll_offset(available_height, terminal_size.width);
                            app_guard.scroll_offset =
                                (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
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
                            let available_height =
                                terminal_size.height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard
                                .calculate_max_scroll_offset(available_height, terminal_size.width);
                            app_guard.scroll_offset =
                                (app_guard.scroll_offset.saturating_add(3)).min(max_scroll);
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
