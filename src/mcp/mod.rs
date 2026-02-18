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

/// Internal tool name used by chat flows to trigger MCP resource reads.
///
/// This name is consumed by the tool-dispatch layer and should remain stable
/// so saved transcripts and prompt templates continue to resolve correctly.
pub const MCP_READ_RESOURCE_TOOL: &str = "mcp_read_resource";

/// Internal tool name used to request a paginated MCP resource listing.
///
/// Chabeau translates this tool invocation into `resources/list` calls on the
/// selected server transport.
pub const MCP_LIST_RESOURCES_TOOL: &str = "mcp_list_resources";

/// Canonical MCP method used when a server requests model-side sampling.
///
/// Both stdio and streamable HTTP transports may emit this request during
/// long-running operations.
pub const MCP_SAMPLING_TOOL: &str = "sampling/createMessage";

/// Chabeau-specific utility tool for recalling prior session context.
///
/// This is not part of the core MCP spec and is intentionally namespaced to
/// avoid collisions with upstream server-defined tools.
pub const MCP_INSTANT_RECALL_TOOL: &str = "chabeau_instant_recall";

/// Synthetic server identifier reserved for in-process session memory.
///
/// Keep this value unique from user-configured MCP server ids.
pub const MCP_SESSION_MEMORY_SERVER_ID: &str = "session";
