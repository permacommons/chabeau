use std::collections::VecDeque;
use std::time::Instant;

use super::{input, App, AppAction, AppActionContext, AppCommand};
use crate::api::{ChatMessage, ChatToolCall, ChatToolCallFunction};
use crate::core::app::picker::build_inspect_text;
use crate::core::app::session::{
    McpPromptRequest, PendingToolCall, ToolCallRequest, ToolResultRecord, ToolResultStatus,
};
use crate::core::chat_stream::StreamParams;
use crate::core::message::{AppMessageKind, Message, ROLE_ASSISTANT, ROLE_USER};
use crate::mcp::permissions::ToolPermissionDecision;
use rust_mcp_sdk::schema::{ContentBlock, PromptMessage};
use serde_json::{Map, Value};

pub(super) fn handle_streaming_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::McpInitCompleted => handle_mcp_init_completed(app, ctx),
        AppAction::McpSendPendingWithoutTools => handle_mcp_send_without_tools(app, ctx),
        AppAction::AppendResponseChunk { content, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            append_response_chunk(app, &content, ctx);
            None
        }
        AppAction::StreamAppMessage {
            kind,
            message,
            stream_id,
        } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            append_stream_app_message(app, kind, message, ctx);
            None
        }
        AppAction::StreamToolCallDelta { delta, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            append_tool_call_delta(app, delta);
            None
        }
        AppAction::ToolPermissionDecision { decision } => {
            handle_tool_permission_decision(app, decision, ctx)
        }
        AppAction::ToolPromptInspect => handle_tool_prompt_inspect(app, ctx),
        AppAction::ToolCallCompleted {
            tool_name,
            tool_call_id,
            result,
        } => handle_tool_call_completed(app, tool_name, tool_call_id, result, ctx),
        AppAction::McpPromptCompleted { request, result } => {
            handle_mcp_prompt_completed(app, request, result, ctx)
        }
        AppAction::StreamErrored { message, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            handle_stream_error(app, message, ctx)
        }
        AppAction::StreamCompleted { stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            finalize_stream(app, ctx)
        }
        AppAction::CancelStreaming => {
            app.cancel_current_stream();
            None
        }
        AppAction::SubmitMessage { message } => spawn_stream_for_message(app, message, ctx),
        AppAction::RefineLastMessage { prompt } => refine_last_message(app, prompt, ctx),
        AppAction::RetryLastMessage => retry_last_message(app, ctx),
        _ => unreachable!("non-streaming action routed to streaming handler"),
    }
}

fn handle_tool_prompt_inspect(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let prompt = app.ui.tool_prompt().cloned()?;

    let server_label = if prompt.server_name.trim().is_empty() {
        prompt.server_id.clone()
    } else {
        prompt.server_name.clone()
    };

    let mut lines = Vec::new();
    lines.push(format!("Tool: {}", prompt.tool_name));
    lines.push(format!("Server: {}", server_label));
    if prompt.server_id.trim() != server_label.trim() {
        lines.push(format!("Server ID: {}", prompt.server_id));
    }
    lines.push(String::new());
    lines.push("Arguments:".to_string());

    let args_text = format_tool_arguments_for_inspect(&prompt.raw_arguments);
    if args_text.trim().is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for line in args_text.lines() {
            lines.push(format!("  {}", line));
        }
    }

    let title = format!("Tool call – {} on {}", prompt.tool_name, server_label);
    let content = build_inspect_text(lines);
    app.open_inspect(title, content);
    input::set_status_message(app, "Inspecting tool call (Esc=Close)".to_string(), ctx);
    None
}

fn format_tool_arguments_for_inspect(raw_arguments: &str) -> String {
    let trimmed = raw_arguments.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string()),
        Err(_) => trimmed.to_string(),
    }
}

pub(super) fn spawn_stream_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if should_defer_for_mcp(app) {
        app.session.pending_mcp_message = Some(message);
        set_status_for_mcp_wait(app, ctx);
        return None;
    }

    let params = prepare_stream_params_for_message(app, message, ctx);
    Some(AppCommand::SpawnStream(params))
}

pub(super) fn retry_last_message(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    prepare_retry_stream(app, ctx)
}

pub(super) fn refine_last_message(
    app: &mut App,
    prompt: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    prepare_refine_stream(app, prompt, ctx)
}

