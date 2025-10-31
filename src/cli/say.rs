//! TUI-less "say" command

use std::error::Error;
use std::io::{self, Write};

use crate::auth::{AuthManager, ProviderAuthStatus};
use crate::character::CharacterService;
use crate::core::app::{self};
use crate::core::chat_stream::{ChatStreamService, StreamMessage};
use crate::core::config::data::Config;
use crate::core::providers::{resolve_session, ResolveSessionError};
use crate::ui::osc;
use ratatui::crossterm::cursor::{MoveToColumn, MoveUp};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, Clear, ClearType};

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
    let prompt = prompt.join(" ");
    if prompt.is_empty() {
        eprintln!("Usage: chabeau say <prompt>");
        std::process::exit(1);
    }

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

    let session = match resolve_session(&auth_manager, &config, provider.as_deref()) {
        Ok(session) => session,
        Err(err) => match err {
            ResolveSessionError::Provider(provider_err) => {
                eprintln!("{}", provider_err);
                let fixes = provider_err.quick_fixes();
                if !fixes.is_empty() {
                    eprintln!();
                    eprintln!("💡 Quick fixes:");
                    for fix in fixes {
                        eprintln!("  • {fix}");
                    }
                }
                std::process::exit(provider_err.exit_code());
            }
            ResolveSessionError::Source(source_err) => {
                eprintln!("❌ Error: {}", source_err);
                std::process::exit(1);
            }
        },
    };

    let mut app = app::new_with_auth(
        app::AppInitConfig {
            model: model.unwrap_or_else(|| "default".to_string()),
            log_file: None,
            provider,
            env_only,
            pre_resolved_session: Some(session),
            character,
            persona,
            preset,
        },
        &config,
        character_service,
    )
    .await?;

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
                eprintln!("❌ Error: {}", err);
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
