//! Session persistence — save and restore conversation state.
//!
//! Sessions are stored as JSON files under the XDG data directory
//! (`~/.local/share/chabeau/sessions/` on Linux). Each session captures the
//! full conversation transcript, tool results, and session context (provider,
//! model, character, persona, preset) so the user can resume later.
//!
//! No authentication material (API keys) is stored in session files.

use crate::api::ChatMessage;
use crate::character::card::CharacterCard;
use crate::core::app::session::{
    McpInitState, ToolPayloadHistoryEntry, ToolPipelineState, ToolResultStatus,
};
use crate::core::config::data::{Persona, Preset};
use crate::core::message::Message;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Errors that can occur during session save/load operations.
#[derive(Debug)]
pub enum SessionError {
    /// Failed to read the session directory.
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Failed to read a session file.
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Failed to parse a session file.
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// Failed to write a session file.
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The session directory could not be created.
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The session file could not be deleted.
    Delete {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::ReadDir { path, source } => {
                write!(
                    f,
                    "Failed to read session dir {}: {}",
                    path.display(),
                    source
                )
            }
            SessionError::ReadFile { path, source } => {
                write!(
                    f,
                    "Failed to read session file {}: {}",
                    path.display(),
                    source
                )
            }
            SessionError::Parse { path, source } => {
                write!(
                    f,
                    "Failed to parse session file {}: {}",
                    path.display(),
                    source
                )
            }
            SessionError::Write { path, source } => {
                write!(
                    f,
                    "Failed to write session file {}: {}",
                    path.display(),
                    source
                )
            }
            SessionError::CreateDir { path, source } => {
                write!(
                    f,
                    "Failed to create session dir {}: {}",
                    path.display(),
                    source
                )
            }
            SessionError::Delete { path, source } => {
                write!(
                    f,
                    "Failed to delete session file {}: {}",
                    path.display(),
                    source
                )
            }
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionError::ReadDir { source, .. }
            | SessionError::ReadFile { source, .. }
            | SessionError::Write { source, .. }
            | SessionError::CreateDir { source, .. }
            | SessionError::Delete { source, .. } => Some(source),
            SessionError::Parse { source, .. } => Some(source),
        }
    }
}

/// A summary of a saved session, returned by [`list_sessions`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub character: Option<String>,
    pub persona: Option<String>,
    pub preset: Option<String>,
    pub message_count: usize,
    pub modified_at: chrono::DateTime<Utc>,
}

/// Full session snapshot — enough to reconstruct an App's conversation state.
/// Stores full character/persona/preset data so sessions survive config changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub character: Option<CharacterCard>,
    pub persona: Option<Persona>,
    pub preset: Option<Preset>,
    pub messages: Vec<Message>,
    pub tool_results: Vec<ChatMessage>,
    pub tool_payloads: Vec<ToolPayload>,
    pub mcp_init_complete: bool,
    pub refine_instructions: String,
    pub refine_prefix: String,
    pub markdown_enabled: bool,
    pub syntax_enabled: bool,
    pub created_at: chrono::DateTime<Utc>,
    pub modified_at: chrono::DateTime<Utc>,
}

/// Serializable wrapper for ToolPayloadHistoryEntry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPayload {
    pub server_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub assistant_message: ChatMessage,
    pub tool_message: ChatMessage,
    pub assistant_message_index: Option<usize>,
}

impl From<&ToolPayloadHistoryEntry> for ToolPayload {
    fn from(entry: &ToolPayloadHistoryEntry) -> Self {
        Self {
            server_id: entry.server_id.clone(),
            tool_call_id: entry.tool_call_id.clone(),
            assistant_message: entry.assistant_message.clone(),
            tool_message: entry.tool_message.clone(),
            assistant_message_index: entry.assistant_message_index,
        }
    }
}

/// Serializable wrapper for ToolResultRecord.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableToolResultRecord {
    pub tool_name: String,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub status: ToolResultStatus,
    pub content: String,
    pub summary: String,
    pub tool_call_id: Option<String>,
    pub raw_arguments: Option<String>,
    pub assistant_message_index: Option<usize>,
}

