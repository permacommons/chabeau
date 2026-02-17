use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum TranscriptRole {
    User,
    Assistant,
    AppInfo,
    AppWarning,
    AppError,
    AppLog,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: TranscriptRole,
    pub content: String,
}

impl TranscriptRole {
    pub fn as_str(self) -> &'static str {
        match self {
            TranscriptRole::User => "user",
            TranscriptRole::Assistant => "assistant",
            TranscriptRole::AppInfo => "app/info",
            TranscriptRole::AppWarning => "app/warning",
            TranscriptRole::AppError => "app/error",
            TranscriptRole::AppLog => "app/log",
            TranscriptRole::ToolCall => "tool/call",
            TranscriptRole::ToolResult => "tool/result",
        }
    }

    pub fn to_api_role(self) -> Option<&'static str> {
        match self {
            TranscriptRole::User => Some("user"),
            TranscriptRole::Assistant => Some("assistant"),
            _ => None,
        }
    }

    pub fn from_api_role(role: &str) -> Result<Self, String> {
        Self::try_from(role)
    }

    pub fn is_user(self) -> bool {
        self == TranscriptRole::User
    }

    pub fn is_assistant(self) -> bool {
        self == TranscriptRole::Assistant
    }

    pub fn is_app(self) -> bool {
        matches!(
            self,
            TranscriptRole::AppInfo
                | TranscriptRole::AppWarning
                | TranscriptRole::AppError
                | TranscriptRole::AppLog
        )
    }

    pub fn app_kind(self) -> Option<AppMessageKind> {
        match self {
            TranscriptRole::AppInfo => Some(AppMessageKind::Info),
            TranscriptRole::AppWarning => Some(AppMessageKind::Warning),
            TranscriptRole::AppError => Some(AppMessageKind::Error),
            TranscriptRole::AppLog => Some(AppMessageKind::Log),
            _ => None,
        }
    }
}

impl AsRef<str> for TranscriptRole {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for TranscriptRole {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl TryFrom<&str> for TranscriptRole {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "user" => Ok(TranscriptRole::User),
            "assistant" => Ok(TranscriptRole::Assistant),
            "app/info" => Ok(TranscriptRole::AppInfo),
            "app/warning" => Ok(TranscriptRole::AppWarning),
            "app/error" => Ok(TranscriptRole::AppError),
            "app/log" => Ok(TranscriptRole::AppLog),
            "tool/call" => Ok(TranscriptRole::ToolCall),
            "tool/result" => Ok(TranscriptRole::ToolResult),
            _ => Err(format!("invalid transcript role: {value}")),
        }
    }
}

impl TryFrom<String> for TranscriptRole {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl From<TranscriptRole> for String {
    fn from(value: TranscriptRole) -> Self {
        value.as_str().to_string()
    }
}

/// Severity for app-authored messages rendered in the transcript but never
/// transmitted to the remote API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMessageKind {
    /// Informational message (e.g., configuration changes, feature notifications).
    Info,

    /// Warning message indicating potential issues or deprecated features.
    Warning,

    /// Error message for failures or invalid operations.
    Error,

    /// Logging output from API requests or internal operations.
    Log,
}

impl AppMessageKind {
    pub fn as_role(self) -> TranscriptRole {
        match self {
            AppMessageKind::Info => TranscriptRole::AppInfo,
            AppMessageKind::Warning => TranscriptRole::AppWarning,
            AppMessageKind::Error => TranscriptRole::AppError,
            AppMessageKind::Log => TranscriptRole::AppLog,
        }
    }
}

impl Message {
    pub fn new(role: TranscriptRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn is_user(&self) -> bool {
        self.role.is_user()
    }

    pub fn is_assistant(&self) -> bool {
        self.role.is_assistant()
    }

    pub fn is_app(&self) -> bool {
        self.role.is_app()
    }

    pub fn app(kind: AppMessageKind, content: impl Into<String>) -> Self {
        Self::new(kind.as_role(), content)
    }

    pub fn app_info(content: impl Into<String>) -> Self {
        Self::new(AppMessageKind::Info.as_role(), content)
    }

    pub fn app_warning(content: impl Into<String>) -> Self {
        Self::new(AppMessageKind::Warning.as_role(), content)
    }

    pub fn app_error(content: impl Into<String>) -> Self {
        Self::new(AppMessageKind::Error.as_role(), content)
    }

    pub fn app_log(content: impl Into<String>) -> Self {
        Self::new(AppMessageKind::Log.as_role(), content)
    }

    pub fn tool_call(content: impl Into<String>) -> Self {
        Self::new(TranscriptRole::ToolCall, content)
    }

    pub fn tool_result(content: impl Into<String>) -> Self {
        Self::new(TranscriptRole::ToolResult, content)
    }
}

pub fn is_app_message_role(role: impl AsRef<str>) -> bool {
    matches!(
        role.as_ref(),
        "app/info" | "app/warning" | "app/error" | "app/log"
    )
}

pub fn app_message_kind_from_role(role: impl AsRef<str>) -> AppMessageKind {
    match role.as_ref() {
        "app/warning" => AppMessageKind::Warning,
        "app/error" => AppMessageKind::Error,
        "app/log" => AppMessageKind::Log,
        _ => AppMessageKind::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_roles_are_not_app_roles() {
        assert!(!is_app_message_role(TranscriptRole::ToolCall));
        assert!(!is_app_message_role(TranscriptRole::ToolResult));
    }

    #[test]
    fn tool_messages_set_roles() {
        let call = Message::tool_call("call");
        let result = Message::tool_result("result");
        assert_eq!(call.role, TranscriptRole::ToolCall);
        assert_eq!(result.role, TranscriptRole::ToolResult);
    }

    #[test]
    fn invalid_role_strings_are_rejected() {
        assert!(TranscriptRole::try_from("app/unknown").is_err());
    }
}
