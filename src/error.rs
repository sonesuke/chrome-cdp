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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_error_browser_creation() {
        let err = Error::Browser("test error".to_string());
        assert_eq!(err.to_string(), "Browser error: test error");
    }

    #[test]
    fn test_error_cdp_creation() {
        let err = Error::Cdp("protocol error".to_string());
        assert_eq!(err.to_string(), "CDP error: protocol error");
    }

    #[test]
    fn test_error_http_creation() {
        let err = Error::Http("connection failed".to_string());
        assert_eq!(err.to_string(), "HTTP error: connection failed");
    }

    #[test]
    fn test_error_websocket_creation() {
        let err = Error::WebSocket("handshake failed".to_string());
        assert_eq!(err.to_string(), "WebSocket error: handshake failed");
    }

    #[test]
    fn test_error_io_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_error_json_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Json(_)));
    }

    #[test]
    fn test_result_type_alias_ok() {
        let result: Result<i32> = Ok(42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_result_type_alias_err() {
        let result: Result<i32> = Err(Error::Browser("failed".to_string()));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::Browser(_)));
    }

    #[test]
    fn test_error_display_formatting() {
        assert_eq!(
            format!("{}", Error::Browser("test".to_string())),
            "Browser error: test"
        );
        assert_eq!(
            format!("{}", Error::Cdp("test".to_string())),
            "CDP error: test"
        );
        assert_eq!(
            format!("{}", Error::Http("test".to_string())),
            "HTTP error: test"
        );
        assert_eq!(
            format!("{}", Error::WebSocket("test".to_string())),
            "WebSocket error: test"
        );
    }
}
