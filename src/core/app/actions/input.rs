use super::{streaming, App, AppAction, AppActionContext, AppCommand};
use crate::commands::{process_input, CommandResult};
use crate::core::app::picker::build_inspect_text;
use crate::core::app::session::{ToolCallRequest, ToolResultRecord};
use crate::core::app::{InspectMode, ToolInspectKind, ToolInspectView};

pub(super) fn handle_input_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::ClearStatus => {
            app.clear_status();
            None
        }
        AppAction::ToggleComposeMode => {
            app.toggle_compose_mode();
            None
        }
        AppAction::CancelFilePrompt => {
            app.cancel_file_prompt();
            None
        }
        AppAction::CancelMcpPromptInput => {
            app.ui.cancel_mcp_prompt_input();
            None
        }
        AppAction::CancelInPlaceEdit => {
            if app.has_in_place_edit() {
                app.cancel_in_place_edit();
                app.clear_input();
            }
            None
        }
        AppAction::SetStatus { message } => {
            set_status_message(app, message, ctx);
            None
        }
        AppAction::ClearInput => {
            app.clear_input();
            if ctx.term_width > 0 {
                app.recompute_input_layout_after_edit(ctx.term_width);
            }
            None
        }
        AppAction::InsertIntoInput { text } => {
            if !text.is_empty() {
                app.insert_into_input(&text, ctx.term_width);
            }
            None
        }
        AppAction::InspectToolResults => {
            open_latest_tool_call_inspect(app, ctx);
            None
        }
        AppAction::InspectToolResultsToggleView => {
            toggle_tool_call_inspect_view(app, ctx);
            None
        }
        AppAction::InspectToolResultsStep { delta } => {
            step_tool_call_inspect(app, delta, ctx);
            None
        }
        AppAction::InspectToolResultsCopy => {
            copy_tool_call_inspect_data(app, ctx);
            None
        }
        AppAction::InspectToolResultsToggleDecode => {
            toggle_tool_call_inspect_decode(app, ctx);
            None
        }
        AppAction::ProcessCommand { input } => handle_process_command(app, input, ctx),
        AppAction::CompleteInPlaceEdit { index, new_text } => {
            app.complete_in_place_edit(index, new_text);
            None
        }
        AppAction::CompleteAssistantEdit { content } => {
            app.complete_assistant_edit(content);
            update_scroll_after_command(app, ctx);
            None
        }
        _ => unreachable!("non-input action routed to input handler"),
    }
}

fn handle_process_command(
    app: &mut App,
    input: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    if input.trim().is_empty() {
        return None;
    }

    match process_input(app, &input) {
        CommandResult::Continue => {
            app.conversation().show_character_greeting_if_needed();
            update_scroll_after_command(app, ctx);
            None
        }
        CommandResult::ContinueWithTranscriptFocus => {
            app.conversation().show_character_greeting_if_needed();
            app.ui.focus_transcript();
            update_scroll_after_command(app, ctx);
            None
        }
        CommandResult::ProcessAsMessage(message) => {
            streaming::spawn_stream_for_message(app, message, ctx)
        }
        CommandResult::OpenModelPicker => match app.prepare_model_picker_request() {
            Ok(request) => Some(AppCommand::LoadModelPicker(request)),
            Err(err) => {
                set_status_message(app, format!("Model picker error: {}", err), ctx);
                None
            }
        },
        CommandResult::OpenProviderPicker => {
            app.open_provider_picker();
            None
        }
        CommandResult::OpenThemePicker => {
            if let Err(err) = app.open_theme_picker() {
                set_status_message(app, format!("Theme picker error: {}", err), ctx);
            }
            None
        }
        CommandResult::OpenCharacterPicker => {
            app.open_character_picker();
            None
        }
        CommandResult::OpenPersonaPicker => {
            app.open_persona_picker();
            None
        }
        CommandResult::OpenPresetPicker => {
            app.open_preset_picker();
            None
        }
        CommandResult::Refine(prompt) => {
            let action = AppAction::RefineLastMessage { prompt };
            streaming::handle_streaming_action(app, action, ctx)
        }
        CommandResult::RunMcpPrompt(request) => Some(AppCommand::RunMcpPrompt(request)),
        CommandResult::RefreshMcp { server_id } => {
            app.ui.focus_transcript();
            update_scroll_after_command(app, ctx);
            Some(AppCommand::RefreshMcp { server_id })
        }
    }
}

