use std::collections::VecDeque;
use std::time::Instant;

use super::{App, AppAction, AppActionContext, AppCommand};
use crate::api::{ChatMessage, ChatToolCall, ChatToolCallFunction};
use crate::core::app::session::{
    McpPromptRequest, McpSamplingRequest, PendingToolCall, ToolCallRequest, ToolFailureKind,
    ToolPayloadHistoryEntry, ToolResultRecord, ToolResultStatus,
};
use crate::core::app::ui_state::ToolPromptRequest;
use crate::core::chat_stream::StreamParams;
use crate::core::config::data::McpToolPayloadRetention;
use crate::core::mcp_sampling::{
    build_sampling_messages, serialize_sampling_params, summarize_sampling_request,
};
use crate::core::message::{AppMessageKind, Message, ROLE_ASSISTANT, ROLE_USER};
use crate::mcp::permissions::ToolPermissionDecision;
use crate::mcp::{MCP_INSTANT_RECALL_TOOL, MCP_SESSION_MEMORY_SERVER_ID};
use rust_mcp_schema::schema_utils::ServerJsonrpcRequest;
use rust_mcp_schema::{
    ClientCapabilities, ClientSampling, ContentBlock, PromptMessage, Role, RpcError,
};
use serde_json::{Map, Value};
use tracing::debug;

#[derive(Debug, Clone)]
struct ToolResultMeta {
    server_label: Option<String>,
    server_id: Option<String>,
    tool_call_id: Option<String>,
    raw_arguments: Option<String>,
    failure_kind: Option<ToolFailureKind>,
}

impl ToolResultMeta {
    fn new(
        server_label: Option<String>,
        server_id: Option<String>,
        tool_call_id: Option<String>,
        raw_arguments: Option<String>,
    ) -> Self {
        Self {
            server_label,
            server_id,
            tool_call_id,
            raw_arguments,
            failure_kind: None,
        }
    }
}

#[derive(Debug, Clone)]
struct PendingToolError {
    tool_name: String,
    server_id: Option<String>,
    tool_call_id: Option<String>,
    raw_arguments: Option<String>,
    error: String,
}

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
        AppAction::ToolCallCompleted {
            tool_name,
            tool_call_id,
            result,
        } => handle_tool_call_completed(app, tool_name, tool_call_id, result, ctx),
        AppAction::McpPromptCompleted { request, result } => {
            handle_mcp_prompt_completed(app, request, result, ctx)
        }
        AppAction::McpServerRequestReceived { request } => {
            handle_mcp_server_request(app, *request, ctx)
        }
        AppAction::McpSamplingFinished => handle_mcp_sampling_finished(app, ctx),
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

fn is_tool_error_payload(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|value| value.get("isError").and_then(Value::as_bool))
        .unwrap_or(false)
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
    let prompt_tool = app.ui.tool_prompt().map(|prompt| prompt.tool_name.clone());
    debug!(
        decision = ?decision,
        prompt_tool = prompt_tool.as_deref().unwrap_or("<none>"),
        "Tool permission decision received"
    );

    if prompt_tool.as_deref() == Some(crate::mcp::MCP_SAMPLING_TOOL) {
        let request = app.session.active_sampling_request.take()?;
        return handle_sampling_permission_decision(app, request, decision, ctx);
    }

    if let Some(request) = app.session.active_tool_request.take() {
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
            let meta = ToolResultMeta::new(
                Some(server_label),
                Some(request.server_id.clone()),
                request.tool_call_id.clone(),
                Some(request.raw_arguments.clone()),
            );
            record_tool_result(
                app,
                &request.tool_name,
                meta,
                message.to_string(),
                status,
                ctx,
            );
            return advance_tool_queue(app, ctx);
        }

        app.session.active_tool_request = Some(request.clone());
        if is_instant_recall_tool(&request.tool_name) {
            return handle_instant_recall_tool_request(app, request, ctx);
        }
        set_status_for_tool_run(app, &request, ctx);
        return Some(AppCommand::RunMcpTool(request));
    }

    let request = app.session.active_sampling_request.take()?;
    handle_sampling_permission_decision(app, request, decision, ctx)
}

