/// Every error the core can return. Exposed to Swift as `UniHubError`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum UniHubError {
    #[error("I/O error: {message}")]
    Io { message: String },

    #[error("invalid data: {message}")]
    InvalidData { message: String },

    #[error("not a Unity project (needs Assets/ and ProjectSettings/): {path}")]
    NotAProject { path: String },

    #[error("project is not in the list: {path}")]
    UnknownProject { path: String },

    #[error("Unity {version} is not installed")]
    EditorNotInstalled { version: String },

    #[error("project has no editor version and none was given: {path}")]
    NoEditorVersion { path: String },

    #[error("network error: {message}")]
    Network { message: String },

    #[error("the Unity CLI is not installed")]
    CliNotInstalled,

    /// The Unity CLI ran but reported a failure. `code` is the CLI's error code,
    /// e.g. `AUTH_ERROR`, or `exit-<n>` when it printed no structured error.
    #[error("Unity CLI: {message}")]
    CliFailed { code: String, message: String },

    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, UniHubError>;

impl From<std::io::Error> for UniHubError {
    fn from(e: std::io::Error) -> Self {
        Self::Io { message: e.to_string() }
    }
}

impl From<serde_json::Error> for UniHubError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidData { message: e.to_string() }
    }
}

impl From<reqwest::Error> for UniHubError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network { message: e.to_string() }
    }
}
