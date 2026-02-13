//! Slash command processing and routing.
//!
//! This module provides a registry-based system for handling slash commands
//! in the chat interface. Commands are dispatched to handlers that can modify
//! application state, open pickers, save/load conversations, or pass input
//! through to the model.
//!
//! The [`process_input`] function is the main entry point, returning a
//! [`CommandResult`] that indicates how the UI should respond.
//!
//! See also: [`dump_conversation_with_overwrite`], [`all_commands`]

mod refine;
mod registry;

pub use registry::{all_commands, matching_commands, CommandInvocation};

use crate::core::app::session::McpPromptRequest;
use crate::core::app::App;
use crate::core::mcp_auth::McpTokenStore;
use crate::core::message::{self, AppMessageKind};
use chrono::Utc;
use registry::DispatchOutcome;
use rust_mcp_schema::PromptArgument;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Result of processing a command or user input.
///
/// Variants indicate how the UI should respond after command execution,
/// including opening pickers, passing input to the model, or continuing
/// the current interaction flow.
pub enum CommandResult {
    /// Continue without taking action (command handled internally).
    Continue,

    /// Continue and shift focus to the transcript area.
    ContinueWithTranscriptFocus,

    /// Process the contained string as a chat message to the model.
    ProcessAsMessage(String),

    /// Open the model selection picker.
    OpenModelPicker,

    /// Open the provider selection picker.
    OpenProviderPicker,

    /// Open the theme selection picker.
    OpenThemePicker,

    /// Open the character selection picker.
    OpenCharacterPicker,

    /// Open the persona selection picker.
    OpenPersonaPicker,

    /// Open the preset selection picker.
    OpenPresetPicker,

    /// Refine or edit the contained text before sending to the model.
    Refine(String),

    /// Run an MCP prompt request.
    RunMcpPrompt(McpPromptRequest),

    /// Refresh MCP listings for a server in the background.
    RefreshMcp { server_id: String },
}

/// Processes user input and dispatches commands.
///
/// This is the main entry point for command handling. If the input matches
/// a registered slash command (e.g., `/help`, `/clear`), it dispatches to
/// the appropriate handler. Otherwise, the input is treated as a message
/// to send to the chat model.
///
/// # Arguments
///
/// * `app` - [`App`] state containing session and UI context
/// * `input` - User input string (may or may not be a command)
///
/// # Returns
///
/// Returns a [`CommandResult`] indicating how the UI should respond.
///
/// # Examples
///
/// ```no_run
/// use chabeau::commands::{process_input, CommandResult};
/// # use chabeau::core::app::App;
/// # use chabeau::character::service::CharacterService;
/// # use chabeau::core::app::AppInitConfig;
/// # use chabeau::core::config::data::Config;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let config = Config::load()?;
/// # let character_service = CharacterService::new();
/// # let init_config = AppInitConfig {
/// #     model: "gpt-4".to_string(),
/// #     log_file: None,
/// #     provider: Some("openai".to_string()),
/// #     env_only: false,
/// #     pre_resolved_session: None,
/// #     character: None,
/// #     persona: None,
/// #     preset: None,
/// #     disable_mcp: false,
/// # };
/// # let mut app = chabeau::core::app::new_with_auth(init_config, &config, character_service).await?;
/// // Process a command
/// let result = process_input(&mut app, "/help");
/// match result {
///     CommandResult::ContinueWithTranscriptFocus => {
///         // Help was displayed, focus on transcript
///     }
///     _ => {}
/// }
///
/// // Process a regular message
/// let result = process_input(&mut app, "What is Rust?");
/// match result {
///     CommandResult::ProcessAsMessage(msg) => {
///         // Send the message to the chat model
///         assert_eq!(msg, "What is Rust?");
///     }
///     _ => {}
/// }
/// # Ok(())
/// # }
/// ```
pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    match registry::registry().dispatch(input) {
        DispatchOutcome::NotACommand | DispatchOutcome::UnknownCommand => {
            if let Some(result) = handle_mcp_prompt_invocation(app, input) {
                return result;
            }
            CommandResult::ProcessAsMessage(input.to_string())
        }
        DispatchOutcome::Invocation(invocation) => {
            let handler = invocation.command.handler;
            handler(app, invocation)
        }
    }
}

