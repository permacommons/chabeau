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
use crate::utils::url::construct_api_url;
use crate::{
    auth::AuthManager, core::builtin_providers::load_builtin_providers, core::config::Config,
};
use futures_util::StreamExt;
use memchr::memchr;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    error::Error,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};
use tui_textarea::{CursorMove, Input as TAInput, Key as TAKey};

// Module-level stream end marker for helper usage and event loop
const STREAM_END_MARKER: &str = "<<STREAM_END>>";

struct StreamParams {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    api_messages: Vec<crate::api::ChatMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    stream_id: u64,
    tx: tokio::sync::mpsc::UnboundedSender<(String, u64)>,
}

fn spawn_stream(params: StreamParams) {
    let StreamParams {
        client,
        base_url,
        api_key,
        model,
        api_messages,
        cancel_token,
        stream_id,
        tx,
    } = params;
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let request = ChatRequest {
            model,
            messages: api_messages,
            stream: true,
        };

        tokio::select! {
            _ = async {
                let chat_url = construct_api_url(&base_url, "chat/completions");
                match client
                    .post(chat_url)
                    .header("Authorization", format!("Bearer {api_key}"))
                    .header("Content-Type", "application/json")
                    .json(&request)
                    .send()
                    .await
                {
                    Ok(response) => {
                        if !response.status().is_success() {
                            let error_text = response
                                .text()
                                .await
                                .unwrap_or_else(|_| "<no body>".to_string());
                            let _ = tx_clone.send((format!("<<API_ERROR>>{}", error_text), stream_id));
                            let _ = tx_clone.send((STREAM_END_MARKER.to_string(), stream_id));
                            return;
                        }

                        let mut stream = response.bytes_stream();
                        let mut buffer: Vec<u8> = Vec::new();

                        while let Some(chunk) = stream.next().await {
                            if cancel_token.is_cancelled() {
                                return;
                            }

                            if let Ok(chunk_bytes) = chunk {
                                buffer.extend_from_slice(&chunk_bytes);

                                while let Some(newline_pos) = memchr(b'\n', &buffer) {
                                    let line_str = match std::str::from_utf8(&buffer[..newline_pos]) {
                                        Ok(s) => s.trim(),
                                        Err(e) => {
                                            eprintln!("Invalid UTF-8 in stream: {e}");
                                            buffer.drain(..=newline_pos);
                                            continue;
                                        }
                                    };

                                    if let Some(data) = line_str.strip_prefix("data: ") {
                                        if data == "[DONE]" {
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
                                    buffer.drain(..=newline_pos);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx_clone.send((format!("<<API_ERROR>>{}", e), stream_id));
                        let _ = tx_clone.send((STREAM_END_MARKER.to_string(), stream_id));
                    }
                }
            } => {}
            _ = cancel_token.cancelled() => {}
        }
    });
}

fn language_to_extension(lang: Option<&str>) -> &'static str {
    if let Some(l) = lang {
        let l = l.trim().to_ascii_lowercase();
        return match l.as_str() {
            "rs" | "rust" => "rs",
            "py" | "python" => "py",
            "sh" | "bash" | "zsh" => "sh",
            "js" | "javascript" => "js",
            "ts" | "typescript" => "ts",
            "json" => "json",
            "yaml" | "yml" => "yml",
            "toml" => "toml",
            "md" | "markdown" => "md",
            "go" => "go",
            "java" => "java",
            "c" => "c",
            "cpp" | "c++" | "cc" | "cxx" => "cpp",
            "html" => "html",
            "css" => "css",
            "sql" => "sql",
            _ => "txt",
        };
    }
    "txt"
}

/// Helper to generate status suffix for picker actions (persistent vs session-only)
fn status_suffix(is_persistent: bool) -> &'static str {
    if is_persistent {
        " (saved to config)"
    } else {
        " (session only)"
    }
}

pub async fn run_chat(
    model: String,
    log: Option<String>,
    provider: Option<String>,
    env_only: bool,
) -> Result<(), Box<dyn Error>> {
    // Startup provider selection policy per spec
    let config = Config::load()?;
    let auth_manager = AuthManager::new();

    // Collect providers that have tokens (ignored when env_only=true)
    let mut token_providers: Vec<String> = Vec::new();
    if !env_only {
        for bp in load_builtin_providers() {
            if auth_manager.get_token(&bp.id).unwrap_or(None).is_some() {
                token_providers.push(bp.id);
            }
        }
        for (id, _display, _url, has_token) in auth_manager.list_custom_providers() {
            if has_token {
                token_providers.push(id);
            }
        }
    }
    // Environment variable counts as OpenAI auth (not token-backed)
    let has_env_openai = std::env::var("OPENAI_API_KEY").is_ok();

    // Decide startup path
    let mut selected_provider: Option<String> = None;
    let mut open_provider_picker = false;
    let total_available = token_providers.len() + if has_env_openai { 1 } else { 0 };
    let multiple_providers_available = total_available > 1;

    // Respect -p when provided (ignored in env-only mode)
    if !env_only {
        if let Some(p) = provider.clone() {
            if !p.is_empty() {
                selected_provider = Some(p);
            }
        }
    }

    if selected_provider.is_none() {
        if env_only {
            // Force env path: verify OPENAI_API_KEY exists
            if !has_env_openai {
                eprintln!("❌ --env used but OPENAI_API_KEY is not set");
                std::process::exit(2);
            }
            // Let authentication fallback handle provider naming via env
            // Do not open provider picker in env-only mode
        }
        if let Some(default_p) = &config.default_provider {
            selected_provider = Some(default_p.clone());
        } else if token_providers.len() == 1 {
            // Exactly one configured provider → select it automatically, even if env is present
            selected_provider = token_providers.first().cloned();
        } else if total_available > 1 {
            open_provider_picker = true;
        } else if has_env_openai {
            // Env-only OpenAI: pass None to let authentication fall back to env vars
            selected_provider = None;
        } else {
            // No auth available, show guidance and exit
            eprintln!("❌ No authentication configured and OPENAI_API_KEY environment variable not set\n\nPlease either:\n1. Run 'chabeau auth' to set up authentication, or\n2. Set environment variables:\n   export OPENAI_API_KEY=\"your-api-key-here\"\n   export OPENAI_BASE_URL=\"https://api.openai.com/v1\"  # Optional");
            std::process::exit(2);
        }
    }

    // Create app accordingly
    let app = if open_provider_picker {
        let app = Arc::new(Mutex::new(
            App::new_uninitialized(log.clone()).await.expect("init app"),
        ));
        {
            let mut app_guard = app.lock().await;
            app_guard.startup_requires_provider = true;
            app_guard.startup_multiple_providers_available = multiple_providers_available;
            app_guard.open_provider_picker();
        }
        app
    } else {
        let app = Arc::new(Mutex::new(
            match App::new_with_auth(model.clone(), log.clone(), selected_provider, env_only).await
            {
                Ok(app) => app,
                Err(e) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("No authentication")
                        || error_msg.contains("OPENAI_API_KEY")
                    {
                        eprintln!("{error_msg}");
                        eprintln!();
                        eprintln!("💡 Quick fixes:");
                        eprintln!("  • chabeau auth                    # Interactive setup");
                        eprintln!("  • chabeau -p                      # Check provider status");
                        eprintln!("  • export OPENAI_API_KEY=sk-...    # Use environment variable (defaults to OpenAI API)");
                        std::process::exit(2);
                    } else {
                        eprintln!("❌ Error: {e}");
                        std::process::exit(1);
                    }
                }
            },
        ));
        // If no model is configured, open the model picker at startup
        let mut need_model_picker = false;
        {
            let app_guard = app.lock().await;
            if app_guard.model.is_empty() {
                need_model_picker = true;
            }
        }
        if need_model_picker {
            let mut app_guard = app.lock().await;
            app_guard.startup_requires_model = true;
            app_guard.startup_multiple_providers_available = multiple_providers_available;
            // If we have env-only auth (no token providers), disable persisting defaults
            let env_only = has_env_openai && token_providers.is_empty();
            app_guard.startup_env_only = env_only;
            if let Err(e) = app_guard.open_model_picker().await {
                app_guard.set_status(format!("Model picker error: {}", e));
            }
        }
        app
    };

    // Sign-off line (no noisy startup banners)
    println!(
        "Chabeau is in the public domain, forever. Contribute: https://github.com/permacommons/chabeau"
    );
    // Color depth print removed; use CHABEAU_COLOR and README tips when debugging

    // Setup terminal only after successful app creation
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel for streaming updates with stream ID
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, u64)>();

    // STREAM_END_MARKER defined at module level

    // Drawing cadence control
    let _last_draw = Instant::now();
    let mut _request_redraw = true;
    let mut last_input_layout_update = Instant::now();
    let _last_tick_instant = Instant::now();
    let _last_input_event: Option<Instant> = None;
    let _pressed_keys: Vec<(String, Instant)> = Vec::new();
    // Perf sampling window (1s) and maxima
    let _window_start = Instant::now();
    let _max_tick_ms: u128 = 0;
    let _max_draw_ms: u128 = 0;
    let _max_input_to_draw_ms: u128 = 0;
    let _max_queue_drain_ms: u128 = 0;
    let _max_poll_delay_ms: u128 = 0;

    // Performance logger (enabled when CHABEAU_PERF_LOG=1)
    // Perf logging disabled

    // Main loop
    let result = 'main_loop: loop {
        let _tick_start = Instant::now();
        {
            let mut app_guard = app.lock().await;
            if app_guard.exit_requested {
                break 'main_loop Ok(());
            }
            terminal.draw(|f| ui(f, &mut app_guard))?;
        }
        // Cache terminal size for this tick
        let term_size = terminal.size().unwrap_or_default();
        // Local throttle helper
        let mut update_if_due = |app_guard: &mut App| {
            if last_input_layout_update.elapsed() >= Duration::from_millis(16) {
                app_guard.recompute_input_layout_after_edit(term_size.width);
                last_input_layout_update = Instant::now();
            }
        };

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Always allow Ctrl+C to quit, even when a modal is open
                    if matches!(key.code, KeyCode::Char('c'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        break 'main_loop Ok(());
                    }
                    // Clear ephemeral status with Ctrl+L
                    if matches!(key.code, KeyCode::Char('l'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        let mut app_guard = app.lock().await;
                        app_guard.clear_status();
                        continue;
                    }
                    // Toggle compose mode with F4
                    if matches!(key.code, KeyCode::F(4)) {
                        let mut app_guard = app.lock().await;
                        app_guard.compose_mode = !app_guard.compose_mode;
                        continue;
                    }
                    // If a picker is open, handle navigation/selection first
                    {
                        let mut app_guard = app.lock().await;
                        let current_picker_mode = app_guard.picker_mode.clone();
                        let provider_name = app_guard.provider_name.clone(); // Extract before mutable borrow
                        if let Some(selected_id) = {
                            if let Some(picker) = &mut app_guard.picker {
                                match key.code {
                                    KeyCode::Esc => {
                                        match current_picker_mode {
                                            Some(crate::core::app::PickerMode::Theme) => {
                                                app_guard.revert_theme_preview();
                                                app_guard.picker = None;
                                                app_guard.picker_mode = None;
                                            }
                                            Some(crate::core::app::PickerMode::Model) => {
                                                if app_guard.startup_requires_model {
                                                    // Startup mandatory model selection
                                                    app_guard.picker = None;
                                                    app_guard.picker_mode = None;
                                                    if app_guard
                                                        .startup_multiple_providers_available
                                                    {
                                                        // Go back to provider picker per spec
                                                        app_guard.startup_requires_model = false;
                                                        app_guard.startup_requires_provider = true;
                                                        // Clear provider selection in title bar during startup bounce-back
                                                        app_guard.provider_name.clear();
                                                        app_guard.provider_display_name =
                                                            "(no provider selected)".to_string();
                                                        app_guard.api_key.clear();
                                                        app_guard.base_url.clear();
                                                        app_guard.open_provider_picker();
                                                    } else {
                                                        // Exit app if no alternative provider
                                                        app_guard.exit_requested = true;
                                                    }
                                                } else {
                                                    app_guard.revert_model_preview();
                                                    if app_guard.in_provider_model_transition {
                                                        app_guard
                                                            .revert_provider_model_transition();
                                                        app_guard.set_status("Selection cancelled");
                                                    }
                                                    app_guard.picker = None;
                                                    app_guard.picker_mode = None;
                                                }
                                            }
                                            Some(crate::core::app::PickerMode::Provider) => {
                                                if app_guard.startup_requires_provider {
                                                    // Startup mandatory provider selection: exit if cancelled
                                                    app_guard.picker = None;
                                                    app_guard.picker_mode = None;
                                                    app_guard.exit_requested = true;
                                                } else {
                                                    app_guard.revert_provider_preview();
                                                    app_guard.picker = None;
                                                    app_guard.picker_mode = None;
                                                }
                                            }
                                            _ => {}
                                        }
                                        None
                                    }
                                    KeyCode::Up => {
                                        picker.move_up();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::Down => {
                                        picker.move_down();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::Char('k') => {
                                        picker.move_up();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::Char('j')
                                        if !key
                                            .modifiers
                                            .contains(event::KeyModifiers::CONTROL) =>
                                    {
                                        picker.move_down();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::Home => {
                                        picker.move_to_start();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::End => {
                                        picker.move_to_end();
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            picker.selected_id().map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    KeyCode::F(6) => {
                                        picker.cycle_sort_mode();
                                        // Re-sort and update title
                                        let _ = picker; // Release borrow
                                        app_guard.sort_picker_items();
                                        app_guard.update_picker_title();
                                        None
                                    }
                                    // Apply selection: Enter (Alt=Persist) or Ctrl+J (Persist)
                                    KeyCode::Enter | KeyCode::Char('j')
                                        if key.code == KeyCode::Enter
                                            || key
                                                .modifiers
                                                .contains(event::KeyModifiers::CONTROL) =>
                                    {
                                        let is_persistent = if key.code == KeyCode::Enter {
                                            key.modifiers.contains(event::KeyModifiers::ALT)
                                        } else {
                                            true
                                        };
                                        // Common apply path
                                        // Theme
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            if let Some(id) =
                                                picker.selected_id().map(|s| s.to_string())
                                            {
                                                let res = if is_persistent {
                                                    app_guard.apply_theme_by_id(&id)
                                                } else {
                                                    app_guard.apply_theme_by_id_session_only(&id)
                                                };
                                                match res {
                                                    Ok(_) => app_guard.set_status(format!(
                                                        "Theme set: {}{}",
                                                        id,
                                                        status_suffix(is_persistent)
                                                    )),
                                                    Err(_e) => app_guard.set_status("Theme error"),
                                                }
                                            }
                                            app_guard.picker = None;
                                            app_guard.picker_mode = None;
                                            Some("__picker_handled__".to_string())
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Model)
                                        {
                                            if let Some(id) =
                                                picker.selected_id().map(|s| s.to_string())
                                            {
                                                let persist =
                                                    is_persistent && !app_guard.startup_env_only;
                                                let res = if persist {
                                                    app_guard.apply_model_by_id_persistent(&id)
                                                } else {
                                                    app_guard.apply_model_by_id(&id);
                                                    Ok(())
                                                };
                                                match res {
                                                    Ok(_) => {
                                                        app_guard.set_status(format!(
                                                            "Model set: {}{}",
                                                            id,
                                                            status_suffix(persist)
                                                        ));
                                                        if app_guard.in_provider_model_transition {
                                                            app_guard
                                                                .complete_provider_model_transition(
                                                                );
                                                        }
                                                        if app_guard.startup_requires_model {
                                                            app_guard.startup_requires_model =
                                                                false;
                                                        }
                                                    }
                                                    Err(e) => app_guard
                                                        .set_status(format!("Model error: {}", e)),
                                                }
                                            }
                                            app_guard.picker = None;
                                            app_guard.picker_mode = None;
                                            Some("__picker_handled__".to_string())
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Provider)
                                        {
                                            if let Some(id) =
                                                picker.selected_id().map(|s| s.to_string())
                                            {
                                                let (res, should_open_model_picker) =
                                                    if is_persistent {
                                                        app_guard
                                                            .apply_provider_by_id_persistent(&id)
                                                    } else {
                                                        app_guard.apply_provider_by_id(&id)
                                                    };
                                                match res {
                                                    Ok(_) => {
                                                        app_guard.set_status(format!(
                                                            "Provider set: {}{}",
                                                            id,
                                                            status_suffix(is_persistent)
                                                        ));
                                                        app_guard.picker = None;
                                                        app_guard.picker_mode = None;
                                                        if should_open_model_picker {
                                                            if app_guard.startup_requires_provider {
                                                                app_guard
                                                                    .startup_requires_provider =
                                                                    false;
                                                                app_guard.startup_requires_model =
                                                                    true;
                                                            }
                                                            let app_clone = app.clone();
                                                            tokio::spawn(async move {
                                                                let mut app_guard =
                                                                    app_clone.lock().await;
                                                                let _ = app_guard
                                                                    .open_model_picker()
                                                                    .await;
                                                            });
                                                        }
                                                    }
                                                    Err(e) => {
                                                        app_guard.set_status(format!(
                                                            "Provider error: {}",
                                                            e
                                                        ));
                                                        app_guard.picker = None;
                                                        app_guard.picker_mode = None;
                                                    }
                                                }
                                            }
                                            Some("__picker_handled__".to_string())
                                        } else {
                                            Some("__picker_handled__".to_string())
                                        }
                                    }
                                    // Ctrl+J: persist selection to config (documented only in /help)
                                    KeyCode::Char('j')
                                        if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                                    {
                                        match current_picker_mode {
                                            Some(crate::core::app::PickerMode::Theme) => {
                                                if let Some(id) =
                                                    picker.selected_id().map(|s| s.to_string())
                                                {
                                                    match app_guard.apply_theme_by_id(&id) {
                                                        Ok(_) => app_guard.set_status(format!(
                                                            "Theme set: {}{}",
                                                            id,
                                                            status_suffix(true)
                                                        )),
                                                        Err(_e) => {
                                                            app_guard.set_status("Theme error")
                                                        }
                                                    }
                                                }
                                                app_guard.picker = None;
                                                app_guard.picker_mode = None;
                                                Some("__picker_handled__".to_string())
                                            }
                                            Some(crate::core::app::PickerMode::Model) => {
                                                if let Some(id) =
                                                    picker.selected_id().map(|s| s.to_string())
                                                {
                                                    let persist = !app_guard.startup_env_only;
                                                    let res = if persist {
                                                        app_guard.apply_model_by_id_persistent(&id)
                                                    } else {
                                                        app_guard.apply_model_by_id(&id);
                                                        Ok(())
                                                    };
                                                    match res {
                                                        Ok(_) => {
                                                            app_guard.set_status(format!(
                                                                "Model set: {}{}",
                                                                id,
                                                                status_suffix(persist)
                                                            ));
                                                            if app_guard
                                                                .in_provider_model_transition
                                                            {
                                                                app_guard
                                                                    .complete_provider_model_transition(
                                                                    );
                                                            }
                                                            if app_guard.startup_requires_model {
                                                                app_guard.startup_requires_model =
                                                                    false;
                                                            }
                                                        }
                                                        Err(e) => app_guard.set_status(format!(
                                                            "Model error: {}",
                                                            e
                                                        )),
                                                    }
                                                }
                                                app_guard.picker = None;
                                                app_guard.picker_mode = None;
                                                Some("__picker_handled__".to_string())
                                            }
                                            Some(crate::core::app::PickerMode::Provider) => {
                                                if let Some(id) =
                                                    picker.selected_id().map(|s| s.to_string())
                                                {
                                                    let (res, should_open_model_picker) = app_guard
                                                        .apply_provider_by_id_persistent(&id);
                                                    match res {
                                                        Ok(_) => {
                                                            app_guard.set_status(format!(
                                                                "Provider set: {}{}",
                                                                id,
                                                                status_suffix(true)
                                                            ));
                                                            app_guard.picker = None;
                                                            app_guard.picker_mode = None;
                                                            if should_open_model_picker {
                                                                let app_clone = app.clone();
                                                                tokio::spawn(async move {
                                                                    let mut app_guard =
                                                                        app_clone.lock().await;
                                                                    let _ = app_guard
                                                                        .open_model_picker()
                                                                        .await;
                                                                });
                                                            }
                                                        }
                                                        Err(e) => app_guard.set_status(format!(
                                                            "Provider error: {}",
                                                            e
                                                        )),
                                                    }
                                                }
                                                Some("__picker_handled__".to_string())
                                            }
                                            _ => Some("__picker_handled__".to_string()),
                                        }
                                    }
                                    KeyCode::Delete => {
                                        // Del key to unset defaults - only works if current selection is a default (has *)
                                        if let Some(selected_item) = picker.get_selected_item() {
                                            if selected_item.label.ends_with('*') {
                                                let item_id = selected_item.id.clone();

                                                // Release picker borrow by ending the scope
                                                let _ = picker;

                                                let result = match current_picker_mode {
                                                    Some(crate::core::app::PickerMode::Model) => {
                                                        app_guard
                                                            .unset_default_model(&provider_name)
                                                    }
                                                    Some(crate::core::app::PickerMode::Theme) => {
                                                        app_guard.unset_default_theme()
                                                    }
                                                    Some(
                                                        crate::core::app::PickerMode::Provider,
                                                    ) => app_guard.unset_default_provider(),
                                                    _ => Err("Unknown picker mode".to_string()),
                                                };
                                                match result {
                                                    Ok(_) => {
                                                        app_guard.set_status(format!(
                                                            "Removed default: {}",
                                                            item_id
                                                        ));
                                                        // Refresh the picker to remove the asterisk
                                                        match current_picker_mode {
                                                            Some(crate::core::app::PickerMode::Model) => {
                                                                // Store app reference for async refresh
                                                                let app_clone = app.clone();
                                                                tokio::spawn(async move {
                                                                    let mut app_guard = app_clone.lock().await;
                                                                    let _ = app_guard.open_model_picker().await;
                                                                });
                                                            }
                                                            Some(crate::core::app::PickerMode::Theme) => {
                                                                app_guard.open_theme_picker();
                                                            }
                                                            Some(crate::core::app::PickerMode::Provider) => {
                                                                app_guard.open_provider_picker();
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    Err(e) => {
                                                        app_guard.set_status(format!(
                                                            "Error removing default: {}",
                                                            e
                                                        ));
                                                    }
                                                }
                                            } else {
                                                app_guard.set_status("Del key only works on default items (marked with *)");
                                            }
                                        }
                                        None
                                    }
                                    KeyCode::Backspace => {
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Model)
                                            && !app_guard.model_search_filter.is_empty()
                                        {
                                            app_guard.model_search_filter.pop();
                                            app_guard.filter_models();
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                            && !app_guard.theme_search_filter.is_empty()
                                        {
                                            app_guard.theme_search_filter.pop();
                                            app_guard.filter_themes();
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Provider)
                                            && !app_guard.provider_search_filter.is_empty()
                                        {
                                            app_guard.provider_search_filter.pop();
                                            app_guard.filter_providers();
                                        }
                                        None
                                    }
                                    KeyCode::Char(c) => {
                                        if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Model)
                                        {
                                            // Add character to filter for model picker
                                            if !c.is_control() {
                                                app_guard.model_search_filter.push(c);
                                                app_guard.filter_models();
                                            }
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Theme)
                                        {
                                            // Add character to filter for theme picker
                                            if !c.is_control() {
                                                app_guard.theme_search_filter.push(c);
                                                app_guard.filter_themes();
                                            }
                                        } else if current_picker_mode
                                            == Some(crate::core::app::PickerMode::Provider)
                                        {
                                            // Add character to filter for provider picker
                                            if !c.is_control() {
                                                app_guard.provider_search_filter.push(c);
                                                app_guard.filter_providers();
                                            }
                                        }
                                        None
                                    }
                                    // No block actions in picker modes
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        } {
                            // Apply preview after releasing mutable borrow of picker (theme only)
                            if current_picker_mode == Some(crate::core::app::PickerMode::Theme)
                                && selected_id != "__picker_handled__"
                            {
                                app_guard.preview_theme_by_id(&selected_id);
                            }
                            continue; // handled by picker
                        } else if app_guard.picker.is_some() {
                            continue;
                        }
                    }
                    // Global: Ctrl+B to enter block select mode or cycle upward when active
                    if matches!(key.code, KeyCode::Char('b'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        let mut app_guard = app.lock().await;
                        if !app_guard.markdown_enabled {
                            app_guard.set_status("Markdown disabled (/markdown on)");
                            continue;
                        }
                        let blocks =
                            crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                &app_guard.messages,
                                &app_guard.theme,
                                Some(term_size.width as usize),
                                crate::ui::layout::TableOverflowPolicy::WrapCells,
                                app_guard.syntax_enabled,
                            );
                        if app_guard.block_select_mode {
                            // Cycle upward like Ctrl+P
                            if let Some(cur) = app_guard.selected_block_index {
                                let total = blocks.len();
                                if total > 0 {
                                    let next = if cur == 0 { total - 1 } else { cur - 1 };
                                    app_guard.selected_block_index = Some(next);
                                    if let Some((start, _len, _)) = blocks.get(next) {
                                        let lines = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(&app_guard.messages, &app_guard.theme, app_guard.markdown_enabled, app_guard.syntax_enabled, Some(term_size.width as usize));
                                        let input_area_height =
                                            app_guard.calculate_input_area_height(term_size.width);
                                        let available_height = term_size
                                            .height
                                            .saturating_sub(input_area_height + 2)
                                            .saturating_sub(1);
                                        let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
                                            &lines,
                                            term_size.width,
                                            available_height,
                                            *start,
                                        );
                                        let max_scroll = app_guard.calculate_max_scroll_offset(
                                            available_height,
                                            term_size.width,
                                        );
                                        app_guard.scroll_offset = desired.min(max_scroll);
                                    }
                                }
                            }
                        } else if blocks.is_empty() {
                            app_guard.set_status("No code blocks");
                        } else {
                            // Enter mode and lock input, select most recent block
                            let last = blocks.len().saturating_sub(1);
                            app_guard.enter_block_select_mode(last);
                            if let Some((start, _len, _)) = blocks.get(last) {
                                let lines = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(&app_guard.messages, &app_guard.theme, app_guard.markdown_enabled, app_guard.syntax_enabled, Some(term_size.width as usize));
                                let input_area_height =
                                    app_guard.calculate_input_area_height(term_size.width);
                                let available_height = term_size
                                    .height
                                    .saturating_sub(input_area_height + 2)
                                    .saturating_sub(1);
                                let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
                                                    &lines,
                                                    term_size.width,
                                                    available_height,
                                                    *start,
                                                );
                                let max_scroll = app_guard
                                    .calculate_max_scroll_offset(available_height, term_size.width);
                                app_guard.scroll_offset = desired.min(max_scroll);
                            }
                        }
                        continue;
                    }

                    // Global: Ctrl+P to enter edit-select mode (or cycle upward)
                    if matches!(key.code, KeyCode::Char('p'))
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        let mut app_guard = app.lock().await;
                        if app_guard.edit_select_mode {
                            // Cycle upwards to previous user message (wrap at start)
                            if let Some(current) = app_guard.selected_user_message_index {
                                let next_idx = app_guard
                                    .prev_user_message_index(current)
                                    .or_else(|| app_guard.last_user_message_index());
                                app_guard.selected_user_message_index = next_idx;
                            } else {
                                app_guard.selected_user_message_index =
                                    app_guard.last_user_message_index();
                            }
                        } else {
                            // Enter edit-select mode only if we have user messages
                            if app_guard.last_user_message_index().is_none() {
                                app_guard.set_status("No user messages");
                                continue;
                            }
                            app_guard.enter_edit_select_mode();
                        }
                        if let Some(idx) = app_guard.selected_user_message_index {
                            app_guard.scroll_index_into_view(
                                idx,
                                term_size.width,
                                term_size.height,
                            );
                        }
                        continue;
                    }

                    // When in edit-select mode, handle navigation and actions
                    {
                        let mut app_guard = app.lock().await;
                        if app_guard.edit_select_mode {
                            match key.code {
                                KeyCode::Esc => {
                                    app_guard.exit_edit_select_mode();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if let Some(current) = app_guard.selected_user_message_index {
                                        let prev = app_guard
                                            .prev_user_message_index(current)
                                            .or_else(|| app_guard.last_user_message_index());
                                        if let Some(prev) = prev {
                                            app_guard.selected_user_message_index = Some(prev);
                                            app_guard.scroll_index_into_view(
                                                prev,
                                                term_size.width,
                                                term_size.height,
                                            );
                                        }
                                    } else {
                                        app_guard.selected_user_message_index =
                                            app_guard.last_user_message_index();
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if let Some(current) = app_guard.selected_user_message_index {
                                        let next = app_guard
                                            .next_user_message_index(current)
                                            .or_else(|| app_guard.first_user_message_index());
                                        if let Some(next) = next {
                                            app_guard.selected_user_message_index = Some(next);
                                            app_guard.scroll_index_into_view(
                                                next,
                                                term_size.width,
                                                term_size.height,
                                            );
                                        }
                                    } else {
                                        app_guard.selected_user_message_index =
                                            app_guard.last_user_message_index();
                                    }
                                }
                                KeyCode::Enter => {
                                    // Truncate selected and everything below, put content into input
                                    if let Some(idx) = app_guard.selected_user_message_index {
                                        if idx < app_guard.messages.len()
                                            && app_guard.messages[idx].role == "user"
                                        {
                                            let content = app_guard.messages[idx].content.clone();
                                            // Cancel any active stream
                                            app_guard.cancel_current_stream();
                                            // Truncate from selected index (drops selected and below)
                                            app_guard.messages.truncate(idx);
                                            app_guard.invalidate_prewrap_cache();
                                            // Rewrite log file to reflect truncation
                                            let _ = app_guard
                                                .logging
                                                .rewrite_log_without_last_response(
                                                    &app_guard.messages,
                                                );
                                            // Put content into input for editing
                                            app_guard.set_input_text(content);
                                            // Exit selection mode
                                            app_guard.exit_edit_select_mode();
                                            // Scroll to bottom of remaining messages
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                        }
                                    }
                                }
                                KeyCode::Char('E') | KeyCode::Char('e') => {
                                    // Edit in place: populate input with content, do NOT truncate
                                    if let Some(idx) = app_guard.selected_user_message_index {
                                        if idx < app_guard.messages.len()
                                            && app_guard.messages[idx].role == "user"
                                        {
                                            let content = app_guard.messages[idx].content.clone();
                                            app_guard.set_input_text(content);
                                            app_guard.start_in_place_edit(idx);
                                            app_guard.exit_edit_select_mode();
                                        }
                                    }
                                }
                                KeyCode::Delete => {
                                    // Delete selected and everything below; do not populate input
                                    if let Some(idx) = app_guard.selected_user_message_index {
                                        if idx < app_guard.messages.len()
                                            && app_guard.messages[idx].role == "user"
                                        {
                                            // Cancel any active stream
                                            app_guard.cancel_current_stream();
                                            app_guard.messages.truncate(idx);
                                            app_guard.invalidate_prewrap_cache();
                                            let _ = app_guard
                                                .logging
                                                .rewrite_log_without_last_response(
                                                    &app_guard.messages,
                                                );
                                            app_guard.exit_edit_select_mode();
                                            // Scroll to bottom of remaining messages
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                            continue; // handled edit-select mode
                        }
                    }

                    // When in block-select mode, handle navigation and actions
                    {
                        let mut app_guard = app.lock().await;
                        if app_guard.block_select_mode {
                            match key.code {
                                KeyCode::Esc => {
                                    app_guard.exit_block_select_mode();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if let Some(cur) = app_guard.selected_block_index {
                                        let total = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                            &app_guard.messages,
                                            &app_guard.theme,
                                            Some(term_size.width as usize),
                                            crate::ui::layout::TableOverflowPolicy::WrapCells,
                                            app_guard.syntax_enabled,
                                        )
                                        .len();
                                        if total > 0 {
                                            let next = if cur == 0 { total - 1 } else { cur - 1 };
                                            app_guard.selected_block_index = Some(next);
                                            // Scroll to block start
                                            let ranges =
                                                crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                                    &app_guard.messages,
                                                    &app_guard.theme,
                                                    Some(term_size.width as usize),
                                                    crate::ui::layout::TableOverflowPolicy::WrapCells,
                                                    app_guard.syntax_enabled,
                                                );
                                            if let Some((start, _len, _)) = ranges.get(next) {
                                                let lines = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(&app_guard.messages, &app_guard.theme, app_guard.markdown_enabled, app_guard.syntax_enabled, Some(term_size.width as usize));
                                                let input_area_height = app_guard
                                                    .calculate_input_area_height(term_size.width);
                                                let available_height = term_size
                                                    .height
                                                    .saturating_sub(input_area_height + 2)
                                                    .saturating_sub(1);
                                                let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
                                                    &lines,
                                                    term_size.width,
                                                    available_height,
                                                    *start,
                                                );
                                                let max_scroll = app_guard
                                                    .calculate_max_scroll_offset(
                                                        available_height,
                                                        term_size.width,
                                                    );
                                                app_guard.scroll_offset = desired.min(max_scroll);
                                            }
                                        }
                                    } else {
                                        app_guard.selected_block_index = Some(0);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if let Some(cur) = app_guard.selected_block_index {
                                        let total = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                            &app_guard.messages,
                                            &app_guard.theme,
                                            Some(term_size.width as usize),
                                            crate::ui::layout::TableOverflowPolicy::WrapCells,
                                            app_guard.syntax_enabled,
                                        )
                                        .len();
                                        if total > 0 {
                                            let next = (cur + 1) % total;
                                            app_guard.selected_block_index = Some(next);
                                            // Scroll to block start
                                            let ranges =
                                                crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                                    &app_guard.messages,
                                                    &app_guard.theme,
                                                    Some(term_size.width as usize),
                                                    crate::ui::layout::TableOverflowPolicy::WrapCells,
                                                    app_guard.syntax_enabled,
                                                );
                                            if let Some((start, _len, _)) = ranges.get(next) {
                                                let lines = crate::utils::scroll::ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(&app_guard.messages, &app_guard.theme, app_guard.markdown_enabled, app_guard.syntax_enabled, Some(term_size.width as usize));
                                                let input_area_height = app_guard
                                                    .calculate_input_area_height(term_size.width);
                                                let available_height = term_size
                                                    .height
                                                    .saturating_sub(input_area_height + 2)
                                                    .saturating_sub(1);
                                                let desired = crate::utils::scroll::ScrollCalculator::scroll_offset_to_line_start(
                                    &lines,
                                    term_size.width,
                                    available_height,
                                    *start,
                                );
                                                let max_scroll = app_guard
                                                    .calculate_max_scroll_offset(
                                                        available_height,
                                                        term_size.width,
                                                    );
                                                app_guard.scroll_offset = desired.min(max_scroll);
                                            }
                                        }
                                    } else {
                                        app_guard.selected_block_index = Some(0);
                                    }
                                }
                                KeyCode::Char('c') | KeyCode::Char('C') => {
                                    if let Some(cur) = app_guard.selected_block_index {
                                        let ranges = crate::ui::markdown::compute_codeblock_ranges_with_width_and_policy(
                                            &app_guard.messages,
                                            &app_guard.theme,
                                            Some(term_size.width as usize),
                                            crate::ui::layout::TableOverflowPolicy::WrapCells,
                                            app_guard.syntax_enabled,
                                        );
                                        if let Some((_start, _len, content)) = ranges.get(cur) {
                                            match crate::utils::clipboard::copy_to_clipboard(
                                                content,
                                            ) {
                                                Ok(()) => app_guard.set_status("Copied code block"),
                                                Err(_e) => app_guard.set_status("Clipboard error"),
                                            }
                                            // Leave block-select mode and scroll to bottom
                                            app_guard.exit_block_select_mode();
                                            app_guard.auto_scroll = true;
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = term_size
                                                .height
                                                .saturating_sub(input_area_height + 2)
                                                .saturating_sub(1);
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                        }
                                    }
                                }
                                KeyCode::Char('s') | KeyCode::Char('S') => {
                                    if let Some(cur) = app_guard.selected_block_index {
                                        let contents = crate::ui::markdown::compute_codeblock_contents_with_lang(&app_guard.messages);
                                        if let Some((content, lang)) = contents.get(cur) {
                                            use chrono::Utc;
                                            use std::fs;
                                            let date = Utc::now().format("%Y-%m-%d");
                                            let ext = language_to_extension(lang.as_deref());
                                            let filename =
                                                format!("chabeau-block-{}.{}", date, ext);
                                            // If the file exists, open a filename prompt; otherwise save immediately.
                                            if std::path::Path::new(&filename).exists() {
                                                app_guard.set_status("File already exists.");
                                                app_guard.start_file_prompt_save_block(
                                                    filename,
                                                    content.clone(),
                                                );
                                            } else {
                                                match fs::write(&filename, content) {
                                                    Ok(()) => app_guard.set_status(format!(
                                                        "Saved to {}",
                                                        filename
                                                    )),
                                                    Err(_e) => app_guard
                                                        .set_status("Error saving code block"),
                                                }
                                            }
                                            // Exit block-select mode (return to regular input)
                                            app_guard.exit_block_select_mode();
                                            // Scroll to bottom after action
                                            app_guard.auto_scroll = true;
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = term_size
                                                .height
                                                .saturating_sub(input_area_height + 2)
                                                .saturating_sub(1);
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }
                    }
                    match key.code {
                        KeyCode::Home => {
                            let mut app_guard = app.lock().await;
                            app_guard.scroll_to_top();
                        }
                        KeyCode::End => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.scroll_to_bottom_view(available_height, term_size.width);
                        }
                        KeyCode::PageUp => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.page_up(available_height);
                        }
                        KeyCode::PageDown => {
                            let mut app_guard = app.lock().await;
                            let input_area_height =
                                app_guard.calculate_input_area_height(term_size.width);
                            let available_height = term_size
                                .height
                                .saturating_sub(input_area_height + 2)
                                .saturating_sub(1);
                            app_guard.page_down(available_height, term_size.width);
                        }
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            break 'main_loop Ok(());
                        }
                        KeyCode::Char('d')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+D: exit if input is empty, else delete forward
                            let mut app_guard = app.lock().await;
                            if app_guard.get_input_text().is_empty() {
                                break 'main_loop Ok(());
                            } else {
                                app_guard.apply_textarea_edit_and_recompute(
                                    term_size.width,
                                    |ta| {
                                        ta.input_without_shortcuts(TAInput {
                                            key: TAKey::Delete,
                                            ctrl: false,
                                            alt: false,
                                            shift: false,
                                        });
                                    },
                                );
                            }
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

                                    // Send the message to API (deduplicated helper)
                                    spawn_stream(StreamParams {
                                        client,
                                        base_url,
                                        api_key,
                                        model,
                                        api_messages,
                                        cancel_token,
                                        stream_id,
                                        tx: tx.clone(),
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
                                    app_guard.set_status(format!("Editor error: {}", e));
                                    // Keep view stable; brief corner status is sufficient
                                    let terminal_size = terminal.size().unwrap_or_default();
                                    let input_area_height =
                                        app_guard.calculate_input_area_height(terminal_size.width);
                                    let available_height = terminal_size
                                        .height
                                        .saturating_sub(input_area_height + 2)
                                        .saturating_sub(1);
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
                                                let chat_url = construct_api_url(&base_url, "chat/completions");
                                                match client
                                                    .post(chat_url)
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
                            if app_guard.file_prompt.is_some() {
                                app_guard.cancel_file_prompt();
                                continue;
                            }
                            if app_guard.edit_select_mode {
                                app_guard.exit_edit_select_mode();
                                continue;
                            }
                            if app_guard.in_place_edit_index.is_some() {
                                app_guard.cancel_in_place_edit();
                                app_guard.clear_input();
                                continue;
                            }
                            if app_guard.is_streaming {
                                // Use the new cancellation mechanism
                                app_guard.cancel_current_stream();
                            }
                        }
                        KeyCode::Enter => {
                            let modifiers = key.modifiers;
                            // Handle filename prompt (Enter: save if new; Alt+Enter: overwrite)
                            {
                                let mut app_guard = app.lock().await;
                                if let Some(prompt) = app_guard.file_prompt.clone() {
                                    let filename = app_guard.get_input_text().trim().to_string();
                                    if filename.is_empty() {
                                        continue;
                                    }
                                    let overwrite = modifiers.contains(event::KeyModifiers::ALT);
                                    match prompt.kind {
                                        crate::core::app::FilePromptKind::Dump => {
                                            // Use commands helper to dump
                                            let res =
                                                crate::commands::dump_conversation_with_overwrite(
                                                    &app_guard, &filename, overwrite,
                                                );
                                            match res {
                                                Ok(()) => {
                                                    app_guard.set_status(format!(
                                                        "Dumped: {}",
                                                        filename
                                                    ));
                                                    app_guard.cancel_file_prompt();
                                                }
                                                Err(e) => {
                                                    let msg = e.to_string();
                                                    if msg.contains("already exists") {
                                                        app_guard
                                                            .set_status("Log file already exists.");
                                                    } else {
                                                        app_guard.set_status(format!(
                                                            "Dump error: {}",
                                                            msg
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        crate::core::app::FilePromptKind::SaveCodeBlock => {
                                            use std::fs;
                                            let exists = std::path::Path::new(&filename).exists();
                                            if exists && !overwrite {
                                                app_guard.set_status("File already exists.");
                                            } else if let Some(content) = prompt.content {
                                                match fs::write(&filename, content) {
                                                    Ok(()) => {
                                                        app_guard.set_status(format!(
                                                            "Saved to {}",
                                                            filename
                                                        ));
                                                        app_guard.cancel_file_prompt();
                                                    }
                                                    Err(_e) => {
                                                        app_guard
                                                            .set_status("Error saving code block");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                            }
                            // Compose/newline logic:
                            // - Compose mode: Enter inserts newline; Alt+Enter sends
                            // - Normal mode: Alt+Enter inserts newline; Enter sends
                            {
                                let app_guard = app.lock().await;
                                let compose = app_guard.compose_mode;
                                let alt = modifiers.contains(event::KeyModifiers::ALT);
                                drop(app_guard);
                                let should_insert_newline = if compose { !alt } else { alt };
                                if should_insert_newline {
                                    let mut app_guard = app.lock().await;
                                    app_guard.apply_textarea_edit_and_recompute(
                                        term_size.width,
                                        |ta| {
                                            ta.insert_str("\n");
                                        },
                                    );
                                    continue;
                                }
                            }
                            {
                                // If editing in place, apply changes to history instead of sending
                                {
                                    let mut app_guard = app.lock().await;
                                    if let Some(idx) = app_guard.in_place_edit_index.take() {
                                        // Apply edit to the selected user message
                                        if idx < app_guard.messages.len()
                                            && app_guard.messages[idx].role == "user"
                                        {
                                            let new_text = app_guard.get_input_text().to_string();
                                            app_guard.messages[idx].content = new_text;
                                            app_guard.invalidate_prewrap_cache();
                                            // Rewrite log file to reflect in-place edit
                                            let _ = app_guard
                                                .logging
                                                .rewrite_log_without_last_response(
                                                    &app_guard.messages,
                                                );
                                        }
                                        app_guard.clear_input();
                                        continue;
                                    }
                                }
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
                                    if app_guard.get_input_text().trim().is_empty() {
                                        continue;
                                    }

                                    let input_text = app_guard.get_input_text().to_string();
                                    app_guard.clear_input();

                                    // Process input for commands
                                    match process_input(&mut app_guard, &input_text) {
                                        CommandResult::Continue => {
                                            // Command was processed, don't send to API
                                            // Update scroll position to ensure latest messages are visible
                                            let term_size = terminal.size().unwrap_or_default();
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                term_size.width,
                                            );
                                            continue;
                                        }
                                        CommandResult::OpenModelPicker => {
                                            // Open model picker asynchronously
                                            match app_guard.open_model_picker().await {
                                                Ok(_) => {
                                                    // Status messages not needed - help is shown in-dialog
                                                }
                                                Err(e) => {
                                                    app_guard.set_status(format!(
                                                        "Model picker error: {}",
                                                        e
                                                    ));
                                                }
                                            }
                                            continue;
                                        }
                                        CommandResult::OpenProviderPicker => {
                                            // Open provider picker
                                            app_guard.open_provider_picker();
                                            // Status messages not needed - help is shown in-dialog
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
                                            let input_area_height = app_guard
                                                .calculate_input_area_height(term_size.width);
                                            let available_height = app_guard
                                                .calculate_available_height(
                                                    term_size.height,
                                                    input_area_height,
                                                );
                                            app_guard.update_scroll_position(
                                                available_height,
                                                terminal.size().unwrap_or_default().width,
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

                                spawn_stream(StreamParams {
                                    client,
                                    base_url,
                                    api_key,
                                    model,
                                    api_messages,
                                    cancel_token,
                                    stream_id,
                                    tx: tx.clone(),
                                });
                            }
                        }
                        // Ctrl+J: newline in normal mode; send in compose mode
                        KeyCode::Char('j')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            let send_now = {
                                let app_guard = app.lock().await;
                                app_guard.compose_mode && app_guard.file_prompt.is_none()
                            };
                            if !send_now {
                                let mut app_guard = app.lock().await;
                                app_guard.apply_textarea_edit_and_recompute(
                                    term_size.width,
                                    |ta| {
                                        ta.insert_str("\n");
                                    },
                                );
                                last_input_layout_update = Instant::now();
                                continue;
                            }
                            // Send path (same as Enter send)
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
                                if app_guard.get_input_text().trim().is_empty() {
                                    continue;
                                }

                                let input_text = app_guard.get_input_text().to_string();
                                app_guard.clear_input();

                                match process_input(&mut app_guard, &input_text) {
                                    CommandResult::Continue => {
                                        let term_size = terminal.size().unwrap_or_default();
                                        let input_area_height =
                                            app_guard.calculate_input_area_height(term_size.width);
                                        let available_height = app_guard
                                            .calculate_available_height(
                                                term_size.height,
                                                input_area_height,
                                            );
                                        app_guard.update_scroll_position(
                                            available_height,
                                            term_size.width,
                                        );
                                        continue;
                                    }
                                    CommandResult::OpenModelPicker => {
                                        match app_guard.open_model_picker().await {
                                            Ok(_) => {}
                                            Err(e) => app_guard
                                                .set_status(format!("Model picker error: {}", e)),
                                        }
                                        continue;
                                    }
                                    CommandResult::OpenProviderPicker => {
                                        app_guard.open_provider_picker();
                                        continue;
                                    }
                                    CommandResult::ProcessAsMessage(message) => {
                                        app_guard.auto_scroll = true;
                                        let (cancel_token, stream_id) =
                                            app_guard.start_new_stream();
                                        let api_messages = app_guard.add_user_message(message);
                                        let input_area_height =
                                            app_guard.calculate_input_area_height(term_size.width);
                                        let available_height = app_guard
                                            .calculate_available_height(
                                                term_size.height,
                                                input_area_height,
                                            );
                                        app_guard.update_scroll_position(
                                            available_height,
                                            terminal.size().unwrap_or_default().width,
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
                            spawn_stream(StreamParams {
                                client,
                                base_url,
                                api_key,
                                model,
                                api_messages,
                                cancel_token,
                                stream_id,
                                tx: tx.clone(),
                            });
                        }
                        KeyCode::Char('a')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Forward to textarea (beginning of line)
                            let mut app_guard = app.lock().await;
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input(TAInput::from(key));
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Char('e')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            // Forward to textarea (end of line)
                            let mut app_guard = app.lock().await;
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input(TAInput::from(key));
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Left => {
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = key.modifiers.contains(event::KeyModifiers::SHIFT);
                            if (compose && !shift) || (!compose && shift) {
                                // Move exactly one character left (ignore selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Back));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll left
                                app_guard.horizontal_scroll_offset =
                                    app_guard.horizontal_scroll_offset.saturating_sub(1);
                            }
                        }
                        KeyCode::Right => {
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = key.modifiers.contains(event::KeyModifiers::SHIFT);
                            if (compose && !shift) || (!compose && shift) {
                                // Move exactly one character right (ignore selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Forward));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll right
                                app_guard.horizontal_scroll_offset =
                                    app_guard.horizontal_scroll_offset.saturating_add(1);
                            }
                        }
                        KeyCode::Char(_) => {
                            let mut app_guard = app.lock().await;
                            // Let textarea handle text input, including multi-byte chars
                            app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                                ta.input(TAInput::from(key));
                            });
                        }
                        KeyCode::Delete => {
                            let mut app_guard = app.lock().await;
                            // Forward delete in input area
                            app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                                ta.input_without_shortcuts(TAInput {
                                    key: TAKey::Delete,
                                    ctrl: false,
                                    alt: false,
                                    shift: false,
                                });
                            });
                        }
                        KeyCode::Backspace => {
                            let mut app_guard = app.lock().await;
                            // Use input_without_shortcuts to ensure Backspace always deletes a single char/newline
                            let input = TAInput::from(key);
                            app_guard.apply_textarea_edit(|ta| {
                                ta.input_without_shortcuts(input);
                            });
                            update_if_due(&mut app_guard);
                        }
                        KeyCode::Up => {
                            let modifiers = key.modifiers;
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = modifiers.contains(event::KeyModifiers::SHIFT);

                            if (compose && !shift) || (!compose && shift) {
                                // Move cursor up exactly one line (no selection)
                                app_guard.apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Up));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll chat history up
                                app_guard.auto_scroll = false;
                                app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let modifiers = key.modifiers;
                            let mut app_guard = app.lock().await;
                            let compose = app_guard.compose_mode;
                            let shift = modifiers.contains(event::KeyModifiers::SHIFT);

                            if (compose && !shift) || (!compose && shift) {
                                // Move cursor down exactly one line (no selection)
                                app_guard
                                    .apply_textarea_edit(|ta| ta.move_cursor(CursorMove::Down));
                                update_if_due(&mut app_guard);
                            } else {
                                // Scroll chat history down
                                app_guard.auto_scroll = false;
                                let input_area_height =
                                    app_guard.calculate_input_area_height(term_size.width);
                                let available_height = term_size
                                    .height
                                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                                    .saturating_sub(1); // 1 for title
                                let max_scroll = app_guard
                                    .calculate_max_scroll_offset(available_height, term_size.width);
                                app_guard.scroll_offset =
                                    (app_guard.scroll_offset.saturating_add(1)).min(max_scroll);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    // Handle paste events - sanitize and add the pasted text to input
                    let mut app_guard = app.lock().await;

                    // Sanitize the pasted text to prevent TUI corruption
                    // Convert tabs to spaces and carriage returns to newlines
                    let sanitized_text = text
                        .replace('\t', "    ") // Convert tabs to 4 spaces
                        .replace('\r', "\n") // Convert carriage returns to newlines
                        .chars()
                        .filter(|&c| {
                            // Allow printable characters and newlines, filter out other control characters
                            c == '\n' || !c.is_control()
                        })
                        .collect::<String>();
                    app_guard.apply_textarea_edit_and_recompute(term_size.width, |ta| {
                        ta.insert_str(&sanitized_text);
                    });
                    last_input_layout_update = Instant::now();
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let mut app_guard = app.lock().await;
                        app_guard.auto_scroll = false;
                        app_guard.scroll_offset = app_guard.scroll_offset.saturating_sub(3);
                    }
                    MouseEventKind::ScrollDown => {
                        let mut app_guard = app.lock().await;
                        app_guard.auto_scroll = false;
                        let input_area_height = app_guard
                            .calculate_input_area_height(terminal.size().unwrap_or_default().width);
                        let available_height = terminal
                            .size()
                            .unwrap_or_default()
                            .height
                            .saturating_sub(input_area_height + 2)
                            .saturating_sub(1);
                        let max_scroll = app_guard.calculate_max_scroll_offset(
                            available_height,
                            terminal.size().unwrap_or_default().width,
                        );
                        app_guard.scroll_offset =
                            (app_guard.scroll_offset.saturating_add(3)).min(max_scroll);
                    }
                    _ => {}
                },
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
            } else if let Some(err) = content.strip_prefix("<<API_ERROR>>") {
                // Display API/network error in the chat area as a system message
                let error_message = format!("Error: {}", err.trim());
                app_guard.add_system_message(error_message);
                // Stop streaming state, since the request failed
                app_guard.is_streaming = false;
                // Ensure the new system message is visible
                let input_area_height = app_guard.calculate_input_area_height(term_size.width);
                let available_height = term_size
                    .height
                    .saturating_sub(input_area_height + 2)
                    .saturating_sub(1);
                app_guard.update_scroll_position(available_height, term_size.width);
                drop(app_guard);
                received_any = true;
            } else {
                let input_area_height = app_guard.calculate_input_area_height(term_size.width);
                let available_height = term_size
                    .height
                    .saturating_sub(input_area_height + 2) // Dynamic input area + borders
                    .saturating_sub(1); // 1 for title
                app_guard.append_to_response(&content, available_height, term_size.width);
                drop(app_guard);
                received_any = true;
            }
        }
        if received_any {
            continue; // Force a redraw after processing all updates
        }

        // End of loop tick: log if this frame was slow
        // end of iteration
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
