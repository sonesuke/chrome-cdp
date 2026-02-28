//! Error types for chrome-cdp

/// Error type for CDP operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Browser-related errors
    #[error("Browser error: {0}")]
    Browser(String),

    /// CDP protocol errors
    #[error("CDP error: {0}")]
    Cdp(String),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP request errors
    #[error("HTTP error: {0}")]
    Http(String),

    /// JSON parsing errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// WebSocket errors
    #[error("WebSocket error: {0}")]
    WebSocket(String),
}

/// Result type for CDP operations
pub type Result<T> = std::result::Result<T, Error>;