/// Serializable wrapper for McpSamplingRequest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableMcpSamplingRequest {
    pub server_id: String,
    pub messages: Vec<ChatMessage>,
}

/// Serializable wrapper for ToolCallRequest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableToolCallRequest {
    pub server_id: String,
    pub tool_name: String,
    pub raw_arguments: String,
}

/// Serializable wrapper for PendingToolCall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializablePendingToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
}

/// Serializable wrapper for ToolPipelineState.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableToolPipeline {
    pub pending_tool_calls: Vec<(u32, SerializablePendingToolCall)>,
    pub pending_tool_queue: Vec<SerializableToolCallRequest>,
    pub active_tool_request: Option<SerializableToolCallRequest>,
    pub pending_sampling_queue: Vec<SerializableMcpSamplingRequest>,
    pub active_sampling_request: Option<SerializableMcpSamplingRequest>,
    pub tool_call_records: Vec<crate::api::ChatToolCall>,
    pub tool_results: Vec<ChatMessage>,
    pub tool_result_history: Vec<SerializableToolResultRecord>,
    pub tool_payload_history: Vec<ToolPayload>,
    pub continuation_messages: Option<SerializableStreamContinuation>,
}

/// Serializable wrapper for StreamContinuation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamContinuation {
    pub api_messages: Vec<ChatMessage>,
    pub api_messages_base: Vec<ChatMessage>,
}

impl From<&ToolPipelineState> for SerializableToolPipeline {
    fn from(state: &ToolPipelineState) -> Self {
        Self {
            pending_tool_calls: state
                .pending_tool_calls
                .iter()
                .map(|(k, v)| {
                    (
                        *k,
                        SerializablePendingToolCall {
                            id: v.id.clone(),
                            name: v.name.clone(),
                            arguments: v.arguments.clone(),
                        },
                    )
                })
                .collect(),
            pending_tool_queue: state
                .pending_tool_queue
                .iter()
                .map(|r| SerializableToolCallRequest {
                    server_id: r.server_id.clone(),
                    tool_name: r.tool_name.clone(),
                    raw_arguments: r.raw_arguments.clone(),
                })
                .collect(),
            active_tool_request: state.active_tool_request.as_ref().map(|r| {
                SerializableToolCallRequest {
                    server_id: r.server_id.clone(),
                    tool_name: r.tool_name.clone(),
                    raw_arguments: r.raw_arguments.clone(),
                }
            }),
            pending_sampling_queue: state
                .pending_sampling_queue
                .iter()
                .map(|r| SerializableMcpSamplingRequest {
                    server_id: r.server_id.clone(),
                    messages: r.messages.clone(),
                })
                .collect(),
            active_sampling_request: state.active_sampling_request.as_ref().map(|r| {
                SerializableMcpSamplingRequest {
                    server_id: r.server_id.clone(),
                    messages: r.messages.clone(),
                }
            }),
            tool_call_records: state.tool_call_records.clone(),
            tool_results: state.tool_results.clone(),
            tool_result_history: state
                .tool_result_history
                .iter()
                .map(|r| SerializableToolResultRecord {
                    tool_name: r.tool_name.clone(),
                    server_name: r.server_name.clone(),
                    server_id: r.server_id.clone(),
                    status: r.status,
                    content: r.content.clone(),
                    summary: r.summary.clone(),
                    tool_call_id: r.tool_call_id.clone(),
                    raw_arguments: r.raw_arguments.clone(),
                    assistant_message_index: r.assistant_message_index,
                })
                .collect(),
            tool_payload_history: state
                .tool_payload_history
                .iter()
                .map(ToolPayload::from)
                .collect(),
            continuation_messages: state.continuation_messages.as_ref().map(|c| {
                SerializableStreamContinuation {
                    api_messages: c.api_messages.clone(),
                    api_messages_base: c.api_messages_base.clone(),
                }
            }),
        }
    }
}