fn handle_mcp_prompt_invocation(app: &mut App, input: &str) -> Option<CommandResult> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    if app.session.mcp_disabled {
        app.conversation()
            .set_status("MCP: **disabled for this session**".to_string());
        return Some(CommandResult::Continue);
    }

    let without_slash = trimmed.trim_start_matches('/');
    let mut parts = without_slash.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or("");
    let args_str = parts.next().unwrap_or("").trim();

    let (server_id, prompt_name) = command.split_once(':')?;
    if server_id.is_empty() || prompt_name.is_empty() {
        return None;
    }

    let (server_id_label, server_name, prompts) = match app.mcp.server(server_id) {
        Some(server) => (
            server.config.id.clone(),
            server.config.display_name.clone(),
            server.cached_prompts.clone(),
        ),
        None => {
            app.conversation()
                .set_status(format!("No MCP server '{}'.", server_id));
            return Some(CommandResult::Continue);
        }
    };

    let Some(prompts) = prompts else {
        app.conversation().set_status(format!(
            "No cached prompts for MCP server '{}'.",
            server_id_label
        ));
        return Some(CommandResult::Continue);
    };
    let prompt = prompts
        .prompts
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(prompt_name));

    let Some(prompt) = prompt else {
        app.conversation().set_status(format!(
            "No MCP prompt '{}' on server '{}'.",
            prompt_name, server_id_label
        ));
        return Some(CommandResult::Continue);
    };

    let parsed_args = match parse_prompt_args(args_str, &prompt.arguments) {
        Ok(map) => map,
        Err(err) => {
            app.conversation().set_status(err);
            return Some(CommandResult::Continue);
        }
    };

    if let Err(err) = validate_prompt_args(&parsed_args, &prompt.arguments) {
        app.conversation().set_status(err);
        return Some(CommandResult::Continue);
    }

    let collected = parsed_args;
    let mut missing = Vec::new();
    for arg in &prompt.arguments {
        if !arg.required.unwrap_or(false) {
            continue;
        }
        if collected.contains_key(&arg.name) {
            continue;
        }
        missing.push(prompt_argument_from_schema(arg));
    }

    if !missing.is_empty() {
        app.ui
            .start_mcp_prompt_input(crate::core::app::ui_state::McpPromptInput {
                server_id: server_id_label.clone(),
                server_name,
                prompt_name: prompt.name.clone(),
                prompt_title: prompt.title.clone(),
                pending_args: missing,
                collected,
                next_index: 0,
            });
        return Some(CommandResult::Continue);
    }

    Some(CommandResult::RunMcpPrompt(McpPromptRequest {
        server_id: server_id_label,
        prompt_name: prompt.name.clone(),
        arguments: collected,
    }))
}

fn parse_prompt_args(
    input: &str,
    prompt_args: &[PromptArgument],
) -> Result<HashMap<String, String>, String> {
    if input.trim().is_empty() {
        return Ok(HashMap::new());
    }

    if prompt_args.len() == 1 {
        match parse_kv_args(input) {
            Ok(map) => return Ok(map),
            Err(_) => {
                let value = parse_single_prompt_value(input)?;
                let mut args = HashMap::new();
                args.insert(prompt_args[0].name.clone(), value);
                return Ok(args);
            }
        }
    }

    parse_kv_args(input)
}

fn parse_single_prompt_value(input: &str) -> Result<String, String> {
    let tokens = tokenize_prompt_args(input)?;
    if tokens.is_empty() {
        return Ok(String::new());
    }
    if tokens.len() == 1 {
        return Ok(tokens[0].clone());
    }
    Ok(tokens.join(" "))
}

fn parse_kv_args(input: &str) -> Result<HashMap<String, String>, String> {
    if input.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let tokens = tokenize_prompt_args(input)?;

    let mut args = HashMap::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!(
                "Invalid prompt argument '{}'. Use key=value.",
                token
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err("Prompt argument name cannot be empty.".to_string());
        }
        args.insert(key.to_string(), value.to_string());
    }

    Ok(args)
}

fn validate_prompt_args(
    args: &HashMap<String, String>,
    prompt_args: &[PromptArgument],
) -> Result<(), String> {
    let mut allowed = Vec::new();
    for arg in prompt_args {
        allowed.push(arg.name.as_str());
    }

    for key in args.keys() {
        if !allowed.iter().any(|name| name == key) {
            let mut allowed_sorted: Vec<&str> = allowed.clone();
            allowed_sorted.sort();
            let allowed_list = if allowed_sorted.is_empty() {
                "none".to_string()
            } else {
                allowed_sorted.join(", ")
            };
            return Err(format!(
                "Unknown prompt argument '{}'. Allowed: {}.",
                key, allowed_list
            ));
        }
    }

    Ok(())
}

