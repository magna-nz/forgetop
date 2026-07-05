//! Core error type shared across the app.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// A provider/network call failed.
    #[error("provider error: {0}")]
    Provider(String),

    /// Configuration or binding problem.
    #[error("config error: {0}")]
    Config(String),

    /// Requested entity was not found.
    #[error("not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
