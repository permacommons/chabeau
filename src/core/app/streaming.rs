use std::collections::HashSet;
use std::time::Instant;

use super::App;
use crate::api::{ChatMessage, ChatToolDefinition, ChatToolFunction};
use crate::core::chat_stream::StreamParams;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const MCP_RESOURCES_MARKER: &str = "MCP resources and templates (by server id):";
const MCP_PAYLOAD_NOTE_MARKER: &str = "MCP tool payload retention note:";

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

    pub fn begin_mcp_operation(&mut self) -> CancellationToken {
        let token = CancellationToken::new();
        self.session.stream_cancel_token = Some(token.clone());
        self.ui.stream_interrupted = false;
        self.ui
            .begin_activity(crate::core::app::ui_state::ActivityKind::McpOperation);
        token
    }

    pub fn end_mcp_operation_if_active(&mut self) {
        if matches!(
            self.ui.activity_kind(),
            Some(crate::core::app::ui_state::ActivityKind::McpOperation)
        ) {
            self.ui
                .end_activity(crate::core::app::ui_state::ActivityKind::McpOperation);
        }
        self.session.stream_cancel_token = None;
    }

    pub fn has_interruptible_activity(&self) -> bool {
        self.ui.is_streaming || self.session.stream_cancel_token.is_some()
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
        let mut can_list_resources = false;
        let mut any_enabled = false;

        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            any_enabled = true;
            if server
                .server_details
                .as_ref()
                .map(|details| details.capabilities.resources.is_some())
                .unwrap_or(true)
            {
                can_list_resources = true;
            }
            if let Some(resources) = &server.cached_resources {
                if !resources.resources.is_empty() {
                    has_resources = true;
                }
                if resources.next_cursor.is_some() {
                    has_resources = true;
                    can_list_resources = true;
                }
            }
            if let Some(templates) = &server.cached_resource_templates {
                if !templates.resource_templates.is_empty() {
                    has_resources = true;
                }
                if templates.next_cursor.is_some() {
                    has_resources = true;
                    can_list_resources = true;
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

        if can_list_resources {
            tools.push(ChatToolDefinition {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: crate::mcp::MCP_LIST_RESOURCES_TOOL.to_string(),
                    description: Some(
                        "List MCP resources or templates for a server. Use the cursor from the MCP resources list to page results. Set kind to \"resources\" (default) or \"templates\"."
                            .to_string(),
                    ),
                    parameters: json!({
                        "type": "object",
                        "required": ["server_id"],
                        "properties": {
                            "server_id": {
                                "type": "string",
                                "description": "MCP server id from the resources list."
                            },
                            "cursor": {
                                "type": "string",
                                "description": "Opaque pagination cursor from a previous response."
                            },
                            "kind": {
                                "type": "string",
                                "description": "List type: \"resources\" (default) or \"templates\"."
                            }
                        }
                    }),
                },
            });
        }

        if any_enabled {
            tools.push(ChatToolDefinition {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: crate::mcp::MCP_INSTANT_RECALL_TOOL.to_string(),
                    description: Some(
                        "Recall the full payload from a prior MCP tool call using a tool_call_id."
                            .to_string(),
                    ),
                    parameters: json!({
                        "type": "object",
                        "required": ["tool_call_id"],
                        "properties": {
                            "tool_call_id": {
                                "type": "string",
                                "description": "Tool call id to recall."
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

                if let Some(cursor) = list.next_cursor.as_deref() {
                    lines.push(format!(
                        "- {}: more resources available (cursor: {}). Use {} with kind=\"resources\".",
                        server.config.id,
                        cursor,
                        crate::mcp::MCP_LIST_RESOURCES_TOOL
                    ));
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

                if let Some(cursor) = list.next_cursor.as_deref() {
                    lines.push(format!(
                        "- {}: more templates available (cursor: {}). Use {} with kind=\"templates\".",
                        server.config.id,
                        cursor,
                        crate::mcp::MCP_LIST_RESOURCES_TOOL
                    ));
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
        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            let policy = match server.config.tool_payloads() {
                crate::core::config::data::McpToolPayloadRetention::Turn => {
                    "default (turn)".to_string()
                }
                crate::core::config::data::McpToolPayloadRetention::Window => {
                    let window = server.config.tool_payload_window();
                    format!("window({})", window)
                }
                crate::core::config::data::McpToolPayloadRetention::All => "all".to_string(),
            };
            entries.push(format!("{}: {}", server.config.id, policy));
        }

        if entries.is_empty() {
            return None;
        }

        let mut note = format!(
            "{MCP_PAYLOAD_NOTE_MARKER} Default MCP tool output policy: only the current turn's raw outputs stay in chat context to save tokens; older outputs are summarized. Full payloads remain available via chabeau_instant_recall using call_id, which reinserts earlier outputs from system memory (NO software limit on retention)."
        );
        note.push_str("\nConfigure per server in config.toml with tool_payloads: turn (current turn only), window (last N raw outputs; set tool_payload_window), all (keep all raw outputs in context; token-expensive).");
        note.push_str("\nMCP tool payload policy by server: ");
        note.push_str(&entries.join(" | "));
        Some(note)
    }

    fn inject_tool_payload_history(&self, api_messages: &mut Vec<ChatMessage>) {
        if self.session.tool_payload_history.is_empty() {
            return;
        }

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
            history_messages.push(entry.tool_message.clone());
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
                            " (call_id={id}; use chabeau_instant_recall for full output)"
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
        return;
    }

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

fn inject_mcp_resources(api_messages: &mut Vec<ChatMessage>, resources_text: &str) {
    if resources_text.trim().is_empty() {
        return;
    }

    inject_or_replace_system_block(api_messages, MCP_RESOURCES_MARKER, resources_text);
}

fn inject_mcp_payload_note(api_messages: &mut Vec<ChatMessage>, note: &str) {
    if note.trim().is_empty() {
        return;
    }

    inject_or_replace_system_block(api_messages, MCP_PAYLOAD_NOTE_MARKER, note);
}

pub(crate) fn abbreviate_args(raw: &str) -> String {
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

fn inject_or_replace_system_block(api_messages: &mut Vec<ChatMessage>, marker: &str, block: &str) {
    if let Some(message) = api_messages.iter_mut().find(|msg| msg.role == "system") {
        if let Some(start) = message.content.find(marker) {
            let suffix_start = message.content[start..]
                .find("\n\n")
                .map(|offset| start + offset)
                .unwrap_or_else(|| message.content.len());
            let mut updated = String::with_capacity(
                message.content.len().saturating_sub(suffix_start - start) + block.len(),
            );
            updated.push_str(&message.content[..start]);
            updated.push_str(block);
            if suffix_start < message.content.len() {
                updated.push_str(&message.content[suffix_start..]);
            }
            message.content = updated;
            return;
        }

        if !message.content.trim().is_empty() {
            message.content.push_str("\n\n");
        }
        message.content.push_str(block);
        return;
    }

    api_messages.insert(
        0,
        ChatMessage {
            role: "system".to_string(),
            content: block.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
    );
}
