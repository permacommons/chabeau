use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::api::ChatRequest;
use crate::core::app::session::ToolCallRequest;
use crate::core::app::StreamingAction;
use crate::core::mcp_auth::McpTokenStore;
use crate::core::mcp_sampling::map_finish_reason;
use crate::core::message::AppMessageKind;
use rust_mcp_schema::schema_utils::ResultFromClient;
use rust_mcp_schema::{CreateMessageContent, CreateMessageResult, Role, TextContent};

use super::ExecutorContext;

const MCP_SAMPLING_DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const MCP_SAMPLING_SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub fn spawn_mcp_tool_call(context: ExecutorContext, request: ToolCallRequest) {
    tokio::spawn(async move {
        let ctx = context.action_context();

        let mut call_context = match context
            .app
            .read(|app| app.mcp.tool_call_context(&request.server_id))
            .await
        {
            Some(call_context) => call_context,
            None => {
                context.dispatcher.dispatch_many(
                    [StreamingAction::ToolCallCompleted {
                        tool_name: request.tool_name.clone(),
                        tool_call_id: request.tool_call_id.clone(),
                        result: Err("MCP server not available.".to_string()),
                    }],
                    ctx,
                );
                return;
            }
        };

        let result = if request
            .tool_name
            .eq_ignore_ascii_case(crate::mcp::MCP_READ_RESOURCE_TOOL)
        {
            let uri = match request
                .arguments
                .as_ref()
                .and_then(|args: &serde_json::Map<String, serde_json::Value>| args.get("uri"))
                .and_then(|value: &serde_json::Value| value.as_str())
            {
                Some(uri) => uri.to_string(),
                None => {
                    context.dispatcher.dispatch_many(
                        [StreamingAction::ToolCallCompleted {
                            tool_name: request.tool_name.clone(),
                            tool_call_id: request.tool_call_id.clone(),
                            result: Err("Resource read requires uri.".to_string()),
                        }],
                        ctx,
                    );
                    return;
                }
            };

            run_cancellable(
                context.cancel_token.as_ref(),
                crate::mcp::client::execute_resource_read(&mut call_context, &uri),
            )
            .await
            .map(|result| serialize_mcp_result(&result))
        } else if request
            .tool_name
            .eq_ignore_ascii_case(crate::mcp::MCP_LIST_RESOURCES_TOOL)
        {
            let Some(arguments) = request.arguments.as_ref() else {
                context.dispatcher.dispatch_many(
                    [StreamingAction::ToolCallCompleted {
                        tool_name: request.tool_name.clone(),
                        tool_call_id: request.tool_call_id.clone(),
                        result: Err("Resource list arguments are required.".to_string()),
                    }],
                    ctx,
                );
                return;
            };

            let (kind, cursor) =
                match crate::core::app::actions::parse_resource_list_kind(arguments) {
                    Ok(values) => values,
                    Err(error) => {
                        context.dispatcher.dispatch_many(
                            [StreamingAction::ToolCallCompleted {
                                tool_name: request.tool_name.clone(),
                                tool_call_id: request.tool_call_id.clone(),
                                result: Err(error),
                            }],
                            ctx,
                        );
                        return;
                    }
                };

            match kind {
                crate::core::app::actions::ResourceListKind::Resources => run_cancellable(
                    context.cancel_token.as_ref(),
                    crate::mcp::client::execute_resource_list(&mut call_context, cursor),
                )
                .await
                .map(|result| serialize_mcp_result(&result)),
                crate::core::app::actions::ResourceListKind::Templates => run_cancellable(
                    context.cancel_token.as_ref(),
                    crate::mcp::client::execute_resource_template_list(&mut call_context, cursor),
                )
                .await
                .map(|result| serialize_mcp_result(&result)),
            }
        } else {
            run_cancellable(
                context.cancel_token.as_ref(),
                crate::mcp::client::execute_tool_call(&mut call_context, &request),
            )
            .await
            .map(|result| serialize_mcp_result(&result))
        };

        let session_id = call_context.session_id.clone();
        let error = result.as_ref().err().cloned();
        context
            .app
            .update(|app| {
                app.mcp
                    .update_tool_call_session(&call_context.server_id, session_id, error);
            })
            .await;

        context.dispatcher.dispatch_many(
            [StreamingAction::ToolCallCompleted {
                tool_name: request.tool_name.clone(),
                tool_call_id: request.tool_call_id.clone(),
                result,
            }],
            ctx,
        );
    });
}

pub fn spawn_mcp_prompt_call(
    context: ExecutorContext,
    request: crate::core::app::session::McpPromptRequest,
) {
    tokio::spawn(async move {
        let ctx = context.action_context();

        let mut call_context = match context
            .app
            .read(|app| app.mcp.prompt_call_context(&request.server_id))
            .await
        {
            Some(call_context) => call_context,
            None => {
                context.dispatcher.dispatch_many(
                    [StreamingAction::McpPromptCompleted {
                        request,
                        result: Err("MCP server not available.".to_string()),
                    }],
                    ctx,
                );
                return;
            }
        };

        let result = crate::mcp::client::execute_prompt(&mut call_context, &request).await;

        let session_id = call_context.session_id.clone();
        let error = result.as_ref().err().cloned();
        context
            .app
            .update(|app| {
                app.mcp
                    .update_prompt_session(&call_context.server_id, session_id, error);
            })
            .await;

        context.dispatcher.dispatch_many(
            [StreamingAction::McpPromptCompleted { request, result }],
            ctx,
        );
    });
}

