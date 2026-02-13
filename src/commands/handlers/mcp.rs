use super::{add_info_and_focus, required_arg, usage_status};
use crate::commands::mcp_prompt_parser::{parse_prompt_args, validate_prompt_args};
use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::session::McpPromptRequest;
use crate::core::app::App;
use crate::core::mcp_auth::McpTokenStore;
use crate::core::message::AppMessageKind;

const USAGE_MCP: &str = "Usage: /mcp <server-id> [on|off|forget]";
const USAGE_YOLO: &str = "Usage: /yolo <server-id> [on|off]";

pub(crate) fn handle_prompt_invocation(app: &mut App, input: &str) -> Option<CommandResult> {
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
    let Some(prompt) = prompts
        .prompts
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(prompt_name))
    else {
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
        if arg.required.unwrap_or(false) && !collected.contains_key(&arg.name) {
            missing.push(prompt_argument_from_schema(arg));
        }
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

pub(crate) fn handle_mcp(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let Some(server_id) = invocation.arg(0) else {
        return handle_mcp_list(app);
    };

    match invocation.args_len() {
        1 => handle_mcp_server(app, server_id),
        2 => {
            let Some(arg) = required_arg(app, &invocation, 1, USAGE_MCP) else {
                return CommandResult::Continue;
            };
            match arg.to_ascii_lowercase().as_str() {
                "on" => handle_mcp_toggle(app, server_id, true),
                "off" => handle_mcp_toggle(app, server_id, false),
                "forget" => handle_mcp_forget(app, server_id),
                _ => usage_status(app, USAGE_MCP),
            }
        }
        _ => usage_status(app, USAGE_MCP),
    }
}

pub(crate) fn handle_yolo(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 || invocation.args_len() > 2 {
        return usage_status(app, USAGE_YOLO);
    }

    let Some(server_id) = required_arg(app, &invocation, 0, USAGE_YOLO) else {
        return CommandResult::Continue;
    };

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
        return add_info_and_focus(app, output);
    }

    let Some(arg) = required_arg(app, &invocation, 1, USAGE_YOLO) else {
        return CommandResult::Continue;
    };
    let new_state = match arg.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => return usage_status(app, USAGE_YOLO),
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
            return add_info_and_focus(app, output);
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
            return add_info_and_focus(app, output);
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
            output.push_str("**Allowed tools (config):** none\n")
        }
        Some(allowed) => output.push_str(&format!(
            "**Allowed tools (config):** {}\n",
            allowed.join(", ")
        )),
        None => output.push_str("**Allowed tools (config):** all\n"),
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

fn prompt_argument_from_schema(
    arg: &rust_mcp_schema::PromptArgument,
) -> crate::core::app::ui_state::McpPromptArgument {
    crate::core::app::ui_state::McpPromptArgument {
        name: arg.name.clone(),
        title: arg.title.clone(),
        description: arg.description.clone(),
        required: arg.required.unwrap_or(false),
    }
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
