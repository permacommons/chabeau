//! Chabeau is a terminal-first chatbot client for working with remote LLM APIs.
//!
//! The crate is organized around a small set of collaborating layers:
//! - [`core`] owns runtime state, provider/model selection, presets, persona
//!   handling, and streaming orchestration.
//! - [`ui`] renders the terminal interface and runs the interactive event loop
//!   that drives user input and display updates.
//! - [`commands`] implements slash-command parsing and command execution used
//!   by the chat loop.
//! - [`mcp`] provides Model Context Protocol integration, including transport,
//!   tool registration, and permission surfaces.
//! - [`api`] defines chat/model payloads used by API clients and provider code.
//!
//! Runtime entrypoints live in the binary crate (`src/main.rs`) and route
//! through [`crate::cli::main`], which initializes and dispatches into
//! [`core::app`] and [`ui::chat_loop`] for interactive sessions.

pub mod api;
pub mod auth;
pub mod character;
pub mod cli;
pub mod commands;
pub mod core;
pub mod mcp;
pub mod ui;
pub mod utils;
