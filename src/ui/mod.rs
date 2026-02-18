//! Terminal UI layer for interactive chat sessions.
//!
//! The UI module owns rendering, layout, keyboard handling, and loop control
//! for the text user interface.
//!
//! Key submodules include:
//! - [`chat_loop`]: the main interaction loop that dispatches user input to
//!   [`crate::commands`] and coordinates streaming via [`crate::core::chat_stream`].
//! - [`renderer`], [`layout`], and [`span`]: view composition and frame output.
//! - [`theme`], [`appearance`], and [`builtin_themes`]: color/style policy.
//! - [`picker`] and [`help`]: selection and discoverability UI affordances.
//!
//! Ownership boundary: this layer presents and captures interaction state, while
//! [`crate::core`] owns domain logic and backend coordination.

pub mod appearance;
pub mod builtin_themes;
pub mod chat_loop;
pub mod help;
pub mod layout;
pub mod markdown;
pub mod osc;
pub mod osc_backend;
pub mod osc_state;
pub mod picker;
pub mod renderer;
pub mod span;
pub mod theme;
pub mod title;