fn append_response_chunk(app: &mut App, chunk: &str, ctx: AppActionContext) {
    if chunk.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.append_to_response(chunk, available_height, ctx.term_width);
}

fn append_stream_app_message(
    app: &mut App,
    kind: AppMessageKind,
    message: String,
    ctx: AppActionContext,
) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    conversation.add_app_message(kind, trimmed.to_string());
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn handle_mcp_init_completed(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    app.session.mcp_init_complete = true;
    app.session.mcp_init_in_progress = false;
    if let Some(message) = app.session.pending_mcp_message.take() {
        app.clear_status();
        spawn_stream_for_message(app, message, ctx)
    } else {
        None
    }
}

fn handle_mcp_send_without_tools(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    if let Some(message) = app.session.pending_mcp_message.take() {
        app.clear_status();
        Some(AppCommand::SpawnStream(prepare_stream_params_for_message(
            app, message, ctx,
        )))
    } else {
        None
    }
}

fn should_defer_for_mcp(app: &App) -> bool {
    if app.session.mcp_tools_unsupported
        || app.session.mcp_init_complete
        || !app.session.mcp_init_in_progress
    {
        return false;
    }

    app.mcp.servers().any(|server| server.config.is_enabled())
}

fn set_status_for_mcp_wait(app: &mut App, ctx: AppActionContext) {
    let input_area_height = app.input_area_height(ctx.term_width);
    let mcp_name = app
        .mcp
        .servers()
        .find(|server| server.config.is_enabled())
        .map(|server| server.config.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "MCP".to_string());
    let mut conversation = app.conversation();
    conversation.set_status(format!(
        "Waiting for MCP tools ({mcp_name})... Press Enter to send without tools."
    ));
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn append_tool_call_delta(app: &mut App, delta: crate::core::chat_stream::ToolCallDelta) {
    let entry = app
        .session
        .pending_tool_calls
        .entry(delta.index)
        .or_insert_with(|| PendingToolCall {
            id: None,
            name: None,
            arguments: String::new(),
        });

    if delta.id.is_some() {
        entry.id = delta.id;
    }
    if delta.name.is_some() {
        entry.name = delta.name;
    }
    if let Some(arguments) = delta.arguments {
        entry.arguments.push_str(&arguments);
    }
}

fn handle_stream_error(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let error_message = message.trim().to_string();
    if app.session.mcp_tools_enabled
        && !app.session.mcp_tools_unsupported
        && is_tool_unsupported_error(&error_message)
    {
        return handle_mcp_unsupported_error(app, ctx);
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        conversation.clear_pending_tool_calls();
        conversation.add_app_message(AppMessageKind::Error, error_message);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
    app.end_streaming();
    None
}

fn handle_mcp_unsupported_error(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    app.session.mcp_tools_unsupported = true;
    app.session.mcp_tools_enabled = false;

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        conversation.clear_pending_tool_calls();
        conversation.add_app_message(
            AppMessageKind::Warning,
            "MCP tool-calling is enabled, but the currently selected model does not support it. It will be disabled until you switch models."
                .to_string(),
        );
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
    app.end_streaming();

    let base_messages = app.session.last_stream_api_messages_base.clone()?;

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    app.ui.focus_transcript();
    app.enable_auto_scroll();

    let input_area_height = app.input_area_height(ctx.term_width);
    let (cancel_token, stream_id) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        conversation.add_assistant_placeholder();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
        (cancel_token, stream_id)
    };

    Some(AppCommand::SpawnStream(app.build_stream_params(
        base_messages,
        cancel_token,
        stream_id,
    )))
}

fn is_tool_unsupported_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let mentions_tools = lower.contains("tools")
        || lower.contains("tool_calls")
        || lower.contains("tool call")
        || lower.contains("function_call")
        || lower.contains("function calling");
    if !mentions_tools {
        return false;
    }

    let unsupported_signals = [
        "not supported",
        "unsupported",
        "unknown field",
        "unknown parameter",
        "unrecognized",
        "unexpected field",
        "invalid parameter",
        "extra fields",
        "does not support",
    ];

    unsupported_signals
        .iter()
        .any(|signal| lower.contains(signal))
}