fn handle_mcp_server_request(
    app: &mut App,
    request: crate::mcp::events::McpServerRequest,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.request_id(),
        method = %request.request.method(),
        "Received MCP server request"
    );
    if app.session.mcp_disabled {
        debug!(
            server_id = %request.server_id,
            request_id = ?request.request.request_id(),
            "MCP server request ignored (MCP disabled)"
        );
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.request_id().clone(),
            error: RpcError::internal_error().with_message("MCP is disabled."),
        });
    }

    let capabilities = ClientCapabilities {
        sampling: Some(ClientSampling::default()),
        ..ClientCapabilities::default()
    };
    if let Err(err) = capabilities.can_handle_request(&request.request) {
        debug!(
            server_id = %request.server_id,
            request_id = ?request.request.request_id(),
            error_code = err.code,
            "MCP server request rejected (unsupported)"
        );
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.request_id().clone(),
            error: err,
        });
    }

    let request_id = request.request.request_id().clone();
    let ServerJsonrpcRequest::CreateMessageRequest(create_request) = request.request else {
        debug!(
            server_id = %request.server_id,
            request_id = ?request_id,
            "MCP server request rejected (unexpected method)"
        );
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id,
            error: RpcError::method_not_found()
                .with_message("Unsupported MCP request from server."),
        });
    };

    let messages = match build_sampling_messages(&create_request) {
        Ok(messages) => messages,
        Err(err) => {
            debug!(
                server_id = %request.server_id,
                request_id = ?create_request.id,
                error = %err,
                "MCP sampling request rejected (invalid params)"
            );
            return Some(AppCommand::SendMcpServerError {
                server_id: request.server_id,
                request_id: create_request.id.clone(),
                error: RpcError::invalid_params().with_message(&err),
            });
        }
    };

    app.session
        .pending_sampling_queue
        .push_back(McpSamplingRequest {
            server_id: request.server_id,
            request: create_request,
            messages,
        });

    debug!(
        pending = app.session.pending_sampling_queue.len(),
        active = app.session.active_sampling_request.is_some(),
        "Queued MCP sampling request"
    );

    advance_sampling_queue(app, ctx)
}

fn handle_sampling_permission_decision(
    app: &mut App,
    request: McpSamplingRequest,
    decision: ToolPermissionDecision,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    debug!(
        server_id = %request.server_id,
        request_id = ?request.request.id,
        decision = ?decision,
        "Sampling permission decision"
    );
    app.ui.cancel_tool_prompt();
    app.clear_status();

    match decision {
        ToolPermissionDecision::AllowOnce => {}
        ToolPermissionDecision::AllowSession | ToolPermissionDecision::Block => app
            .mcp_permissions
            .record(&request.server_id, crate::mcp::MCP_SAMPLING_TOOL, decision),
        ToolPermissionDecision::DenyOnce => {}
    }

    if matches!(
        decision,
        ToolPermissionDecision::DenyOnce | ToolPermissionDecision::Block
    ) {
        let message = match decision {
            ToolPermissionDecision::Block => "Sampling blocked by user.",
            _ => "Sampling denied by user.",
        };
        return Some(AppCommand::SendMcpServerError {
            server_id: request.server_id,
            request_id: request.request.id.clone(),
            error: RpcError {
                code: -1,
                message: message.to_string(),
                data: None,
            },
        });
    }

    app.session.active_sampling_request = Some(request.clone());
    set_status_for_sampling_run(app, &request, ctx);
    Some(AppCommand::RunMcpSampling(Box::new(request)))
}

