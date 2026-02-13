pub(super) mod config;
pub(super) mod core;
pub(super) mod io;
pub(super) mod mcp;

use crate::commands::CommandResult;
use crate::core::app::App;
use crate::core::message::AppMessageKind;

pub(super) fn usage_status(app: &mut App, usage: &'static str) -> CommandResult {
    app.conversation().set_status(usage);
    CommandResult::Continue
}

pub(super) fn required_arg<'a>(
    app: &mut App,
    invocation: &crate::commands::registry::CommandInvocation<'a>,
    index: usize,
    usage: &'static str,
) -> Option<&'a str> {
    match invocation.arg(index) {
        Some(value) => Some(value),
        None => {
            app.conversation().set_status(usage);
            None
        }
    }
}

pub(super) fn add_info_and_focus(app: &mut App, content: String) -> CommandResult {
    app.conversation()
        .add_app_message(AppMessageKind::Info, content);
    app.ui.focus_transcript();
    CommandResult::ContinueWithTranscriptFocus
}
