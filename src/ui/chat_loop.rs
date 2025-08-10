//! Main chat event loop and UI rendering
//!
//! This module contains the main event loop that handles user input, renders the UI,
//! and manages the chat session.

use crate::api::{ChatRequest, ChatResponse};
use crate::commands::process_input;
use crate::commands::CommandResult;
use crate::core::app::App;
use crate::ui::renderer::ui;
use crate::utils::editor::handle_external_editor;
use futures_util::StreamExt;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{error::Error, io, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex};

pub async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Create app with authentication - model selection logic is now handled in App::new_with_auth
    let app = Arc::new(Mutex::new(
        match App::new_with_auth(model, log, provider).await {
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
        },
    ));

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
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
            let mut should_redraw = false;
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            break Ok(());
                        }
                        KeyCode::Char('t')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Handle CTRL+T for external editor
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
                                        let input_area_height = app_guard
                                            .calculate_input_area_height(terminal_size.width);
                                        let available_height = terminal_size
                                            .height
                                            .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                            .saturating_sub(1); // 1 for title
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

                                                            if let Ok(chunk_bytes) = chunk {
                                                                let chunk_str = String::from_utf8_lossy(&chunk_bytes);
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
                                            }
                                        }
                                    });
                                }
                                Ok(None) => {
                                    // Editor returned no content or user cancelled
                                    let mut app_guard = app.lock().await;
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let input_area_height =
                                        app_guard.calculate_input_area_height(terminal_size.width);
                                    let available_height = terminal_size
                                        .height
                                        .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                        .saturating_sub(1); // 1 for title
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
                                    let input_area_height =
                                        app_guard.calculate_input_area_height(terminal_size.width);
                                    let available_height = terminal_size
                                        .height
                                        .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                        .saturating_sub(1); // 1 for title
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
                                let input_area_height =
                                    app_guard.calculate_input_area_height(terminal_size.width);
                                let available_height = terminal_size
                                    .height
                                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                    .saturating_sub(1); // 1 for title

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

                                                    if let Ok(chunk_bytes) = chunk {
                                                        let chunk_str = String::from_utf8_lossy(&chunk_bytes);
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
                            let modifiers = key.modifiers;
                            // Use Alt+Enter for newlines since Shift+Enter and Ctrl+Enter
                            // are not reliably detected in all terminals
                            if modifiers.contains(event::KeyModifiers::ALT) {
                                // Alt+Enter: insert newline in input
                                let mut app_guard = app.lock().await;
                                app_guard.input.push('\n');
                                // Update input scroll to keep cursor visible
                                let terminal_size = terminal.size().unwrap_or_default();
                                let input_area_height =
                                    app_guard.calculate_input_area_height(terminal_size.width);
                                app_guard
                                    .update_input_scroll(input_area_height, terminal_size.width);
                                should_redraw = true;
                            } else {
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
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(terminal_size.width);
                                            let available_height = terminal_size
                                                .height
                                                .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                                .saturating_sub(1); // 1 for title
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
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(terminal_size.width);
                                            let available_height = terminal_size
                                                .height
                                                .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                                .saturating_sub(1); // 1 for title
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

                                                        if let Ok(chunk_bytes) = chunk {
                                                            let chunk_str = String::from_utf8_lossy(&chunk_bytes);
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
                                        }
                                    }
                                });
                            }
                        }
                        KeyCode::Char(c) => {
                            let mut app_guard = app.lock().await;
                            app_guard.input.push(c);
                            // Update input scroll to keep cursor visible
                            let terminal_size = terminal.size().unwrap_or_default();
                            let input_area_height =
                                app_guard.calculate_input_area_height(terminal_size.width);
                            app_guard.update_input_scroll(input_area_height, terminal_size.width);
                        }
                        KeyCode::Backspace => {
                            let mut app_guard = app.lock().await;
                            app_guard.input.pop();
                            // Update input scroll to keep cursor visible
                            let terminal_size = terminal.size().unwrap_or_default();
                            let input_area_height =
                                app_guard.calculate_input_area_height(terminal_size.width);
                            app_guard.update_input_scroll(input_area_height, terminal_size.width);
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
                            let input_area_height =
                                app_guard.calculate_input_area_height(terminal_size.width);
                            let available_height = terminal_size
                                .height
                                .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                .saturating_sub(1); // 1 for title
                            let max_scroll = app_guard
                                .calculate_max_scroll_offset(available_height, terminal_size.width);
                            app_guard.scroll_offset =
                                (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    // Handle paste events - add the pasted text directly to input
                    let mut app_guard = app.lock().await;
                    app_guard.input.push_str(&text);
                    // Update input scroll to keep cursor visible
                    let terminal_size = terminal.size().unwrap_or_default();
                    let input_area_height =
                        app_guard.calculate_input_area_height(terminal_size.width);
                    app_guard.update_input_scroll(input_area_height, terminal_size.width);
                    should_redraw = true;
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
                            let input_area_height =
                                app_guard.calculate_input_area_height(terminal_size.width);
                            let available_height = terminal_size
                                .height
                                .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                .saturating_sub(1); // 1 for title
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

            // If we need to redraw immediately (e.g., after Alt+Enter), continue to next loop iteration
            if should_redraw {
                continue;
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
                let input_area_height = app_guard.calculate_input_area_height(terminal_size.width);
                let available_height = terminal_size
                    .height
                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                    .saturating_sub(1); // 1 for title
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
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}
