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

    if invocation.args_len() > 1 {
        app.conversation()
            .set_status("Usage: /mcp <server-id>".to_string());
        return CommandResult::Continue;
    }

    handle_mcp_server(app, server_id.unwrap())
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
        let yolo_marker = if server.config.is_yolo() {
            " — **YOLO**"
        } else {
            ""
        };
        output.push_str(&format!(
            "- **{}** ({}){}\n",
            server.config.id, server.config.display_name, yolo_marker
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

    app.conversation()
        .set_status("Refreshing MCP data...".to_string());
    app.ui
        .begin_activity(crate::core::app::ActivityKind::McpRefresh);
    CommandResult::RefreshMcp {
        server_id: server_id.to_string(),
    }
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
mod tests {
    use super::*;
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::core::config::data::{Config, McpServerConfig, Persona};
    use crate::core::message::ROLE_ASSISTANT;
    use crate::core::persona::PersonaManager;
    use crate::utils::test_utils::{create_test_app, create_test_message, with_test_config_env};
    use std::fs;
    use std::io::Read;
    use std::path::Path;
    use tempfile::tempdir;
    use toml::Value;

    fn read_config(path: &Path) -> Value {
        let contents = std::fs::read_to_string(path).unwrap();
        toml::from_str(&contents).unwrap()
    }

    #[test]
    fn clear_command_resets_transcript_state() {
        let mut app = create_test_app();
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));
        app.ui.current_response = "partial".to_string();
        app.session.retrying_message_index = Some(1);
        app.session.is_refining = true;
        app.session.original_refining_content = Some("original".to_string());
        app.session.last_refine_prompt = Some("prompt".to_string());
        app.session.has_received_assistant_message = true;
        app.session.character_greeting_shown = true;

        app.get_prewrapped_lines_cached(80);
        assert!(app.ui.prewrap_cache.is_some());

        let result = process_input(&mut app, "/clear");
        assert!(matches!(result, CommandResult::Continue));
        assert!(app.ui.messages.is_empty());
        assert!(app.ui.current_response.is_empty());
        assert_eq!(app.ui.status.as_deref(), Some("Transcript cleared"));
        assert!(app.ui.prewrap_cache.is_none());
        assert!(app.session.retrying_message_index.is_none());
        assert!(!app.session.is_refining);
        assert!(app.session.original_refining_content.is_none());
        assert!(app.session.last_refine_prompt.is_none());
        assert!(!app.session.has_received_assistant_message);
        assert!(!app.session.character_greeting_shown);
    }

    #[test]
    fn clear_command_shows_character_greeting_when_available() {
        let mut app = create_test_app();
        let greeting_text = "Greetings from TestBot!".to_string();
        let character = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestBot".to_string(),
                description: String::new(),
                personality: String::new(),
                scenario: String::new(),
                first_mes: greeting_text.clone(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        app.session.set_character(character);
        app.session.character_greeting_shown = true;
        app.session.has_received_assistant_message = true;
        app.ui
            .messages
            .push_back(create_test_message(ROLE_ASSISTANT, &greeting_text));
        app.ui
            .messages
            .push_back(create_test_message("user", "Hi!"));

        let result = process_input(&mut app, "/clear");
        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(app.ui.status.as_deref(), Some("Transcript cleared"));
        assert_eq!(app.ui.messages.len(), 1);
        let greeting = app.ui.messages.front().unwrap();
        assert_eq!(greeting.role, ROLE_ASSISTANT);
        assert_eq!(greeting.content, greeting_text);
        assert!(app.session.character_greeting_shown);
        assert!(!app.session.has_received_assistant_message);
    }

    #[test]
    fn registry_lists_commands() {
        let commands = super::all_commands();
        assert!(commands.iter().any(|cmd| cmd.name == "help"));
        assert!(commands.iter().any(|cmd| cmd.name == "markdown"));
        assert!(super::registry::find_command("help").is_some());
    }

    #[test]
    fn help_command_includes_registry_metadata() {
        let mut app = create_test_app();
        let result = process_input(&mut app, "/help");
        assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
        let last_message = app.ui.messages.back().expect("help message");
        assert!(last_message
            .content
            .contains("- `/help` — Show available commands"));
    }

    #[test]
    fn commands_dispatch_case_insensitively() {
        with_test_config_env(|_| {
            let mut app = create_test_app();
            app.ui.markdown_enabled = false;
            let result = process_input(&mut app, "/MarkDown On");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.markdown_enabled);
        });
    }

    #[test]
    fn dispatch_provides_multi_word_arguments() {
        use super::registry::DispatchOutcome;

        let registry = super::registry::registry();
        match registry.dispatch("/character Jean Luc Picard") {
            DispatchOutcome::Invocation(invocation) => {
                assert_eq!(invocation.command.name, "character");
                assert_eq!(invocation.args_text(), "Jean Luc Picard");
                let args: Vec<_> = invocation.args_iter().collect();
                assert_eq!(args, vec!["Jean", "Luc", "Picard"]);
                assert_eq!(invocation.arg(1), Some("Luc"));
            }
            other => panic!("unexpected dispatch outcome: {:?}", other),
        }
    }

    #[test]
    fn dispatch_reports_unknown_commands() {
        use super::registry::DispatchOutcome;

        let registry = super::registry::registry();
        assert!(matches!(
            registry.dispatch("/does-not-exist"),
            DispatchOutcome::UnknownCommand
        ));
    }

    #[test]
    fn markdown_command_rejects_invalid_argument() {
        with_test_config_env(|_| {
            let mut app = create_test_app();
            let result = process_input(&mut app, "/markdown banana");
            assert!(matches!(result, CommandResult::Continue));
            assert_eq!(
                app.ui.status.as_deref(),
                Some("Usage: /markdown [on|off|toggle]")
            );
        });
    }

    #[test]
    fn test_dump_conversation() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));
        app.ui.messages.push_back(create_test_message(
            crate::core::message::ROLE_APP_INFO,
            "App message",
        ));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("test_dump.txt");

        // Test the dump_conversation function
        assert!(dump_conversation(&app, dump_file_path.to_str().unwrap()).is_ok());

        // Read the dumped file and verify its contents
        let mut file = File::open(&dump_file_path).unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();

        // Check that the contents match what we expect
        assert!(contents.contains("You: Hello"));
        assert!(contents.contains("Hi there!"));
        // App messages should be excluded from dumps
        assert!(!contents.contains("App message"));

        // Clean up
        drop(file);
        fs::remove_file(&dump_file_path).unwrap();
    }

    #[test]
    fn dump_conversation_uses_persona_display_name() {
        let mut app = create_test_app();

        let config = Config {
            personas: vec![Persona {
                id: "captain".to_string(),
                display_name: "Captain".to_string(),
                bio: None,
            }],
            ..Default::default()
        };

        app.persona_manager = PersonaManager::load_personas(&config).unwrap();
        app.persona_manager
            .set_active_persona("captain")
            .expect("Failed to activate persona");

        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));

        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("persona_dump.txt");
        dump_conversation_with_overwrite(&app, dump_file_path.to_str().unwrap(), true)
            .expect("failed to dump conversation");

        let contents = fs::read_to_string(&dump_file_path).expect("failed to read dump file");
        assert!(
            contents.contains("Captain: Hello"),
            "Dump should include persona display name, contents: {contents}"
        );
    }

    #[test]
    fn markdown_command_updates_state_and_persists() {
        with_test_config_env(|config_root| {
            let config_path = config_root.join("chabeau").join("config.toml");
            let mut app = create_test_app();
            app.ui.markdown_enabled = true;

            let result = process_input(&mut app, "/markdown off");
            assert!(matches!(result, CommandResult::Continue));
            assert!(!app.ui.markdown_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Markdown disabled"));

            assert!(config_path.exists());
            let config = read_config(&config_path);
            assert_eq!(config["markdown"].as_bool(), Some(false));

            let result = process_input(&mut app, "/markdown toggle");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.markdown_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Markdown enabled"));

            let config = read_config(&config_path);
            assert_eq!(config["markdown"].as_bool(), Some(true));
        });
    }

    #[test]
    fn syntax_command_updates_state_and_persists() {
        with_test_config_env(|config_root| {
            let config_path = config_root.join("chabeau").join("config.toml");
            let mut app = create_test_app();
            app.ui.syntax_enabled = true;

            let result = process_input(&mut app, "/syntax off");
            assert!(matches!(result, CommandResult::Continue));
            assert!(!app.ui.syntax_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Syntax off"));

            assert!(config_path.exists());
            let config = read_config(&config_path);
            assert_eq!(config["syntax"].as_bool(), Some(false));

            let result = process_input(&mut app, "/syntax toggle");
            assert!(matches!(result, CommandResult::Continue));
            assert!(app.ui.syntax_enabled);
            assert_eq!(app.ui.status.as_deref(), Some("Syntax on"));

            let config = read_config(&config_path);
            assert_eq!(config["syntax"].as_bool(), Some(true));
        });
    }

    #[test]
    fn test_dump_conversation_file_exists() {
        // Create a mock app with some messages
        let mut app = create_test_app();

        // Add messages
        app.ui
            .messages
            .push_back(create_test_message("user", "Hello"));
        app.ui
            .messages
            .push_back(create_test_message("assistant", "Hi there!"));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("test_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Create a file that already exists
        fs::write(&dump_file_path, "existing content").unwrap();

        // Test the dump_conversation function with existing file
        // This should fail because the file already exists
        let result = dump_conversation(&app, dump_filename);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));

        // Check that the existing file content is still there
        let contents = fs::read_to_string(&dump_file_path).unwrap();
        assert_eq!(contents, "existing content");

        // Clean up
        fs::remove_file(&dump_file_path).unwrap();
    }

    #[test]
    fn test_process_input_dump_with_filename() {
        let mut app = create_test_app();

        // Add a message to test dumping
        app.ui
            .messages
            .push_back(create_test_message("user", "Test message"));

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("custom_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should set a status about the dump
        assert!(app.ui.status.is_some());
        assert!(app.ui.status.as_ref().unwrap().starts_with("Dumped: "));

        // Clean up
        fs::remove_file(dump_filename).ok();
    }

    #[test]
    fn test_process_input_dump_empty_conversation() {
        let mut app = create_test_app();

        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let dump_file_path = temp_dir.path().join("empty_dump.txt");
        let dump_filename = dump_file_path.to_str().unwrap();

        // Process the /dump command with an empty conversation
        let result = process_input(&mut app, &format!("/dump {}", dump_filename));

        // Should continue (not process as message)
        assert!(matches!(result, CommandResult::Continue));

        // Should set a status with an error
        assert!(app.ui.status.is_some());
        assert!(app.ui.status.as_ref().unwrap().starts_with("Dump error:"));
    }

    #[test]
    fn theme_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/theme");
        assert!(matches!(res, CommandResult::OpenThemePicker));
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn model_command_returns_open_picker_result() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/model");
        assert!(matches!(res, CommandResult::OpenModelPicker));
    }

    #[test]
    fn model_command_with_id_sets_model() {
        let mut app = create_test_app();
        let original_model = app.session.model.clone();
        let res = process_input(&mut app, "/model gpt-4");
        assert!(matches!(res, CommandResult::Continue));
        assert_eq!(app.session.model, "gpt-4");
        assert_ne!(app.session.model, original_model);
    }

    #[test]
    fn provider_command_with_same_id_reuses_session() {
        let mut app = create_test_app();
        app.picker.provider_model_transition_state = Some((
            "prev-provider".into(),
            "Prev".into(),
            "prev-model".into(),
            "prev-key".into(),
            "https://prev.example".into(),
        ));
        app.picker.in_provider_model_transition = false;

        let result = process_input(&mut app, "/provider TEST");

        assert!(matches!(result, CommandResult::Continue));
        assert_eq!(app.session.provider_name, "test");
        assert_eq!(app.session.api_key, "test-key");
        assert_eq!(app.ui.status.as_deref(), Some("Provider set: TEST"));
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
    }

    #[test]
    fn theme_picker_supports_filtering() {
        let mut app = create_test_app();
        app.open_theme_picker().expect("theme picker opens");

        // Should store all themes for filtering
        assert!(app
            .theme_picker_state()
            .map(|state| !state.all_items.is_empty())
            .unwrap_or(false));

        // Should start with empty filter
        assert!(app
            .theme_picker_state()
            .map(|state| state.search_filter.is_empty())
            .unwrap_or(true));

        // Add a filter and verify filtering works
        if let Some(state) = app.theme_picker_state_mut() {
            state.search_filter.push_str("dark");
        }
        app.filter_themes();

        if let Some(picker) = app.picker_state() {
            // Should have filtered results
            let total = app
                .theme_picker_state()
                .map(|state| state.all_items.len())
                .unwrap_or(0);
            assert!(picker.items.len() <= total);
            // Title should show filter status
            assert!(picker.title.contains("filter: 'dark'"));
        }
    }

    #[test]
    fn picker_supports_home_end_navigation_and_metadata() {
        let mut app = create_test_app();
        app.open_theme_picker().expect("theme picker opens");

        if let Some(picker) = app.picker_state_mut() {
            // Test Home key (move to start)
            picker.selected = picker.items.len() - 1; // Move to last
            picker.move_to_start();
            assert_eq!(picker.selected, 0);

            // Test End key (move to end)
            picker.move_to_end();
            assert_eq!(picker.selected, picker.items.len() - 1);

            // Test metadata is available
            let metadata = picker.get_selected_metadata();
            assert!(metadata.is_some());

            // Test sort mode cycling
            let original_sort = picker.sort_mode.clone();
            picker.cycle_sort_mode();
            assert_ne!(picker.sort_mode, original_sort);

            // Test items have metadata
            assert!(picker.items.iter().any(|item| item.metadata.is_some()));
        }
    }

    #[test]
    fn theme_picker_shows_a_z_sort_indicators() {
        let mut app = create_test_app();

        // Open theme picker - should default to A-Z (Name mode)
        app.open_theme_picker().expect("theme picker opens");

        if let Some(picker) = app.picker_state() {
            // Should default to Name mode (A-Z)
            assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Name);
            // Title should show "Sort by: A-Z"
            assert!(
                picker.title.contains("Sort by: A-Z"),
                "Theme picker should show 'Sort by: A-Z', got: {}",
                picker.title
            );
        }

        // Cycle to Z-A mode
        if let Some(picker) = app.picker_state_mut() {
            picker.cycle_sort_mode();
        }
        app.sort_picker_items();
        app.update_picker_title();

        if let Some(picker) = app.picker_state() {
            // Should now be in Date mode (Z-A for themes)
            assert_eq!(picker.sort_mode, crate::ui::picker::SortMode::Date);
            // Title should show "Sort by: Z-A"
            assert!(
                picker.title.contains("Sort by: Z-A"),
                "Theme picker should show 'Sort by: Z-A', got: {}",
                picker.title
            );
        }
    }

    #[test]
    fn character_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/character");
        assert!(matches!(res, CommandResult::OpenCharacterPicker));
    }

    #[test]
    fn character_command_with_invalid_name_shows_error() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/character nonexistent_character");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
        let status = app.ui.status.as_ref().unwrap();
        assert!(
            status.contains("Character error") || status.contains("not found"),
            "Expected error message, got: {}",
            status
        );
    }

    #[test]
    fn character_command_registered_in_help() {
        let commands = super::all_commands();
        assert!(commands.iter().any(|cmd| cmd.name == "character"));

        let character_cmd = commands.iter().find(|cmd| cmd.name == "character").unwrap();
        assert_eq!(character_cmd.usages.len(), 2);
        assert!(character_cmd.usages[0].syntax.contains("/character"));
        assert!(character_cmd.usages[1].syntax.contains("<name>"));
    }

    #[test]
    fn persona_command_opens_picker() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/persona");
        assert!(matches!(res, CommandResult::OpenPersonaPicker));
    }

    #[test]
    fn persona_command_with_invalid_id_shows_error() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/persona nonexistent_persona");
        assert!(matches!(res, CommandResult::Continue));
        assert!(app.ui.status.is_some());
        let status = app.ui.status.as_ref().unwrap();
        assert!(
            status.contains("Persona error") || status.contains("not found"),
            "Expected error message, got: {}",
            status
        );
    }

    #[test]
    fn persona_command_with_valid_id_updates_user_display_name() {
        let mut app = create_test_app();
        let mut config = crate::core::config::data::Config::default();
        config.personas.push(crate::core::config::data::Persona {
            id: "alice-dev".to_string(),
            display_name: "Alice".to_string(),
            bio: Some("A senior software developer".to_string()),
        });
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config).unwrap();
        assert_eq!(app.ui.user_display_name, "You");

        let res = process_input(&mut app, "/persona alice-dev");

        assert!(matches!(res, CommandResult::Continue));
        assert_eq!(app.ui.user_display_name, "Alice");
    }

    #[test]
    fn mcp_command_lists_empty_config() {
        let mut app = create_test_app();
        let res = process_input(&mut app, "/mcp");
        assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
        let last = app.ui.messages.back().expect("app message");
        assert!(last.content.contains("MCP servers"));
        assert!(last.content.contains("No MCP servers configured"));
    }

    #[test]
    fn mcp_command_highlights_disabled_state() {
        let mut app = create_test_app();
        app.session.mcp_disabled = true;
        let res = process_input(&mut app, "/mcp");
        assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
        let last = app.ui.messages.back().expect("app message");
        assert!(last.content.contains("MCP: **disabled for this session**"));
    }

    #[test]
    fn mcp_command_highlights_yolo_servers() {
        let mut app = create_test_app();
        app.config.mcp_servers.push(McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            yolo: Some(true),
        });
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let res = process_input(&mut app, "/mcp");
        assert!(matches!(res, CommandResult::ContinueWithTranscriptFocus));
        let last = app.ui.messages.back().expect("app message");
        assert!(last.content.contains("**YOLO**"));
    }

    #[test]
    fn mcp_command_includes_allowed_tools() {
        let mut app = create_test_app();
        app.config
            .mcp_servers
            .push(crate::core::config::data::McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                base_url: Some("https://mcp.example.com".to_string()),
                command: None,
                args: None,
                env: None,
                transport: Some("streamable-http".to_string()),
                allowed_tools: Some(vec!["weather.lookup".to_string(), "time.now".to_string()]),
                protocol_version: Some("2024-11-05".to_string()),
                enabled: Some(true),
                yolo: None,
            });
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let res = process_input(&mut app, "/mcp alpha");
        assert!(matches!(
            res,
            CommandResult::RefreshMcp {
                server_id: ref id
            } if id == "alpha"
        ));
        assert_eq!(app.ui.status.as_deref(), Some("Refreshing MCP data..."));
        assert_eq!(
            app.ui.activity_indicator,
            Some(crate::core::app::ActivityKind::McpRefresh)
        );
    }

    #[test]
    fn yolo_command_shows_and_persists() {
        with_test_config_env(|config_root| {
            let config_path = config_root.join("chabeau").join("config.toml");
            let mut config = Config::default();
            config.mcp_servers.push(McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                base_url: Some("https://mcp.example.com".to_string()),
                command: None,
                args: None,
                env: None,
                transport: Some("streamable-http".to_string()),
                allowed_tools: None,
                protocol_version: None,
                enabled: Some(true),
                yolo: None,
            });
            config.save().expect("save config");

            let mut app = create_test_app();
            app.config = config.clone();
            app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

            let result = process_input(&mut app, "/yolo alpha");
            assert!(matches!(result, CommandResult::ContinueWithTranscriptFocus));
            let last = app.ui.messages.back().expect("app message");
            assert!(last.content.contains("YOLO: disabled"));

            let result = process_input(&mut app, "/yolo alpha on");
            assert!(matches!(result, CommandResult::Continue));
            let status = app.ui.status.as_deref().unwrap_or_default();
            assert!(status.contains("YOLO enabled"));
            assert!(status.contains("saved to config.toml"));

            let config = read_config(&config_path);
            let yolo = config
                .get("mcp_servers")
                .and_then(|servers| servers.as_array())
                .and_then(|servers| servers.first())
                .and_then(|server| server.get("yolo"))
                .and_then(|value| value.as_bool());
            assert_eq!(yolo, Some(true));
        });
    }

    #[test]
    fn parse_kv_args_supports_quotes() {
        let args = parse_kv_args("topic=\"soil health\" lang=en").expect("parse");
        assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
        assert_eq!(args.get("lang").map(String::as_str), Some("en"));
    }

    #[test]
    fn parse_kv_args_rejects_missing_equals() {
        let err = parse_kv_args("topic").unwrap_err();
        assert!(err.contains("key=value"));
    }

    #[test]
    fn parse_prompt_args_single_argument_accepts_bare_value() {
        let prompt_args = vec![PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        }];
        let args = parse_prompt_args("soil", &prompt_args).expect("parse");
        assert_eq!(args.get("topic").map(String::as_str), Some("soil"));
    }

    #[test]
    fn parse_prompt_args_single_argument_accepts_quoted_value() {
        let prompt_args = vec![PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        }];
        let args = parse_prompt_args("\"soil health\"", &prompt_args).expect("parse");
        assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
    }

    #[test]
    fn parse_prompt_args_single_argument_accepts_unquoted_spaces() {
        let prompt_args = vec![PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        }];
        let args = parse_prompt_args("soil health", &prompt_args).expect("parse");
        assert_eq!(args.get("topic").map(String::as_str), Some("soil health"));
    }

    #[test]
    fn parse_prompt_args_multiple_arguments_requires_key_value() {
        let prompt_args = vec![
            PromptArgument {
                name: "topic".to_string(),
                title: None,
                description: None,
                required: Some(true),
            },
            PromptArgument {
                name: "lang".to_string(),
                title: None,
                description: None,
                required: Some(true),
            },
        ];
        let err = parse_prompt_args("soil", &prompt_args).unwrap_err();
        assert!(err.contains("key=value"));
    }

    #[test]
    fn validate_prompt_args_rejects_unknown_keys() {
        let prompt_args = vec![PromptArgument {
            name: "topic".to_string(),
            title: None,
            description: None,
            required: Some(true),
        }];
        let mut args = HashMap::new();
        args.insert("foo".to_string(), "bar".to_string());
        let err = validate_prompt_args(&args, &prompt_args).unwrap_err();
        assert!(err.contains("Unknown prompt argument"));
        assert!(err.contains("topic"));
    }
}