pub(super) fn set_status_message(app: &mut App, message: String, ctx: AppActionContext) {
    app.conversation().set_status(message);
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    }
}

fn update_scroll_after_command(app: &mut App, ctx: AppActionContext) {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

struct ToolInspectSnapshot {
    results: Vec<ToolResultRecord>,
    pending: Vec<ToolCallRequest>,
}

impl ToolInspectSnapshot {
    fn total(&self) -> usize {
        self.results.len().saturating_add(self.pending.len())
    }
}

fn tool_inspect_snapshot(app: &App) -> ToolInspectSnapshot {
    let mut pending = Vec::new();
    if let Some(request) = app.session.active_tool_request.clone() {
        pending.push(request);
    }
    pending.extend(app.session.pending_tool_queue.iter().cloned());
    ToolInspectSnapshot {
        results: app.session.tool_result_history.clone(),
        pending,
    }
}

fn open_latest_tool_call_inspect(app: &mut App, ctx: AppActionContext) {
    let snapshot = tool_inspect_snapshot(app);
    if snapshot.total() == 0 {
        set_status_message(app, "No tool calls to inspect yet.".to_string(), ctx);
        return;
    }

    let decoded = app
        .inspect_state()
        .map(|state| state.decoded)
        .unwrap_or(false);

    if app.ui.tool_prompt().is_some() && !snapshot.pending.is_empty() {
        let index = snapshot.results.len();
        open_tool_call_inspect_at(app, index, ToolInspectView::Request, decoded, ctx);
        return;
    }

    if !snapshot.results.is_empty() {
        let index = snapshot.results.len().saturating_sub(1);
        open_tool_call_inspect_at(app, index, ToolInspectView::Result, decoded, ctx);
        return;
    }

    open_tool_call_inspect_at(
        app,
        snapshot.results.len(),
        ToolInspectView::Request,
        decoded,
        ctx,
    );
}

fn step_tool_call_inspect(app: &mut App, delta: i32, ctx: AppActionContext) {
    let Some(state) = app.inspect_state() else {
        return;
    };
    let InspectMode::ToolCalls { index, view, .. } = state.mode else {
        return;
    };
    let snapshot = tool_inspect_snapshot(app);
    let total = snapshot.total();
    if total < 2 {
        return;
    }
    let step = if delta >= 0 {
        1usize
    } else {
        total.saturating_sub(1)
    };
    let next = (index + step) % total;
    open_tool_call_inspect_at(app, next, view, state.decoded, ctx);
}

fn toggle_tool_call_inspect_view(app: &mut App, ctx: AppActionContext) {
    let Some(state) = app.inspect_state() else {
        return;
    };
    let InspectMode::ToolCalls { index, view, kind } = state.mode else {
        return;
    };
    if matches!(kind, ToolInspectKind::Pending) {
        set_status_message(
            app,
            "Pending tool calls have no result to show yet.".to_string(),
            ctx,
        );
        return;
    }
    open_tool_call_inspect_at(app, index, view.toggle(), state.decoded, ctx);
}

fn toggle_tool_call_inspect_decode(app: &mut App, ctx: AppActionContext) {
    let Some(state) = app.inspect_state() else {
        return;
    };
    let InspectMode::ToolCalls { index, view, .. } = state.mode else {
        return;
    };
    if !state.decoded && !tool_call_inspect_has_nested_json(app, index, view) {
        set_status_message(app, "No nested JSON detected.".to_string(), ctx);
        return;
    }
    open_tool_call_inspect_at(app, index, view, !state.decoded, ctx);
}

fn open_tool_call_inspect_at(
    app: &mut App,
    index: usize,
    view: ToolInspectView,
    decoded: bool,
    ctx: AppActionContext,
) {
    let snapshot = tool_inspect_snapshot(app);
    if snapshot.total() == 0 {
        set_status_message(app, "No tool calls to inspect yet.".to_string(), ctx);
        return;
    }

    let (title, content, kind, view) = if index < snapshot.results.len() {
        let record = snapshot
            .results
            .get(index)
            .cloned()
            .unwrap_or_else(|| snapshot.results.last().cloned().unwrap());
        let title = match view {
            ToolInspectView::Result => build_tool_result_title(
                &record,
                index.min(snapshot.results.len().saturating_sub(1)),
                snapshot.results.len(),
            ),
            ToolInspectView::Request => build_tool_request_title(
                &record,
                index.min(snapshot.results.len().saturating_sub(1)),
                snapshot.results.len(),
            ),
        };
        let content = match view {
            ToolInspectView::Result => build_tool_result_content(&record, decoded),
            ToolInspectView::Request => build_tool_request_content(&record, decoded),
        };
        (title, content, ToolInspectKind::Result, view)
    } else {
        let pending_index = index.saturating_sub(snapshot.results.len());
        let request = snapshot
            .pending
            .get(pending_index)
            .cloned()
            .unwrap_or_else(|| snapshot.pending.last().cloned().unwrap());
        let title = build_tool_pending_title(
            &request,
            pending_index.min(snapshot.pending.len().saturating_sub(1)),
            snapshot.pending.len(),
            app,
        );
        let content = build_tool_pending_content(&request, app, decoded);
        (
            title,
            content,
            ToolInspectKind::Pending,
            ToolInspectView::Request,
        )
    };

    app.open_tool_call_inspect(title, content, index, view, kind, decoded);
    let status = match kind {
        ToolInspectKind::Pending => {
            "Inspecting tool call (permission pending • Esc=Close)".to_string()
        }
        ToolInspectKind::Result => "Inspecting tool result (Esc=Close)".to_string(),
    };
    set_status_message(app, status, ctx);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolInspectCopyKind {
    Request,
    Response,
}

impl ToolInspectCopyKind {
    fn label(self) -> &'static str {
        match self {
            ToolInspectCopyKind::Request => "request",
            ToolInspectCopyKind::Response => "response",
        }
    }
}

struct ToolInspectCopyData {
    kind: ToolInspectCopyKind,
    content: String,
}

fn copy_tool_call_inspect_data(app: &mut App, ctx: AppActionContext) {
    let Some(data) = tool_call_inspect_copy_data(app) else {
        set_status_message(app, "No tool data to copy.".to_string(), ctx);
        return;
    };

    match crate::utils::clipboard::copy_to_clipboard(&data.content) {
        Ok(()) => set_status_message(
            app,
            format!("Copied {} to clipboard.", data.kind.label()),
            ctx,
        ),
        Err(_) => set_status_message(app, "Clipboard error.".to_string(), ctx),
    }
}

fn tool_call_inspect_copy_data(app: &App) -> Option<ToolInspectCopyData> {
    let state = app.inspect_state()?;
    let InspectMode::ToolCalls { index, view, .. } = state.mode else {
        return None;
    };

    let snapshot = tool_inspect_snapshot(app);
    if snapshot.total() == 0 {
        return None;
    }

    let (kind, content) = if index < snapshot.results.len() {
        let record = snapshot
            .results
            .get(index)
            .cloned()
            .or_else(|| snapshot.results.last().cloned())?;
        match view {
            ToolInspectView::Result => (
                ToolInspectCopyKind::Response,
                format_tool_payload_for_inspect(&record.content, state.decoded),
            ),
            ToolInspectView::Request => {
                let args_text = record
                    .raw_arguments
                    .as_deref()
                    .map(|raw| format_tool_arguments_for_inspect(raw, state.decoded))
                    .unwrap_or_default();
                (ToolInspectCopyKind::Request, args_text)
            }
        }
    } else {
        let pending_index = index.saturating_sub(snapshot.results.len());
        let request = snapshot
            .pending
            .get(pending_index)
            .cloned()
            .or_else(|| snapshot.pending.last().cloned())?;
        (
            ToolInspectCopyKind::Request,
            format_tool_arguments_for_inspect(&request.raw_arguments, state.decoded),
        )
    };

    if content.trim().is_empty() {
        return None;
    }

    Some(ToolInspectCopyData { kind, content })
}

fn build_tool_result_title(record: &ToolResultRecord, index: usize, total: usize) -> String {
    let position = format!("{}/{}", index + 1, total.max(1));
    let status = record.status.display();
    format!(
        "Tool call (completed, {position}) – {} ({status})",
        record.tool_name
    )
}

fn build_tool_result_content(record: &ToolResultRecord, decoded: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Tool: {}", record.tool_name));
    lines.push(format!("Status: {}", record.status.display()));
    if let Some(server) = record.server_name.as_ref() {
        if !server.trim().is_empty() {
            lines.push(format!("Server: {}", server));
        }
    }
    if let Some(tool_call_id) = record.tool_call_id.as_ref() {
        if !tool_call_id.trim().is_empty() {
            lines.push(format!("Tool call id: {}", tool_call_id));
        }
    }
    lines.push(String::new());
    lines.push(result_label(decoded));

    let payload = format_tool_payload_for_inspect(&record.content, decoded);
    if payload.trim().is_empty() {
        lines.push("  (empty)".to_string());
    } else {
        for line in payload.lines() {
            lines.push(format!("  {}", line));
        }
    }

    build_inspect_text(lines)
}

fn build_tool_request_title(record: &ToolResultRecord, index: usize, total: usize) -> String {
    let position = format!("{}/{}", index + 1, total.max(1));
    format!("Tool call (completed, {position}) – {}", record.tool_name)
}

fn build_tool_request_content(record: &ToolResultRecord, decoded: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Tool: {}", record.tool_name));

    let server_label = record
        .server_name
        .as_ref()
        .or(record.server_id.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(server) = server_label.as_ref() {
        lines.push(format!("Server: {}", server));
    }

    if let (Some(server_id), Some(server_label)) =
        (record.server_id.as_ref(), server_label.as_ref())
    {
        if server_id.trim() != server_label.trim() {
            lines.push(format!("Server ID: {}", server_id));
        }
    }

    if let Some(tool_call_id) = record.tool_call_id.as_ref() {
        if !tool_call_id.trim().is_empty() {
            lines.push(format!("Tool call id: {}", tool_call_id));
        }
    }

    lines.push(String::new());
    lines.push(arguments_label(decoded));

    let args_text = record
        .raw_arguments
        .as_deref()
        .map(|raw| format_tool_arguments_for_inspect(raw, decoded))
        .unwrap_or_default();
    if args_text.trim().is_empty() {
        if record.raw_arguments.is_some() {
            lines.push("  (none)".to_string());
        } else {
            lines.push("  (unavailable)".to_string());
        }
    } else {
        for line in args_text.lines() {
            lines.push(format!("  {}", line));
        }
    }

    build_inspect_text(lines)
}

fn build_tool_pending_title(
    request: &ToolCallRequest,
    index: usize,
    total: usize,
    app: &App,
) -> String {
    let position = format!("{}/{}", index + 1, total.max(1));
    let server_label = resolve_server_label(app, &request.server_id);
    format!(
        "Tool call (pending, {position}) – {} on {} (permission pending)",
        request.tool_name, server_label
    )
}

fn build_tool_pending_content(request: &ToolCallRequest, app: &App, decoded: bool) -> String {
    let mut lines = Vec::new();
    let server_label = resolve_server_label(app, &request.server_id);
    lines.push(format!("Tool: {}", request.tool_name));
    lines.push(format!("Server: {}", server_label));
    if request.server_id.trim() != server_label.trim() {
        lines.push(format!("Server ID: {}", request.server_id));
    }
    if let Some(tool_call_id) = request.tool_call_id.as_ref() {
        if !tool_call_id.trim().is_empty() {
            lines.push(format!("Tool call id: {}", tool_call_id));
        }
    }
    lines.push(String::new());
    lines.push(arguments_label(decoded));
    let args_text = format_tool_arguments_for_inspect(&request.raw_arguments, decoded);
    if args_text.trim().is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for line in args_text.lines() {
            lines.push(format!("  {}", line));
        }
    }
    build_inspect_text(lines)
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

fn result_label(decoded: bool) -> String {
    if decoded {
        "Result (decoded):".to_string()
    } else {
        "Result:".to_string()
    }
}

fn arguments_label(decoded: bool) -> String {
    if decoded {
        "Arguments (decoded):".to_string()
    } else {
        "Arguments:".to_string()
    }
}

const NESTED_JSON_MAX_DEPTH: usize = 4;

fn format_tool_payload_for_inspect(payload: &str, decoded: bool) -> String {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            let value = if decoded {
                let (decoded_value, _) = decode_nested_json_value(value, NESTED_JSON_MAX_DEPTH);
                decoded_value
            } else {
                value
            };
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string())
        }
        Err(_) => trimmed.to_string(),
    }
}