fn handle_mcp_sampling_finished(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    app.session.active_sampling_request = None;
    if let Some(request) = app.session.active_tool_request.clone() {
        set_status_for_tool_run(app, &request, ctx);
    } else {
        app.clear_status();
    }
    advance_sampling_queue(app, ctx)
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
            let is_tool_error = is_tool_error_payload(&payload);
            let mut meta = ToolResultMeta::new(
                server_label,
                request.as_ref().map(|req| req.server_id.clone()),
                tool_call_id.clone(),
                request.as_ref().map(|req| req.raw_arguments.clone()),
            );
            if is_tool_error {
                meta.failure_kind = Some(ToolFailureKind::ToolError);
            }
            record_tool_result(
                app,
                &tool_name,
                meta,
                payload,
                if is_tool_error {
                    ToolResultStatus::Error
                } else {
                    ToolResultStatus::Success
                },
                ctx,
            );
        }
        Err(err) => {
            let mut meta = ToolResultMeta::new(
                server_label,
                request.as_ref().map(|req| req.server_id.clone()),
                tool_call_id,
                request.as_ref().map(|req| req.raw_arguments.clone()),
            );
            meta.failure_kind = Some(ToolFailureKind::ToolCallFailure);
            record_tool_result(
                app,
                &tool_name,
                meta,
                format!("Tool call failure: {err}"),
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
                Role::User => ROLE_USER,
                Role::Assistant => ROLE_ASSISTANT,
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
    let mut pending_errors: Vec<PendingToolError> = Vec::new();

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
            pending_errors.push(PendingToolError {
                tool_name: tool_name.clone(),
                server_id: None,
                tool_call_id: Some(tool_call_id.clone()),
                raw_arguments: Some(raw_arguments.clone()),
                error: "Missing tool name.".to_string(),
            });
            continue;
        }

        if tool_name.eq_ignore_ascii_case(crate::mcp::MCP_READ_RESOURCE_TOOL) {
            match parse_resource_read_arguments(&raw_arguments) {
                Ok((server_id, _uri, arguments)) => {
                    match app.mcp.server(&server_id) {
                        Some(server) if server.config.is_enabled() => {}
                        Some(_) => {
                            pending_errors.push(PendingToolError {
                                tool_name: tool_name.clone(),
                                server_id: Some(server_id.clone()),
                                tool_call_id: Some(tool_call_id.clone()),
                                raw_arguments: Some(raw_arguments.clone()),
                                error: format!("MCP server is disabled: {server_id}."),
                            });
                            continue;
                        }
                        None => {
                            pending_errors.push(PendingToolError {
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
                    pending_errors.push(PendingToolError {
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
                    pending_errors.push(PendingToolError {
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
                    pending_errors.push(PendingToolError {
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
                    pending_errors.push(PendingToolError {
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
        let mut meta = ToolResultMeta::new(
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
            let meta = ToolResultMeta::new(
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
    meta: ToolResultMeta,
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
    meta: &ToolResultMeta,
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

fn set_status_for_sampling_run(app: &mut App, request: &McpSamplingRequest, ctx: AppActionContext) {
    let input_area_height = app.input_area_height(ctx.term_width);
    let server_name = resolve_server_label(app, &request.server_id);
    let mut conversation = app.conversation();
    conversation.set_status(format!(
        "Generating text for MCP tool response on {}...",
        server_name
    ));
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
        assert_eq!(tool_call.content, "lookup | Arguments: q=\"mcp\"");

        let tool_result = app
            .ui
            .messages
            .iter()
            .find(|msg| msg.role == ROLE_TOOL_RESULT)
            .expect("missing tool result message");
        assert!(tool_result.content.contains("lookup"));
    }

    #[test]
    fn tool_call_completed_flags_tool_error_payloads() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.active_tool_request = Some(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "lookup".to_string(),
            arguments: None,
            raw_arguments: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
        });

        let payload = serde_json::json!({
            "content": [],
            "isError": true,
        })
        .to_string();

        let result = handle_streaming_action(
            &mut app,
            AppAction::ToolCallCompleted {
                tool_name: "lookup".to_string(),
                tool_call_id: Some("call-1".to_string()),
                result: Ok(payload),
            },
            ctx,
        );
        assert!(result.is_none());

        let record = app
            .session
            .tool_result_history
            .last()
            .expect("missing tool result record");
        assert_eq!(record.status, ToolResultStatus::Error);
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolError));
    }

    #[test]
    fn tool_call_completed_flags_call_failures() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.active_tool_request = Some(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "lookup".to_string(),
            arguments: None,
            raw_arguments: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
        });

        let result = handle_streaming_action(
            &mut app,
            AppAction::ToolCallCompleted {
                tool_name: "lookup".to_string(),
                tool_call_id: Some("call-1".to_string()),
                result: Err("timeout".to_string()),
            },
            ctx,
        );
        assert!(result.is_none());

        let record = app
            .session
            .tool_result_history
            .last()
            .expect("missing tool result record");
        assert_eq!(record.status, ToolResultStatus::Error);
        assert_eq!(record.failure_kind, Some(ToolFailureKind::ToolCallFailure));
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
