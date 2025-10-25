//! TUI-less "say" command

use std::error::Error;
use std::io::{self, Write};

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
use ratatui::{
    backend::CrosstermBackend,
    crossterm::cursor,
    layout::Rect,
    widgets::{Paragraph, Wrap},
    Terminal,
};

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
    let use_markdown = config.markdown.unwrap_or(false);

    // Use a temporary terminal for rendering, but without the alternate screen
    let mut terminal = if use_markdown {
        Some(Terminal::new(CrosstermBackend::new(io::stdout()))?)
    } else {
        None
    };

    let initial_cursor = if use_markdown {
        Some(cursor::position()?)
    } else {
        None
    };

    let result: Result<Option<Rect>, Box<dyn Error>> = loop {
        match rx.recv().await {
            Some((StreamMessage::Chunk(content), _)) => {
                full_response.push_str(&content);

                if let (Some(terminal), Some((start_x, start_y))) =
                    (&mut terminal, initial_cursor)
                {
                    terminal.draw(|f| {
                        let total_area = f.area();
                        let render_area = Rect::new(
                            start_x,
                            start_y,
                            total_area.width,
                            total_area.height.saturating_sub(start_y),
                        );
                        let (paragraph, lines) = markdown_paragraph(&full_response, render_area);
                        let mut paragraph = paragraph;
                        if lines > render_area.height {
                            paragraph = paragraph.scroll(((lines - render_area.height) as u16, 0));
                        }
                        f.render_widget(paragraph, render_area);
                    })?;
                } else {
                    print!("{}", content);
                    io::stdout().flush()?;
                }
            }
            Some((StreamMessage::Error(err), _)) => {
                break Err(err.into());
            }
            Some((StreamMessage::End, _)) => {
                break Ok(None);
            }
            None => break Ok(None),
            _ => {}
        }
    };

    // Ensure there's a newline at the end of the output
    println!();

    if let Err(e) = result {
        eprintln!("❌ Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

fn markdown_paragraph(content: &str, size: Rect) -> (Paragraph<'_>, u16) {
    let monochrome_theme = Theme::monochrome();
    let render_config = MessageRenderConfig::markdown(false) // Disable syntax highlighting
        .with_terminal_width(Some(size.width as usize), TableOverflowPolicy::WrapCells);

    let rendered = markdown::render_message_with_config(
        &Message {
            role: ROLE_ASSISTANT.to_string(),
            content: content.to_string(),
        },
        &monochrome_theme,
        render_config,
    );

    let lines = rendered.lines.len() as u16;
    (Paragraph::new(rendered.lines).wrap(Wrap { trim: false }), lines)
}
