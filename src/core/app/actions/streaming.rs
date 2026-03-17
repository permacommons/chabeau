//! Streaming and MCP action reducer for in-flight assistant turns.
//!
//! # Ownership boundary
//! This module owns routing of [`super::StreamingAction`] variants into focused
//! stream/tool/sampling reducers and validates stream-id freshness before
//! mutating state. It delegates protocol details to helper submodules and MCP
//! execution to commands returned to the app runtime.
//!
//! # Main structures and invariants
//! The reducer ensures stale stream chunks are ignored, stream lifecycle events
//! are sequenced, and tool-call pipelines are normalized before command
//! dispatch.
//!
//! # Call flow entrypoints
//! Called from [`super::apply_action`] for `AppAction::Streaming`. It mutates
//! `App` state and may emit [`super::AppCommand`] values such as `SpawnStream`,
//! `RunMcpTool`, `RunMcpPrompt`, and `RunMcpSampling`.

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
use jsonschema::error::{
    TypeKind, ValidationError as JsonSchemaValidationError, ValidationErrorKind,
};
use rust_mcp_schema::{ContentBlock, PromptMessage, Role, RpcError};
use serde::Serialize;
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
                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                        failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                                failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                        failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                        failure_kind: Some(ToolFailureKind::ToolCallFailure),
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
                        failure_kind: Some(ToolFailureKind::ToolCallFailure),
                    });
                    continue;
                }
            };

            let parsed_arguments = match parse_tool_arguments_value(&raw_arguments) {
                Ok(arguments) => arguments,
                Err(err) => {
                    pending_errors.push(tool_calls::PendingToolError {
                        tool_name: tool_name.clone(),
                        server_id: Some(server_id),
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: format!("Invalid tool arguments: {err}"),
                        failure_kind: Some(ToolFailureKind::ToolCallFailure),
                    });
                    continue;
                }
            };

            let validation_value = parsed_arguments
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new()));
            if let Err(payload) =
                validate_mcp_tool_arguments(app, &server_id, &tool_name, &validation_value)
            {
                pending_errors.push(tool_calls::PendingToolError {
                    tool_name: tool_name.clone(),
                    server_id: Some(server_id),
                    tool_call_id: Some(tool_call_id.clone()),
                    raw_arguments: Some(raw_arguments.clone()),
                    error: payload,
                    failure_kind: Some(ToolFailureKind::ToolError),
                });
                continue;
            }

            match value_to_argument_map(parsed_arguments) {
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
                        server_id: Some(server_id),
                        tool_call_id: Some(tool_call_id.clone()),
                        raw_arguments: Some(raw_arguments.clone()),
                        error: format!("Invalid tool arguments: {err}"),
                        failure_kind: Some(ToolFailureKind::ToolError),
                    });
                    continue;
                }
            }
        }
    }

    app.session.tool_pipeline.tool_call_records = tool_call_records;
    app.session.tool_pipeline.pending_tool_queue = pending_queue;
    app.session.tool_pipeline.active_tool_request = None;
    app.session.tool_pipeline.tool_results.clear();

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
        meta.failure_kind = error.failure_kind;
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
    if app.session.tool_pipeline.active_tool_request.is_some() {
        return None;
    }

    let Some(request) = app.session.tool_pipeline.pending_tool_queue.pop_front() else {
        return spawn_stream_after_tools(app, ctx);
    };

    if is_instant_recall_tool(&request.tool_name) {
        app.session.tool_pipeline.active_tool_request = Some(request.clone());
        return handle_instant_recall_tool_request(app, request, ctx);
    }

    if is_mcp_yolo_enabled(app, &request.server_id) {
        app.session.tool_pipeline.active_tool_request = Some(request.clone());
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

        app.session.tool_pipeline.active_tool_request = Some(request.clone());
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
                .tool_pipeline
                .tool_call_records
                .iter()
                .position(|record| record.id.as_str() == id.as_str())
        })
        .unwrap_or(0);
    let display_name = format!("Allow {} to run {}?", server_name, tool_name);
    app.session.tool_pipeline.active_tool_request = Some(request);
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
        pending = app.session.tool_pipeline.pending_sampling_queue.len(),
        active = app.session.tool_pipeline.active_sampling_request.is_some(),
        "Advance MCP sampling queue"
    );
    if app.session.tool_pipeline.active_sampling_request.is_some() {
        return None;
    }

    if app.ui.tool_prompt().is_some() {
        return None;
    }

    let request = app
        .session
        .tool_pipeline
        .pending_sampling_queue
        .pop_front()?;
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.id,
        pending = app.session.tool_pipeline.pending_sampling_queue.len(),
        "Dequeued MCP sampling request"
    );

    if is_mcp_yolo_enabled(app, &request.server_id) {
        app.session.tool_pipeline.active_sampling_request = Some(request.clone());
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
            app.session.tool_pipeline.active_sampling_request = Some(request.clone());
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
    app.session.tool_pipeline.active_sampling_request = Some(request.clone());
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
    let base_messages = app
        .session
        .tool_pipeline
        .continuation_messages
        .as_ref()
        .map(|continuation| continuation.api_messages.clone())?;

    let mut api_messages = base_messages;
    if !app.session.tool_pipeline.tool_call_records.is_empty() {
        api_messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            name: None,
            tool_call_id: None,
            tool_calls: Some(app.session.tool_pipeline.tool_call_records.clone()),
        });
    }

    api_messages.extend(app.session.tool_pipeline.tool_results.clone());

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

