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
use ratatui::crossterm::terminal;

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
        let configured_providers: Vec<_> = auth_manager
            .get_all_providers_with_auth_status()
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
    loop {
        match rx.recv().await {
            Some((StreamMessage::Chunk(content), _)) => {
                full_response.push_str(&content);
                if !config.markdown.unwrap_or(false) {
                    print!("{}", content);
                    io::stdout().flush()?;
                }
            }
            Some((StreamMessage::Error(err), _)) => {
                eprintln!("\n\n❌ Error: {}", err);
                std::process::exit(1);
            }
            Some((StreamMessage::End, _)) => {
                if !config.markdown.unwrap_or(false) {
                    println!();
                }
                break;
            }
            None => break,
            _ => {}
        }
    }

    if config.markdown.unwrap_or(false) {
        let monochrome_theme = Theme::monochrome();
        let terminal_width = terminal::size().ok().map(|(w, _)| w as usize);
        let rendered = markdown::render_message_with_config(
            &Message {
                role: ROLE_ASSISTANT.to_string(),
                content: full_response,
            },
            &monochrome_theme,
            MessageRenderConfig::markdown(true)
                .with_terminal_width(terminal_width, TableOverflowPolicy::WrapCells),
        );
        for line in rendered.lines {
            println!("{}", line);
        }
    }

    Ok(())
}