fn finalize_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let input_area_height = app.input_area_height(ctx.term_width);
    let pending_tool_calls = {
        let mut conversation = app.conversation();
        let pending = conversation.take_pending_tool_calls();
        if !pending.is_empty() {
            let available_height =
                conversation.calculate_available_height(ctx.term_height, input_area_height);
            conversation.update_scroll_position(available_height, ctx.term_width);
        }
        conversation.finalize_response();
        pending
    };

    app.end_streaming();

    if pending_tool_calls.is_empty() {
        app.session.last_stream_api_messages = None;
        return None;
    }

    prepare_tool_flow(app, pending_tool_calls, ctx)
}

fn handle_tool_permission_decision(
    app: &mut App,
    decision: ToolPermissionDecision,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let request = app.session.active_tool_request.take()?;

    app.ui.cancel_tool_prompt();
    app.clear_status();

    match decision {
        ToolPermissionDecision::AllowOnce => {}
        ToolPermissionDecision::AllowSession | ToolPermissionDecision::Block => app
            .mcp_permissions
            .record(&request.server_id, &request.tool_name, decision),
        ToolPermissionDecision::DenyOnce => {}
    }

    if matches!(
        decision,
        ToolPermissionDecision::DenyOnce | ToolPermissionDecision::Block
    ) {
        let message = match decision {
            ToolPermissionDecision::Block => "Tool blocked by user.",
            _ => "Tool denied by user.",
        };
        let server_label = resolve_server_label(app, &request.server_id);
        let status = match decision {
            ToolPermissionDecision::Block => ToolResultStatus::Blocked,
            _ => ToolResultStatus::Denied,
        };
        record_tool_result(
            app,
            &request.tool_name,
            Some(server_label),
            request.tool_call_id.clone(),
            message.to_string(),
            status,
            ctx,
        );
        return advance_tool_queue(app, ctx);
    }

    app.session.active_tool_request = Some(request.clone());
    set_status_for_tool_run(app, &request, ctx);
    Some(AppCommand::RunMcpTool(request))
}

fn handle_tool_call_completed(
    app: &mut App,
    tool_name: String,
    tool_call_id: Option<String>,
    result: Result<String, String>,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let request = app.session.active_tool_request.take();
    let server_label = request
        .as_ref()
        .map(|req| resolve_server_label(app, &req.server_id));
    app.clear_status();

    match result {
        Ok(payload) => {
            record_tool_result(
                app,
                &tool_name,
                server_label,
                tool_call_id,
                payload,
                ToolResultStatus::Success,
                ctx,
            );
        }
        Err(err) => {
            record_tool_result(
                app,
                &tool_name,
                server_label,
                tool_call_id,
                format!("Tool error: {err}"),
                ToolResultStatus::Error,
                ctx,
            );
        }
    }

    advance_tool_queue(app, ctx)
}

fn handle_mcp_prompt_completed(
    app: &mut App,
    request: McpPromptRequest,
    result: Result<rust_mcp_sdk::schema::GetPromptResult, String>,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    app.clear_status();

    let prompt_result = match result {
        Ok(result) => result,
        Err(err) => {
            app.conversation().add_app_message(
                AppMessageKind::Error,
                format!(
                    "MCP prompt {}:{} failed: {}",
                    request.server_id, request.prompt_name, err
                ),
            );
            return None;
        }
    };

    if prompt_result.messages.is_empty() {
        app.conversation().add_app_message(
            AppMessageKind::Warning,
            format!(
                "MCP prompt {}:{} returned no messages.",
                request.server_id, request.prompt_name
            ),
        );
        return None;
    }

    app.ui.focus_transcript();
    app.enable_auto_scroll();

    let term_width = ctx.term_width.max(1);
    let term_height = ctx.term_height.max(1);
    let input_area_height = app.input_area_height(term_width);

    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        conversation.remove_trailing_empty_assistant_messages();
        for message in prompt_result.messages.iter() {
            let content = prompt_message_content_to_string(message);
            let role = match message.role {
                rust_mcp_sdk::schema::Role::User => ROLE_USER,
                rust_mcp_sdk::schema::Role::Assistant => ROLE_ASSISTANT,
            };
            conversation.add_message(Message {
                role: role.to_string(),
                content,
            });
        }

        conversation.add_app_message(
            AppMessageKind::Info,
            format!(
                "Applied MCP prompt {}:{}.",
                request.server_id, request.prompt_name
            ),
        );

        let available_height =
            conversation.calculate_available_height(term_height, input_area_height);
        conversation.update_scroll_position(available_height, term_width);

        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.api_messages_from_history();
        (cancel_token, stream_id, api_messages)
    };

    Some(AppCommand::SpawnStream(app.build_stream_params(
        api_messages,
        cancel_token,
        stream_id,
    )))
}