fn format_tool_arguments_for_inspect(raw_arguments: &str, decoded: bool) -> String {
    let trimmed = raw_arguments.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            let value = if decoded {
                let (decoded_value, _) = decode_nested_json_value(value, NESTED_JSON_MAX_DEPTH);
                decoded_value
            } else {
                value
            };
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string())
        }
        Err(_) => trimmed.to_string(),
    }
}

fn tool_call_inspect_has_nested_json(app: &App, index: usize, view: ToolInspectView) -> bool {
    let snapshot = tool_inspect_snapshot(app);
    if snapshot.total() == 0 {
        return false;
    }

    if index < snapshot.results.len() {
        let record = snapshot
            .results
            .get(index)
            .cloned()
            .or_else(|| snapshot.results.last().cloned());
        let Some(record) = record else {
            return false;
        };
        match view {
            ToolInspectView::Result => contains_nested_json(&record.content),
            ToolInspectView::Request => record
                .raw_arguments
                .as_deref()
                .map(contains_nested_json)
                .unwrap_or(false),
        }
    } else {
        let pending_index = index.saturating_sub(snapshot.results.len());
        let request = snapshot
            .pending
            .get(pending_index)
            .cloned()
            .or_else(|| snapshot.pending.last().cloned());
        request
            .as_ref()
            .map(|req| contains_nested_json(&req.raw_arguments))
            .unwrap_or(false)
    }
}

