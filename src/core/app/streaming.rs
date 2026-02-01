use std::collections::HashSet;
use std::time::Instant;

use super::App;
use crate::api::{ChatMessage, ChatToolDefinition, ChatToolFunction};
use crate::core::chat_stream::StreamParams;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const MCP_RESOURCES_MARKER: &str = "MCP resources and templates (by server id):";
const MCP_PAYLOAD_NOTE_MARKER: &str = "MCP tool payload retention note:";
const MCP_SESSION_MEMORY_MARKER: &str = "SESSION MEMORY (pinned tool outputs; oldest first):";
const MCP_TOOL_LEDGER_MARKER: &str = "SESSION TOOL LEDGER (call_id • tool • args • status):";
const MCP_SESSION_MEMORY_HINT_MARKER: &str = "SESSION MEMORY HINT:";
const TOOL_RESULT_PINNED_PLACEHOLDER: &str =
    "TOOL RESULT PINNED (added to session memory); unpin to apply tool call retention policy.";

impl App {
    pub fn is_current_stream(&self, stream_id: u64) -> bool {
        self.session.current_stream_id == stream_id
    }

    pub fn end_streaming(&mut self) {
        self.ui.end_streaming();
    }

    pub fn cancel_current_stream(&mut self) {
        self.conversation().cancel_current_stream();
    }

    pub fn enable_auto_scroll(&mut self) {
        self.ui.auto_scroll = true;
    }

    pub fn build_stream_params(
        &mut self,
        api_messages: Vec<ChatMessage>,
        cancel_token: CancellationToken,
        stream_id: u64,
    ) -> StreamParams {
        self.session.last_stream_api_messages_base = Some(api_messages.clone());
        let tools = if self.session.mcp_tools_unsupported {
            None
        } else {
            self.collect_mcp_tools()
        };
        let mut api_messages = api_messages;
        if tools.is_some() {
            inject_mcp_preamble(&mut api_messages);
            if let Some(resources_text) = self.collect_mcp_resource_list() {
                inject_mcp_resources(&mut api_messages, &resources_text);
            }
            self.session.mcp_tools_enabled = true;
        } else {
            self.session.mcp_tools_enabled = false;
        }
        if let Some(note) = self.build_mcp_payload_note() {
            inject_mcp_payload_note(&mut api_messages, &note);
        }
        if let Some(ledger) = self.build_tool_ledger() {
            inject_mcp_tool_ledger(&mut api_messages, &ledger);
        }
        if let Some(hint) = self.build_session_memory_hint() {
            inject_mcp_session_memory_hint(&mut api_messages, &hint);
        }
        if let Some(session_memory) = self.build_session_memory_block() {
            inject_mcp_session_memory(&mut api_messages, &session_memory);
        }
        self.inject_tool_payload_history(&mut api_messages);
        self.inject_tool_summary_history(&mut api_messages);
        self.session.last_stream_api_messages = Some(api_messages.clone());
        StreamParams {
            client: self.session.client.clone(),
            base_url: self.session.base_url.clone(),
            api_key: self.session.api_key.clone(),
            provider_name: self.session.provider_name.clone(),
            model: self.session.model.clone(),
            api_messages,
            tools,
            cancel_token,
            stream_id,
        }
    }

    pub fn last_retry_time(&self) -> Instant {
        self.session.last_retry_time
    }

    pub fn update_last_retry_time(&mut self, instant: Instant) {
        self.session.last_retry_time = instant;
    }

    fn collect_mcp_tools(&self) -> Option<Vec<ChatToolDefinition>> {
        let mut tools = Vec::new();
        let mut has_resources = false;
        let mut any_enabled = false;
        let mut allow_session_memory = false;

        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            any_enabled = true;
            if matches!(
                server.config.tool_payloads(),
                crate::core::config::data::McpToolPayloadRetention::Turn
                    | crate::core::config::data::McpToolPayloadRetention::Window
            ) {
                allow_session_memory = true;
            }
            if let Some(resources) = &server.cached_resources {
                if !resources.resources.is_empty() {
                    has_resources = true;
                }
            }
            if let Some(templates) = &server.cached_resource_templates {
                if !templates.resource_templates.is_empty() {
                    has_resources = true;
                }
            }
            let Some(list) = &server.cached_tools else {
                continue;
            };

            let allowed_tools = server.allowed_tools();
            let server_label = if server.config.display_name.is_empty() {
                server.config.id.as_str()
            } else {
                server.config.display_name.as_str()
            };

            for tool in &list.tools {
                if let Some(allowed) = allowed_tools {
                    if !allowed
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&tool.name))
                    {
                        continue;
                    }
                }