fn prompt_message_content_to_string(message: &PromptMessage) -> String {
    prompt_content_to_string(&message.content)
}

fn prompt_content_to_string(content: &ContentBlock) -> String {
    match content {
        ContentBlock::TextContent(text) => text.text.clone(),
        _ => serde_json::to_string(content)
            .unwrap_or_else(|_| "Unsupported prompt content.".to_string()),
    }
}

fn prepare_tool_flow(
    app: &mut App,
    pending_tool_calls: Vec<(u32, PendingToolCall)>,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let mut tool_call_records = Vec::new();
    let mut pending_queue = VecDeque::new();
    let mut pending_errors: Vec<(String, Option<String>, String)> = Vec::new();

    for (index, call) in pending_tool_calls {
        let tool_name = call.name.clone().unwrap_or_else(|| "unknown".to_string());
        let tool_call_id = call
            .id
            .clone()
            .unwrap_or_else(|| format!("tool-call-{index}"));
        let raw_arguments = call.arguments.trim().to_string();

        tool_call_records.push(ChatToolCall {
            id: tool_call_id.clone(),
            kind: "function".to_string(),
            function: ChatToolCallFunction {
                name: tool_name.clone(),
                arguments: raw_arguments.clone(),
            },
        });

        if call.name.is_none() {
            pending_errors.push((
                tool_name.clone(),
                Some(tool_call_id.clone()),
                "Missing tool name.".to_string(),
            ));
            continue;
        }

        if tool_name.eq_ignore_ascii_case(crate::mcp::MCP_READ_RESOURCE_TOOL) {
            match parse_resource_read_arguments(&raw_arguments) {
                Ok((server_id, _uri, arguments)) => {
                    match app.mcp.server(&server_id) {
                        Some(server) if server.config.is_enabled() => {}
                        Some(_) => {
                            pending_errors.push((
                                tool_name.clone(),
                                Some(tool_call_id.clone()),
                                format!("MCP server is disabled: {server_id}."),
                            ));
                            continue;
                        }
                        None => {
                            pending_errors.push((
                                tool_name.clone(),
                                Some(tool_call_id.clone()),
                                format!("Unknown MCP server id: {server_id}."),
                            ));
                            continue;
                        }
                    }
                    pending_queue.push_back(ToolCallRequest {
                        server_id,
                        tool_name: tool_name.clone(),
                        arguments: Some(arguments),
                        raw_arguments,
                        tool_call_id: Some(tool_call_id),
                    });
                    continue;
                }
                Err(err) => {
                    pending_errors.push((tool_name.clone(), Some(tool_call_id.clone()), err));
                    continue;
                }
            }
        } else {
            let server_id = match resolve_tool_server(app, &tool_name) {
                Ok((server_id, _)) => server_id,
                Err(err) => {
                    pending_errors.push((tool_name.clone(), Some(tool_call_id.clone()), err));
                    continue;
                }
            };

            match parse_tool_arguments(&raw_arguments) {
                Ok(arguments) => {
                    pending_queue.push_back(ToolCallRequest {
                        server_id,
                        tool_name: tool_name.clone(),
                        arguments,
                        raw_arguments,
                        tool_call_id: Some(tool_call_id),
                    });
                    continue;
                }
                Err(err) => {
                    pending_errors.push((
                        tool_name.clone(),
                        Some(tool_call_id.clone()),
                        format!("Invalid tool arguments: {err}"),
                    ));
                    continue;
                }
            }
        }
    }

    app.session.tool_call_records = tool_call_records;
    app.session.pending_tool_queue = pending_queue;
    app.session.active_tool_request = None;
    app.session.tool_results.clear();

    for (tool_name, tool_call_id, error) in pending_errors {
        record_tool_result(
            app,
            &tool_name,
            None,
            tool_call_id,
            error,
            ToolResultStatus::Error,
            ctx,
        );
    }

    advance_tool_queue(app, ctx)
}

