//! Central action types and reducers for the core app state machine.
//!
//! # Ownership boundary
//! This module owns action enums, dispatch envelopes, and top-level reducer
//! fan-out (`apply_action` / `apply_actions`). It delegates feature-specific
//! mutations to sibling reducers (`input`, `picker`, `streaming`, prompt
//! handlers), which mutate `App` and optionally return [`AppCommand`] side
//! effects.
//!
//! # Main structures and invariants
//! - [`AppAction`] is the root intent type consumed by the event loop.
//! - [`AppActionEnvelope`] couples an action with terminal dimensions so reducers
//!   can compute scroll/layout-aware transitions.
//! - Reducers return at most one [`AppCommand`] per action, preserving ordering
//!   when `apply_actions` processes a batch.
//!
//! # Call flow entrypoints
//! The chat event loop dispatches actions through [`AppActionDispatcher`] and
//! drains them into [`apply_actions`]. Returned commands are executed by async
//! executors, which later dispatch new actions back into this reducer layer.

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

/// Root action union consumed by the app reducer loop.
pub enum AppAction {
    Streaming(StreamingAction),
    Input(InputAction),
    Picker(PickerAction),
    Prompt(PromptAction),
}

/// Actions that drive stream lifecycle, tool calls, and MCP callbacks.
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

/// Actions emitted while a picker overlay is active.
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

/// Actions for modal prompt flows (file and MCP argument prompts).
pub enum PromptAction {
    File(FilePromptAction),
    Mcp(McpPromptAction),
}

/// Completion actions for file-path prompt workflows.
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

/// Actions emitted by the MCP prompt-argument modal.
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

/// Action payload coupled with terminal-size context for reducers.
pub struct AppActionEnvelope {
    pub action: AppAction,
    pub context: AppActionContext,
}

#[derive(Clone)]
/// Thread-safe dispatcher used by UI/executor tasks to enqueue actions.
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

    pub fn dispatch_input_many<I>(&self, actions: I, ctx: AppActionContext)
    where
        I: IntoIterator,
        I::Item: Into<InputAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action: AppAction::Input(action.into()),
                context: ctx,
            });
        }
    }

    pub fn dispatch_streaming_many<I>(&self, actions: I, ctx: AppActionContext)
    where
        I: IntoIterator,
        I::Item: Into<StreamingAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action: AppAction::Streaming(action.into()),
                context: ctx,
            });
        }
    }

    pub fn dispatch_picker_many<I>(&self, actions: I, ctx: AppActionContext)
    where
        I: IntoIterator,
        I::Item: Into<PickerAction>,
    {
        for action in actions.into_iter() {
            let _ = self.tx.send(AppActionEnvelope {
                action: AppAction::Picker(action.into()),
                context: ctx,
            });
        }
    }
}

/// Deferred side effects returned by reducers for async execution.
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

/// Applies a batch of action envelopes and collects emitted commands.
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

/// Applies a single action envelope to the app state machine.
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
