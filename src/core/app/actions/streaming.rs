use std::collections::VecDeque;
use std::time::Instant;

use super::{App, AppActionContext, AppCommand, StreamingAction};
use crate::api::{ChatMessage, ChatToolCall, ChatToolCallFunction};
use crate::core::app::session::{
    McpPromptRequest, McpSamplingRequest, PendingToolCall, ToolCallRequest, ToolFailureKind,
    ToolPayloadHistoryEntry, ToolResultRecord, ToolResultStatus,
};
use crate::core::app::ui_state::ToolPromptRequest;
use crate::core::chat_stream::StreamParams;
use crate::core::config::data::McpToolPayloadRetention;
use crate::core::mcp_sampling::{serialize_sampling_params, summarize_sampling_request};
use crate::core::message::{AppMessageKind, Message, TranscriptRole};
use crate::mcp::permissions::ToolPermissionDecision;
use crate::mcp::{MCP_INSTANT_RECALL_TOOL, MCP_SESSION_MEMORY_SERVER_ID};
use rust_mcp_schema::{ContentBlock, PromptMessage, Role, RpcError};
use serde_json::{Map, Value};
use tracing::debug;

#[path = "mcp_gate.rs"]
mod mcp_gate;
#[path = "sampling.rs"]
mod sampling;
#[path = "stream_errors.rs"]
mod stream_errors;
#[path = "stream_lifecycle.rs"]
mod stream_lifecycle;
#[path = "tool_calls.rs"]
mod tool_calls;

pub(super) fn handle_streaming_action(
    app: &mut App,
    action: StreamingAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        StreamingAction::McpInitCompleted => mcp_gate::handle_mcp_init_completed(app, ctx),
        StreamingAction::McpSendPendingWithoutTools => {
            mcp_gate::handle_mcp_send_without_tools(app, ctx)
        }
        StreamingAction::AppendResponseChunk { content, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            stream_lifecycle::append_response_chunk(app, &content, ctx);
            None
        }
        StreamingAction::StreamAppMessage {
            kind,
            message,
            stream_id,
        } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            stream_lifecycle::append_stream_app_message(app, kind, message, ctx);
            None
        }
        StreamingAction::StreamToolCallDelta { delta, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            stream_lifecycle::append_tool_call_delta(app, delta);
            None
        }
        StreamingAction::ToolPermissionDecision { decision } => {
            tool_calls::handle_tool_permission_decision(app, decision, ctx)
        }
        StreamingAction::ToolCallCompleted {
            tool_name,
            tool_call_id,
            result,
        } => tool_calls::handle_tool_call_completed(app, tool_name, tool_call_id, result, ctx),
        StreamingAction::McpPromptCompleted { request, result } => {
            handle_mcp_prompt_completed(app, request, result, ctx)
        }
        StreamingAction::McpServerRequestReceived { request } => {
            sampling::handle_mcp_server_request(app, *request, ctx)
        }
        StreamingAction::McpSamplingFinished => sampling::handle_mcp_sampling_finished(app, ctx),
        StreamingAction::StreamErrored { message, stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            stream_errors::handle_stream_error(app, message, ctx)
        }
        StreamingAction::StreamCompleted { stream_id } => {
            if !app.is_current_stream(stream_id) {
                return None;
            }
            stream_lifecycle::finalize_stream(app, ctx)
        }
        StreamingAction::CancelStreaming => {
            app.cancel_current_stream();
            None
        }
        StreamingAction::SubmitMessage { message } => {
            stream_lifecycle::spawn_stream_for_message(app, message, ctx)
        }
        StreamingAction::RefineLastMessage { prompt } => {
            stream_lifecycle::refine_last_message(app, prompt, ctx)
        }
        StreamingAction::RetryLastMessage => stream_lifecycle::retry_last_message(app, ctx),
    }
}

pub(super) fn spawn_stream_for_message(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    stream_lifecycle::spawn_stream_for_message(app, message, ctx)
}