fn tokenize_prompt_args(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for ch in input.chars() {
        match ch {
            '"' | '\'' => {
                if let Some(q) = in_quote {
                    if q == ch {
                        in_quote = None;
                    } else {
                        current.push(ch);
                    }
                } else {
                    in_quote = Some(ch);
                }
            }
            c if c.is_whitespace() && in_quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if let Some(q) = in_quote {
        return Err(format!("Unclosed quote ({}) in prompt arguments.", q));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn prompt_argument_from_schema(
    arg: &PromptArgument,
) -> crate::core::app::ui_state::McpPromptArgument {
    crate::core::app::ui_state::McpPromptArgument {
        name: arg.name.clone(),
        title: arg.title.clone(),
        description: arg.description.clone(),
        required: arg.required.unwrap_or(false),
    }
}

pub(super) fn handle_help(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    let mut help_md = crate::ui::help::builtin_help_md().to_string();
    help_md.push_str("\n\n## Commands\n");
    for command in all_commands() {
        for usage in command.usages {
            help_md.push_str(&format!("- `{}` — {}\n", usage.syntax, usage.description));
        }
        for line in command.extra_help {
            help_md.push_str(line);
            help_md.push('\n');
        }
    }
    app.conversation()
        .add_app_message(AppMessageKind::Info, help_md);
    CommandResult::ContinueWithTranscriptFocus
}

pub(super) fn handle_clear(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    {
        let mut conversation = app.conversation();
        conversation.clear_transcript();
        conversation.show_character_greeting_if_needed();
        conversation.set_status("Transcript cleared".to_string());
    }
    CommandResult::Continue
}

pub(super) fn handle_mcp(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let server_id = invocation.arg(0);
    if server_id.is_none() {
        return handle_mcp_list(app);
    }

    match invocation.args_len() {
        1 => handle_mcp_server(app, server_id.unwrap()),
        2 => {
            let arg = invocation.arg(1).unwrap_or_default();
            match arg.to_ascii_lowercase().as_str() {
                "on" => handle_mcp_toggle(app, server_id.unwrap(), true),
                "off" => handle_mcp_toggle(app, server_id.unwrap(), false),
                "forget" => handle_mcp_forget(app, server_id.unwrap()),
                _ => {
                    app.conversation()
                        .set_status("Usage: /mcp <server-id> [on|off|forget]".to_string());
                    CommandResult::Continue
                }
            }
        }
        _ => {
            app.conversation()
                .set_status("Usage: /mcp <server-id> [on|off|forget]".to_string());
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_yolo(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        app.conversation()
            .set_status("Usage: /yolo <server-id> [on|off]".to_string());
        return CommandResult::Continue;
    }
    if invocation.args_len() > 2 {
        app.conversation()
            .set_status("Usage: /yolo <server-id> [on|off]".to_string());
        return CommandResult::Continue;
    }

    let server_id = invocation.arg(0).unwrap();
    let Some(server) = app.mcp.server(server_id) else {
        app.conversation()
            .set_status(format!("Unknown MCP server: {}", server_id));
        return CommandResult::Continue;
    };
    let server_label = server.config.id.clone();
    let current_yolo = server.config.is_yolo();

    if invocation.args_len() == 1 {
        let mut output = format!("## MCP YOLO for {}\n", server.config.display_name);
        output.push_str(&format!("Server id: `{}`\n", server_label));
        output.push_str(&format!(
            "YOLO: {}\n",
            if current_yolo {
                "**enabled**"
            } else {
                "disabled"
            }
        ));
        output.push_str(&format!(
            "Toggle with `/yolo {} on|off` (saved to config.toml).\n",
            server_label
        ));
        app.conversation()
            .add_app_message(AppMessageKind::Info, output);
        app.ui.focus_transcript();
        return CommandResult::ContinueWithTranscriptFocus;
    }

    let arg = invocation.arg(1).unwrap_or_default();
    let new_state = match arg.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => {
            app.conversation()
                .set_status("Usage: /yolo <server-id> [on|off]".to_string());
            return CommandResult::Continue;
        }
    };

    if let Some(server) = app.mcp.server_mut(server_id) {
        server.config.yolo = Some(new_state);
    }
    if let Some(server) = app
        .config
        .mcp_servers
        .iter_mut()
        .find(|server| server.id.eq_ignore_ascii_case(server_id))
    {
        server.yolo = Some(new_state);
    }

    let saved = match crate::core::config::data::Config::load() {
        Ok(mut cfg) => {
            if let Some(server) = cfg
                .mcp_servers
                .iter_mut()
                .find(|server| server.id.eq_ignore_ascii_case(server_id))
            {
                server.yolo = Some(new_state);
                cfg.save().is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    };

    let state_word = if new_state { "enabled" } else { "disabled" };
    let persist_note = if saved {
        "saved to config.toml"
    } else {
        "config.toml not saved"
    };
    app.conversation().set_status(format!(
        "YOLO {} for {} ({})",
        state_word, server_label, persist_note
    ));
    CommandResult::Continue
}

pub(super) fn handle_log(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => {
            let timestamp = chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string();
            let was_active = app.session.logging.is_active();
            let log_message = if was_active {
                format!("Logging paused at {}", timestamp)
            } else {
                format!("Logging resumed at {}", timestamp)
            };

            match app.session.logging.toggle_logging(&log_message) {
                Ok(message) => {
                    // Add log message to transcript
                    app.conversation()
                        .add_app_message(crate::core::message::AppMessageKind::Log, log_message);
                    app.conversation().set_status(message);
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation().set_status(format!("Log error: {}", e));
                    CommandResult::Continue
                }
            }
        }
        1 => {
            let filename = invocation.arg(0).unwrap();
            match app.session.logging.set_log_file(filename.to_string()) {
                Ok(message) => {
                    // Add log message to transcript
                    let timestamp = chrono::Local::now()
                        .format("%Y-%m-%d %H:%M:%S %Z")
                        .to_string();
                    let log_message = format!("Logging started at {}", timestamp);
                    app.conversation()
                        .add_app_message(crate::core::message::AppMessageKind::Log, log_message);
                    app.conversation().set_status(message);
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation()
                        .set_status(format!("Logfile error: {}", e));
                    CommandResult::Continue
                }
            }
        }
        _ => {
            app.conversation().set_status("Usage: /log [filename]");
            CommandResult::Continue
        }
    }
}

fn handle_mcp_list(app: &mut App) -> CommandResult {
    let servers: Vec<_> = app.mcp.servers().collect();
    let mut output = String::from("## MCP servers\n");
    if app.session.mcp_disabled {
        output.push_str("MCP: **disabled for this session**\n");
    } else {
        output.push_str("MCP: enabled\n");
    }
    output.push('\n');
    if servers.is_empty() {
        output.push_str("No MCP servers configured. Add `[[mcp_servers]]` to config.toml.\n");
        app.conversation()
            .add_app_message(AppMessageKind::Info, output);
        return CommandResult::ContinueWithTranscriptFocus;
    }

    for server in servers {
        let disabled_marker = if server.config.is_enabled() {
            ""
        } else {
            " — **disabled**"
        };
        let yolo_marker = if server.config.is_yolo() {
            " — **YOLO**"
        } else {
            ""
        };
        output.push_str(&format!(
            "- **{}** ({}){}{}\n",
            server.config.id, server.config.display_name, disabled_marker, yolo_marker
        ));
    }
    output
        .push_str("\nUse `/mcp <server-id>` to fetch tools, resources, prompts, and templates.\n");

    app.conversation()
        .add_app_message(AppMessageKind::Info, output);
    CommandResult::ContinueWithTranscriptFocus
}

fn handle_mcp_server(app: &mut App, server_id: &str) -> CommandResult {
    if app.mcp.server(server_id).is_none() {
        app.conversation()
            .set_status(format!("Unknown MCP server: {}", server_id));
        return CommandResult::Continue;
    }

    if app.session.mcp_disabled {
        if let Some(server) = app.mcp.server(server_id) {
            let mut output = format!("## MCP for {}\n", server.config.display_name);
            output.push_str("MCP: **disabled for this session**\n");
            output.push_str(&format!("Server id: `{}`\n", server.config.id));
            output.push_str(&format!(
                "YOLO: {}\n",
                if server.config.is_yolo() {
                    "**enabled**"
                } else {
                    "disabled"
                }
            ));
            output.push('\n');
            app.conversation()
                .add_app_message(AppMessageKind::Info, output);
            app.ui.focus_transcript();
            return CommandResult::ContinueWithTranscriptFocus;
        }
    }

    if let Some(server) = app.mcp.server(server_id) {
        if !server.config.is_enabled() {
            let mut output = format!("## MCP for {}\n", server.config.display_name);
            output.push_str("MCP: **disabled**\n");
            output.push_str(&format!("Server id: `{}`\n", server.config.id));
            output.push_str("Status: disabled\n");
            output.push_str(&format!(
                "YOLO: {}\n",
                if server.config.is_yolo() {
                    "**enabled**"
                } else {
                    "disabled"
                }
            ));
            output.push('\n');
            app.conversation()
                .add_app_message(AppMessageKind::Info, output);
            app.ui.focus_transcript();
            return CommandResult::ContinueWithTranscriptFocus;
        }
    }

    app.conversation()
        .set_status("Refreshing MCP data...".to_string());
    app.ui
        .begin_activity(crate::core::app::ActivityKind::McpRefresh);
    CommandResult::RefreshMcp {
        server_id: server_id.to_string(),
    }
}

fn handle_mcp_toggle(app: &mut App, server_id: &str, new_state: bool) -> CommandResult {
    let Some(server) = app.mcp.server(server_id) else {
        app.conversation()
            .set_status(format!("Unknown MCP server: {}", server_id));
        return CommandResult::Continue;
    };
    let was_enabled = server.config.is_enabled();
    let server_label = server.config.id.clone();

    if let Some(server) = app.mcp.server_mut(server_id) {
        server.config.enabled = Some(new_state);
        if !new_state {
            clear_mcp_runtime_state(server);
        }
    }
    if let Some(server) = app
        .config
        .mcp_servers
        .iter_mut()
        .find(|server| server.id.eq_ignore_ascii_case(server_id))
    {
        server.enabled = Some(new_state);
    }

    let saved = match crate::core::config::data::Config::load() {
        Ok(mut cfg) => {
            if let Some(server) = cfg
                .mcp_servers
                .iter_mut()
                .find(|server| server.id.eq_ignore_ascii_case(server_id))
            {
                server.enabled = Some(new_state);
                cfg.save().is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    };

    let state_word = if new_state { "enabled" } else { "disabled" };
    let persist_note = if saved {
        "saved to config.toml"
    } else {
        "config.toml not saved"
    };
    if new_state && !was_enabled && !app.session.mcp_disabled {
        app.conversation().set_status(format!(
            "Refreshing MCP data for {} ({})",
            server_label, persist_note
        ));
        app.ui
            .begin_activity(crate::core::app::ActivityKind::McpRefresh);
        return CommandResult::RefreshMcp {
            server_id: server_id.to_string(),
        };
    }

    app.conversation().set_status(format!(
        "MCP {} for {} ({})",
        state_word, server_label, persist_note
    ));
    CommandResult::Continue
}

fn handle_mcp_forget(app: &mut App, server_id: &str) -> CommandResult {
    let Some(server) = app.mcp.server(server_id) else {
        app.conversation()
            .set_status(format!("Unknown MCP server: {}", server_id));
        return CommandResult::Continue;
    };
    let server_label = server.config.id.clone();

    if let Some(server) = app.mcp.server_mut(server_id) {
        server.config.enabled = Some(false);
        clear_mcp_runtime_state(server);
    }
    if let Some(server) = app
        .config
        .mcp_servers
        .iter_mut()
        .find(|server| server.id.eq_ignore_ascii_case(server_id))
    {
        server.enabled = Some(false);
    }

    let saved = match crate::core::config::data::Config::load() {
        Ok(mut cfg) => {
            if let Some(server) = cfg
                .mcp_servers
                .iter_mut()
                .find(|server| server.id.eq_ignore_ascii_case(server_id))
            {
                server.enabled = Some(false);
                cfg.save().is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    };

    app.mcp_permissions.clear_server(server_id);
    app.session.clear_mcp_tool_records(server_id);

    let persist_note = if saved {
        "saved to config.toml"
    } else {
        "config.toml not saved"
    };
    app.conversation().set_status(format!(
        "Forgot MCP data for {} ({})",
        server_label, persist_note
    ));
    CommandResult::Continue
}

fn clear_mcp_runtime_state(server: &mut crate::mcp::client::McpServerState) {
    server.clear_runtime_state();
}

pub(crate) fn build_mcp_server_output(
    server: &crate::mcp::client::McpServerState,
    keyring_enabled: bool,
    token_store: &McpTokenStore,
) -> String {
    let mut output = format!("## MCP for {}\n", server.config.display_name);
    if server.config.is_enabled() {
        output.push_str("MCP: enabled\n");
    } else {
        output.push_str("MCP: **disabled**\n");
    }
    output.push_str(&format!("Server id: `{}`\n", server.config.id));
    output.push_str(&format!(
        "Status: {}\n",
        if server.config.is_enabled() {
            "enabled"
        } else {
            "disabled"
        }
    ));
    output.push_str(&format!(
        "YOLO: {}\n",
        if server.config.is_yolo() {
            "**enabled** (permission prompts bypassed)"
        } else {
            "disabled"
        }
    ));
    output.push_str(&format!(
        "Transport: {}\n",
        server
            .config
            .transport
            .as_deref()
            .unwrap_or("streamable-http")
    ));
    output.push_str(&format!(
        "Connected: {}\n",
        if server.connected { "yes" } else { "no" }
    ));
    match crate::mcp::client::McpTransportKind::from_config(&server.config) {
        Ok(crate::mcp::client::McpTransportKind::Stdio) => {
            output.push_str("Token: not used (stdio)\n");
        }
        _ if keyring_enabled => match token_store.get_token(&server.config.id) {
            Ok(Some(_)) => output.push_str("Token: present\n"),
            Ok(None) => output.push_str("Token: missing\n"),
            Err(err) => output.push_str(&format!("Token: error ({})\n", err)),
        },
        _ => output.push_str("Token: unknown (keyring disabled)\n"),
    }

    let (tools_cap, resources_cap, prompts_cap, cap_reported) = server_capability_statuses(server);

    output.push('\n');
    match server.config.allowed_tools.as_ref() {
        Some(allowed) if allowed.is_empty() => {
            output.push_str("**Allowed tools (config):** none\n");
        }
        Some(allowed) => {
            output.push_str(&format!(
                "**Allowed tools (config):** {}\n",
                allowed.join(", ")
            ));
        }
        None => {
            output.push_str("**Allowed tools (config):** all\n");
        }
    }

    output.push('\n');
    if let Some(list) = &server.cached_tools {
        if list.tools.is_empty() {
            output.push_str(match (tools_cap, cap_reported) {
                (CapabilityStatus::Unsupported, true) => {
                    "**Tools:** not supported (per server capabilities).\n"
                }
                (_, false) => "**Tools:** unknown (server did not report capabilities).\n",
                _ => "**Tools:** none in cached listing.\n",
            });
        } else {
            output.push_str("**Tools:**\n");
            for tool in &list.tools {
                output.push_str(&format!("- {}\n", tool.name));
            }
        }
    } else {
        output.push_str("**Tools:** no cached listing yet.\n");
    }

    output.push('\n');
    if let Some(list) = &server.cached_resources {
        if list.resources.is_empty() {
            output.push_str(match (resources_cap, cap_reported) {
                (CapabilityStatus::Unsupported, true) => {
                    "**Resources:** not supported (per server capabilities).\n"
                }
                (_, false) => "**Resources:** unknown (server did not report capabilities).\n",
                _ => "**Resources:** none in cached listing.\n",
            });
        } else {
            output.push_str("**Resources:**\n");
            for resource in &list.resources {
                output.push_str(&format!("- {}\n", resource.uri));
            }
        }
    } else {
        output.push_str("**Resources:** no cached listing yet.\n");
    }

    output.push('\n');
    if let Some(list) = &server.cached_resource_templates {
        if list.resource_templates.is_empty() {
            output.push_str(match (resources_cap, cap_reported) {
                (CapabilityStatus::Unsupported, true) => {
                    "**Resource templates:** not supported (per server capabilities).\n"
                }
                (_, false) => {
                    "**Resource templates:** unknown (server did not report capabilities).\n"
                }
                _ => "**Resource templates:** none in cached listing.\n",
            });
        } else {
            output.push_str("**Resource templates:**\n");
            for template in &list.resource_templates {
                output.push_str(&format!(
                    "- {} ({})\n",
                    template.name, template.uri_template
                ));
            }
        }
    } else {
        output.push_str("**Resource templates:** no cached listing yet.\n");
    }

    output.push('\n');
    if let Some(list) = &server.cached_prompts {
        if list.prompts.is_empty() {
            output.push_str(match (prompts_cap, cap_reported) {
                (CapabilityStatus::Unsupported, true) => {
                    "**Prompts:** not supported (per server capabilities).\n"
                }
                (_, false) => "**Prompts:** unknown (server did not report capabilities).\n",
                _ => "**Prompts:** none in cached listing.\n",
            });
        } else {
            output.push_str("**Prompts:**\n");
            for prompt in &list.prompts {
                output.push_str(&format!("- {}\n", prompt.name));
            }
        }
    } else {
        output.push_str("**Prompts:** no cached listing yet.\n");
    }

    if let Some(err) = &server.last_error {
        output.push_str(&format!("\nLast error: {}\n", err));
    }

    output
}

#[derive(Clone, Copy)]
enum CapabilityStatus {
    Supported,
    Unsupported,
}

fn server_capability_statuses(
    server: &crate::mcp::client::McpServerState,
) -> (CapabilityStatus, CapabilityStatus, CapabilityStatus, bool) {
    let Some(details) = server.server_details.as_ref() else {
        return (
            CapabilityStatus::Supported,
            CapabilityStatus::Supported,
            CapabilityStatus::Supported,
            false,
        );
    };
    let caps = &details.capabilities;
    let tools = if caps.tools.is_some() {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unsupported
    };
    let resources = if caps.resources.is_some() {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unsupported
    };
    let prompts = if caps.prompts.is_some() {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unsupported
    };
    (tools, resources, prompts, true)
}

pub(super) fn handle_dump(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => {
            let timestamp = Utc::now().format("%Y-%m-%d").to_string();
            let filename = format!("chabeau-log-{}.txt", timestamp);
            match dump_conversation(app, &filename) {
                Ok(()) => handle_dump_result(app, Ok(()), &filename),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("already exists") {
                        app.conversation().set_status("Log file already exists.");
                        app.ui.start_file_prompt_dump(filename);
                        CommandResult::Continue
                    } else {
                        handle_dump_result(app, Err(e), &filename)
                    }
                }
            }
        }
        1 => {
            let filename = invocation.arg(0).unwrap();
            handle_dump_result(app, dump_conversation(app, filename), filename)
        }
        _ => {
            app.conversation().set_status("Usage: /dump [filename]");
            CommandResult::Continue
        }
    }
}

pub(super) fn handle_theme(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenThemePicker
    } else {
        let id = invocation.arg(0).unwrap();
        let res = {
            let mut controller = app.theme_controller();
            controller.apply_theme_by_id(id)
        };
        match res {
            Ok(_) => {
                app.conversation().set_status(format!("Theme set: {}", id));
                CommandResult::Continue
            }
            Err(_e) => {
                app.conversation().set_status("Theme error");
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_model(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenModelPicker
    } else {
        let model_id = invocation.arg(0).unwrap();
        {
            let mut controller = app.provider_controller();
            controller.apply_model_by_id(model_id);
        }
        app.conversation()
            .set_status(format!("Model set: {}", model_id));
        CommandResult::Continue
    }
}

pub(super) fn handle_provider(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenProviderPicker
    } else {
        let provider_id = invocation.arg(0).unwrap();
        let (result, should_open_model_picker) = {
            let mut controller = app.provider_controller();
            controller.apply_provider_by_id(provider_id)
        };
        match result {
            Ok(_) => {
                app.conversation()
                    .set_status(format!("Provider set: {}", provider_id));
                if should_open_model_picker {
                    CommandResult::OpenModelPicker
                } else {
                    CommandResult::Continue
                }
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Provider error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_markdown(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.markdown_enabled,
        ToggleText {
            usage: "Usage: /markdown [on|off|toggle]",
            feature: "Markdown",
            on_word: "enabled",
            off_word: "disabled",
        },
        |app, new_state| app.ui.markdown_enabled = new_state,
        |cfg, new_state| cfg.markdown = Some(new_state),
    )
}

pub(super) fn handle_syntax(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.syntax_enabled,
        ToggleText {
            usage: "Usage: /syntax [on|off|toggle]",
            feature: "Syntax",
            on_word: "on",
            off_word: "off",
        },
        |app, new_state| app.ui.syntax_enabled = new_state,
        |cfg, new_state| cfg.syntax = Some(new_state),
    )
}

pub(super) fn handle_character(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_text().is_empty() {
        CommandResult::OpenCharacterPicker
    } else {
        let character_name = invocation.args_text();
        match app.character_service.resolve(character_name) {
            Ok(card) => {
                let name = card.data.name.clone();
                app.session.set_character(card);
                app.conversation()
                    .set_status(format!("Character set: {}", name));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Character error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_persona(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenPersonaPicker
    } else {
        let persona_id = invocation.arg(0).unwrap();
        match app.persona_manager.set_active_persona(persona_id) {
            Ok(()) => {
                let active_persona_name = app
                    .persona_manager
                    .get_active_persona()
                    .map(|p| p.display_name.clone());

                let persona_name = active_persona_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());

                if active_persona_name.is_some() {
                    let display_name = app.persona_manager.get_display_name();
                    app.ui.update_user_display_name(display_name);
                } else {
                    app.ui.update_user_display_name("You".to_string());
                }
                app.conversation()
                    .set_status(format!("Persona activated: {}", persona_name));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Persona error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(super) fn handle_preset(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        CommandResult::OpenPresetPicker
    } else {
        let preset_id = invocation.arg(0).unwrap();
        if preset_id.eq_ignore_ascii_case("off") || preset_id == "[turn_off_preset]" {
            app.preset_manager.clear_active_preset();
            app.conversation()
                .set_status("Preset deactivated".to_string());
            CommandResult::Continue
        } else {
            match app.preset_manager.set_active_preset(preset_id) {
                Ok(()) => {
                    app.conversation()
                        .set_status(format!("Preset activated: {}", preset_id));
                    CommandResult::Continue
                }
                Err(e) => {
                    app.conversation()
                        .set_status(format!("Preset error: {}", e));
                    CommandResult::Continue
                }
            }
        }
    }
}

struct ToggleText {
    usage: &'static str,
    feature: &'static str,
    on_word: &'static str,
    off_word: &'static str,
}

fn handle_toggle_command<F, G>(
    app: &mut App,
    invocation: CommandInvocation<'_>,
    current_state: bool,
    text: ToggleText,
    mut apply_ui: F,
    mut persist_config: G,
) -> CommandResult
where
    F: FnMut(&mut App, bool),
    G: FnMut(&mut crate::core::config::data::Config, bool),
{
    let action = match invocation.toggle_action() {
        Ok(action) => action,
        Err(_) => {
            app.conversation().set_status(text.usage);
            return CommandResult::Continue;
        }
    };

    let new_state = action.apply(current_state);
    apply_ui(app, new_state);

    let state_word = if new_state {
        text.on_word
    } else {
        text.off_word
    };

    match crate::core::config::data::Config::load() {
        Ok(mut cfg) => {
            persist_config(&mut cfg, new_state);
            let status = if cfg.save().is_ok() {
                format!("{} {}", text.feature, state_word)
            } else {
                format!("{} {} (unsaved)", text.feature, state_word)
            };
            app.conversation().set_status(status);
        }
        Err(_) => {
            app.conversation()
                .set_status(format!("{} {}", text.feature, state_word));
        }
    }

    CommandResult::Continue
}

/// Dumps the conversation to a markdown file.
///
/// This function exports the chat transcript to a markdown file, including
/// user messages, assistant responses, and log entries. App-level messages
/// (info, warnings, errors) are excluded from the export.
///
/// # Arguments
///
/// * `app` - [`App`] state containing the conversation
/// * `filename` - Output file path
/// * `overwrite` - Whether to overwrite if the file already exists
///
/// # Errors
///
/// Returns an error if:
/// - The conversation is empty
/// - The file exists and `overwrite` is false
/// - File creation or writing fails
pub fn dump_conversation_with_overwrite(
    app: &App,
    filename: &str,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Filter out non-log app messages (keep user, assistant, and log messages)
    let conversation_messages: Vec<_> = app
        .ui
        .messages
        .iter()
        .filter(|msg| !message::is_app_message_role(&msg.role) || msg.role == message::ROLE_APP_LOG)
        .collect();

    if conversation_messages.is_empty() {
        return Err("No conversation to dump - the chat history is empty.".into());
    }

    // Check if file already exists
    if !overwrite && std::path::Path::new(filename).exists() {
        return Err(format!(
            "File '{}' already exists. Please specify a different filename with /dump <filename>.",
            filename
        )
        .into());
    }

    let file = File::create(filename)?;
    let mut writer = BufWriter::new(file);

    let user_display_name = app.persona_manager.get_display_name();

    for msg in conversation_messages {
        match msg.role.as_str() {
            "user" => writeln!(writer, "{}: {}", user_display_name, msg.content)?,
            message::ROLE_APP_LOG => writeln!(writer, "## {}", msg.content)?,
            message::ROLE_TOOL_CALL => writeln!(writer, "Tool call: {}", msg.content)?,
            message::ROLE_TOOL_RESULT => writeln!(writer, "Tool result: {}", msg.content)?,
            _ => writeln!(writer, "{}", msg.content)?, // For assistant messages
        }
        writeln!(writer)?; // Empty line for spacing
    }

    writer.flush()?;
    Ok(())
}

fn dump_conversation(app: &App, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    dump_conversation_with_overwrite(app, filename, false)
}

fn handle_dump_result(
    app: &mut App,
    result: Result<(), Box<dyn std::error::Error>>,
    filename: &str,
) -> CommandResult {
    match result {
        Ok(_) => {
            app.conversation()
                .set_status(format!("Dumped: {}", filename));
            CommandResult::Continue
        }
        Err(e) => {
            app.conversation().set_status(format!("Dump error: {}", e));
            CommandResult::Continue
        }
    }
}

#[cfg(test)]
mod tests;
