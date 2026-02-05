//! MCP server setting handlers.

use crate::cli::settings::error::SettingError;
use crate::cli::settings::helpers::{format_bool, mutate_config, parse_bool};
use crate::cli::settings::{SetContext, SettingHandler};
use crate::core::config::data::Config;

/// Handler for the `mcp` setting (enable/disable MCP servers and yolo mode).
///
/// Supports:
/// - `chabeau set mcp <server> on/off` - enable/disable server
/// - `chabeau set mcp <server> yolo on/off` - enable/disable yolo mode
pub struct McpHandler;

impl McpHandler {
    fn validate_server(ctx: &SetContext<'_>, server_id: &str) -> Result<(), SettingError> {
        if ctx.config.get_mcp_server(server_id).is_some() {
            return Ok(());
        }

        let available: Vec<_> = ctx
            .config
            .list_mcp_servers()
            .iter()
            .map(|s| s.id.as_str())
            .collect();

        let hint = if available.is_empty() {
            Some("No MCP servers are configured. Add servers to config.toml first.".into())
        } else {
            Some(format!("Available servers: {}", available.join(", ")))
        };

        Err(SettingError::UnknownItem {
            kind: "MCP server",
            input: server_id.to_string(),
            hint,
        })
    }
}

impl SettingHandler for McpHandler {
    fn key(&self) -> &'static str {
        "mcp"
    }

    fn set(&self, args: &[String], ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        if args.len() < 2 {
            return Err(SettingError::MissingArgs {
                hint: "To configure an MCP server:",
                example: "chabeau set mcp <server> on/off\nchabeau set mcp <server> yolo on/off",
            });
        }

        let server_id = &args[0];
        Self::validate_server(ctx, server_id)?;

        // Check if this is a yolo setting: `mcp <server> yolo on/off`
        if args.len() >= 3 && args[1].eq_ignore_ascii_case("yolo") {
            let value_input = args[2..].join(" ");
            let yolo =
                parse_bool(&value_input).ok_or(SettingError::InvalidBoolean(value_input))?;
            let display = format_bool(yolo);
            let server_id_owned = server_id.clone();

            mutate_config(move |config| {
                if let Some(server) = config
                    .mcp_servers
                    .iter_mut()
                    .find(|s| s.id.eq_ignore_ascii_case(&server_id_owned))
                {
                    server.yolo = Some(yolo);
                }
                Ok(())
            })?;

            return Ok(format!(
                "✅ Set MCP server '{}' yolo mode to: {}",
                server_id, display
            ));
        }

        // Otherwise it's an enabled setting: `mcp <server> on/off`
        let value_input = args[1..].join(" ");
        let enabled =
            parse_bool(&value_input).ok_or(SettingError::InvalidBoolean(value_input))?;
        let display = format_bool(enabled);
        let server_id_owned = server_id.clone();

        mutate_config(move |config| {
            if let Some(server) = config
                .mcp_servers
                .iter_mut()
                .find(|s| s.id.eq_ignore_ascii_case(&server_id_owned))
            {
                server.enabled = Some(enabled);
            }
            Ok(())
        })?;

        Ok(format!("✅ Set MCP server '{}' to: {}", server_id, display))
    }

    fn unset(&self, args: Option<&str>, ctx: &mut SetContext<'_>) -> Result<String, SettingError> {
        let value = args.ok_or(SettingError::MissingArgs {
            hint: "To reset an MCP server setting:",
            example: "chabeau unset mcp <server>\nchabeau unset mcp \"<server> yolo\"",
        })?;

        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.is_empty() {
            return Err(SettingError::MissingArgs {
                hint: "To reset an MCP server setting:",
                example: "chabeau unset mcp <server>\nchabeau unset mcp \"<server> yolo\"",
            });
        }

        let server_id = parts[0];
        Self::validate_server(ctx, server_id)?;

        // Check if unsetting yolo: `unset mcp "<server> yolo"`
        if parts.len() >= 2 && parts[1].eq_ignore_ascii_case("yolo") {
            let server_id_owned = server_id.to_string();

            mutate_config(move |config| {
                if let Some(server) = config
                    .mcp_servers
                    .iter_mut()
                    .find(|s| s.id.eq_ignore_ascii_case(&server_id_owned))
                {
                    server.yolo = None;
                }
                Ok(())
            })?;

            return Ok(format!(
                "✅ Unset MCP server '{}' yolo mode (will use default: off)",
                server_id
            ));
        }

        // Otherwise unsetting enabled state
        let server_id_owned = server_id.to_string();

        mutate_config(move |config| {
            if let Some(server) = config
                .mcp_servers
                .iter_mut()
                .find(|s| s.id.eq_ignore_ascii_case(&server_id_owned))
            {
                server.enabled = None;
            }
            Ok(())
        })?;

        Ok(format!(
            "✅ Unset MCP server '{}' enabled state (will use default: on)",
            server_id
        ))
    }

    fn format(&self, config: &Config) -> String {
        let servers = config.list_mcp_servers();
        if servers.is_empty() {
            "  mcp: (no servers configured)".to_string()
        } else {
            let mut output = String::from("  mcp:\n");
            for server in servers {
                let enabled = format_bool(server.is_enabled());
                let yolo = if server.is_yolo() { " [yolo]" } else { "" };
                output.push_str(&format!("    {}: {}{}\n", server.id, enabled, yolo));
            }
            output.pop(); // Remove trailing newline
            output
        }
    }
}
