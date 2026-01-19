use super::{input, App, AppAction, AppActionContext, AppCommand};
use crate::core::app::session::McpPromptRequest;

pub(super) fn handle_mcp_prompt_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::CompleteMcpPromptArg { value } => {
            handle_complete_mcp_prompt_arg(app, value, ctx)
        }
        _ => unreachable!("non-mcp prompt action routed to mcp prompt handler"),
    }
}

fn handle_complete_mcp_prompt_arg(
    app: &mut App,
    value: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    let prompt_state = app.ui.mcp_prompt_input().cloned()?;

    let Some(arg) = prompt_state.pending_args.get(prompt_state.next_index) else {
        app.ui.cancel_mcp_prompt_input();
        return None;
    };

    let trimmed = value.trim().to_string();
    if arg.required && trimmed.is_empty() {
        let label = arg.title.as_deref().unwrap_or(&arg.name);
        input::set_status_message(app, format!("Value required for {}", label), ctx);
        return None;
    }

    let mut updated = prompt_state.clone();
    if !trimmed.is_empty() || arg.required {
        updated.collected.insert(arg.name.clone(), trimmed);
    }
    updated.next_index = updated.next_index.saturating_add(1);

    if updated.next_index < updated.pending_args.len() {
        app.ui.start_mcp_prompt_input(updated);
        app.clear_status();
        return None;
    }

    app.ui.cancel_mcp_prompt_input();
    app.clear_status();

    Some(AppCommand::RunMcpPrompt(McpPromptRequest {
        server_id: updated.server_id,
        prompt_name: updated.prompt_name,
        arguments: updated.collected,
    }))
}
