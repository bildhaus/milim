//! The crate-wide error type.

use std::fmt;

/// Convenient `Result` alias used throughout milim.
pub type Result<T> = std::result::Result<T, Error>;

/// Top-level error type. Variants map onto HTTP error responses in `milim-server`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request was malformed or failed validation (HTTP 400).
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// A requested model is not available locally or remotely (HTTP 404).
    #[error("model not found: {0}")]
    ModelNotFound(String),

    /// A requested non-model resource does not exist (HTTP 404).
    #[error("not found: {0}")]
    NotFound(String),

    /// Authentication failed or was missing (HTTP 401).
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// An upstream/remote provider returned an error.
    #[error("upstream error: {0}")]
    Upstream(String),

    /// The inference backend failed while generating.
    #[error("inference error: {0}")]
    Inference(String),

    /// I/O failure (config, model files, sockets).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// (De)serialization failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Anything that doesn't fit the buckets above.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Short, stable machine code used in OpenAI-style error envelopes.
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidRequest(_) => "invalid_request_error",
            Error::ModelNotFound(_) => "model_not_found",
            Error::NotFound(_) => "not_found",
            Error::Unauthorized(_) => "authentication_error",
            Error::Upstream(_) => "upstream_error",
            Error::Inference(_) => "inference_error",
            Error::Io(_) => "io_error",
            Error::Json(_) => "invalid_request_error",
            Error::Other(_) => "internal_error",
        }
    }
}

/// Helper to build an [`Error::Other`] from anything displayable.
pub fn other(msg: impl fmt::Display) -> Error {
    Error::Other(msg.to_string())
}