                let Ok(parameters) = serde_json::to_value(&tool.input_schema) else {
                    continue;
                };

                let description = tool
                    .description
                    .clone()
                    .or_else(|| tool.title.clone())
                    .or_else(|| tool.annotations.as_ref().and_then(|ann| ann.title.clone()))
                    .map(|desc| format!("[MCP: {}] {}", server_label, desc))
                    .or_else(|| Some(format!("[MCP: {}] MCP tool", server_label)));

                tools.push(ChatToolDefinition {
                    kind: "function".to_string(),
                    function: ChatToolFunction {
                        name: tool.name.clone(),
                        description,
                        parameters,
                    },
                });
            }
        }

        if has_resources {
            tools.push(ChatToolDefinition {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: crate::mcp::MCP_READ_RESOURCE_TOOL.to_string(),
                    description: Some(
                        "Read an MCP resource by server_id and uri from the MCP resources list (including templates)."
                            .to_string(),
                    ),
                    parameters: json!({
                        "type": "object",
                        "required": ["server_id", "uri"],
                        "properties": {
                            "server_id": {
                                "type": "string",
                                "description": "MCP server id from the resources list."
                            },
                            "uri": {
                                "type": "string",
                                "description": "Resource URI to read."
                            }
                        }
                    }),
                },
            });
        }

        if any_enabled && allow_session_memory {
            tools.push(ChatToolDefinition {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: crate::mcp::MCP_SESSION_MEMORY_PIN_TOOL.to_string(),
                    description: Some(
                        "Pin a tool result into session memory (system prompt) using a tool_call_id."
                            .to_string(),
                    ),
                    parameters: json!({
                        "type": "object",
                        "required": ["tool_call_id"],
                        "properties": {
                            "tool_call_id": {
                                "type": "string",
                                "description": "Tool call id to pin into session memory."
                            },
                            "note": {
                                "type": "string",
                                "description": "Optional note to store with the pin."
                            }
                        }
                    }),
                },
            });
            tools.push(ChatToolDefinition {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: crate::mcp::MCP_SESSION_MEMORY_UNPIN_TOOL.to_string(),
                    description: Some(
                        "Remove a pinned tool result from session memory using a tool_call_id."
                            .to_string(),
                    ),
                    parameters: json!({
                        "type": "object",
                        "required": ["tool_call_id"],
                        "properties": {
                            "tool_call_id": {
                                "type": "string",
                                "description": "Pinned tool call id to remove."
                            }
                        }
                    }),
                },
            });
        }

        if tools.is_empty() {
            None
        } else {
            Some(tools)
        }
    }

    fn collect_mcp_resource_list(&self) -> Option<String> {
        let mut lines = Vec::new();

        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            if let Some(list) = &server.cached_resources {
                for resource in &list.resources {
                    let mut line = format!("- {}: {}", server.config.id, resource.uri);
                    if let Some(title) = resource.title.as_ref().or(resource.description.as_ref()) {
                        if !title.trim().is_empty() {
                            line.push_str(&format!(" - {}", title.trim()));
                        }
                    }
                    lines.push(line);
                }
            }

            if let Some(list) = &server.cached_resource_templates {
                for template in &list.resource_templates {
                    let mut line = format!(
                        "- {}: template {} ({})",
                        server.config.id, template.name, template.uri_template
                    );
                    if let Some(title) = template.title.as_ref().or(template.description.as_ref()) {
                        if !title.trim().is_empty() {
                            line.push_str(&format!(" - {}", title.trim()));
                        }
                    }
                    lines.push(line);
                }
            }
        }

        if lines.is_empty() {
            None
        } else {
            let mut output = String::from(MCP_RESOURCES_MARKER);
            output.push('\n');
            output.push_str(&lines.join("\n"));
            Some(output)
        }
    }

    fn build_mcp_payload_note(&self) -> Option<String> {
        if self.session.mcp_disabled {
            return None;
        }

        let mut entries = Vec::new();
        let mut all_default = true;
        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            let policy = match server.config.tool_payloads() {
                crate::core::config::data::McpToolPayloadRetention::Turn => {
                    all_default = false;
                    "turn".to_string()
                }
                crate::core::config::data::McpToolPayloadRetention::Window => {
                    let window = server.config.tool_payload_window();
                    if window != crate::core::config::data::DEFAULT_MCP_TOOL_PAYLOAD_WINDOW {
                        all_default = false;
                    }
                    format!("window({})", window)
                }
                crate::core::config::data::McpToolPayloadRetention::All => {
                    all_default = false;
                    "all".to_string()
                }
            };
            entries.push(format!("{}={}", server.config.id, policy));
        }

        if entries.is_empty() {
            return None;
        }

        let mut note = if all_default {
            format!(
                "{MCP_PAYLOAD_NOTE_MARKER} Tool call retention policy: window({}) for all MCP servers (last {} tool payloads per server kept; older outputs summarized).",
                crate::core::config::data::DEFAULT_MCP_TOOL_PAYLOAD_WINDOW,
                crate::core::config::data::DEFAULT_MCP_TOOL_PAYLOAD_WINDOW
            )
        } else {
            format!(
                "{MCP_PAYLOAD_NOTE_MARKER} Tool call retention policy: {} (window(n) keeps the last n tool payloads per server; older outputs summarized).",
                entries.join(", ")
            )
        };
        note.push_str(" Re-run tools if details are missing.");
        Some(note)
    }

    fn build_tool_ledger(&self) -> Option<String> {
        if self.session.mcp_disabled || self.session.tool_result_history.is_empty() {
            return None;
        }

        let mut needs_ledger = false;
        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            if matches!(
                server.config.tool_payloads(),
                crate::core::config::data::McpToolPayloadRetention::Turn
                    | crate::core::config::data::McpToolPayloadRetention::Window
            ) {
                needs_ledger = true;
                break;
            }
        }
        if !needs_ledger {
            return None;
        }

        let pinned_ids: HashSet<_> = self
            .session
            .pinned_tool_payloads
            .iter()
            .map(|entry| entry.tool_call_id.clone())
            .collect();

        let mut lines = Vec::new();
        lines.push(MCP_TOOL_LEDGER_MARKER.to_string());
        for (idx, record) in self.session.tool_result_history.iter().enumerate() {
            let call_id = record
                .tool_call_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("unknown");
            let server = record
                .server_name
                .as_ref()
                .or(record.server_id.as_ref())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty());
            let mut tool_label = record.tool_name.clone();
            if let Some(server) = server {
                tool_label.push('@');
                tool_label.push_str(server);
            }
            let args = match record.raw_arguments.as_deref() {
                Some(raw) if raw.trim().is_empty() => "(none)".to_string(),
                Some(raw) => abbreviate_args(raw),
                None => "(unavailable)".to_string(),
            };
            let status = record.status.label();
            let mut line = format!(
                "{}) call_id={} • {} • args: {} • {}",
                idx + 1,
                call_id,
                tool_label,
                args,
                status
            );
            if pinned_ids.contains(call_id) {
                line.push_str(" [PINNED, PAYLOAD BELOW]");
            }
            lines.push(line);
        }

        Some(lines.join("\n"))
    }

    fn build_session_memory_hint(&self) -> Option<String> {
        if self.session.mcp_disabled {
            return None;
        }

        let mut pinning_enabled = false;
        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            if matches!(
                server.config.tool_payloads(),
                crate::core::config::data::McpToolPayloadRetention::Turn
                    | crate::core::config::data::McpToolPayloadRetention::Window
            ) {
                pinning_enabled = true;
                break;
            }
        }
        if !pinning_enabled {
            return None;
        }

        Some(format!(
            "{MCP_SESSION_MEMORY_HINT_MARKER} You can pin tool outputs to session memory even after they are no longer visible in the transcript. Use chabeau_pin_to_session_memory with a call_id from the session tool ledger."
        ))
    }

    fn build_session_memory_block(&self) -> Option<String> {
        if self.session.pinned_tool_payloads.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push(MCP_SESSION_MEMORY_MARKER.to_string());
        for (idx, entry) in self.session.pinned_tool_payloads.iter().enumerate() {
            let mut header = format!("{}) {}", idx + 1, entry.tool_name);
            if let Some(server) = entry
                .server_name
                .as_ref()
                .or(entry.server_id.as_ref())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                header.push_str(&format!(" on {}", server));
            }
            header.push_str(&format!(" (call_id={})", entry.tool_call_id));
            if let Some(note) = entry.note.as_ref().map(|value| value.trim()) {
                if !note.is_empty() {
                    header.push_str(&format!(" | note: {}", note));
                }
            }
            lines.push(header);
            lines.push(entry.content.clone());
        }

        Some(lines.join("\n"))
    }

    fn inject_tool_payload_history(&self, api_messages: &mut Vec<ChatMessage>) {
        if self.session.tool_payload_history.is_empty() {
            return;
        }

        let pinned_ids: HashSet<_> = self
            .session
            .pinned_tool_payloads
            .iter()
            .map(|entry| entry.tool_call_id.clone())
            .collect();

        let mut existing = HashSet::new();
        for message in api_messages.iter() {
            if message.role == "tool" {
                if let Some(id) = message.tool_call_id.as_ref() {
                    existing.insert(id.clone());
                }
            }
        }

        let mut history_messages = Vec::new();
        for entry in &self.session.tool_payload_history {
            if let Some(id) = entry.tool_call_id.as_ref() {
                if existing.contains(id) {
                    continue;
                }
            }
            history_messages.push(entry.assistant_message.clone());
            if entry
                .tool_call_id
                .as_ref()
                .is_some_and(|id| pinned_ids.contains(id))
            {
                let mut tool_message = entry.tool_message.clone();
                tool_message.content = TOOL_RESULT_PINNED_PLACEHOLDER.to_string();
                history_messages.push(tool_message);
            } else {
                history_messages.push(entry.tool_message.clone());
            }
        }

        if history_messages.is_empty() {
            return;
        }

        let insert_pos = api_messages
            .iter()
            .rposition(|msg| msg.role == "user")
            .unwrap_or(api_messages.len());
        api_messages.splice(insert_pos..insert_pos, history_messages);
    }

    fn inject_tool_summary_history(&self, api_messages: &mut Vec<ChatMessage>) {
        if self.session.tool_result_history.is_empty() {
            return;
        }

        let pinned_ids: HashSet<_> = self
            .session
            .pinned_tool_payloads
            .iter()
            .map(|entry| entry.tool_call_id.clone())
            .collect();

        let mut raw_ids = HashSet::new();
        for entry in &self.session.tool_payload_history {
            if let Some(id) = entry.tool_call_id.as_ref() {
                raw_ids.insert(id.clone());
            }
        }
        for message in &self.session.tool_results {
            if let Some(id) = message.tool_call_id.as_ref() {
                raw_ids.insert(id.clone());
            }
        }

        let mut summaries = Vec::new();
        for record in &self.session.tool_result_history {
            if let Some(id) = record.tool_call_id.as_ref() {
                if raw_ids.contains(id) {
                    continue;
                }
                if pinned_ids.contains(id) {
                    summaries.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: TOOL_RESULT_PINNED_PLACEHOLDER.to_string(),
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    continue;
                }
            }
            summaries.push(ChatMessage {
                role: "assistant".to_string(),
                content: {
                    let mut summary = format!(
                        "TOOL SUMMARY (system-added per MCP payload policy): {}",
                        record.summary
                    );
                    if let Some(id) = record
                        .tool_call_id
                        .as_ref()
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                    {
                        summary.push_str(&format!(
                            " (call_id={id}; pin to store full output in session memory)"
                        ));
                    }
                    summary
                },
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }

        if summaries.is_empty() {
            return;
        }

        let insert_pos = api_messages
            .iter()
            .rposition(|msg| msg.role == "user")
            .unwrap_or(api_messages.len());
        api_messages.splice(insert_pos..insert_pos, summaries);
    }
}

fn inject_mcp_preamble(api_messages: &mut Vec<ChatMessage>) {
    let preamble = crate::core::builtin_mcp::builtin_mcp_preamble().trim();
    if preamble.is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(preamble) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(preamble);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: preamble.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn inject_mcp_resources(api_messages: &mut Vec<ChatMessage>, resources_text: &str) {
    if resources_text.trim().is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(MCP_RESOURCES_MARKER) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(resources_text);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: resources_text.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn inject_mcp_payload_note(api_messages: &mut Vec<ChatMessage>, note: &str) {
    if note.trim().is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(MCP_PAYLOAD_NOTE_MARKER) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(note);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: note.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn inject_mcp_tool_ledger(api_messages: &mut Vec<ChatMessage>, ledger: &str) {
    if ledger.trim().is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(MCP_TOOL_LEDGER_MARKER) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(ledger);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: ledger.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn inject_mcp_session_memory_hint(api_messages: &mut Vec<ChatMessage>, hint: &str) {
    if hint.trim().is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(MCP_SESSION_MEMORY_HINT_MARKER) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(hint);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: hint.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn inject_mcp_session_memory(api_messages: &mut Vec<ChatMessage>, memory_text: &str) {
    if memory_text.trim().is_empty() {
        return;
    }

    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if !message.content.contains(MCP_SESSION_MEMORY_MARKER) {
            if !message.content.trim().is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(memory_text);
        }
    } else {
        api_messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: memory_text.to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        );
    }
}

fn abbreviate_args(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "(none)".to_string();
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
