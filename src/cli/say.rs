//! TUI-less "say" command

use std::error::Error;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::auth::AuthManager;
use crate::character::CharacterService;
use crate::core::app::{self};
use crate::core::chat_stream::{ChatStreamService, StreamMessage};
use crate::core::config::data::Config;
use crate::core::message::{Message, ROLE_ASSISTANT, ROLE_USER};
use crate::core::providers::{resolve_session, ResolveSessionError};
use crate::ui::layout::TableOverflowPolicy;
use crate::ui::markdown::{self, MessageRenderConfig};
use crate::ui::theme::Theme;
use ratatui::crossterm::{cursor, terminal, QueueableCommand};

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
            .filter(|(_, _, has_token)| *has_token)
            .map(|(id, _, _)| id)
            .collect();
        if configured_providers.len() > 1 {
            eprintln!("Multiple providers are configured. Please specify a provider with the -p flag.");
            eprintln!("Available providers: {}", configured_providers.join(", "));
            std::process::exit(1);
        }
    }

    let character_service = CharacterService::new();

    let session = match resolve_session(&auth_manager, &config, provider.as_deref()) {
        Ok(session) => session,
        Err(err) => {
            match err {
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
            }
        }
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

    let messages = vec![Message {
        role: ROLE_USER.to_string(),
        content: prompt,
    }];

    let params = app.conversation().stream_parameters(messages, None);

    let (stream_service, mut rx) = ChatStreamService::new();
    stream_service.spawn_stream(params);

    let mut full_response = String::new();
    let mut last_line_count = 0;
    let use_markdown = config.markdown.unwrap_or(false);
    let mut stdout = io::stdout();
    let mut last_render_time = Instant::now();
    let render_interval = Duration::from_millis(50); // ~20 FPS

    loop {
        match rx.recv().await {
            Some((StreamMessage::Chunk(content), _)) => {
                full_response.push_str(&content);
                if use_markdown {
                    if last_render_time.elapsed() >= render_interval {
                        last_line_count = render_and_print_markdown(
                            &full_response,
                            last_line_count,
                            &mut stdout,
                        )?;
                        last_render_time = Instant::now();
                    }
                } else {
                    print!("{}", content);
                    stdout.flush()?;
                }
            }
            Some((StreamMessage::Error(err), _)) => {
                eprintln!("\n\n❌ Error: {}", err);
                std::process::exit(1);
            }
            Some((StreamMessage::End, _)) => {
                if use_markdown {
                    // Final render to ensure the full content is displayed
                    render_and_print_markdown(&full_response, last_line_count, &mut stdout)?;
                } else {
                    println!();
                }
                break;
            }
            None => break,
            _ => {}
        }
    }

    Ok(())
}

fn render_and_print_markdown(
    content: &str,
    last_line_count: usize,
    stdout: &mut io::Stdout,
) -> Result<usize, Box<dyn Error>> {
    // Clear the previously rendered output
    if last_line_count > 0 {
        stdout.queue(cursor::MoveUp(last_line_count as u16))?;
        stdout.queue(terminal::Clear(terminal::ClearType::FromCursorDown))?;
    }

    let monochrome_theme = Theme::monochrome();
    let terminal_width = terminal::size().ok().map(|(w, _)| w as usize);
    let rendered = markdown::render_message_with_config(
        &Message {
            role: ROLE_ASSISTANT.to_string(),
            content: content.to_string(),
        },
        &monochrome_theme,
        MessageRenderConfig::markdown(true)
            .with_terminal_width(terminal_width, TableOverflowPolicy::WrapCells),
    );

    for line in &rendered.lines {
        println!("{}", line);
    }

    stdout.flush()?;

    Ok(rendered.lines.len())
}