pub fn spawn_mcp_sampling_call(
    context: ExecutorContext,
    request: crate::core::app::session::McpSamplingRequest,
) {
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let ctx = context.action_context();

        let (client, base_url, api_key, provider_name, model) = context
            .app
            .read(|app| {
                (
                    app.session.client.clone(),
                    app.session.base_url.clone(),
                    app.session.api_key.clone(),
                    app.session.provider_name.clone(),
                    app.session.model.clone(),
                )
            })
            .await;

        debug!(
            server_id = %request.server_id,
            request_id = ?request.request.id,
            "Starting MCP sampling request"
        );

        let messages = request.messages;
        let stop = if request.request.params.stop_sequences.is_empty() {
            None
        } else {
            Some(request.request.params.stop_sequences.clone())
        };

        let chat_request = ChatRequest {
            model: model.clone(),
            messages,
            stream: false,
            tools: None,
            max_tokens: Some(request.request.params.max_tokens),
            temperature: request.request.params.temperature,
            stop,
        };

        let mut request_context = match context
            .app
            .read(|app| app.mcp.server_request_context(&request.server_id))
            .await
        {
            Some(request_context) => request_context,
            None => {
                context
                    .dispatcher
                    .dispatch_many([StreamingAction::McpSamplingFinished], ctx);
                return;
            }
        };

        let default_sampling_timeout = MCP_SAMPLING_DEFAULT_TIMEOUT;
        let sampling_timeout =
            crate::core::mcp_sampling::sampling_timeout_for_request(&request.request)
                .unwrap_or(default_sampling_timeout);

        let completion = match tokio::time::timeout(
            sampling_timeout,
            crate::core::chat_stream::request_chat_completion(
                &client,
                &base_url,
                &api_key,
                &provider_name,
                chat_request,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Sampling timed out after {}s",
                sampling_timeout.as_secs()
            )),
        };

        if context
            .cancel_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            let session_id = request_context.session_id.clone();
            context
                .app
                .update(|app| {
                    app.mcp.update_server_request_session(
                        &request_context.server_id,
                        session_id,
                        Some("MCP operation interrupted by user.".to_string()),
                    );
                })
                .await;
            context
                .dispatcher
                .dispatch_many([StreamingAction::McpSamplingFinished], ctx);
            return;
        }

        let send_operation = tokio::time::timeout(MCP_SAMPLING_SEND_TIMEOUT, async {
            match completion {
                Ok(result) => {
                    let content =
                        CreateMessageContent::from(TextContent::new(result.content, None, None));
                    let create_result = CreateMessageResult {
                        content,
                        meta: None,
                        model: model.clone(),
                        role: Role::Assistant,
                        stop_reason: map_finish_reason(result.finish_reason),
                    };
                    crate::mcp::client::send_client_result(
                        &mut request_context,
                        request.request.id.clone(),
                        ResultFromClient::CreateMessageResult(create_result),
                    )
                    .await
                }
                Err(error) => {
                    debug!(
                        server_id = %request.server_id,
                        request_id = ?request.request.id,
                        error = %error,
                        elapsed_ms = start.elapsed().as_millis(),
                        "MCP sampling completion failed"
                    );
                    crate::mcp::client::send_client_error(
                        &mut request_context,
                        request.request.id.clone(),
                        rust_mcp_schema::RpcError::internal_error().with_message(&error),
                    )
                    .await
                }
            }
        });

        let send_result = if let Some(token) = context.cancel_token.as_ref() {
            tokio::select! {
                _ = token.cancelled() => Err("MCP operation interrupted by user.".to_string()),
                timed = send_operation => {
                    match timed {
                        Ok(result) => result,
                        Err(_) => Err("Timed out sending MCP sampling response.".to_string()),
                    }
                }
            }
        } else {
            match send_operation.await {
                Ok(result) => result,
                Err(_) => Err("Timed out sending MCP sampling response.".to_string()),
            }
        };

        let session_id = request_context.session_id.clone();
        let error = send_result.err();
        context
            .app
            .update(|app| {
                app.mcp.update_server_request_session(
                    &request_context.server_id,
                    session_id,
                    error,
                );
            })
            .await;

        context
            .dispatcher
            .dispatch_many([StreamingAction::McpSamplingFinished], ctx);
    });
}

