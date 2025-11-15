//! TUI-less "say" command

use std::error::Error;
use std::io::{self, IsTerminal, Read, Write};

use crate::auth::{AuthManager, ProviderAuthStatus};
use crate::character::CharacterService;
use crate::core::app::session::{
    exit_if_env_only_missing_env, exit_with_provider_resolution_error,
};
use crate::core::app::{self};
use crate::core::chat_stream::{ChatStreamService, StreamMessage};
use crate::core::config::data::Config;
use crate::core::message::AppMessageKind;
use crate::core::providers::ProviderResolutionError;
use crate::ui::osc;
use ratatui::crossterm::cursor::{MoveToColumn, MoveUp};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, Clear, ClearType};
use ratatui::text::Line;

// Tracks newline bookkeeping for plain (non-terminal) output. This allows the
// redirected mode to mimic terminal rendering without relying on ANSI control
// codes.
#[derive(Default)]
struct PlainStreamState {
    started: bool,
    last_chunk_ended_with_newline: bool,
}

impl PlainStreamState {
    fn new() -> Self {
        Self {
            started: false,
            last_chunk_ended_with_newline: true,
        }
    }

    fn write_prefix<W: Write>(&mut self, writer: &mut W, lines: &[String]) -> io::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }

        if lines.len() == 1 {
            writer.write_all(lines[0].as_bytes())?;
        } else {
            for line in lines.iter().take(lines.len() - 1) {
                writeln!(writer, "{}", line)?;
            }
            writer.write_all(lines.last().unwrap().as_bytes())?;
        }

        writer.flush()?;
        self.started = true;
        self.last_chunk_ended_with_newline = false;
        Ok(())
    }

    fn write_chunk<W: Write>(&mut self, writer: &mut W, content: &str) -> io::Result<()> {
        writer.write_all(content.as_bytes())?;
        writer.flush()?;
        if !content.is_empty() {
            self.started = true;
            self.last_chunk_ended_with_newline = content.ends_with('\n');
        }
        Ok(())
    }

    fn write_line<W: Write>(&mut self, writer: &mut W, content: &str) -> io::Result<()> {
        if self.started && !self.last_chunk_ended_with_newline {
            writer.write_all(b"\n")?;
        }
        writeln!(writer, "{}", content)?;
        writer.flush()?;
        self.started = true;
        self.last_chunk_ended_with_newline = true;
        Ok(())
    }

    fn ensure_trailing_newline<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        if self.started && !self.last_chunk_ended_with_newline {
            writer.write_all(b"\n")?;
            writer.flush()?;
            self.last_chunk_ended_with_newline = true;
        }
        Ok(())
    }
}

// Switches between terminal redraw mode (OSC encoded, multi-line diffing) and a
// simple newline based stream suitable for redirected stdout.
enum OutputMode {
    Terminal { previous_lines: Vec<String> },
    Plain { state: PlainStreamState },
}

// Produces OSC-encoded terminal lines for the app's current content at the
// specified terminal width.
fn encoded_terminal_lines(app: &mut app::App, term_width: u16) -> Vec<String> {
    let metadata = app.get_prewrapped_span_metadata_cached(term_width).clone();
    let lines = app.get_prewrapped_lines_cached(term_width).clone();
    osc::encode_lines_with_links_with_underline(&lines, &metadata)
}

// Re-computes the OSC-encoded lines and redraws any changed content while
// minimizing cursor movement.
fn redraw_terminal_lines(
    app: &mut app::App,
    term_width: u16,
    stdout: &mut io::Stdout,
    previous_lines: &mut Vec<String>,
    persist: bool,
) -> io::Result<()> {
    let new_lines = encoded_terminal_lines(app, term_width);

    let mut common_prefix_len = 0usize;
    let max_prefix = previous_lines.len().min(new_lines.len());
    while common_prefix_len < max_prefix
        && previous_lines[common_prefix_len] == new_lines[common_prefix_len]
    {
        common_prefix_len += 1;
    }

    if previous_lines.len() > common_prefix_len {
        let lines_to_move_up = (previous_lines.len() - common_prefix_len) as u16;
        if lines_to_move_up > 0 {
            execute!(stdout, MoveUp(lines_to_move_up))?;
        }
    }

    for line in new_lines.iter().skip(common_prefix_len) {
        execute!(stdout, Clear(ClearType::CurrentLine), MoveToColumn(0))?;
        println!("{}", line);
    }

    stdout.flush()?;
    if persist {
        *previous_lines = new_lines;
    }

    Ok(())
}

impl OutputMode {
    fn new(stdout_is_terminal: bool) -> Self {
        if stdout_is_terminal {
            Self::Terminal {
                previous_lines: Vec::new(),
            }
        } else {
            Self::Plain {
                state: PlainStreamState::new(),
            }
        }
    }