fn handle_mcp_prompt_completed(
    app: &mut App,
    request: McpPromptRequest,
    result: Result<rust_mcp_schema::GetPromptResult, String>,
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
                Role::User => TranscriptRole::User,
                Role::Assistant => TranscriptRole::Assistant,
            };
            conversation.add_message(Message::new(role, content));
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
    let mut pending_errors: Vec<tool_calls::PendingToolError> = Vec::new();

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
            pending_errors.push(tool_calls::PendingToolError {
                tool_name: tool_name.clone(),
                server_id: None,
                tool_call_id: Some(tool_call_id.clone()),
                raw_arguments: Some(raw_arguments.clone()),
                error: "Missing tool name.".to_string(),
            });
            continue;
        }

        if tool_name.eq_ignore_ascii_case(crate::mcp::MCP_LIST_RESOURCES_TOOL) {
            match parse_resource_list_arguments(&raw_arguments) {
                Ok((server_id, _kind, _cursor, arguments)) => {
                    match app.mcp.server(&server_id) {
                        Some(server) if !server.config.is_enabled() => {
                            pending_errors.push(tool_calls::PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!("MCP server is disabled: {server_id}."),
                            });
                            continue;
                        }
                        Some(server) if !server_supports_resources(server) => {
                            pending_errors.push(tool_calls::PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!(
                                    "MCP server does not support resources: {server_id}."
                                ),
                            });
                            continue;
                        }
                        Some(_) => {}
                        None => {
                            pending_errors.push(tool_calls::PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!("Unknown MCP server id: {server_id}."),
                            });
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
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: None,
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: err,
                    });
                    continue;
                }
            }
        } else if tool_name.eq_ignore_ascii_case(crate::mcp::MCP_READ_RESOURCE_TOOL) {
            match parse_resource_read_arguments(&raw_arguments) {
                Ok((server_id, _uri, arguments)) => {
                    match app.mcp.server(&server_id) {
                        Some(server) if server.config.is_enabled() => {}
                        Some(_) => {
                            pending_errors.push(tool_calls::PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!("MCP server is disabled: {server_id}."),
                            });
                            continue;
                        }
                        None => {
                            pending_errors.push(tool_calls::PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!("Unknown MCP server id: {server_id}."),
                            });
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
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: None,
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: err,
                    });
                    continue;
                }
            }
        } else if is_instant_recall_tool(&tool_name) {
            match parse_instant_recall_arguments(&raw_arguments) {
                Ok(arguments) => {
                    pending_queue.push_back(ToolCallRequest {
                        server_id: MCP_SESSION_MEMORY_SERVER_ID.to_string(),
                        tool_name: tool_name.clone(),
                        arguments: Some(arguments),
                        raw_arguments,
                        tool_call_id: Some(tool_call_id),
                    });
                    continue;
                }
                Err(err) => {
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: Some(MCP_SESSION_MEMORY_SERVER_ID.to_string()),
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: err,
                    });
                    continue;
                }
            }
        } else {
            let server_id = match resolve_tool_server(app, &tool_name) {
                Ok((server_id, _)) => server_id,
                Err(err) => {
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: None,
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: err,
                    });
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
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: None,
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: format!("Invalid tool arguments: {err}"),
                    });
                    continue;
                }
            }
        }
    }

    app.session.tool_call_records = tool_call_records;
    app.session.pending_tool_queue = pending_queue;
    app.session.active_tool_request = None;
    app.session.tool_results.clear();

    for error in pending_errors {
        let mut meta = tool_calls::ToolResultMeta::new(
            error
                .server_id
                .as_ref()
                .map(|id| resolve_server_label(app, id)),
            error.server_id.clone(),
            error.tool_call_id.clone(),
            error.raw_arguments.clone(),
        );
        meta.failure_kind = Some(ToolFailureKind::ToolCallFailure);
        record_tool_result(
            app,
            &error.tool_name,
            meta,
            error.error,
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

    if is_instant_recall_tool(&request.tool_name) {
        app.session.active_tool_request = Some(request.clone());
        return handle_instant_recall_tool_request(app, request, ctx);
    }

    if is_mcp_yolo_enabled(app, &request.server_id) {
        app.session.active_tool_request = Some(request.clone());
        set_status_for_tool_run(app, &request, ctx);
        return Some(AppCommand::RunMcpTool(request));
    }

    if let Some(decision) = app
        .mcp_permissions
        .decision_for(&request.server_id, &request.tool_name)
    {
        if decision == ToolPermissionDecision::Block {
            let server_label = resolve_server_label(app, &request.server_id);
            let meta = tool_calls::ToolResultMeta::new(
                Some(server_label),
                Some(request.server_id.clone()),
                request.tool_call_id.clone(),
                Some(request.raw_arguments.clone()),
            );
            record_tool_result(
                app,
                &request.tool_name,
                meta,
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

    let server_name = resolve_server_label(app, &request.server_id);

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
    let display_name = format!("Allow {} to run {}?", server_name, tool_name);
    app.session.active_tool_request = Some(request);
    app.ui.start_tool_prompt(ToolPromptRequest {
        server_id,
        server_name,
        tool_name,
        display_name: Some(display_name),
        args_summary,
        raw_arguments,
        batch_index,
    });
    None
}

fn advance_sampling_queue(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    debug!(
        pending = app.session.pending_sampling_queue.len(),
        active = app.session.active_sampling_request.is_some(),
        "Advance MCP sampling queue"
    );
    if app.session.active_sampling_request.is_some() {
        return None;
    }

    if app.ui.tool_prompt().is_some() {
        return None;
    }

    let request = app.session.pending_sampling_queue.pop_front()?;
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.id,
        pending = app.session.pending_sampling_queue.len(),
        "Dequeued MCP sampling request"
    );

    if is_mcp_yolo_enabled(app, &request.server_id) {
        app.session.active_sampling_request = Some(request.clone());
        set_status_for_sampling_run(app, &request, ctx);
        return Some(AppCommand::RunMcpSampling(Box::new(request)));
    }

    if let Some(decision) = app
        .mcp_permissions
        .decision_for(&request.server_id, crate::mcp::MCP_SAMPLING_TOOL)
    {
        if decision == ToolPermissionDecision::Block {
            return Some(AppCommand::SendMcpServerError {
                server_id: request.server_id,
                request_id: request.request.id.clone(),
                error: RpcError {
                    code: -1,
                    message: "Sampling blocked by user.".to_string(),
                    data: None,
                },
            });
        }

        if decision == ToolPermissionDecision::AllowSession {
            debug!(
                server_id = %request.server_id,
                request_id = ?request.request.id,
                "Auto-allowing sampling for session"
            );
            app.session.active_sampling_request = Some(request.clone());
            set_status_for_sampling_run(app, &request, ctx);
            return Some(AppCommand::RunMcpSampling(Box::new(request)));
        }
    }

    let server_name = resolve_server_label(app, &request.server_id);
    let args_summary = summarize_sampling_request(&request.request);
    let raw_arguments = serialize_sampling_params(&request.request);
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.id,
        "Prompting user for sampling permission"
    );
    app.session.active_sampling_request = Some(request.clone());
    let display_name = format!(
        "Allow {} to generate text with {}?",
        server_name, app.session.model
    );
    app.ui.start_tool_prompt(ToolPromptRequest {
        server_id: request.server_id,
        server_name,
        tool_name: crate::mcp::MCP_SAMPLING_TOOL.to_string(),
        display_name: Some(display_name),
        args_summary,
        raw_arguments,
        batch_index: 0,
    });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceListKind {
    Resources,
    Templates,
}

type ResourceListArgs = (String, ResourceListKind, Option<String>, Map<String, Value>);

fn parse_resource_list_arguments(raw: &str) -> Result<ResourceListArgs, String> {
    let mut arguments = parse_tool_arguments(raw)?
        .ok_or_else(|| "Resource list arguments are required.".to_string())?;
    let server_id = arguments
        .get("server_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Resource list requires server_id.".to_string())?;
    let (kind, cursor) = parse_resource_list_kind(&arguments)?;
    arguments.insert(
        "kind".to_string(),
        Value::String(match kind {
            ResourceListKind::Resources => "resources".to_string(),
            ResourceListKind::Templates => "templates".to_string(),
        }),
    );
    Ok((server_id, kind, cursor, arguments))
}

pub(crate) fn parse_resource_list_kind(
    arguments: &Map<String, Value>,
) -> Result<(ResourceListKind, Option<String>), String> {
    let cursor = arguments
        .get("cursor")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    if let Some(cursor) = cursor.as_deref() {
        if cursor.trim().is_empty() {
            return Err("Resource list cursor cannot be empty.".to_string());
        }
    }
    let kind = match arguments
        .get("kind")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("resources") | Some("resource") => ResourceListKind::Resources,
        Some("templates") | Some("template") | Some("resource_templates") => {
            ResourceListKind::Templates
        }
        Some(other) => {
            return Err(format!(
                "Resource list kind must be \"resources\" or \"templates\" (got {other})."
            ))
        }
    };
    Ok((kind, cursor))
}

fn server_supports_resources(server: &crate::mcp::client::McpServerState) -> bool {
    server
        .server_details
        .as_ref()
        .map(|details| details.capabilities.resources.is_some())
        .unwrap_or(true)
}

fn parse_instant_recall_arguments(raw: &str) -> Result<Map<String, Value>, String> {
    let arguments = parse_tool_arguments(raw)?
        .ok_or_else(|| "Instant recall arguments are required.".to_string())?;
    let tool_call_id = arguments
        .get("tool_call_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Instant recall requires tool_call_id.".to_string())?;
    if tool_call_id.trim().is_empty() {
        return Err("Instant recall requires tool_call_id.".to_string());
    }
    Ok(arguments)
}

fn is_instant_recall_tool(tool_name: &str) -> bool {
    tool_name.eq_ignore_ascii_case(MCP_INSTANT_RECALL_TOOL)
}

fn summarize_tool_arguments(raw: &str) -> String {
    let summary = crate::core::app::streaming::abbreviate_args(raw);
    if summary == "(none)" {
        String::new()
    } else {
        summary
    }
}

fn handle_instant_recall_tool_request(
    app: &mut App,
    request: ToolCallRequest,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let tool_call_id = request.tool_call_id.clone();

    let result = recall_tool_payload(app, &request);
    let payload = match result {
        Ok((payload, summary)) => {
            let summary_label = summary.unwrap_or_else(|| "Unknown tool call".to_string());
            let recall_label = tool_call_id.as_deref().map_or_else(
                || format!("Recalling result from previous tool call: {summary_label}"),
                |id| format!("Recalling result from previous tool call {id}: {summary_label}"),
            );
            app.conversation()
                .add_app_message(AppMessageKind::Info, recall_label);
            payload
        }
        Err(error) => {
            app.conversation().add_app_message(
                AppMessageKind::Warning,
                format!("Instant recall failed: {error}"),
            );
            error
        }
    };

    let tool_message = ChatMessage {
        role: "tool".to_string(),
        content: payload,
        name: None,
        tool_call_id,
        tool_calls: None,
    };
    app.session.tool_results.push(tool_message);

    app.clear_status();
    app.session.active_tool_request = None;
    advance_tool_queue(app, ctx)
}

fn recall_tool_payload(
    app: &App,
    request: &ToolCallRequest,
) -> Result<(String, Option<String>), String> {
    let arguments = request
        .arguments
        .as_ref()
        .ok_or_else(|| "Instant recall arguments are required.".to_string())?;
    let tool_call_id = arguments
        .get("tool_call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Instant recall requires tool_call_id.".to_string())?
        .trim()
        .to_string();
    if tool_call_id.is_empty() {
        return Err("Instant recall requires tool_call_id.".to_string());
    }
    let record = app
        .session
        .tool_result_history
        .iter()
        .find(|entry| entry.tool_call_id.as_deref() == Some(tool_call_id.as_str()))
        .ok_or_else(|| format!("No tool result found for tool_call_id={tool_call_id}."))?;

    Ok((record.content.clone(), Some(record.summary.clone())))
}

fn record_tool_result(
    app: &mut App,
    tool_name: &str,
    meta: tool_calls::ToolResultMeta,
    payload: String,
    status: ToolResultStatus,
    ctx: AppActionContext,
) {
    let tool_call_id = meta.tool_call_id.clone();
    let raw_arguments = meta.raw_arguments.clone();
    let assistant_message_index = app.session.active_assistant_message_index;
    let server_id = meta.server_id.clone();
    let server_label = meta.server_label.clone();
    let failure_kind = meta.failure_kind;
    let summary = build_tool_result_summary(tool_name, &meta, status);
    let (payload_policy, payload_window) =
        resolve_tool_payload_policy(app, meta.server_id.as_deref());
    let mut transcript_payload = tool_name.to_string();
    if let Some(server) = server_label.as_ref() {
        if !server.trim().is_empty() {
            transcript_payload.push_str(" on ");
            transcript_payload.push_str(server);
        }
    }
    transcript_payload.push_str(" (");
    let status_label = match status {
        ToolResultStatus::Error => failure_kind.map_or(status.label(), ToolFailureKind::label),
        _ => status.label(),
    };
    transcript_payload.push_str(status_label);
    transcript_payload.push(')');

    let input_area_height = app.input_area_height(ctx.term_width);
    {
        let mut conversation = app.conversation();
        conversation.add_tool_result_message(transcript_payload);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }

    let context_payload = match payload_policy {
        McpToolPayloadRetention::Turn
        | McpToolPayloadRetention::Window
        | McpToolPayloadRetention::All => payload.clone(),
    };

    app.session.tool_results.push(ChatMessage {
        role: "tool".to_string(),
        content: context_payload.clone(),
        name: None,
        tool_call_id: tool_call_id.clone(),
        tool_calls: None,
    });

    app.session.tool_result_history.push(ToolResultRecord {
        tool_name: tool_name.to_string(),
        server_name: server_label,
        server_id: server_id.clone(),
        status,
        failure_kind,
        content: payload.clone(),
        summary,
        tool_call_id: tool_call_id.clone(),
        raw_arguments: raw_arguments.clone(),
        assistant_message_index,
    });

    if matches!(
        payload_policy,
        McpToolPayloadRetention::Window | McpToolPayloadRetention::All
    ) {
        if let Some(tool_call_id) = tool_call_id.clone() {
            let assistant_message = ChatMessage {
                role: "assistant".to_string(),
                content: String::new(),
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: tool_call_id.clone(),
                    kind: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: tool_name.to_string(),
                        arguments: raw_arguments.clone().unwrap_or_default(),
                    },
                }]),
            };
            let tool_message = ChatMessage {
                role: "tool".to_string(),
                content: payload.clone(),
                name: None,
                tool_call_id: Some(tool_call_id.clone()),
                tool_calls: None,
            };
            app.session
                .tool_payload_history
                .push(ToolPayloadHistoryEntry {
                    server_id: server_id.clone(),
                    tool_call_id: Some(tool_call_id),
                    assistant_message,
                    tool_message,
                    assistant_message_index,
                });
            if payload_policy == McpToolPayloadRetention::Window && payload_window > 0 {
                trim_tool_payload_history(app, server_id.as_deref(), payload_window);
            }
        }
    }
}

