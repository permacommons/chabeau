use super::usage_status;
use crate::auth::AuthManager;
use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::App;
use crate::core::config::data::Config;
use crate::core::providers::resolve_env_session;
use crate::core::session_store::{list_sessions, save_session, SessionSummary};
use crate::ui::picker::PickerItem;

const USAGE_SAVE: &str = "Usage: /save [name]";
const USAGE_LOAD: &str = "Usage: /load [session-id]";
const OPENAI_PROVIDER: &str = "openai";
const OPENAI_DISPLAY_NAME: &str = "OpenAI";
const OPENAI_COMPATIBLE_PROVIDER: &str = "openai-compatible";
const OPENAI_COMPATIBLE_DISPLAY_NAME: &str = "OpenAI-compatible";
const OPENAI_ENV_FALLBACK_WARNING: &str = "using OPENAI_API_KEY fallback";

struct RestoredSessionProvider {
    api_key: String,
    provider_display_name: String,
    warning: Option<String>,
}

#[derive(Clone)]
struct CurrentSessionProvider {
    api_key: String,
    base_url: String,
    provider_name: String,
    provider_display_name: String,
    model: String,
}

enum LoadedSessionProvider {
    Restored(RestoredSessionProvider),
    Current {
        provider: CurrentSessionProvider,
        warning: String,
    },
}

fn openai_env_fallback_display_name(provider: &str) -> Option<&'static str> {
    match provider {
        OPENAI_PROVIDER => Some(OPENAI_DISPLAY_NAME),
        OPENAI_COMPATIBLE_PROVIDER => Some(OPENAI_COMPATIBLE_DISPLAY_NAME),
        _ => None,
    }
}

fn provider_ids_match(saved: &str, resolved: &str) -> bool {
    saved.eq_ignore_ascii_case(resolved)
}

fn current_provider_warning(
    snapshot_provider: &str,
    current: &CurrentSessionProvider,
    reason: &str,
) -> String {
    let current_provider = if current.provider_name.trim().is_empty() {
        "(no provider selected)"
    } else {
        &current.provider_name
    };
    format!(
        "provider '{}' unavailable; using current provider '{}': {}",
        snapshot_provider, current_provider, reason
    )
}

fn current_provider_fallback(
    snapshot_provider: &str,
    current: &CurrentSessionProvider,
    reason: &str,
) -> LoadedSessionProvider {
    LoadedSessionProvider::Current {
        provider: current.clone(),
        warning: current_provider_warning(snapshot_provider, current, reason),
    }
}

fn resolve_session_provider_after_auth_error(
    snapshot_provider: &str,
    current: &CurrentSessionProvider,
    err: String,
) -> LoadedSessionProvider {
    if let Some(provider_display_name) = openai_env_fallback_display_name(snapshot_provider) {
        return match resolve_env_session() {
            Ok(session) => {
                let (api_key, _, _, _) = session.into_tuple();
                LoadedSessionProvider::Restored(RestoredSessionProvider {
                    api_key,
                    provider_display_name: provider_display_name.to_string(),
                    warning: Some(OPENAI_ENV_FALLBACK_WARNING.to_string()),
                })
            }
            Err(_) => current_provider_fallback(snapshot_provider, current, &err),
        };
    }

    if provider_ids_match(&current.provider_name, snapshot_provider) && !current.api_key.is_empty()
    {
        return LoadedSessionProvider::Restored(RestoredSessionProvider {
            api_key: current.api_key.clone(),
            provider_display_name: current.provider_display_name.clone(),
            warning: Some(format!("using current credentials: {}", err)),
        });
    }

    current_provider_fallback(snapshot_provider, current, &err)
}

