use thiserror::Error;

/// Every failure in WorkCraft is a value. Nothing in the engine panics on bad input.
#[derive(Debug, Error)]
pub enum Error {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("mcp error: {0}")]
    Mcp(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("cancelled")]
    Cancelled,
    #[error("denied: {0}")]
    Denied(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("app not installed ({app}): {hint}")]
    AppNotInstalled { app: String, hint: String },
    #[error("app not running ({app}): {hint}")]
    AppNotRunning { app: String, hint: String },
    #[error("denied by root ({app}: {path}): {hint}")]
    DeniedByRoot {
        app: String,
        path: String,
        hint: String,
    },
    #[error("tool failed ({tool}): {message} (hint: {hint})")]
    ToolFailed {
        tool: String,
        message: String,
        hint: String,
    },
}

impl Error {
    /// Actionable recovery hint for the model or user, when available.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::AppNotInstalled { hint, .. }
            | Self::AppNotRunning { hint, .. }
            | Self::DeniedByRoot { hint, .. }
            | Self::ToolFailed { hint, .. } => Some(hint),
            _ => None,
        }
    }

    /// Machine-readable error code for structured tool results.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AppNotInstalled { .. } => "app_not_installed",
            Self::AppNotRunning { .. } => "app_not_running",
            Self::Timeout(_) => "timeout",
            Self::DeniedByRoot { .. } | Self::Denied(_) => "denied_by_root",
            Self::ToolFailed { .. } | Self::Tool(_) => "tool_failed",
            Self::Mcp(_) => "mcp_error",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid_input",
            Self::Cancelled => "cancelled",
            Self::Provider(_) => "provider_error",
            Self::Http(_) => "http_error",
            Self::Store(_) => "store_error",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}
