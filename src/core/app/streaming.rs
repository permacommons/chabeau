use std::time::Instant;

use super::App;
use crate::api::{ChatMessage, ChatToolDefinition, ChatToolFunction};
use crate::core::chat_stream::StreamParams;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const MCP_RESOURCES_MARKER: &str = "MCP resources (by server id):";

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

        for server in self.mcp.servers() {
            if !server.config.is_enabled() {
                continue;
            }
            if let Some(resources) = &server.cached_resources {
                if !resources.resources.is_empty() {
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
                        "Read an MCP resource by server_id and uri from the MCP resources list."
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
            let Some(list) = &server.cached_resources else {
                continue;
            };
            if list.resources.is_empty() {
                continue;
            }

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

        if lines.is_empty() {
            None
        } else {
            let mut output = String::from(MCP_RESOURCES_MARKER);
            output.push('\n');
            output.push_str(&lines.join("\n"));
            Some(output)
        }
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