pub(crate) fn handle_save(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let name = match invocation.args_len() {
        0 => generate_session_name(&app.ui.messages),
        1 => invocation.arg(0).unwrap_or("Untitled").to_string(),
        _ => return usage_status(app, USAGE_SAVE),
    };

    let messages = app.ui.messages.iter().cloned().collect::<Vec<_>>();
    let result = save_session(
        &app.session.session_id,
        &name,
        &app.session.provider_name,
        &app.session.model,
        &app.session.base_url,
        app.session.get_character().cloned(),
        app.persona_manager.get_active_persona().cloned(),
        app.preset_manager.get_active_preset().cloned(),
        &messages,
        &app.session.tool_pipeline.tool_results,
        &app.session.tool_pipeline.tool_payload_history,
        &app.session.mcp_init,
        &app.session.refine_instructions,
        &app.session.refine_prefix,
        app.ui.markdown_enabled,
        app.ui.syntax_enabled,
    );

    match result {
        Ok(()) => {
            let session_id = app.session.session_id.clone();
            app.conversation()
                .set_status(format!("Session saved: {} ({})", name, session_id));
            CommandResult::ContinueWithTranscriptFocus
        }
        Err(e) => {
            app.conversation().set_status(format!("Save error: {}", e));
            CommandResult::ContinueWithTranscriptFocus
        }
    }
}

pub(crate) fn handle_load(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => match list_sessions() {
            Ok(sessions) => {
                if sessions.is_empty() {
                    app.conversation().set_status("No saved sessions found.");
                    return CommandResult::ContinueWithTranscriptFocus;
                }
                show_session_picker(app, sessions);
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Session list error: {}", e));
                CommandResult::ContinueWithTranscriptFocus
            }
        },
        1 => {
            let Some(id) = invocation.arg(0) else {
                return usage_status(app, USAGE_LOAD);
            };
            match do_load_session(app, id) {
                Ok(()) => CommandResult::ContinueWithTranscriptFocus,
                Err(e) => {
                    app.conversation().set_status(format!("Load error: {}", e));
                    CommandResult::ContinueWithTranscriptFocus
                }
            }
        }
        _ => usage_status(app, USAGE_LOAD),
    }
}

pub(crate) fn handle_sessions(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    match list_sessions() {
        Ok(sessions) => {
            if sessions.is_empty() {
                app.conversation().set_status("No saved sessions found.");
                return CommandResult::ContinueWithTranscriptFocus;
            }
            show_session_picker(app, sessions);
            CommandResult::Continue
        }
        Err(e) => {
            app.conversation()
                .set_status(format!("Session list error: {}", e));
            CommandResult::ContinueWithTranscriptFocus
        }
    }
}

fn generate_session_name(
    messages: &std::collections::VecDeque<crate::core::message::Message>,
) -> String {
    for msg in messages {
        if msg.is_user() && !msg.content.is_empty() {
            let first_line = msg.content.lines().next().unwrap_or("");
            let truncated = if first_line.chars().count() > 50 {
                first_line.chars().take(47).collect::<String>() + "..."
            } else {
                first_line.to_string()
            };
            return truncated;
        }
    }
    "Untitled session".to_string()
}

fn resolve_session_provider_for_load(
    snapshot_provider: &str,
    current: &CurrentSessionProvider,
) -> LoadedSessionProvider {
    let resolved = AuthManager::new()
        .map_err(|e| e.to_string())
        .and_then(|auth_manager| {
            let config = Config::load_test_safe().map_err(|e| e.to_string())?;
            auth_manager
                .resolve_authentication(Some(snapshot_provider), &config)
                .map_err(|e| e.to_string())
        });

    match resolved {
        Ok((api_key, _, resolved_provider, provider_display_name))
            if provider_ids_match(snapshot_provider, &resolved_provider) =>
        {
            LoadedSessionProvider::Restored(RestoredSessionProvider {
                api_key,
                provider_display_name,
                warning: None,
            })
        }
        Ok((_, _, resolved_provider, _)) => current_provider_fallback(
            snapshot_provider,
            current,
            &format!("resolved credentials for '{}'", resolved_provider),
        ),
        Err(err) => resolve_session_provider_after_auth_error(snapshot_provider, current, err),
    }
}