    // Print the conversation prefix before streaming begins so both terminal
    // and plain modes start from the same baseline.
    fn render_prefix(
        &mut self,
        app: &mut app::App,
        term_width: u16,
        stdout: &mut io::Stdout,
    ) -> io::Result<()> {
        match self {
            OutputMode::Terminal { previous_lines } => {
                let encoded = encoded_terminal_lines(app, term_width);
                for line in &encoded {
                    println!("{}", line);
                }
                stdout.flush()?;
                *previous_lines = encoded;
                Ok(())
            }
            OutputMode::Plain { state } => {
                let lines = app.get_prewrapped_lines_cached(term_width).clone();
                let plain_lines = plain_text_lines(&lines);
                state.write_prefix(stdout, &plain_lines)
            }
        }
    }

    // Stream assistant tokens as they arrive.
    fn on_chunk(
        &mut self,
        content: &str,
        app: &mut app::App,
        term_width: u16,
        stdout: &mut io::Stdout,
    ) -> io::Result<()> {
        match self {
            OutputMode::Terminal { previous_lines } => {
                redraw_terminal_lines(app, term_width, stdout, previous_lines, true)
            }
            OutputMode::Plain { state } => state.write_chunk(stdout, content),
        }
    }

    // Render API errors inline, ensuring terminal mode preserves previous
    // successful output.
    fn on_error(
        &mut self,
        error: &str,
        app: &mut app::App,
        term_width: u16,
        stdout: &mut io::Stdout,
    ) -> io::Result<()> {
        match self {
            OutputMode::Terminal { previous_lines } => {
                redraw_terminal_lines(app, term_width, stdout, previous_lines, false)
            }
            OutputMode::Plain { state } => state.write_line(stdout, error),
        }
    }

    fn finish(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        match self {
            OutputMode::Terminal { .. } => Ok(()),
            OutputMode::Plain { state } => state.ensure_trailing_newline(stdout),
        }
    }
}

fn plain_text_lines(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut buf = String::new();
            for span in &line.spans {
                buf.push_str(span.content.as_ref());
            }
            buf
        })
        .collect()
}

