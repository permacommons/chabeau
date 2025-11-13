#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Prefix applied to transcript messages emitted by Chabeau itself.
pub const APP_MESSAGE_ROLE_PREFIX: &str = "app";
pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";
pub const ROLE_APP_INFO: &str = "app/info";
pub const ROLE_APP_WARNING: &str = "app/warning";
pub const ROLE_APP_ERROR: &str = "app/error";
pub const ROLE_APP_LOG: &str = "app/log";

/// Severity for app-authored messages rendered in the transcript but never
/// transmitted to the remote API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMessageKind {
    Info,
    Warning,
    Error,
    Log,
}

impl AppMessageKind {
    pub fn as_role(self) -> &'static str {
        match self {
            AppMessageKind::Info => ROLE_APP_INFO,
            AppMessageKind::Warning => ROLE_APP_WARNING,
            AppMessageKind::Error => ROLE_APP_ERROR,
            AppMessageKind::Log => ROLE_APP_LOG,
        }
    }

    pub fn from_suffix(suffix: &str) -> Self {
        match suffix {
            "warning" => AppMessageKind::Warning,
            "error" => AppMessageKind::Error,
            "log" => AppMessageKind::Log,
            _ => AppMessageKind::Info,
        }
    }
}

impl Message {
    pub fn app(kind: AppMessageKind, content: impl Into<String>) -> Self {
        let content = content.into();
        match kind {
            AppMessageKind::Info => Self::app_info(content),
            AppMessageKind::Warning => Self::app_warning(content),
            AppMessageKind::Error => Self::app_error(content),
            AppMessageKind::Log => Self::app_log(content),
        }
    }

    pub fn app_info(content: impl Into<String>) -> Self {
        Self {
            role: AppMessageKind::Info.as_role().to_string(),
            content: content.into(),
        }
    }

    pub fn app_warning(content: impl Into<String>) -> Self {
        Self {
            role: AppMessageKind::Warning.as_role().to_string(),
            content: content.into(),
        }
    }

    pub fn app_error(content: impl Into<String>) -> Self {
        Self {
            role: AppMessageKind::Error.as_role().to_string(),
            content: content.into(),
        }
    }

    pub fn app_log(content: impl Into<String>) -> Self {
        Self {
            role: AppMessageKind::Log.as_role().to_string(),
            content: content.into(),
        }
    }
}

pub fn is_app_message_role(role: &str) -> bool {
    role == ROLE_APP_INFO
        || role == ROLE_APP_WARNING
        || role == ROLE_APP_ERROR
        || role == ROLE_APP_LOG
        || role.starts_with("app/")
}

pub fn app_message_kind_from_role(role: &str) -> AppMessageKind {
    if let Some((prefix, suffix)) = role.split_once('/') {
        if prefix == APP_MESSAGE_ROLE_PREFIX {
            return AppMessageKind::from_suffix(suffix);
        }
    }
    AppMessageKind::Info
}