fn contains_nested_json(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return false;
    };
    detect_nested_json_value(&value, NESTED_JSON_MAX_DEPTH)
}

fn detect_nested_json_value(value: &serde_json::Value, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    match value {
        serde_json::Value::String(text) => {
            if !looks_like_json(text) {
                return false;
            }
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
                return false;
            };
            matches!(
                parsed,
                serde_json::Value::Array(_) | serde_json::Value::Object(_)
            )
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| detect_nested_json_value(item, depth.saturating_sub(1))),
        serde_json::Value::Object(map) => map
            .values()
            .any(|value| detect_nested_json_value(value, depth.saturating_sub(1))),
        _ => false,
    }
}

fn decode_nested_json_value(value: serde_json::Value, depth: usize) -> (serde_json::Value, bool) {
    if depth == 0 {
        return (value, false);
    }
    match value {
        serde_json::Value::String(text) => {
            if !looks_like_json(&text) {
                return (serde_json::Value::String(text), false);
            }
            let parsed = serde_json::from_str::<serde_json::Value>(text.trim());
            if let Ok(parsed) = parsed {
                if matches!(
                    parsed,
                    serde_json::Value::Array(_) | serde_json::Value::Object(_)
                ) {
                    let (decoded, _) = decode_nested_json_value(parsed, depth.saturating_sub(1));
                    return (decoded, true);
                }
            }
            (serde_json::Value::String(text), false)
        }
        serde_json::Value::Array(items) => {
            let mut decoded_any = false;
            let decoded_items = items
                .into_iter()
                .map(|item| {
                    let (decoded, did_decode) =
                        decode_nested_json_value(item, depth.saturating_sub(1));
                    decoded_any |= did_decode;
                    decoded
                })
                .collect();
            (serde_json::Value::Array(decoded_items), decoded_any)
        }
        serde_json::Value::Object(map) => {
            let mut decoded_any = false;
            let decoded_map = map
                .into_iter()
                .map(|(key, value)| {
                    let (decoded, did_decode) =
                        decode_nested_json_value(value, depth.saturating_sub(1));
                    decoded_any |= did_decode;
                    (key, decoded)
                })
                .collect();
            (serde_json::Value::Object(decoded_map), decoded_any)
        }
        other => (other, false),
    }
}

fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::session::ToolResultStatus;
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn process_command_submits_message() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let cmd = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "hello there".into(),
            },
            ctx,
        );

        assert!(matches!(cmd, Some(AppCommand::SpawnStream(_))));
    }

    #[test]
    fn process_command_opens_theme_picker() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let _ = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/theme".into(),
            },
            ctx,
        );

        assert!(app.picker_session().is_some());
    }

    #[test]
    fn help_command_focuses_transcript() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.ui.focus_input();

        let cmd = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/help".into(),
            },
            ctx,
        );

        assert!(cmd.is_none());
        assert!(app.ui.is_transcript_focused());
    }

    #[test]
    fn mcp_command_focuses_transcript() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.ui.focus_input();
        app.config
            .mcp_servers
            .push(crate::core::config::data::McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha MCP".to_string(),
                transport: None,
                base_url: Some("https://mcp.example.com".to_string()),
                command: None,
                args: None,
                env: None,
                enabled: Some(true),
                allowed_tools: None,
                protocol_version: None,
                yolo: None,
            });
        app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);

        let cmd = handle_input_action(
            &mut app,
            AppAction::ProcessCommand {
                input: "/mcp alpha".into(),
            },
            ctx,
        );

        assert!(matches!(cmd, Some(AppCommand::RefreshMcp { .. })));
        assert!(app.ui.is_transcript_focused());
    }

    #[test]
    fn tool_request_inspect_formats_arguments() {
        let record = ToolResultRecord {
            tool_name: "mcp_read_resource".to_string(),
            server_name: Some("Example Server".to_string()),
            server_id: Some("example".to_string()),
            status: ToolResultStatus::Success,
            content: "{\"ok\":true}".to_string(),
            tool_call_id: Some("tool-call-1".to_string()),
            raw_arguments: Some("{\"uri\":\"mcp://example/resource\"}".to_string()),
        };

        let content = build_tool_request_content(&record, false);
        assert!(content.contains("Arguments:"));
        assert!(content.contains("\"uri\": \"mcp://example/resource\""));
    }

    #[test]
    fn tool_request_inspect_handles_missing_arguments() {
        let record = ToolResultRecord {
            tool_name: "no_args".to_string(),
            server_name: None,
            server_id: None,
            status: ToolResultStatus::Success,
            content: "{}".to_string(),
            tool_call_id: None,
            raw_arguments: None,
        };

        let content = build_tool_request_content(&record, false);
        assert!(content.contains("(unavailable)"));
    }

    #[test]
    fn inspect_tool_calls_prefers_pending_prompt() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.tool_result_history.push(ToolResultRecord {
            tool_name: "completed_tool".to_string(),
            server_name: Some("Alpha MCP".to_string()),
            server_id: Some("alpha".to_string()),
            status: ToolResultStatus::Success,
            content: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
            raw_arguments: Some("{\"ok\":true}".to_string()),
        });
        app.session.pending_tool_queue.push_back(ToolCallRequest {
            server_id: "alpha".to_string(),
            tool_name: "pending_tool".to_string(),
            arguments: None,
            raw_arguments: "{\"q\":\"pending\"}".to_string(),
            tool_call_id: Some("call-2".to_string()),
        });
        app.ui
            .start_tool_prompt(crate::core::app::ui_state::ToolPromptRequest {
                server_id: "alpha".to_string(),
                server_name: "Alpha MCP".to_string(),
                tool_name: "pending_tool".to_string(),
                display_name: None,
                args_summary: "q=pending".to_string(),
                raw_arguments: "{\"q\":\"pending\"}".to_string(),
                batch_index: 0,
            });

        let cmd = handle_input_action(&mut app, AppAction::InspectToolResults, ctx);

        assert!(cmd.is_none());
        let inspect = app.inspect_state().expect("expected inspect state");
        assert!(inspect.title.contains("pending"));
        assert!(matches!(
            inspect.mode,
            InspectMode::ToolCalls {
                kind: ToolInspectKind::Pending,
                ..
            }
        ));
    }

    #[test]
    fn inspect_tool_calls_labels_completed() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        app.session.tool_result_history.push(ToolResultRecord {
            tool_name: "completed_tool".to_string(),
            server_name: Some("Alpha MCP".to_string()),
            server_id: Some("alpha".to_string()),
            status: ToolResultStatus::Success,
            content: "{}".to_string(),
            tool_call_id: Some("call-1".to_string()),
            raw_arguments: Some("{\"ok\":true}".to_string()),
        });

        let cmd = handle_input_action(&mut app, AppAction::InspectToolResults, ctx);

        assert!(cmd.is_none());
        let inspect = app.inspect_state().expect("expected inspect state");
        assert!(inspect.title.contains("completed"));
        assert!(matches!(
            inspect.mode,
            InspectMode::ToolCalls {
                kind: ToolInspectKind::Result,
                ..
            }
        ));
    }

    #[test]
    fn tool_call_inspect_copy_data_uses_request_payload() {
        let mut app = create_test_app();
        app.session.tool_result_history.push(ToolResultRecord {
            tool_name: "mcp_read_resource".to_string(),
            server_name: Some("Alpha MCP".to_string()),
            server_id: Some("alpha".to_string()),
            status: ToolResultStatus::Success,
            content: "{\"ok\":true}".to_string(),
            tool_call_id: Some("call-1".to_string()),
            raw_arguments: Some("{\"uri\":\"mcp://alpha/doc\"}".to_string()),
        });
        app.open_tool_call_inspect(
            "Inspect".to_string(),
            "Body".to_string(),
            0,
            ToolInspectView::Request,
            ToolInspectKind::Result,
            false,
        );

        let data = tool_call_inspect_copy_data(&app).expect("copy data");
        assert_eq!(data.kind, ToolInspectCopyKind::Request);
        assert!(data.content.contains("\"uri\": \"mcp://alpha/doc\""));
    }

    #[test]
    fn tool_call_inspect_copy_data_uses_response_payload() {
        let mut app = create_test_app();
        app.session.tool_result_history.push(ToolResultRecord {
            tool_name: "mcp_read_resource".to_string(),
            server_name: Some("Alpha MCP".to_string()),
            server_id: Some("alpha".to_string()),
            status: ToolResultStatus::Success,
            content: "{\"ok\":true}".to_string(),
            tool_call_id: Some("call-1".to_string()),
            raw_arguments: Some("{\"uri\":\"mcp://alpha/doc\"}".to_string()),
        });
        app.open_tool_call_inspect(
            "Inspect".to_string(),
            "Body".to_string(),
            0,
            ToolInspectView::Result,
            ToolInspectKind::Result,
            false,
        );

        let data = tool_call_inspect_copy_data(&app).expect("copy data");
        assert_eq!(data.kind, ToolInspectCopyKind::Response);
        assert!(data.content.contains("\"ok\": true"));
    }

    #[test]
    fn tool_call_inspect_copy_data_decodes_nested_json() {
        let mut app = create_test_app();
        app.session.tool_result_history.push(ToolResultRecord {
            tool_name: "mcp_list_documents".to_string(),
            server_name: Some("Alpha MCP".to_string()),
            server_id: Some("alpha".to_string()),
            status: ToolResultStatus::Success,
            content: "{\"content\":[{\"text\":\"{\\\"ok\\\":true}\",\"type\":\"text\"}]}"
                .to_string(),
            tool_call_id: Some("call-1".to_string()),
            raw_arguments: Some("{\"q\":\"nested\"}".to_string()),
        });
        app.open_tool_call_inspect(
            "Inspect".to_string(),
            "Body".to_string(),
            0,
            ToolInspectView::Result,
            ToolInspectKind::Result,
            true,
        );

        let data = tool_call_inspect_copy_data(&app).expect("copy data");
        assert!(data.content.contains("\"ok\": true"));
        assert!(!data.content.contains("\\\"ok\\\""));
    }

    #[test]
    fn nested_json_detection_matches_payload() {
        let payload = "{\"content\":[{\"text\":\"{\\\"ok\\\":true}\",\"type\":\"text\"}]}";
        assert!(contains_nested_json(payload));
        assert!(!contains_nested_json(
            "{\"content\":[{\"text\":\"plain\"}]}"
        ));
    }
}
