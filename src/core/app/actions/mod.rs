mod file_prompt;
mod input;
mod picker;
mod streaming;

use tokio::sync::mpsc;

use super::App;
use crate::api::ModelsResponse;
use crate::core::app::ModelPickerRequest;
use crate::core::chat_stream::StreamParams;
use crate::core::message::AppMessageKind;

pub enum AppAction {
    AppendResponseChunk {
        content: String,
        stream_id: u64,
    },
    StreamAppMessage {
        kind: AppMessageKind,
        message: String,
        stream_id: u64,
    },
    StreamErrored {
        message: String,
        stream_id: u64,
    },
    StreamCompleted {
        stream_id: u64,
    },
    ClearStatus,
    ToggleComposeMode,
    CancelFilePrompt,
    CancelInPlaceEdit,
    CancelStreaming,
    SetStatus {
        message: String,
    },
    ClearInput,
    InsertIntoInput {
        text: String,
    },
    SubmitMessage {
        message: String,
    },
    RefineLastMessage {
        prompt: String,
    },
    RetryLastMessage,
    ProcessCommand {
        input: String,
    },
    PickerEscape,
    PickerMoveUp,
    PickerMoveDown,
    PickerMoveToStart,
    PickerMoveToEnd,
    PickerCycleSortMode,
    PickerApplySelection {
        persistent: bool,
    },
    PickerUnsetDefault,
    PickerBackspace,
    PickerTypeChar {
        ch: char,
    },
    PickerInspectSelection,
    PickerInspectScroll {
        lines: i32,
    },
    PickerInspectScrollToStart,
    PickerInspectScrollToEnd,
    CompleteFilePromptDump {
        filename: String,
        overwrite: bool,
    },
    CompleteFilePromptSaveBlock {
        filename: String,
        content: String,
        overwrite: bool,
    },
    CompleteInPlaceEdit {
        index: usize,
        new_text: String,
    },
    CompleteAssistantEdit {
        content: String,
    },
    ModelPickerLoaded {
        default_model_for_provider: Option<String>,
        models_response: ModelsResponse,
    },
    ModelPickerLoadFailed {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AppActionContext {
    pub term_width: u16,
    pub term_height: u16,
}

pub struct AppActionEnvelope {
    pub action: AppAction,
    pub context: AppActionContext,
}

#[derive(Clone)]
pub struct AppActionDispatcher {
    tx: mpsc::UnboundedSender<AppActionEnvelope>,
}

impl AppActionDispatcher {
    pub fn new(tx: mpsc::UnboundedSender<AppActionEnvelope>) -> Self {
        Self { tx }
    }

    pub fn dispatch_many<I>(&self, actions: I, ctx: AppActionContext)
    where
        I: IntoIterator<Item = AppAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action,
                context: ctx,
            });
        }
    }
}

pub enum AppCommand {
    SpawnStream(StreamParams),
    LoadModelPicker(ModelPickerRequest),
}

pub fn apply_actions(
    app: &mut App,
    envelopes: impl IntoIterator<Item = AppActionEnvelope>,
) -> Vec<AppCommand> {
    let mut commands = Vec::new();
    for envelope in envelopes {
        if let Some(cmd) = apply_action(app, envelope.action, envelope.context) {
            commands.push(cmd);
        }
    }
    commands
}

pub fn apply_action(app: &mut App, action: AppAction, ctx: AppActionContext) -> Option<AppCommand> {
    match action {
        AppAction::AppendResponseChunk { .. }
        | AppAction::StreamAppMessage { .. }
        | AppAction::StreamErrored { .. }
        | AppAction::StreamCompleted { .. }
        | AppAction::CancelStreaming
        | AppAction::SubmitMessage { .. }
        | AppAction::RefineLastMessage { .. }
        | AppAction::RetryLastMessage => streaming::handle_streaming_action(app, action, ctx),

        AppAction::ClearStatus
        | AppAction::ToggleComposeMode
        | AppAction::CancelFilePrompt
        | AppAction::CancelInPlaceEdit
        | AppAction::SetStatus { .. }
        | AppAction::ClearInput
        | AppAction::InsertIntoInput { .. }
        | AppAction::ProcessCommand { .. }
        | AppAction::CompleteInPlaceEdit { .. }
        | AppAction::CompleteAssistantEdit { .. } => input::handle_input_action(app, action, ctx),

        AppAction::PickerEscape
        | AppAction::PickerMoveUp
        | AppAction::PickerMoveDown
        | AppAction::PickerMoveToStart
        | AppAction::PickerMoveToEnd
        | AppAction::PickerCycleSortMode
        | AppAction::PickerApplySelection { .. }
        | AppAction::PickerUnsetDefault
        | AppAction::PickerBackspace
        | AppAction::PickerTypeChar { .. }
        | AppAction::PickerInspectSelection
        | AppAction::PickerInspectScroll { .. }
        | AppAction::PickerInspectScrollToStart
        | AppAction::PickerInspectScrollToEnd
        | AppAction::ModelPickerLoaded { .. }
        | AppAction::ModelPickerLoadFailed { .. } => picker::handle_picker_action(app, action, ctx),

        AppAction::CompleteFilePromptDump { .. }
        | AppAction::CompleteFilePromptSaveBlock { .. } => {
            file_prompt::handle_file_prompt_action(app, action, ctx)
        }
    }
}