pub fn spawn_mcp_server_error(
    context: ExecutorContext,
    server_id: String,
    request_id: rust_mcp_schema::RequestId,
    error: rust_mcp_schema::RpcError,
) {
    tokio::spawn(async move {
        let ctx = context.action_context();

        let mut request_context = match context
            .app
            .read(|app| app.mcp.server_request_context(&server_id))
            .await
        {
            Some(request_context) => request_context,
            None => {
                context
                    .dispatcher
                    .dispatch_many([StreamingAction::McpSamplingFinished], ctx);
                return;
            }
        };

        let send_result =
            crate::mcp::client::send_client_error(&mut request_context, request_id, error).await;
        let session_id = request_context.session_id.clone();
        let error = send_result.err();
        context
            .app
            .update(|app| {
                app.mcp.update_server_request_session(
                    &request_context.server_id,
                    session_id,
                    error,
                );
            })
            .await;

        context
            .dispatcher
            .dispatch_many([StreamingAction::McpSamplingFinished], ctx);
    });
}

pub fn spawn_mcp_refresh(context: ExecutorContext, server_id: String) {
    tokio::spawn(async move {
        let mcp_disabled = context.app.read(|app| app.session.mcp_disabled).await;
        if mcp_disabled {
            context
                .app
                .update(|app| {
                    app.conversation().add_app_message(
                        AppMessageKind::Info,
                        "MCP: **disabled for this session**\nMCP refresh skipped.".to_string(),
                    );
                    app.ui.focus_transcript();
                    app.clear_status();
                    app.ui
                        .end_activity(crate::core::app::ActivityKind::McpRefresh);
                })
                .await;
            context.dispatcher.dispatch_input_many(
                [crate::core::app::StatusAction::ClearStatus],
                Default::default(),
            );
            return;
        }

        let keyring_enabled = context
            .app
            .read(|app| !cfg!(test) && !app.session.startup_env_only)
            .await;
        let token_store = McpTokenStore::new_with_keyring(keyring_enabled);

        let mut mcp = context.app.read(|app| app.mcp.clone()).await;
        mcp.connect_server(&server_id, &token_store).await;
        mcp.refresh_tools(&server_id).await;
        mcp.refresh_resources(&server_id).await;
        mcp.refresh_resource_templates(&server_id).await;
        mcp.refresh_prompts(&server_id).await;

        let output = mcp.server(&server_id).map(|server| {
            crate::commands::build_mcp_server_output(server, keyring_enabled, &token_store)
        });

        context
            .app
            .update(|app| {
                app.mcp = mcp;
                if let Some(message) = output {
                    app.conversation()
                        .add_app_message(AppMessageKind::Info, message);
                    app.ui.focus_transcript();
                    let term_size = app.ui.last_term_size;
                    if term_size.width > 0 && term_size.height > 0 {
                        let input_area_height = app.input_area_height(term_size.width);
                        let mut conversation = app.conversation();
                        let available_height = conversation
                            .calculate_available_height(term_size.height, input_area_height);
                        conversation.update_scroll_position(available_height, term_size.width);
                    }
                } else {
                    app.conversation()
                        .set_status(format!("Unknown MCP server: {}", server_id));
                }
                app.clear_status();
                app.ui
                    .end_activity(crate::core::app::ActivityKind::McpRefresh);
            })
            .await;

        context.dispatcher.dispatch_input_many(
            [crate::core::app::StatusAction::ClearStatus],
            Default::default(),
        );
    });
}

fn serialize_mcp_result<T: Serialize>(result: &T) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|_| "Unable to serialize MCP result.".to_string())
}

pub async fn run_cancellable<F, T>(
    cancel_token: Option<&CancellationToken>,
    operation: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Some(token) = cancel_token {
        tokio::select! {
            _ = token.cancelled() => Err("MCP operation interrupted by user.".to_string()),
            result = operation => result,
        }
    } else {
        operation.await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{AppAction, AppActionDispatcher};
    use crate::ui::chat_loop::AppHandle;
    use crate::ui::theme::Theme;
    use ratatui::prelude::Size;
    use serde_json::{Map, Value};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    fn app_handle() -> AppHandle {
        let app = crate::core::app::App::new_test_app(Theme::dark_default(), true, true);
        AppHandle::new(Arc::new(Mutex::new(app)))
    }

    #[tokio::test]
    async fn run_cancellable_returns_cancelled_error() {
        let token = CancellationToken::new();
        token.cancel();

        let result = run_cancellable(Some(&token), async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok::<_, String>(())
        })
        .await;

        assert_eq!(
            result,
            Err("MCP operation interrupted by user.".to_string())
        );
    }

    #[tokio::test]
    async fn spawn_mcp_tool_call_dispatches_missing_server_error() {
        let app = app_handle();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let dispatcher = AppActionDispatcher::new(tx);
        let context = ExecutorContext {
            app,
            dispatcher,
            cancel_token: None,
            term_size: Size::new(80, 24),
        };

        let request = ToolCallRequest {
            server_id: "missing".to_string(),
            tool_name: "tool".to_string(),
            arguments: Some(Map::from_iter([("a".to_string(), Value::from(1))])),
            raw_arguments: "{\"a\":1}".to_string(),
            tool_call_id: Some("call-1".to_string()),
        };

        spawn_mcp_tool_call(context, request);

        let action = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("action timeout")
            .expect("action missing");

        assert!(matches!(
            action.action,
            AppAction::Streaming(StreamingAction::ToolCallCompleted { result: Err(_), .. })
        ));
    }
}