pub fn do_load_session(app: &mut App, id: &str) -> Result<(), String> {
    let snapshot = crate::core::session_store::load_session(id).map_err(|e| e.to_string())?;
    let current_provider = CurrentSessionProvider {
        api_key: app.session.api_key.clone(),
        base_url: app.session.base_url.clone(),
        provider_name: app.session.provider_name.clone(),
        provider_display_name: app.session.provider_display_name.clone(),
        model: app.session.model.clone(),
    };
    let provider = resolve_session_provider_for_load(&snapshot.provider, &current_provider);

    app.session.session_id = snapshot.id.clone();
    app.ui.messages = snapshot.messages.into_iter().collect();
    app.ui.invalidate_prewrap_cache();

    app.session.tool_pipeline.tool_results = snapshot.tool_results;
    app.session.tool_pipeline.tool_payload_history = snapshot
        .tool_payloads
        .into_iter()
        .map(|tp| crate::core::app::session::ToolPayloadHistoryEntry {
            server_id: tp.server_id,
            tool_call_id: tp.tool_call_id,
            assistant_message: tp.assistant_message,
            tool_message: tp.tool_message,
            assistant_message_index: tp.assistant_message_index,
        })
        .collect();

    app.session.mcp_init = crate::core::app::session::McpInitState {
        in_progress: false,
        complete: snapshot.mcp_init_complete,
        deferred_message: None,
    };

    app.session.refine_instructions = snapshot.refine_instructions;
    app.session.refine_prefix = snapshot.refine_prefix;

    app.ui.markdown_enabled = snapshot.markdown_enabled;
    app.ui.syntax_enabled = snapshot.syntax_enabled;

    let warning = match provider {
        LoadedSessionProvider::Restored(auth) => {
            app.session.api_key = auth.api_key;
            app.session.base_url = snapshot.base_url;
            app.session.provider_name = snapshot.provider;
            app.session.provider_display_name = auth.provider_display_name;
            app.session.model = snapshot.model;
            auth.warning
        }
        LoadedSessionProvider::Current { provider, warning } => {
            app.session.api_key = provider.api_key;
            app.session.base_url = provider.base_url;
            app.session.provider_name = provider.provider_name;
            app.session.provider_display_name = provider.provider_display_name;
            app.session.model = provider.model;
            Some(warning)
        }
    };

    // Restore character, persona, preset from stored data
    if let Some(character) = snapshot.character {
        app.session.set_character(character);
    }
    if let Some(persona) = snapshot.persona {
        app.persona_manager.set_active_persona(&persona.id).ok();
    }
    if let Some(preset) = snapshot.preset {
        app.preset_manager.set_active_preset(&preset.id).ok();
    }

    let status = if let Some(warning) = warning {
        format!("Loaded session: {} ({})", snapshot.name, warning)
    } else {
        format!("Loaded session: {}", snapshot.name)
    };
    app.conversation().set_status(status);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::provider_ids_match;

    #[test]
    fn provider_ids_match_rejects_different_resolved_provider() {
        assert!(provider_ids_match("openai", "OpenAI"));
        assert!(!provider_ids_match("anthropic", "openai"));
    }
}

fn show_session_picker(app: &mut App, sessions: Vec<SessionSummary>) {
    let items: Vec<PickerItem> = sessions
        .iter()
        .map(|s| {
            let provider = &s.provider;
            let model = &s.model;
            let char_label = s.character.as_deref().unwrap_or("-");
            let meta = format!(
                "{} | {} | {} | {} msgs | {}",
                provider,
                model,
                char_label,
                s.message_count,
                s.modified_at.format("%Y-%m-%d %H:%M")
            );
            PickerItem {
                id: s.id.clone(),
                label: s.name.clone(),
                metadata: Some(meta),
                inspect_metadata: None,
                sort_key: Some(s.modified_at.to_string()),
            }
        })
        .collect();

    app.picker.open_saved_session_picker(sessions, items);
}