/// Serializable wrapper for McpInitState.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableMcpInitState {
    pub in_progress: bool,
    pub complete: bool,
    pub deferred_message: Option<String>,
}

impl From<&McpInitState> for SerializableMcpInitState {
    fn from(state: &McpInitState) -> Self {
        Self {
            in_progress: state.in_progress,
            complete: state.complete,
            deferred_message: state.deferred_message.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Directory helpers
// ---------------------------------------------------------------------------

/// Returns the directory where session files are stored.
///
/// Uses `~/.local/share/chabeau/sessions/` (XDG_DATA_HOME) on Linux, or the
/// platform-equivalent data directory on other platforms.
fn session_dir() -> Result<PathBuf, SessionError> {
    let base = if let Some(override_dir) = std::env::var_os("CHABEAU_DATA_DIR") {
        PathBuf::from(override_dir)
    } else {
        let proj_dirs = directories::BaseDirs::new().ok_or_else(|| SessionError::CreateDir {
            path: PathBuf::new(),
            source: std::io::Error::other("no user home directory"),
        })?;
        proj_dirs.data_local_dir().join("chabeau").join("sessions")
    };
    let base_clone = base.clone();
    fs::create_dir_all(&base).map_err(|source| SessionError::CreateDir {
        path: base_clone,
        source,
    })?;
    Ok(base)
}

/// Returns the full path for a session file given its ID.
fn session_path(id: &str) -> Result<PathBuf, SessionError> {
    Ok(session_dir()?.join(format!("{}.json", id)))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new session snapshot from the current App state.
///
/// Captures the conversation transcript, tool results, and session context
/// (provider, model, character, persona, preset). API keys are not included.
#[allow(clippy::too_many_arguments)]
pub fn save_session(
    session_id: &str,
    name: &str,
    provider: &str,
    model: &str,
    base_url: &str,
    character: Option<CharacterCard>,
    persona: Option<Persona>,
    preset: Option<Preset>,
    messages: &[Message],
    tool_results: &[ChatMessage],
    tool_payloads: &[ToolPayloadHistoryEntry],
    mcp_init: &McpInitState,
    refine_instructions: &str,
    refine_prefix: &str,
    markdown_enabled: bool,
    syntax_enabled: bool,
) -> Result<(), SessionError> {
    let now = Utc::now();
    let snapshot = SessionSnapshot {
        id: session_id.to_string(),
        name: name.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        base_url: base_url.to_string(),
        character,
        persona,
        preset,
        messages: messages.to_vec(),
        tool_results: tool_results.to_vec(),
        tool_payloads: tool_payloads.iter().map(ToolPayload::from).collect(),
        mcp_init_complete: mcp_init.complete,
        refine_instructions: refine_instructions.to_string(),
        refine_prefix: refine_prefix.to_string(),
        markdown_enabled,
        syntax_enabled,
        created_at: now,
        modified_at: now,
    };

    let path = session_path(&snapshot.id)?;
    let contents =
        serde_json::to_string_pretty(&snapshot).map_err(|source| SessionError::Parse {
            path: path.clone(),
            source,
        })?;

    // Write atomically via temp file in the same directory
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| SessionError::Write {
            path: path.clone(),
            source,
        })?;
    temp.write_all(contents.as_bytes())
        .map_err(|source| SessionError::Write {
            path: path.clone(),
            source,
        })?;
    temp.as_file()
        .sync_all()
        .map_err(|source| SessionError::Write {
            path: path.clone(),
            source,
        })?;
    temp.persist(&path).map_err(|source| SessionError::Write {
        path: path.clone(),
        source: std::io::Error::other(source),
    })?;

    Ok(())
}

/// Load a session snapshot from disk by ID.
pub fn load_session(id: &str) -> Result<SessionSnapshot, SessionError> {
    let path = session_path(id)?;
    let contents = fs::read_to_string(&path).map_err(|source| SessionError::ReadFile {
        path: path.clone(),
        source,
    })?;
    let snapshot: SessionSnapshot =
        serde_json::from_str(&contents).map_err(|source| SessionError::Parse {
            path: path.clone(),
            source,
        })?;
    Ok(snapshot)
}

/// List all saved sessions, sorted by modified_at descending (newest first).
pub fn list_sessions() -> Result<Vec<SessionSummary>, SessionError> {
    let dir = session_dir()?;
    let entries = dir.read_dir().map_err(|source| SessionError::ReadDir {
        path: dir.clone(),
        source,
    })?;

    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| SessionError::ReadDir {
            path: dir.clone(),
            source,
        })?;
        let filename = entry.file_name().to_string_lossy().trim_end().to_string();
        if !filename.ends_with(".json") {
            continue;
        }

        // Try to load the summary without full validation
        let path = entry.path();
        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let snapshot: SessionSnapshot = match serde_json::from_str(&contents) {
            Ok(s) => s,
            Err(_) => continue,
        };

        summaries.push(SessionSummary {
            id: snapshot.id,
            name: snapshot.name,
            provider: snapshot.provider,
            model: snapshot.model,
            character: snapshot.character.as_ref().map(|c| c.data.name.clone()),
            persona: snapshot.persona.as_ref().map(|p| p.display_name.clone()),
            preset: snapshot.preset.as_ref().map(|p| p.id.clone()),
            message_count: snapshot.messages.len(),
            modified_at: snapshot.modified_at,
        });
    }

    summaries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(summaries)
}

/// Delete a session file by ID.
pub fn delete_session(id: &str) -> Result<(), SessionError> {
    let path = session_path(id)?;
    fs::remove_file(&path).map_err(|source| SessionError::Delete { path, source })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::{Message, TranscriptRole};
    use std::env;
    use std::sync::{LazyLock, Mutex};

    static TEST_ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn with_data_dir<F, R>(dir: &Path, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _lock = TEST_ENV_MUTEX.lock().unwrap();
        let old = env::var("CHABEAU_DATA_DIR").ok();
        env::set_var("CHABEAU_DATA_DIR", dir);
        let result = f();
        if let Some(val) = old {
            env::set_var("CHABEAU_DATA_DIR", val);
        } else {
            env::remove_var("CHABEAU_DATA_DIR");
        }
        result
    }

    fn make_test_messages() -> Vec<Message> {
        vec![
            Message::new(TranscriptRole::User, "Hello, world!"),
            Message::new(TranscriptRole::Assistant, "Hi there!"),
            Message::new(TranscriptRole::User, "How are you?"),
        ]
    }

    fn make_test_tool_results() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "tool".to_string(),
            content: "tool output".to_string(),
            name: None,
            tool_call_id: Some("call-1".to_string()),
            tool_calls: None,
        }]
    }

    fn make_test_tool_payloads() -> Vec<ToolPayloadHistoryEntry> {
        vec![ToolPayloadHistoryEntry {
            server_id: Some("test-server".to_string()),
            tool_call_id: Some("call-1".to_string()),
            assistant_message: ChatMessage {
                role: "assistant".to_string(),
                content: "calling tool".to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            tool_message: ChatMessage {
                role: "tool".to_string(),
                content: "tool result".to_string(),
                name: None,
                tool_call_id: Some("call-1".to_string()),
                tool_calls: None,
            },
            assistant_message_index: Some(1),
        }]
    }

    fn save_test_session(session_id: &str, name: &str) {
        let character = Some(CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: crate::character::card::CharacterData {
                name: "test-char".to_string(),
                description: "Test character".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        });
        let persona = Some(Persona {
            id: "test-persona".to_string(),
            display_name: "Test Persona".to_string(),
            bio: None,
        });
        let preset = Some(Preset {
            id: "test-preset".to_string(),
            pre: String::new(),
            post: String::new(),
        });

        save_session(
            session_id,
            name,
            "test-provider",
            "test-model",
            "https://example.com/v1",
            character,
            persona,
            preset,
            &make_test_messages(),
            &make_test_tool_results(),
            &make_test_tool_payloads(),
            &McpInitState {
                in_progress: false,
                complete: true,
                deferred_message: None,
            },
            "refine instructions",
            "refine prefix",
            true,
            true,
        )
        .expect("save");
    }

    #[test]
    fn session_save_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let snapshot = with_data_dir(dir.path(), || {
            save_test_session("test-001", "Roundtrip Test");
            load_session("test-001").expect("load")
        });

        assert_eq!(snapshot.id, "test-001");
        assert_eq!(snapshot.name, "Roundtrip Test");
        assert_eq!(snapshot.provider, "test-provider");
        assert_eq!(snapshot.model, "test-model");
        assert_eq!(snapshot.base_url, "https://example.com/v1");
        assert_eq!(
            snapshot.character.as_ref().map(|c| c.data.name.as_str()),
            Some("test-char")
        );
        assert_eq!(
            snapshot.persona.as_ref().map(|p| p.id.as_str()),
            Some("test-persona")
        );
        assert_eq!(
            snapshot.preset.as_ref().map(|p| p.id.as_str()),
            Some("test-preset")
        );
        assert_eq!(snapshot.messages.len(), 3);
        assert_eq!(snapshot.messages[0].role, TranscriptRole::User);
        assert_eq!(snapshot.messages[0].content, "Hello, world!");
        assert_eq!(snapshot.tool_results.len(), 1);
        assert_eq!(snapshot.tool_payloads.len(), 1);
        assert!(snapshot.mcp_init_complete);
        assert_eq!(snapshot.refine_instructions, "refine instructions");
        assert_eq!(snapshot.refine_prefix, "refine prefix");
        assert!(snapshot.markdown_enabled);
        assert!(snapshot.syntax_enabled);
    }

    #[test]
    fn session_save_with_optional_none_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let snapshot = with_data_dir(dir.path(), || {
            save_session(
                "test-none",
                "No Optionals",
                "prov",
                "mod",
                "https://example.com/v1",
                None,
                None,
                None,
                &[],
                &[],
                &[],
                &McpInitState::default(),
                "",
                "",
                false,
                false,
            )
            .expect("save");
            load_session("test-none").expect("load")
        });
        assert!(snapshot.character.is_none());
        assert!(snapshot.persona.is_none());
        assert!(snapshot.preset.is_none());
        assert!(snapshot.messages.is_empty());
        assert!(!snapshot.mcp_init_complete);
        assert!(!snapshot.markdown_enabled);
        assert!(!snapshot.syntax_enabled);
    }

    #[test]
    fn session_save_creates_json_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_data_dir(dir.path(), || {
            save_test_session("test-file", "File Test");
        });

        let path = dir.path().join("test-file.json");
        assert!(path.exists(), "session file should exist");
        let contents = fs::read_to_string(&path).expect("read file");
        assert!(contents.contains("File Test"));
    }

    #[test]
    fn session_save_overwrites_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let snapshot = with_data_dir(dir.path(), || {
            save_test_session("test-overwrite", "Version 1");
            save_test_session("test-overwrite", "Version 2");
            load_session("test-overwrite").expect("load")
        });
        assert_eq!(snapshot.name, "Version 2");
    }

    #[test]
    fn session_list_returns_summaries_sorted() {
        let dir = tempfile::tempdir().expect("tempdir");

        let sessions = with_data_dir(dir.path(), || {
            save_test_session("sess-a", "Session A");
            std::thread::sleep(std::time::Duration::from_millis(10));
            save_test_session("sess-b", "Session B");
            std::thread::sleep(std::time::Duration::from_millis(10));
            save_test_session("sess-c", "Session C");
            list_sessions().expect("list")
        });

        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].id, "sess-c");
        assert_eq!(sessions[1].id, "sess-b");
        assert_eq!(sessions[2].id, "sess-a");
        assert_eq!(sessions[0].message_count, 3);
        assert_eq!(sessions[0].provider, "test-provider");
        assert_eq!(sessions[0].model, "test-model");
    }

    #[test]
    fn session_list_skips_corrupt_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions = with_data_dir(dir.path(), || {
            save_test_session("valid", "Valid Session");

            fs::write(dir.path().join("corrupt.json"), "not json{{{").expect("write corrupt");
            fs::write(dir.path().join("readme.txt"), "hello").expect("write txt");

            list_sessions().expect("list")
        });

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "valid");
    }

    #[test]
    fn session_delete_removes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_data_dir(dir.path(), || {
            save_test_session("test-delete", "To Delete");
            let sess_path = dir.path().join("test-delete.json");
            assert!(sess_path.exists());

            delete_session("test-delete").expect("delete");
            assert!(!sess_path.exists());
        });
    }

    #[test]
    fn session_delete_nonexistent_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = with_data_dir(dir.path(), || delete_session("nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = with_data_dir(dir.path(), || load_session("does-not-exist"));
        assert!(result.is_err());
        match result {
            Err(SessionError::ReadFile { .. }) => {}
            other => panic!("expected ReadFile error, got: {:?}", other),
        }
    }

    #[test]
    fn load_corrupt_json_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = with_data_dir(dir.path(), || {
            fs::write(dir.path().join("bad.json"), "{{{invalid json}}}").expect("write corrupt");
            load_session("bad")
        });

        assert!(result.is_err());
        match result {
            Err(SessionError::Parse { .. }) => {}
            other => panic!("expected Parse error, got: {:?}", other),
        }
    }

    #[test]
    fn chabeau_data_dir_env_override() {
        let custom_dir = tempfile::tempdir().expect("tempdir");
        let sessions = with_data_dir(custom_dir.path(), || {
            save_test_session("env-test", "Env Override");
            list_sessions().expect("list")
        });
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "env-test");
    }

    #[test]
    fn tool_payload_conversion_roundtrip() {
        let entry = ToolPayloadHistoryEntry {
            server_id: Some("srv-1".to_string()),
            tool_call_id: Some("tc-42".to_string()),
            assistant_message: ChatMessage {
                role: "assistant".to_string(),
                content: "calling weather".to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            tool_message: ChatMessage {
                role: "tool".to_string(),
                content: "sunny, 25C".to_string(),
                name: None,
                tool_call_id: Some("tc-42".to_string()),
                tool_calls: None,
            },
            assistant_message_index: Some(3),
        };

        let payload = ToolPayload::from(&entry);

        assert_eq!(payload.server_id, entry.server_id);
        assert_eq!(payload.tool_call_id, entry.tool_call_id);
        assert_eq!(
            payload.assistant_message.content,
            entry.assistant_message.content
        );
        assert_eq!(payload.tool_message.content, entry.tool_message.content);
        assert_eq!(
            payload.assistant_message_index,
            entry.assistant_message_index
        );
    }

    #[test]
    fn serializable_tool_pipeline_conversion() {
        let mut state = ToolPipelineState::default();
        state.pending_tool_calls.insert(
            1,
            crate::core::app::session::PendingToolCall {
                id: Some("id-1".to_string()),
                name: Some("weather".to_string()),
                arguments: "{}".to_string(),
            },
        );
        state
            .pending_tool_queue
            .push_back(crate::core::app::session::ToolCallRequest {
                server_id: "srv".to_string(),
                tool_name: "lookup".to_string(),
                arguments: None,
                raw_arguments: "{}".to_string(),
                tool_call_id: None,
            });
        state.set_continuation(
            vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            vec![],
        );

        let serializable = SerializableToolPipeline::from(&state);

        assert_eq!(serializable.pending_tool_calls.len(), 1);
        assert_eq!(serializable.pending_tool_queue.len(), 1);
        assert!(serializable.continuation_messages.is_some());
        assert_eq!(
            serializable.continuation_messages.unwrap().api_messages[0].content,
            "hi"
        );
    }

    #[test]
    fn serializable_mcp_init_conversion() {
        let state = McpInitState {
            in_progress: true,
            complete: false,
            deferred_message: Some("wait for init".to_string()),
        };

        let serializable = SerializableMcpInitState::from(&state);

        assert!(serializable.in_progress);
        assert!(!serializable.complete);
        assert_eq!(
            serializable.deferred_message,
            Some("wait for init".to_string())
        );
    }
}
