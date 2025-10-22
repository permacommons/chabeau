use crate::commands::{CommandResult, RefineParams};
use crate::core::app::App;
use crate::commands::registry::CommandInvocation;

const DEFAULT_REFINE_INSTRUCTIONS: &str = "Messages that begin with REFINE: are instructions to regenerate the previous message, but with a change. The change is that follows after REFINE:. For example, REFINE: shorter means to shorten the previous message. But REFINE: instructions can also be more elaborate, multi-paragraph. Try to follow them as closely as possible in re-generating the message. Do NOT add any acknowledgment or question beyond just the re-generated message. The re-generated message will fully replace the previous one in the transcript, so it must be a seamless replacement.";

pub(super) fn handle_refine(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let prompt = invocation.args_text();
    if prompt.is_empty() {
        app.conversation()
            .set_status("Usage: /refine <prompt>");
        return CommandResult::Continue;
    }

    if !app.conversation().can_retry() {
        app.conversation()
            .set_status("No previous message to refine.");
        return CommandResult::Continue;
    }

    let config = app.config.clone();
    let instructions = config
        .refine_instructions
        .unwrap_or_else(|| DEFAULT_REFINE_INSTRUCTIONS.to_string());
    let prefix = config.refine_prefix.unwrap_or_else(|| "REFINE:".to_string());

    CommandResult::Refine(RefineParams {
        prompt: prompt.to_string(),
        instructions,
        prefix,
    })
}