fn advance_tool_queue(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    if app.session.active_tool_request.is_some() {
        return None;
    }

    let Some(request) = app.session.pending_tool_queue.pop_front() else {
        return spawn_stream_after_tools(app, ctx);
    };

    if let Some(decision) = app
        .mcp_permissions
        .decision_for(&request.server_id, &request.tool_name)
    {
        if decision == ToolPermissionDecision::Block {
            let server_label = resolve_server_label(app, &request.server_id);
            record_tool_result(
                app,
                &request.tool_name,
                Some(server_label),
                request.tool_call_id.clone(),
                "Tool blocked by user.".to_string(),
                ToolResultStatus::Blocked,
                ctx,
            );
            return advance_tool_queue(app, ctx);
        }

        app.session.active_tool_request = Some(request.clone());
        set_status_for_tool_run(app, &request, ctx);
        return Some(AppCommand::RunMcpTool(request));
    }

    let server_name = app
        .mcp
        .server(&request.server_id)
        .map(|server| {
            if server.config.display_name.trim().is_empty() {
                server.config.id.clone()
            } else {
                server.config.display_name.clone()
            }
        })
        .unwrap_or_else(|| request.server_id.clone());

    let args_summary = summarize_tool_arguments(&request.raw_arguments);
    let server_id = request.server_id.clone();
    let tool_name = request.tool_name.clone();
    let raw_arguments = request.raw_arguments.clone();
    let batch_index = request
        .tool_call_id
        .as_ref()
        .and_then(|id| {
            app.session
                .tool_call_records
                .iter()
                .position(|record| record.id.as_str() == id.as_str())
        })
        .unwrap_or(0);
    app.session.active_tool_request = Some(request);
    app.ui.start_tool_prompt(
        server_id,
        server_name,
        tool_name,
        args_summary,
        raw_arguments,
        batch_index,
    );
    None
}

fn spawn_stream_after_tools(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let base_messages = app.session.last_stream_api_messages.clone()?;

    let mut api_messages = base_messages;
    if !app.session.tool_call_records.is_empty() {
        api_messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            name: None,
            tool_call_id: None,
            tool_calls: Some(app.session.tool_call_records.clone()),
        });
    }

    api_messages.extend(app.session.tool_results.clone());

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    app.ui.focus_transcript();
    app.enable_auto_scroll();

    let input_area_height = app.input_area_height(ctx.term_width);
    let (cancel_token, stream_id) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        conversation.add_assistant_placeholder();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
        (cancel_token, stream_id)
    };

    Some(AppCommand::SpawnStream(app.build_stream_params(
        api_messages,
        cancel_token,
        stream_id,
    )))
}

fn resolve_tool_server(app: &App, tool_name: &str) -> Result<(String, String), String> {
    let mut matches = Vec::new();

    for server in app.mcp.servers() {
        if !server.config.is_enabled() {
            continue;
        }
        let Some(list) = &server.cached_tools else {
            continue;
        };

        let allowed_tools = server.allowed_tools();
        for tool in &list.tools {
            if !tool.name.eq_ignore_ascii_case(tool_name) {
                continue;
            }
            if let Some(allowed) = allowed_tools {
                if !allowed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&tool.name))
                {
                    continue;
                }
            }

            let display_name = if server.config.display_name.trim().is_empty() {
                server.config.id.clone()
            } else {
                server.config.display_name.clone()
            };
            matches.push((server.config.id.clone(), display_name));
            break;
        }
    }

    match matches.len() {
        0 => Err(format!("No MCP server provides tool '{tool_name}'.")),
        1 => Ok(matches.remove(0)),
        _ => {
            let labels = matches
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Tool '{tool_name}' is available on multiple MCP servers: {labels}."
            ))
        }
    }
}

fn parse_tool_arguments(raw: &str) -> Result<Option<Map<String, Value>>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|err| err.to_string())?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => Err("Tool arguments must be a JSON object.".to_string()),
    }
}

fn parse_resource_read_arguments(
    raw: &str,
) -> Result<(String, String, Map<String, Value>), String> {
    let arguments = parse_tool_arguments(raw)?
        .ok_or_else(|| "Resource read arguments are required.".to_string())?;
    let server_id = arguments
        .get("server_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Resource read requires server_id.".to_string())?;
    let uri = arguments
        .get("uri")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Resource read requires uri.".to_string())?;
    Ok((server_id, uri, arguments))
}

