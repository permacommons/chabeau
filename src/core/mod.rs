//! Core runtime services and domain state for Chabeau.
//!
//! This module owns configuration, provider and preset resolution, persona
//! behavior, credential surfaces, and stream lifecycle management.
//!
//! Key submodules include:
//! - [`app`]: startup/shutdown orchestration consumed by the binary entrypoint.
//! - [`chat_stream`]: streaming chat execution and incremental response flow,
//!   coordinated with [`crate::ui::chat_loop`] and [`crate::commands`].
//! - [`config`], [`providers`], and [`preset`]: model/provider settings and
//!   runtime defaults.
//! - [`mcp_auth`] and [`mcp_sampling`]: MCP-specific auth and sampling bridges.
//! - [`text_wrapping`] and [`message`]: shared message/text shaping utilities
//!   used by both core flows and UI rendering.
//!
//! Ownership boundary: this layer contains application logic and state
//! transitions, while [`crate::ui`] handles presentation and interaction.

pub mod app;
pub mod builtin_mcp;
pub mod builtin_oauth;
pub mod builtin_presets;
pub mod builtin_providers;
pub mod chat_stream;
pub mod config;
pub mod keyring;
pub mod mcp_auth;
pub mod mcp_sampling;
pub mod message;
pub mod oauth;
pub mod persona;
#[cfg(test)]
pub mod persona_integration_tests;
pub mod preset;
pub mod providers;
mod shared_selection;
pub mod text_wrapping;