fn find_mcp_tool<'a>(
    app: &'a App,
    server_id: &str,
    tool_name: &str,
) -> Option<&'a rust_mcp_schema::Tool> {
    let server = app.mcp.server(server_id)?;
    let list = server.cached_tools.as_ref()?;
    let allowed_tools = server.allowed_tools();
    list.tools.iter().find(|tool| {
        tool.name.eq_ignore_ascii_case(tool_name)
            && allowed_tools.is_none_or(|allowed| {
                allowed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&tool.name))
            })
    })
}

fn find_mcp_tool_validator<'a>(
    app: &'a App,
    server_id: &str,
    tool_name: &str,
) -> Option<&'a crate::mcp::client::CachedToolSchemaValidator> {
    app.mcp.server(server_id)?.tool_validator(tool_name)
}

fn parse_tool_arguments_value(raw: &str) -> Result<Option<Value>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn parse_tool_arguments(raw: &str) -> Result<Option<Map<String, Value>>, String> {
    parse_tool_arguments_value(raw).and_then(value_to_argument_map)
}

fn value_to_argument_map(value: Option<Value>) -> Result<Option<Map<String, Value>>, String> {
    match value {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err("Tool arguments must be a JSON object.".to_string()),
    }
}

#[derive(Debug, Serialize)]
struct ToolValidationErrorPayload {
    #[serde(rename = "isError")]
    is_error: bool,
    error: ToolValidationErrorDetail,
}

#[derive(Debug, Serialize)]
struct ToolValidationErrorDetail {
    kind: &'static str,
    tool: String,
    server_id: String,
    message: &'static str,
    violations: Vec<ToolValidationViolation>,
}

#[derive(Debug, Serialize)]
struct ToolValidationViolation {
    path: String,
    expected: String,
    actual: String,
    issue: String,
    hint: String,
}

