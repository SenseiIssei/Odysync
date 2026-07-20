//! Error type shared across the workspace.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// The backend's underlying tool is not installed or not on PATH.
    #[error("{backend} is not available on this system: {detail}")]
    BackendUnavailable { backend: String, detail: String },

    /// A spawned process returned a non-zero exit code.
    #[error("{command} exited with code {code}: {stderr}")]
    CommandFailed {
        command: String,
        code: i32,
        stderr: String,
    },

    /// A spawned process exceeded its time budget and was killed.
    #[error("{command} timed out after {seconds}s")]
    CommandTimeout { command: String, seconds: u64 },

    /// Output from a package manager did not match any known shape.
    #[error("could not parse {what}: {detail}")]
    Parse { what: String, detail: String },

    /// An integrity or signature check failed; nothing was installed.
    #[error("verification failed for {package}: {detail}")]
    Verification { package: String, detail: String },

    #[error("configuration error: {0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn parse(what: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::Parse {
            what: what.into(),
            detail: detail.into(),
        }
    }

    pub fn unavailable(backend: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::BackendUnavailable {
            backend: backend.into(),
            detail: detail.into(),
        }
    }
}