fn summarize_tool_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut summary = String::new();
    let mut count = 0;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if summary.ends_with(' ') {
                continue;
            }
            summary.push(' ');
        } else {
            summary.push(ch);
        }
        count += 1;
        if count >= 60 {
            summary.push('…');
            break;
        }
    }
    summary
}

fn record_tool_result(
    app: &mut App,
    tool_name: &str,
    server_label: Option<String>,
    tool_call_id: Option<String>,
    payload: String,
    status: ToolResultStatus,
    ctx: AppActionContext,
) {
    let mut transcript_payload = tool_name.to_string();
    if let Some(server) = server_label.as_ref() {
        if !server.trim().is_empty() {
            transcript_payload.push_str(" on ");
            transcript_payload.push_str(server);
        }
    }
    transcript_payload.push_str(" (");
    transcript_payload.push_str(status.label());
    transcript_payload.push(')');

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.add_tool_result_message(transcript_payload);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }

    app.session.tool_results.push(ChatMessage {
        role: "tool".to_string(),
        content: payload.clone(),
        name: None,
        tool_call_id: tool_call_id.clone(),
        tool_calls: None,
    });

    app.session.tool_result_history.push(ToolResultRecord {
        tool_name: tool_name.to_string(),
        server_name: server_label,
        status,
        content: payload,
        tool_call_id,
    });
}