fn validate_mcp_tool_arguments(
    app: &App,
    server_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<(), String> {
    let Some(tool) = find_mcp_tool(app, server_id, tool_name) else {
        return Ok(());
    };
    let Some(validation_state) = find_mcp_tool_validator(app, server_id, &tool.name) else {
        return Ok(());
    };
    let Some(validator) = validation_state.validator.as_ref() else {
        return Ok(());
    };

    let violations: Vec<_> = validator
        .iter_errors(arguments)
        .flat_map(validation_error_to_violations)
        .collect();
    if violations.is_empty() {
        return Ok(());
    }

    let payload = serde_json::to_string(&ToolValidationErrorPayload {
        is_error: true,
        error: ToolValidationErrorDetail {
            kind: "invalid_arguments",
            tool: tool.name.clone(),
            server_id: server_id.to_string(),
            message: "Tool arguments did not match the expected schema.",
            violations,
        },
    })
    .map_err(|err| err.to_string())?;
    Err(payload)
}

fn validation_error_to_violations(
    error: JsonSchemaValidationError<'_>,
) -> Vec<ToolValidationViolation> {
    let instance = error.instance();
    let kind = error.kind();
    let base_path = format_instance_path(error.instance_path());
    let actual_type = json_type_name(instance).to_string();
    let issue = error.to_string();

    match kind {
        ValidationErrorKind::Required { property } => {
            let name = property.as_str().unwrap_or("value");
            vec![build_violation(
                &join_json_path(&base_path, name),
                "present",
                "missing",
                issue,
                format!("Include `{name}` in the tool arguments object."),
            )]
        }
        ValidationErrorKind::AdditionalProperties { unexpected }
        | ValidationErrorKind::UnevaluatedProperties { unexpected } => unexpected
            .iter()
            .map(|name| {
                let actual = instance
                    .as_object()
                    .and_then(|object: &Map<String, Value>| object.get(name))
                    .map(json_type_name)
                    .unwrap_or("missing");
                build_violation(
                    &join_json_path(&base_path, name),
                    "no additional properties",
                    actual,
                    format!("Unexpected argument `{name}`."),
                    format!("Remove `{name}` or add it to the tool schema."),
                )
            })
            .collect(),
        ValidationErrorKind::Type { kind } => {
            let expected = type_kind_label(kind);
            vec![build_violation(
                &base_path,
                &expected,
                &actual_type,
                type_mismatch_issue(instance, &expected),
                type_mismatch_hint(&base_path, instance, &expected),
            )]
        }
        _ => vec![build_violation(
            &base_path,
            &expected_label_for_error(kind),
            &actual_label_for_error(instance, &actual_type, kind),
            issue,
            hint_for_error(&base_path, instance, kind),
        )],
    }
}

fn expected_label_for_error(kind: &ValidationErrorKind) -> String {
    match kind {
        ValidationErrorKind::AdditionalItems { .. }
        | ValidationErrorKind::UnevaluatedItems { .. } => "no additional items".to_string(),
        ValidationErrorKind::AnyOf { .. } => "anyOf".to_string(),
        ValidationErrorKind::Constant { expected_value } => json_value_label(expected_value),
        ValidationErrorKind::Contains => "contains".to_string(),
        ValidationErrorKind::Enum { .. } => "enum".to_string(),
        ValidationErrorKind::ExclusiveMaximum { limit } => format!("exclusiveMaximum {limit}"),
        ValidationErrorKind::ExclusiveMinimum { limit } => format!("exclusiveMinimum {limit}"),
        ValidationErrorKind::FalseSchema => "true".to_string(),
        ValidationErrorKind::Format { format } => format!("format {format}"),
        ValidationErrorKind::MaxItems { limit } => format!("maxItems {limit}"),
        ValidationErrorKind::Maximum { limit } => format!("maximum {limit}"),
        ValidationErrorKind::MaxLength { limit } => format!("maxLength {limit}"),
        ValidationErrorKind::MaxProperties { limit } => format!("maxProperties {limit}"),
        ValidationErrorKind::MinItems { limit } => format!("minItems {limit}"),
        ValidationErrorKind::Minimum { limit } => format!("minimum {limit}"),
        ValidationErrorKind::MinLength { limit } => format!("minLength {limit}"),
        ValidationErrorKind::MinProperties { limit } => format!("minProperties {limit}"),
        ValidationErrorKind::MultipleOf { multiple_of } => format!("multipleOf {multiple_of}"),
        ValidationErrorKind::Not { .. } => "not".to_string(),
        ValidationErrorKind::OneOfMultipleValid { .. }
        | ValidationErrorKind::OneOfNotValid { .. } => "oneOf".to_string(),
        ValidationErrorKind::Pattern { pattern } => format!("pattern /{pattern}/"),
        ValidationErrorKind::PropertyNames { .. } => "propertyNames".to_string(),
        ValidationErrorKind::UniqueItems => "uniqueItems".to_string(),
        ValidationErrorKind::Type { kind } => type_kind_label(kind),
        ValidationErrorKind::Custom { .. } => "custom".to_string(),
        ValidationErrorKind::BacktrackLimitExceeded { .. }
        | ValidationErrorKind::ContentEncoding { .. }
        | ValidationErrorKind::ContentMediaType { .. }
        | ValidationErrorKind::FromUtf8 { .. }
        | ValidationErrorKind::Referencing(_) => "schema".to_string(),
        ValidationErrorKind::Required { .. }
        | ValidationErrorKind::AdditionalProperties { .. }
        | ValidationErrorKind::UnevaluatedProperties { .. } => unreachable!(),
    }
}

fn actual_label_for_error(
    instance: &Value,
    actual_type: &str,
    kind: &ValidationErrorKind,
) -> String {
    match kind {
        ValidationErrorKind::MaxLength { .. } | ValidationErrorKind::MinLength { .. } => instance
            .as_str()
            .map(|text| format!("length {}", text.chars().count()))
            .unwrap_or_else(|| actual_type.to_string()),
        ValidationErrorKind::MaxItems { .. } | ValidationErrorKind::MinItems { .. } => instance
            .as_array()
            .map(|items| format!("{} items", items.len()))
            .unwrap_or_else(|| actual_type.to_string()),
        ValidationErrorKind::MaxProperties { .. } | ValidationErrorKind::MinProperties { .. } => {
            instance
                .as_object()
                .map(|object| format!("{} properties", object.len()))
                .unwrap_or_else(|| actual_type.to_string())
        }
        ValidationErrorKind::UniqueItems => "duplicate items".to_string(),
        _ => actual_type.to_string(),
    }
}

fn hint_for_error(path: &str, value: &Value, kind: &ValidationErrorKind) -> String {
    match kind {
        ValidationErrorKind::AdditionalItems { .. }
        | ValidationErrorKind::UnevaluatedItems { .. } => {
            "Remove the extra item or update the tool schema.".to_string()
        }
        ValidationErrorKind::AnyOf { .. } => {
            "Update the value to match one of the schema alternatives.".to_string()
        }
        ValidationErrorKind::Constant { .. } => {
            "Use the exact value required by the tool schema.".to_string()
        }
        ValidationErrorKind::Contains => {
            "Include at least one item that matches the schema in `contains`.".to_string()
        }
        ValidationErrorKind::Enum { .. } => {
            "Choose one of the enum values declared in the tool schema.".to_string()
        }
        ValidationErrorKind::ExclusiveMaximum { .. } | ValidationErrorKind::Maximum { .. } => {
            "Reduce the value to satisfy the maximum.".to_string()
        }
        ValidationErrorKind::ExclusiveMinimum { .. } | ValidationErrorKind::Minimum { .. } => {
            "Increase the value to satisfy the minimum.".to_string()
        }
        ValidationErrorKind::Format { format } => {
            format!("Provide a value that satisfies the `{format}` format.")
        }
        ValidationErrorKind::MaxItems { .. } => "Remove items from the array.".to_string(),
        ValidationErrorKind::MaxLength { .. } => "Shorten the string value.".to_string(),
        ValidationErrorKind::MaxProperties { .. } => {
            "Remove properties from the object.".to_string()
        }
        ValidationErrorKind::MinItems { .. } => "Add more items to the array.".to_string(),
        ValidationErrorKind::MinLength { .. } => "Provide a longer string value.".to_string(),
        ValidationErrorKind::MinProperties { .. } => {
            "Add more properties to the object.".to_string()
        }
        ValidationErrorKind::MultipleOf { .. } => {
            "Adjust the value to match the required step size.".to_string()
        }
        ValidationErrorKind::OneOfMultipleValid { .. }
        | ValidationErrorKind::OneOfNotValid { .. } => {
            "Update the value to satisfy exactly one schema alternative.".to_string()
        }
        ValidationErrorKind::Pattern { .. } => {
            "Update the value to satisfy the pattern declared in the tool schema.".to_string()
        }
        ValidationErrorKind::PropertyNames { .. } => {
            "Rename the object keys to satisfy the schema.".to_string()
        }
        ValidationErrorKind::Type { kind } => {
            let expected = type_kind_label(kind);
            type_mismatch_hint(path, value, &expected)
        }
        ValidationErrorKind::UniqueItems => "Remove duplicate items from the array.".to_string(),
        _ => "Update the value to satisfy the tool schema.".to_string(),
    }
}

fn format_instance_path(path: &impl std::fmt::Display) -> String {
    let text = path.to_string();
    if text.is_empty() {
        "/".to_string()
    } else {
        text
    }
}

fn type_kind_label(kind: &TypeKind) -> String {
    match kind {
        TypeKind::Single(kind) => kind.to_string(),
        TypeKind::Multiple(kinds) => kinds
            .iter()
            .map(|kind| kind.to_string())
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_value_label(value: &Value) -> String {
    match value {
        Value::String(text) => format!("const {text:?}"),
        _ => format!("const {}", value),
    }
}

fn type_mismatch_issue(value: &Value, expected_types: &str) -> String {
    if expected_types
        .split('|')
        .map(str::trim)
        .any(|kind| kind == "object")
        && is_serialized_json_object(value)
    {
        "Expected a JSON object, but received a serialized JSON string.".to_string()
    } else {
        format!(
            "Expected {}, but received {}.",
            expected_types,
            json_type_name(value)
        )
    }
}

fn type_mismatch_hint(path: &str, value: &Value, expected_types: &str) -> String {
    let field_name = path.rsplit('/').find(|segment| !segment.is_empty());
    if expected_types
        .split('|')
        .map(str::trim)
        .any(|kind| kind == "object")
        && is_serialized_json_object(value)
    {
        return field_name.map_or_else(
            || "Send tool arguments as a JSON object, not as a quoted JSON string.".to_string(),
            |field| format!("Pass `{field}` as an object, not as a quoted JSON string."),
        );
    }

    field_name.map_or_else(
        || {
            format!(
                "Send tool arguments using the expected JSON types: {}.",
                expected_types
            )
        },
        |field| {
            format!(
                "Update `{field}` to use the expected JSON type: {}.",
                expected_types
            )
        },
    )
}

fn is_serialized_json_object(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    matches!(serde_json::from_str::<Value>(text), Ok(Value::Object(_)))
}

fn build_violation(
    path: &str,
    expected: &str,
    actual: &str,
    issue: String,
    hint: String,
) -> ToolValidationViolation {
    ToolValidationViolation {
        path: if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        },
        expected: expected.to_string(),
        actual: actual.to_string(),
        issue,
        hint,
    }
}

fn join_json_path(base: &str, segment: &str) -> String {
    if base.is_empty() || base == "/" {
        format!("/{segment}")
    } else {
        format!("{base}/{segment}")
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
    app.session.tool_pipeline.tool_results.push(tool_message);

    app.clear_status();
    app.session.tool_pipeline.active_tool_request = None;
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
        .tool_pipeline
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

    app.session.tool_pipeline.tool_results.push(ChatMessage {
        role: "tool".to_string(),
        content: context_payload.clone(),
        name: None,
        tool_call_id: tool_call_id.clone(),
        tool_calls: None,
    });

    app.session
        .tool_pipeline
        .tool_result_history
        .push(ToolResultRecord {
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
                .tool_pipeline
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
        .tool_pipeline
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

    let mut keep = Vec::with_capacity(app.session.tool_pipeline.tool_payload_history.len());
    let mut drop = count - window;
    for entry in app.session.tool_pipeline.tool_payload_history.iter() {
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
    app.session.tool_pipeline.tool_payload_history = keep;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::data::McpServerConfig;
    use crate::utils::test_utils::create_test_app;
    use rust_mcp_schema::{ListToolsResult, Tool, ToolInputSchema};
    use std::collections::HashMap;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    fn add_test_tool(
        app: &mut App,
        server_id: &str,
        tool_name: &str,
        input_schema: ToolInputSchema,
    ) {
        app.config.mcp_servers.push(McpServerConfig {
            id: server_id.to_string(),
            display_name: "Alpha MCP".to_string(),
            base_url: Some("https://mcp.example.com".to_string()),
            command: None,
            args: None,
            env: None,
            headers: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        });
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let tool = Tool {
            annotations: None,
            description: Some("Test tool".to_string()),
            execution: None,
            icons: Vec::new(),
            input_schema,
            meta: None,
            name: tool_name.to_string(),
            output_schema: None,
            title: None,
        };

        if let Some(server) = app.mcp.server_mut(server_id) {
            server.set_cached_tools(ListToolsResult {
                meta: None,
                next_cursor: None,
                tools: vec![tool],
            });
        } else {
            panic!("missing MCP server state");
        }
    }

    #[test]
    fn prepare_tool_flow_returns_structured_schema_validation_error() {
        let mut app = create_test_app();

        let mut nested_object = Map::new();
        nested_object.insert("type".to_string(), Value::String("object".to_string()));

        let mut properties = HashMap::new();
        properties.insert("filters".to_string(), nested_object);
        let input_schema = ToolInputSchema::new(Vec::new(), Some(properties), None);
        add_test_tool(&mut app, "alpha", "search", input_schema);

        let command = prepare_tool_flow(
            &mut app,
            vec![(
                0,
                PendingToolCall {
                    id: Some("call-1".to_string()),
                    name: Some("search".to_string()),
                    arguments: r#"{"filters":"{\"tag\":\"compost\"}"}"#.to_string(),
                },
            )],
            default_ctx(),
        );

        assert!(command.is_none());
        assert!(app.session.tool_pipeline.pending_tool_queue.is_empty());

        let record = app
            .session
            .tool_pipeline
            .tool_result_history
            .last()
            .expect("tool error record");
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolError));

        let payload: Value = serde_json::from_str(&record.content).expect("validation payload");
        assert_eq!(payload["isError"], Value::Bool(true));
        assert_eq!(payload["error"]["kind"], "invalid_arguments");
        assert_eq!(payload["error"]["violations"][0]["path"], "/filters");
        assert_eq!(payload["error"]["violations"][0]["expected"], "object");
        assert_eq!(payload["error"]["violations"][0]["actual"], "string");
        assert!(payload["error"]["violations"][0]["hint"]
            .as_str()
            .expect("hint")
            .contains("quoted JSON string"));
    }

    #[test]
    fn prepare_tool_flow_flags_missing_required_arguments_before_dispatch() {
        let mut app = create_test_app();

        let mut query_schema = Map::new();
        query_schema.insert("type".to_string(), Value::String("string".to_string()));

        let mut properties = HashMap::new();
        properties.insert("query".to_string(), query_schema);
        let input_schema = ToolInputSchema::new(vec!["query".to_string()], Some(properties), None);
        add_test_tool(&mut app, "alpha", "search", input_schema);

        let command = prepare_tool_flow(
            &mut app,
            vec![(
                0,
                PendingToolCall {
                    id: Some("call-1".to_string()),
                    name: Some("search".to_string()),
                    arguments: String::new(),
                },
            )],
            default_ctx(),
        );

        assert!(command.is_none());
        assert!(app.session.tool_pipeline.pending_tool_queue.is_empty());

        let record = app
            .session
            .tool_pipeline
            .tool_result_history
            .last()
            .expect("tool error record");
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolError));

        let payload: Value = serde_json::from_str(&record.content).expect("validation payload");
        assert_eq!(payload["error"]["violations"][0]["path"], "/query");
        assert_eq!(payload["error"]["violations"][0]["expected"], "present");
        assert_eq!(payload["error"]["violations"][0]["actual"], "missing");
    }

    #[test]
    fn prepare_tool_flow_rejects_any_of_union_mismatches() {
        let mut app = create_test_app();

        let mut nullable_object = Map::new();
        nullable_object.insert(
            "anyOf".to_string(),
            Value::Array(vec![
                Value::Object(Map::from_iter([(
                    "type".to_string(),
                    Value::String("object".to_string()),
                )])),
                Value::Object(Map::from_iter([(
                    "type".to_string(),
                    Value::String("null".to_string()),
                )])),
            ]),
        );

        let mut properties = HashMap::new();
        properties.insert("filters".to_string(), nullable_object);
        let input_schema = ToolInputSchema::new(Vec::new(), Some(properties), None);
        add_test_tool(&mut app, "alpha", "search", input_schema);

        let command = prepare_tool_flow(
            &mut app,
            vec![(
                0,
                PendingToolCall {
                    id: Some("call-1".to_string()),
                    name: Some("search".to_string()),
                    arguments: r#"{"filters":7}"#.to_string(),
                },
            )],
            default_ctx(),
        );

        assert!(command.is_none());
        assert!(app.session.tool_pipeline.pending_tool_queue.is_empty());

        let record = app
            .session
            .tool_pipeline
            .tool_result_history
            .last()
            .expect("tool error record");
        let payload: Value = serde_json::from_str(&record.content).expect("validation payload");
        assert_eq!(payload["error"]["violations"][0]["path"], "/filters");
        assert_eq!(payload["error"]["violations"][0]["expected"], "anyOf");
    }

    #[test]
    fn prepare_tool_flow_accepts_any_of_union_matches() {
        let mut app = create_test_app();

        let mut nullable_object = Map::new();
        nullable_object.insert(
            "anyOf".to_string(),
            Value::Array(vec![
                Value::Object(Map::from_iter([(
                    "type".to_string(),
                    Value::String("object".to_string()),
                )])),
                Value::Object(Map::from_iter([(
                    "type".to_string(),
                    Value::String("null".to_string()),
                )])),
            ]),
        );

        let mut properties = HashMap::new();
        properties.insert("filters".to_string(), nullable_object);
        let input_schema = ToolInputSchema::new(Vec::new(), Some(properties), None);
        add_test_tool(&mut app, "alpha", "search", input_schema);

        let command = prepare_tool_flow(
            &mut app,
            vec![(
                0,
                PendingToolCall {
                    id: Some("call-1".to_string()),
                    name: Some("search".to_string()),
                    arguments: r#"{"filters":null}"#.to_string(),
                },
            )],
            default_ctx(),
        );

        assert!(command.is_none());
        assert!(app.session.tool_pipeline.pending_tool_queue.is_empty());
        assert!(app.session.tool_pipeline.active_tool_request.is_some());
        assert!(app.ui.tool_prompt().is_some());
    }
}