fn resolve_tool_payload_policy(
    app: &App,
    server_id: Option<&str>,
) -> (McpToolPayloadRetention, usize) {
    let Some(server_id) = server_id else {
        return (McpToolPayloadRetention::Turn, 0);
    };
    let Some(server) = app.mcp.server(server_id) else {
        return (McpToolPayloadRetention::Turn, 0);
    };
    (
        server.config.tool_payloads(),
        server.config.tool_payload_window(),
    )
}

fn trim_tool_payload_history(app: &mut App, server_id: Option<&str>, window: usize) {
    let Some(server_id) = server_id else {
        return;
    };
    let count = app
        .session
        .tool_payload_history
        .iter()
        .filter(|entry| {
            entry
                .server_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(server_id))
        })
        .count();
    if count <= window {
        return;
    }

    let mut keep = Vec::with_capacity(app.session.tool_payload_history.len());
    let mut drop = count - window;
    for entry in app.session.tool_payload_history.iter() {
        let matches = entry
            .server_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(server_id));
        if matches && drop > 0 {
            drop -= 1;
            continue;
        }
        keep.push(entry.clone());
    }
    app.session.tool_payload_history = keep;
}

fn build_tool_result_summary(
    tool_name: &str,
    meta: &tool_calls::ToolResultMeta,
    status: ToolResultStatus,
) -> String {
    let mut summary = tool_name.to_string();
    if let Some(server) = meta.server_label.as_ref() {
        if !server.trim().is_empty() {
            summary.push_str(" on ");
            summary.push_str(server);
        }
    }
    summary.push_str(" (");
    let status_label = match status {
        ToolResultStatus::Error => meta
            .failure_kind
            .map_or(status.label(), ToolFailureKind::label),
        _ => status.label(),
    };
    summary.push_str(status_label);
    summary.push(')');

    if let Some(raw_arguments) = meta.raw_arguments.as_ref() {
        let arg_summary = summarize_tool_arguments(raw_arguments);
        if !arg_summary.is_empty() {
            summary.push_str(" args: ");
            summary.push_str(&arg_summary);
        }
    }

    summary
}

fn set_status_for_tool_run(app: &mut App, _request: &ToolCallRequest, ctx: AppActionContext) {
    let input_area_height = app.input_area_height(ctx.term_width);
    let _token = app.begin_mcp_operation();
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn set_status_for_sampling_run(
    app: &mut App,
    _request: &McpSamplingRequest,
    ctx: AppActionContext,
) {
    let input_area_height = app.input_area_height(ctx.term_width);
    let _token = app.begin_mcp_operation();
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

fn is_mcp_yolo_enabled(app: &App, server_id: &str) -> bool {
    app.mcp
        .server(server_id)
        .map(|server| server.config.is_yolo())
        .unwrap_or(false)
}

fn resolve_server_label(app: &App, server_id: &str) -> String {
    if server_id.eq_ignore_ascii_case(MCP_SESSION_MEMORY_SERVER_ID) {
        return "Instant recall".to_string();
    }
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
