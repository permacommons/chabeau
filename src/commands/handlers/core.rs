use crate::commands::registry::CommandInvocation;
use crate::commands::{all_commands, CommandResult};
use crate::core::app::App;
use crate::core::message::AppMessageKind;

pub(crate) fn handle_help(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    let mut help_md = crate::ui::help::builtin_help_md().to_string();
    help_md.push_str("\n\n## Commands\n");
    for command in all_commands() {
        for usage in command.usages {
            help_md.push_str(&format!("- `{}` — {}\n", usage.syntax, usage.description));
        }
        for line in command.extra_help {
            help_md.push_str(line);
            help_md.push('\n');
        }
    }
    app.conversation()
        .add_app_message(AppMessageKind::Info, help_md);
    CommandResult::ContinueWithTranscriptFocus
}

pub(crate) fn handle_clear(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    let mut conversation = app.conversation();
    conversation.clear_transcript();
    conversation.show_character_greeting_if_needed();
    conversation.set_status("Transcript cleared".to_string());
    CommandResult::Continue
}
