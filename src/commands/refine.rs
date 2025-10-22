use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::App;

pub(super) fn handle_refine(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let prompt = invocation.args_text();
    if prompt.is_empty() {
        app.conversation().set_status("Usage: /refine <prompt>");
        return CommandResult::Continue;
    }

    if !app.conversation().can_retry() {
        app.conversation()
            .set_status("No previous message to refine.");
        return CommandResult::Continue;
    }

    CommandResult::Refine(prompt.to_string())
}
