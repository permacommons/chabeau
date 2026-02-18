//! Model Context Protocol (MCP) integration surfaces.
//!
//! This module encapsulates client-side MCP connectivity, server registry
//! coordination, event ingestion, and policy checks for tool access.
//!
//! Key submodules include:
//! - [`client`]: MCP client orchestration and request execution.
//! - [`transport`]: transport/session wiring for MCP traffic.
//! - [`registry`]: available server/tool metadata management.
//! - [`events`] and [`permissions`]: runtime event propagation and permission
//!   decisions consumed by chat flows.
//!
//! Ownership boundary: MCP protocol concerns live here; higher-level flow
//! control remains in [`crate::core::chat_stream`] and interaction stays in
//! [`crate::ui::chat_loop`].

pub mod client;
pub mod events;
pub mod permissions;
pub mod registry;
pub mod transport;

pub const MCP_READ_RESOURCE_TOOL: &str = "mcp_read_resource";
pub const MCP_LIST_RESOURCES_TOOL: &str = "mcp_list_resources";
pub const MCP_SAMPLING_TOOL: &str = "sampling/createMessage";
pub const MCP_INSTANT_RECALL_TOOL: &str = "chabeau_instant_recall";
pub const MCP_SESSION_MEMORY_SERVER_ID: &str = "session";
