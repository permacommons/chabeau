use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    env,
    error::Error,
    io,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};

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
  Backspace         Delete characters in the input field")]
struct Args {
    #[arg(short, long, default_value = "gpt-4o", help = "OpenAI model to use for chat")]
    model: String,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponseDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseChoice {
    delta: ChatResponseDelta,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatResponseChoice>,
}

#[derive(Clone)]
struct Message {
    role: String,
    content: String,
}

struct App {
    messages: VecDeque<Message>,
    input: String,
    input_mode: bool,
    current_response: String,
    client: Client,
    model: String,
    api_key: String,
    base_url: String,
    scroll_offset: u16,
    auto_scroll: bool, // Track if we should auto-scroll to bottom
}

impl App {
    fn build_display_lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();

        for msg in &self.messages {
            if msg.role == "user" {
                // User messages: cyan with "You:" prefix and indentation
                lines.push(Line::from(vec![
                    Span::styled("You: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&msg.content, Style::default().fg(Color::Cyan)),
                ]));
                lines.push(Line::from(""));  // Empty line for spacing
            } else if msg.role == "system" {
                // System messages: gray/dim color
                lines.push(Line::from(Span::styled(&msg.content, Style::default().fg(Color::DarkGray))));
                lines.push(Line::from(""));  // Empty line for spacing
            } else if !msg.content.is_empty() {
                // Assistant messages: no prefix, just content in white/default color
                // Split content into lines for proper wrapping
                for content_line in msg.content.lines() {
                    if content_line.trim().is_empty() {
                        lines.push(Line::from(""));
                    } else {
                        lines.push(Line::from(Span::styled(content_line, Style::default().fg(Color::White))));
                    }
                }
                lines.push(Line::from(""));  // Empty line for spacing
            }
        }

        lines
    }

    fn calculate_max_scroll_offset(&self, available_height: u16) -> u16 {
        let total_lines = self.build_display_lines().len() as u16;
        if total_lines > available_height {
            total_lines.saturating_sub(available_height)
        } else {
            0
        }
    }

    fn new(model: String) -> Result<Self, Box<dyn Error>> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|_| {
            "❌ Error: OPENAI_API_KEY environment variable not set

Please set your OpenAI API key:
export OPENAI_API_KEY=\"your-api-key-here\"

Optionally, you can also set a custom base URL:
export OPENAI_BASE_URL=\"https://api.openai.com/v1\""
        })?;

        let base_url = env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

        // Print configuration info
        eprintln!("🚀 Starting Chabeau - Terminal Chat Interface");
        eprintln!("📡 Using model: {}", model);
        eprintln!("🌐 API endpoint: {}", base_url);
        eprintln!("💡 Press Ctrl+C to quit, Enter to send messages");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        Ok(App {
            messages: VecDeque::new(),
            input: String::new(),
            input_mode: true,
            current_response: String::new(),
            client: Client::new(),
            model,
            api_key,
            base_url,
            scroll_offset: 0,
            auto_scroll: true,
        })
    }

    fn add_user_message(&mut self, content: String) -> Vec<ChatMessage> {
        let user_message = Message {
            role: "user".to_string(),
            content,
        };
        self.messages.push_back(user_message);

        // Start assistant message
        let assistant_message = Message {
            role: "assistant".to_string(),
            content: String::new(),
        };
        self.messages.push_back(assistant_message);
        self.current_response.clear();

        // Prepare messages for API (excluding the empty assistant message we just added)
        let mut api_messages = Vec::new();
        for msg in self.messages.iter().take(self.messages.len() - 1) {
            api_messages.push(ChatMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        api_messages
    }

    fn append_to_response(&mut self, content: &str, available_height: u16) {
        self.current_response.push_str(content);
        if let Some(last_msg) = self.messages.back_mut() {
            if last_msg.role == "assistant" {
                last_msg.content = self.current_response.clone();
            }
        }
        // Auto-scroll to bottom when new content arrives, but only if auto_scroll is enabled
        if self.auto_scroll {
            // Calculate the scroll offset needed to show the bottom
            let total_lines = self.build_display_lines().len() as u16;
            if total_lines > available_height {
                self.scroll_offset = total_lines.saturating_sub(available_height);
            } else {
                self.scroll_offset = 0;
            }
        }
    }

}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(f.area());

    // Use the shared method to build display lines
    let lines = app.build_display_lines();

    // Calculate scroll position
    let available_height = chunks[0].height.saturating_sub(1); // Account for title only (no borders)
    let total_lines = lines.len() as u16;

    // Always use the app's scroll_offset, but ensure it's within bounds
    let max_offset = if total_lines > available_height {
        total_lines.saturating_sub(available_height)
    } else {
        0
    };
    let scroll_offset = app.scroll_offset.min(max_offset);

    let messages_paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Chat - Chabeau")
        )
        .wrap(Wrap { trim: true })
        .scroll((scroll_offset, 0));

    f.render_widget(messages_paragraph, chunks[0]);

    // Input area
    let input_style = if app.input_mode {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let input_title = "Type your message (Press Enter to send, Ctrl+C to quit)".to_string();

    let input = Paragraph::new(app.input.as_str())
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_title)
        )
        .wrap(Wrap { trim: true });

    f.render_widget(input, chunks[1]);

    // Set cursor position
    if app.input_mode {
        f.set_cursor_position((
            chunks[1].x + app.input.len() as u16 + 1,
            chunks[1].y + 1,
        ));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // Create app first (this checks environment variables)
    let app = Arc::new(Mutex::new(match App::new(args.model) {
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
                        KeyCode::Enter => {
                            let (api_messages, client, model, api_key, base_url) = {
                                let mut app_guard = app.lock().await;
                                if app_guard.input.trim().is_empty() {
                                    continue;
                                }

                                let input_text = app_guard.input.clone();
                                app_guard.input.clear();
                                let api_messages = app_guard.add_user_message(input_text);
                                (
                                    api_messages,
                                    app_guard.client.clone(),
                                    app_guard.model.clone(),
                                    app_guard.api_key.clone(),
                                    app_guard.base_url.clone(),
                                )
                            };

                            let tx_clone = tx.clone();
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
                            app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(1);
                            // If scrolled to bottom, re-enable auto-scroll
                            let terminal_height = terminal.size().unwrap_or_default().height;
                            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height);
                            if app_guard.scroll_offset == 0 || app_guard.scroll_offset >= max_scroll {
                                app_guard.auto_scroll = true;
                            }
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
                            let terminal_height = terminal.size().unwrap_or_default().height;
                            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height);
                            app_guard.scroll_offset = (app_guard.scroll_offset.saturating_add(3)).min(max_scroll);
                        }
                        MouseEventKind::ScrollDown => {
                            let mut app_guard = app.lock().await;
                            app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(3);
                            // If scrolled to bottom, re-enable auto-scroll
                            let terminal_height = terminal.size().unwrap_or_default().height;
                            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
                            let max_scroll = app_guard.calculate_max_scroll_offset(available_height);
                            if app_guard.scroll_offset == 0 || app_guard.scroll_offset >= max_scroll {
                                app_guard.auto_scroll = true;
                            }
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
            let mut app_guard = app.lock().await;
            let terminal_height = terminal.size().unwrap_or_default().height;
            let available_height = terminal_height.saturating_sub(3).saturating_sub(1); // 3 for input area, 1 for title
            app_guard.append_to_response(&content, available_height);
            drop(app_guard);
            received_any = true;
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