fn resolve_prompt_from_args(
    mut prompt: String,
    stdin: &mut dyn Read,
    stdin_is_terminal: bool,
) -> io::Result<Option<String>> {
    if prompt.trim().is_empty() && !stdin_is_terminal {
        let mut buffer = String::new();
        stdin.read_to_string(&mut buffer)?;
        let trimmed = buffer.trim_end().to_string();
        if !trimmed.is_empty() {
            prompt = trimmed;
        }
    }

    if prompt.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(prompt))
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_say(
    prompt: Vec<String>,
    model: Option<String>,
    provider: Option<String>,
    env_only: bool,
    character: Option<String>,
    persona: Option<String>,
    preset: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let mut stdin = io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    let prompt = match resolve_prompt_from_args(prompt.join(" "), &mut stdin, stdin_is_terminal)? {
        Some(prompt) => prompt,
        None => {
            eprintln!("Usage: chabeau say <prompt>");
            std::process::exit(1);
        }
    };

    // Configuration and auth need to be loaded before we can send any request.
    let config = Config::load()?;
    let auth_manager = AuthManager::new()?;

    if provider.is_none() && config.default_provider.is_none() {
        let (providers, _) = auth_manager.get_all_providers_with_auth_status();
        let configured_providers: Vec<_> = providers
            .into_iter()
            .filter(|ProviderAuthStatus { has_token, .. }| *has_token)
            .map(|ProviderAuthStatus { id, .. }| id)
            .collect();
        if configured_providers.len() > 1 {
            eprintln!(
                "Multiple providers are configured. Please specify a provider with the -p flag."
            );
            eprintln!("Available providers: {}", configured_providers.join(", "));
            std::process::exit(1);
        }
    }

    let character_service = CharacterService::new();

    exit_if_env_only_missing_env(env_only);

    let mut app = app::new_with_auth(
        app::AppInitConfig {
            model: model.unwrap_or_else(|| "default".to_string()),
            log_file: None,
            provider,
            env_only,
            pre_resolved_session: None,
            character,
            persona,
            preset,
        },
        &config,
        character_service,
    )
    .await
    .inspect_err(|err| {
        if let Some(provider_err) = err.downcast_ref::<ProviderResolutionError>() {
            exit_with_provider_resolution_error(provider_err);
        }
    })?;

    // The first render needs an initial width to prewrap conversation lines.
    let (term_width, _) = terminal::size().unwrap_or((80, 24));

    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(prompt.clone());
        (cancel_token, stream_id, api_messages)
    };

    let params = app.build_stream_params(api_messages, cancel_token, stream_id);

    // Begin streaming completions on a background task and listen for chunks
    // over the channel receiver.
    let (stream_service, mut rx) = ChatStreamService::new();
    stream_service.spawn_stream(params);

    let mut stdout = io::stdout();
    let stdout_is_terminal = stdout.is_terminal();
    let mut output_mode = OutputMode::new(stdout_is_terminal);
    output_mode.render_prefix(&mut app, term_width, &mut stdout)?;

    // Drive the stream until completion, flushing chunks to stdout as they
    // arrive.
    loop {
        match rx.recv().await {
            Some((StreamMessage::Chunk(content), _)) => {
                let (term_width, term_height) = terminal::size().unwrap_or((80, 24));
                {
                    let mut conversation = app.conversation();
                    let available_height = conversation.calculate_available_height(term_height, 0);
                    conversation.append_to_response(&content, available_height, term_width);
                }

                output_mode.on_chunk(&content, &mut app, term_width, &mut stdout)?;
            }
            Some((StreamMessage::Error(err), _)) => {
                let trimmed = err.trim();
                if trimmed.is_empty() {
                    std::process::exit(1);
                }

                let (term_width, term_height) = terminal::size().unwrap_or((80, 24));
                {
                    let mut conversation = app.conversation();
                    conversation.remove_trailing_empty_assistant_messages();
                    conversation.add_app_message(AppMessageKind::Error, trimmed.to_string());
                    let available_height = conversation.calculate_available_height(term_height, 0);
                    conversation.update_scroll_position(available_height, term_width);
                }

                output_mode.on_error(trimmed, &mut app, term_width, &mut stdout)?;
                std::process::exit(1);
            }
            Some((StreamMessage::End, _)) => {
                let mut conversation = app.conversation();
                conversation.finalize_response();
                output_mode.finish(&mut stdout)?;
                break;
            }
            None => break,
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{plain_text_lines, resolve_prompt_from_args, PlainStreamState};
    use crate::ui::osc;
    use crate::ui::span::SpanKind;
    use ratatui::text::{Line, Span};
    use std::io::Cursor;

    #[test]
    fn resolve_prompt_prefers_cli_args() {
        let mut stdin = Cursor::new("ignored");
        let prompt = resolve_prompt_from_args("hello world".into(), &mut stdin, false)
            .expect("prompt should resolve");
        assert_eq!(prompt, Some("hello world".into()));
    }

    #[test]
    fn resolve_prompt_reads_from_piped_stdin() {
        let mut stdin = Cursor::new("hello from pipe\n\n");
        let prompt = resolve_prompt_from_args(String::new(), &mut stdin, false)
            .expect("prompt should resolve");
        assert_eq!(prompt, Some("hello from pipe".into()));
    }

    #[test]
    fn resolve_prompt_handles_empty_sources() {
        let mut stdin = Cursor::new("");
        let prompt = resolve_prompt_from_args(String::new(), &mut stdin, false)
            .expect("prompt should resolve");
        assert!(prompt.is_none());
    }

    #[test]
    fn plain_prefix_rendering_stays_escape_free_when_redirected() {
        let lines = vec![Line::from(vec![Span::raw("Docs"), Span::raw(" link")])];
        let metadata = vec![vec![SpanKind::link("https://example.com"), SpanKind::Text]];
        let encoded = osc::encode_lines_with_links_with_underline(&lines, &metadata);
        assert!(encoded[0].contains('\x1b'));

        let plain_lines = plain_text_lines(&lines);
        let mut buffer = Vec::new();
        let mut state = PlainStreamState::new();
        state
            .write_prefix(&mut buffer, &plain_lines)
            .expect("should write prefix");
        let captured = String::from_utf8(buffer).expect("valid utf8");
        assert!(!captured.contains('\x1b'));
    }

    #[test]
    fn plain_stream_state_inserts_newline_for_redirected_streams() {
        let mut state = PlainStreamState::new();
        let mut buffer = Vec::new();
        state
            .write_chunk(&mut buffer, "partial response")
            .expect("chunk should write");
        state
            .ensure_trailing_newline(&mut buffer)
            .expect("should insert newline");
        let captured = String::from_utf8(buffer).expect("valid utf8");
        assert_eq!(captured, "partial response\n");
    }

    #[test]
    fn plain_stream_state_writes_errors_on_new_lines() {
        let mut state = PlainStreamState::new();
        let mut buffer = Vec::new();
        state
            .write_chunk(&mut buffer, "partial response")
            .expect("chunk should write");
        state
            .write_line(&mut buffer, "oops")
            .expect("line should write");
        let captured = String::from_utf8(buffer).expect("valid utf8");
        assert_eq!(captured, "partial response\noops\n");
    }
}