fn set_status_for_tool_run(app: &mut App, request: &ToolCallRequest, ctx: AppActionContext) {
    let input_area_height = app.input_area_height(ctx.term_width);
    let server_name = resolve_server_label(app, &request.server_id);
    let mut conversation = app.conversation();
    conversation.set_status(format!(
        "Running MCP tool {} on {}...",
        request.tool_name, server_name
    ));
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn resolve_server_label(app: &App, server_id: &str) -> String {
    app.mcp
        .server(server_id)
        .map(|server| {
            if server.config.display_name.trim().is_empty() {
                server.config.id.clone()
            } else {
                server.config.display_name.clone()
            }
        })
        .unwrap_or_else(|| server_id.to_string())
}

fn prepare_stream_params_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> StreamParams {
    app.ui.focus_transcript();
    let term_width = ctx.term_width.max(1);
    let term_height = ctx.term_height.max(1);
    app.enable_auto_scroll();
    let input_area_height = app.input_area_height(term_width);
    let (cancel_token, stream_id, api_messages) = {
        let mut conversation = app.conversation();
        let (cancel_token, stream_id) = conversation.start_new_stream();
        let api_messages = conversation.add_user_message(message);
        let available_height =
            conversation.calculate_available_height(term_height, input_area_height);
        conversation.update_scroll_position(available_height, term_width);
        (cancel_token, stream_id, api_messages)
    };

    app.build_stream_params(api_messages, cancel_token, stream_id)
}

fn prepare_retry_stream(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let now = Instant::now();
    if now.duration_since(app.last_retry_time()).as_millis() < 200 {
        return None;
    }

    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let maybe_params = {
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation
            .prepare_retry(available_height, ctx.term_width)
            .map(|api_messages| {
                let (cancel_token, stream_id) = conversation.start_new_stream();
                (api_messages, cancel_token, stream_id)
            })
    };

    if let Some((api_messages, cancel_token, stream_id)) = maybe_params {
        app.update_last_retry_time(now);
        app.ui.focus_transcript();
        Some(AppCommand::SpawnStream(app.build_stream_params(
            api_messages,
            cancel_token,
            stream_id,
        )))
    } else {
        None
    }
}

fn prepare_refine_stream(
    app: &mut App,
    prompt: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return None;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let maybe_params = {
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation
            .prepare_refine(prompt, available_height, ctx.term_width, false)
            .map(|api_messages| {
                let (cancel_token, stream_id) = conversation.start_new_stream();
                (api_messages, cancel_token, stream_id)
            })
    };

    if let Some((api_messages, cancel_token, stream_id)) = maybe_params {
        app.update_last_retry_time(Instant::now());
        app.ui.focus_transcript();
        Some(AppCommand::SpawnStream(app.build_stream_params(
            api_messages,
            cancel_token,
            stream_id,
        )))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::{
        AppMessageKind, ROLE_APP_ERROR, ROLE_APP_WARNING, ROLE_ASSISTANT, ROLE_TOOL_CALL,
        ROLE_TOOL_RESULT, ROLE_USER,
    };
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn stream_app_message_adds_trimmed_content_and_keeps_stream_alive() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = handle_streaming_action(
            &mut app,
            AppAction::SubmitMessage {
                message: "Hello there".into(),
            },
            ctx,
        );

        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            Some(_) => panic!("unexpected app command returned for submit message"),
            None => panic!("expected spawn stream command"),
        };

        assert!(app.ui.is_streaming);

        let result = handle_streaming_action(
            &mut app,
            AppAction::StreamAppMessage {
                kind: AppMessageKind::Warning,
                message: "  invalid utf8  ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());

        let last_message = app
            .ui
            .messages
            .back()
            .expect("expected trailing app message");
        assert_eq!(last_message.role, ROLE_APP_WARNING);
        assert_eq!(last_message.content, "invalid utf8");

        assert!(app.ui.is_streaming);
    }

    #[test]
    fn stream_errored_drops_empty_assistant_placeholder() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = handle_streaming_action(
            &mut app,
            AppAction::SubmitMessage {
                message: "Hello there".into(),
            },
            ctx,
        );

        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            Some(_) => panic!("unexpected app command returned for submit message"),
            None => panic!("expected spawn stream command"),
        };

        assert!(app
            .ui
            .messages
            .iter()
            .any(|msg| msg.role == ROLE_ASSISTANT && msg.content.is_empty()));

        let result = handle_streaming_action(
            &mut app,
            AppAction::StreamErrored {
                message: " network failure ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());

        assert!(app
            .ui
            .messages
            .iter()
            .all(|msg| msg.role != ROLE_ASSISTANT || !msg.content.trim().is_empty()));

        let last_message = app
            .ui
            .messages
            .back()
            .expect("expected trailing error message");
        assert_eq!(last_message.role, ROLE_APP_ERROR);
        assert_eq!(last_message.content, "network failure");

        assert_eq!(app.ui.messages.len(), 2);
        let first = app.ui.messages.front().expect("missing user message");
        assert_eq!(first.role, ROLE_USER);
        assert_eq!(first.content, "Hello there");
    }

    #[test]
    fn stream_tool_call_delta_flushes_on_complete() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = handle_streaming_action(
            &mut app,
            AppAction::SubmitMessage {
                message: "Run a tool".into(),
            },
            ctx,
        );

        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            Some(_) => panic!("unexpected app command returned for submit message"),
            None => panic!("expected spawn stream command"),
        };

        handle_streaming_action(
            &mut app,
            AppAction::StreamToolCallDelta {
                stream_id,
                delta: crate::core::chat_stream::ToolCallDelta {
                    index: 0,
                    id: Some("call-1".into()),
                    name: Some("lookup".into()),
                    arguments: Some("{\"q\":".into()),
                },
            },
            ctx,
        );

        handle_streaming_action(
            &mut app,
            AppAction::StreamToolCallDelta {
                stream_id,
                delta: crate::core::chat_stream::ToolCallDelta {
                    index: 0,
                    id: None,
                    name: None,
                    arguments: Some("\"mcp\"}".into()),
                },
            },
            ctx,
        );

        let command =
            handle_streaming_action(&mut app, AppAction::StreamCompleted { stream_id }, ctx);

        assert!(app.session.pending_tool_calls.is_empty());
        assert!(matches!(command, Some(AppCommand::SpawnStream(_))));

        let tool_call = app
            .ui
            .messages
            .iter()
            .find(|msg| msg.role == ROLE_TOOL_CALL)
            .expect("missing tool call message");
        assert_eq!(tool_call.content, "lookup {\"q\":\"mcp\"}");

        let tool_result = app
            .ui
            .messages
            .iter()
            .find(|msg| msg.role == ROLE_TOOL_RESULT)
            .expect("missing tool result message");
        assert!(tool_result.content.contains("lookup"));
    }

    #[test]
    fn tool_unsupported_detection_requires_tool_signal() {
        assert!(!is_tool_unsupported_error("API Error: not supported"));
        assert!(is_tool_unsupported_error(
            "API Error: tools are not supported for this model"
        ));
        assert!(is_tool_unsupported_error(
            "API Error: Unknown field: tool_calls"
        ));
    }
}
