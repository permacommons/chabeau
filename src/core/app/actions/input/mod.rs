mod command;
mod compose;
mod inspect;
mod status;

use super::{App, AppAction, AppActionContext, AppCommand};

pub(crate) use status::set_status_message;

pub enum InputAction {
    Compose(ComposeAction),
    Command(CommandAction),
    Inspect(InspectAction),
    Status(StatusAction),
}

pub enum ComposeAction {
    ToggleComposeMode,
    CancelFilePrompt,
    CancelMcpPromptInput,
    CancelInPlaceEdit,
    ClearInput,
    InsertIntoInput { text: String },
}

pub enum CommandAction {
    ProcessCommand { input: String },
    CompleteInPlaceEdit { index: usize, new_text: String },
    CompleteAssistantEdit { content: String },
}

pub enum InspectAction {
    Open,
    ToggleView,
    Step { delta: i32 },
    Copy,
    ToggleDecode,
}

pub enum StatusAction {
    SetStatus { message: String },
    ClearStatus,
}

pub(super) fn handle_input_action(
    app: &mut App,
    action: InputAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        InputAction::Compose(action) => compose::handle_compose_action(app, action, ctx),
        InputAction::Command(action) => command::handle_command_action(app, action, ctx),
        InputAction::Inspect(action) => inspect::handle_inspect_action(app, action, ctx),
        InputAction::Status(action) => status::handle_status_action(app, action, ctx),
    }
}

pub(super) fn update_scroll_after_command(app: &mut App, ctx: AppActionContext) {
    if ctx.term_width == 0 || ctx.term_height == 0 {
        return;
    }

    let input_area_height = app.input_area_height(ctx.term_width);
    let mut conversation = app.conversation();
    let available_height =
        conversation.calculate_available_height(ctx.term_height, input_area_height);
    conversation.update_scroll_position(available_height, ctx.term_width);
}

impl From<ComposeAction> for InputAction {
    fn from(value: ComposeAction) -> Self {
        Self::Compose(value)
    }
}

impl From<CommandAction> for InputAction {
    fn from(value: CommandAction) -> Self {
        Self::Command(value)
    }
}

impl From<InspectAction> for InputAction {
    fn from(value: InspectAction) -> Self {
        Self::Inspect(value)
    }
}

impl From<StatusAction> for InputAction {
    fn from(value: StatusAction) -> Self {
        Self::Status(value)
    }
}

impl From<InputAction> for AppAction {
    fn from(value: InputAction) -> Self {
        Self::Input(value)
    }
}
