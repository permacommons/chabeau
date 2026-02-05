//! Settings management for CLI set/unset commands.
//!
//! This module provides a trait-based architecture for handling configuration
//! settings, with different handler types for different setting patterns:
//!
//! - Simple settings (e.g., `default-provider`, `theme`)
//! - Boolean settings (e.g., `markdown`, `syntax`)
//! - String settings (e.g., `refine-instructions`, `refine-prefix`)
//! - Provider-keyed settings (e.g., `default-model`)
//! - Provider+model-keyed settings (e.g., `default-character`, `default-persona`)

pub mod error;
pub mod handlers;
pub mod helpers;
pub mod registry;

pub use error::SettingError;
pub use registry::SettingRegistry;

use crate::character::CharacterService;
use crate::core::config::data::Config;

/// Context provided to setting handlers during set/unset operations.
pub struct SetContext<'a> {
    pub config: &'a Config,
    pub character_service: &'a mut CharacterService,
}

/// Trait for handling a configuration setting.
///
/// Each implementation handles a specific configuration key,
/// providing set, unset, and format operations.
pub trait SettingHandler: Send + Sync {
    /// Returns the configuration key this handler manages.
    fn key(&self) -> &'static str;

    /// Set the configuration value.
    ///
    /// # Arguments
    /// * `args` - The arguments provided after the key (may be empty)
    /// * `ctx` - Context containing config snapshot and services
    ///
    /// # Returns
    /// A success message to display, or an error.
    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError>;

    /// Unset (clear) the configuration value.
    ///
    /// # Arguments
    /// * `args` - Optional argument (e.g., provider name for provider-keyed settings)
    /// * `_ctx` - Context (unused by most handlers)
    ///
    /// # Returns
    /// A success message to display, or an error.
    fn unset(&self, args: Option<&str>, _ctx: &mut SetContext<'_>) -> Result<String, SettingError>;

    /// Format the current value for display in `chabeau set` output.
    fn format(&self, config: &Config) -> String;
}
