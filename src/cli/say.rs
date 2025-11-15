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

    let (term_width, _) = terminal::size().unwrap_or((80, 24));

    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(prompt.clone());
        (cancel_token, stream_id, api_messages)
    };

    let prefix_lines: Vec<String> = {
        let metadata = app.get_prewrapped_span_metadata_cached(term_width).clone();
        let lines = app.get_prewrapped_lines_cached(term_width).clone();
        osc::encode_lines_with_links_with_underline(&lines, &metadata)
    };
    for line in &prefix_lines {
        println!("{}", line);
    }

    let params = app.build_stream_params(api_messages, cancel_token, stream_id);

    let (stream_service, mut rx) = ChatStreamService::new();
    stream_service.spawn_stream(params);

    let mut stdout = io::stdout();
    let mut previous_lines = prefix_lines.clone();

    loop {
        match rx.recv().await {
            Some((StreamMessage::Chunk(content), _)) => {
                let (term_width, term_height) = terminal::size().unwrap_or((80, 24));
                {
                    let mut conversation = app.conversation();
                    let available_height = conversation.calculate_available_height(term_height, 0);
                    conversation.append_to_response(&content, available_height, term_width);
                }

                let new_lines: Vec<String> = {
                    let metadata = app.get_prewrapped_span_metadata_cached(term_width).clone();
                    let lines = app.get_prewrapped_lines_cached(term_width).clone();
                    osc::encode_lines_with_links_with_underline(&lines, &metadata)
                };

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
                previous_lines = new_lines;
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

                let new_lines: Vec<String> = {
                    let metadata = app.get_prewrapped_span_metadata_cached(term_width).clone();
                    let lines = app.get_prewrapped_lines_cached(term_width).clone();
                    osc::encode_lines_with_links_with_underline(&lines, &metadata)
                };

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
                std::process::exit(1);
            }
            Some((StreamMessage::End, _)) => {
                let mut conversation = app.conversation();
                conversation.finalize_response();
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
    use super::resolve_prompt_from_args;
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
}
