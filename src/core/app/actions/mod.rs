mod file_prompt;
mod input;
mod mcp_prompt;
mod picker;
mod streaming;

pub use input::{CommandAction, ComposeAction, InputAction, InspectAction, StatusAction};
pub(crate) use streaming::{parse_resource_list_kind, ResourceListKind};

use tokio::sync::mpsc;

use super::App;
use crate::api::ModelsResponse;
use crate::core::app::session::{McpPromptRequest, ToolCallRequest};
use crate::core::app::ModelPickerRequest;
use crate::core::chat_stream::StreamParams;
use crate::core::chat_stream::ToolCallDelta;
use crate::core::message::AppMessageKind;
use crate::mcp::events::McpServerRequest;

pub enum AppAction {
    Streaming(StreamingAction),
    Input(InputAction),
    Picker(PickerAction),
    Prompt(PromptAction),
}

pub enum StreamingAction {
    AppendResponseChunk {
        content: String,
        stream_id: u64,
    },
    StreamAppMessage {
        kind: AppMessageKind,
        message: String,
        stream_id: u64,
    },
    StreamToolCallDelta {
        delta: ToolCallDelta,
        stream_id: u64,
    },
    McpInitCompleted,
    McpSendPendingWithoutTools,
    ToolPermissionDecision {
        decision: crate::mcp::permissions::ToolPermissionDecision,
    },
    ToolCallCompleted {
        tool_name: String,
        tool_call_id: Option<String>,
        result: Result<String, String>,
    },
    McpPromptCompleted {
        request: McpPromptRequest,
        result: Result<rust_mcp_schema::GetPromptResult, String>,
    },
    McpServerRequestReceived {
        request: Box<McpServerRequest>,
    },
    McpSamplingFinished,
    StreamErrored {
        message: String,
        stream_id: u64,
    },
    StreamCompleted {
        stream_id: u64,
    },
    CancelStreaming,
    SubmitMessage {
        message: String,
    },
    RefineLastMessage {
        prompt: String,
    },
    RetryLastMessage,
}

pub enum PickerAction {
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
    ModelPickerLoaded {
        default_model_for_provider: Option<String>,
        models_response: ModelsResponse,
    },
    ModelPickerLoadFailed {
        error: String,
    },
}

pub enum PromptAction {
    File(FilePromptAction),
    Mcp(McpPromptAction),
}

pub enum FilePromptAction {
    CompleteDump {
        filename: String,
        overwrite: bool,
    },
    CompleteSaveBlock {
        filename: String,
        content: String,
        overwrite: bool,
    },
}

pub enum McpPromptAction {
    CompleteArg { value: String },
}

impl From<StreamingAction> for AppAction {
    fn from(value: StreamingAction) -> Self {
        Self::Streaming(value)
    }
}

impl From<PickerAction> for AppAction {
    fn from(value: PickerAction) -> Self {
        Self::Picker(value)
    }
}

impl From<PromptAction> for AppAction {
    fn from(value: PromptAction) -> Self {
        Self::Prompt(value)
    }
}

impl From<FilePromptAction> for AppAction {
    fn from(value: FilePromptAction) -> Self {
        Self::Prompt(PromptAction::File(value))
    }
}

impl From<McpPromptAction> for AppAction {
    fn from(value: McpPromptAction) -> Self {
        Self::Prompt(PromptAction::Mcp(value))
    }
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
        I: IntoIterator,
        I::Item: Into<AppAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action: action.into(),
                context: ctx,
            });
        }
    }
}

pub enum AppCommand {
    SpawnStream(StreamParams),
    LoadModelPicker(ModelPickerRequest),
    RunMcpTool(ToolCallRequest),
    RunMcpPrompt(crate::core::app::session::McpPromptRequest),
    RunMcpSampling(Box<crate::core::app::session::McpSamplingRequest>),
    SendMcpServerError {
        server_id: String,
        request_id: rust_mcp_schema::RequestId,
        error: rust_mcp_schema::RpcError,
    },
    RefreshMcp {
        server_id: String,
    },
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
        AppAction::Streaming(action) => streaming::handle_streaming_action(app, action, ctx),
        AppAction::Input(action) => input::handle_input_action(app, action, ctx),
        AppAction::Picker(action) => picker::handle_picker_action(app, action, ctx),
        AppAction::Prompt(PromptAction::File(action)) => {
            file_prompt::handle_file_prompt_action(app, action, ctx)
        }
        AppAction::Prompt(PromptAction::Mcp(action)) => {
            mcp_prompt::handle_mcp_prompt_action(app, action, ctx)
        }
    }
}
