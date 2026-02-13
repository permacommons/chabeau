//! Slash command processing and routing.

mod handlers;
mod mcp_prompt_parser;
mod refine;
mod registry;

pub use handlers::io::dump_conversation_with_overwrite;
pub(crate) use handlers::mcp::build_mcp_server_output;
pub use registry::{all_commands, matching_commands, CommandInvocation};

use crate::core::app::App;
use registry::DispatchOutcome;

/// Result of processing a command or user input.
pub enum CommandResult {
    Continue,
    ContinueWithTranscriptFocus,
    ProcessAsMessage(String),
    OpenModelPicker,
    OpenProviderPicker,
    OpenThemePicker,
    OpenCharacterPicker,
    OpenPersonaPicker,
    OpenPresetPicker,
    Refine(String),
    RunMcpPrompt(crate::core::app::session::McpPromptRequest),
    RefreshMcp { server_id: String },
}

/// Processes user input and dispatches commands.
pub fn process_input(app: &mut App, input: &str) -> CommandResult {
    match registry::registry().dispatch(input) {
        DispatchOutcome::NotACommand | DispatchOutcome::UnknownCommand => {
            if let Some(result) = handlers::mcp::handle_prompt_invocation(app, input) {
                return result;
            }
            CommandResult::ProcessAsMessage(input.to_string())
        }
        DispatchOutcome::Invocation(invocation) => {
            let handler = invocation.command.handler;
            handler(app, invocation)
        }
    }
}

#[cfg(test)]
mod tests;
