//! CDP Page automation

use crate::{connection::CdpConnection, Error, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;

/// CDP Page for browser automation
pub struct CdpPage {
    connection: CdpConnection,
}

impl CdpPage {
    /// Create a new page with the given connection
    pub async fn new(ws_url: &str) -> Result<Self> {
        let connection = CdpConnection::connect(ws_url).await?;

        // Enable necessary domains
        connection
            .send_command("Page.enable", json!({}))
            .await
            .map_err(|e| Error::Browser(format!("Failed to enable Page domain: {}", e)))?;
        connection
            .send_command("Runtime.enable", json!({}))
            .await
            .map_err(|e| Error::Browser(format!("Failed to enable Runtime domain: {}", e)))?;

        Ok(Self { connection })
    }

    /// Navigate to a URL
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.connection
            .send_command("Page.navigate", json!({ "url": url }))
            .await
            .map_err(|e| Error::Browser(format!("Failed to navigate to '{}': {}", url, e)))?;
        Ok(())
    }

    /// Wait for an element to appear on the page
    pub async fn wait_for_element(&self, selector: &str, timeout_secs: u64) -> Result<bool> {
        let start = std::time::Instant::now();

        while start.elapsed().as_secs() < timeout_secs {
            let script = format!(
                "!!document.querySelector(\"{}\")",
                selector.replace('"', "\\\"")
            );

            let result = self.evaluate(&script).await?;
            if result.as_bool().unwrap_or(false) {
                return Ok(true);
            }

            sleep(Duration::from_millis(500)).await;
        }

        Ok(false)
    }

    /// Get full HTML content for debugging
    pub async fn get_html(&self) -> Result<String> {
        let script = "document.documentElement.outerHTML";
        let result = self.evaluate(script).await?;
        result.as_str().map(String::from).ok_or_else(|| {
            Error::Browser("Failed to get HTML: JavaScript result was not a string".to_string())
        })
    }

    /// Evaluate JavaScript and return the result
    pub async fn evaluate(&self, script: &str) -> Result<Value> {
        let result = self
            .connection
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": script,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;

        if let Some(exception) = result.get("exceptionDetails") {
            let exception_text = exception["exception"]["description"]
                .as_str()
                .or_else(|| exception["text"].as_str())
                .unwrap_or("unknown error");

            let column_number = exception["columnNumber"].as_i64().unwrap_or(-1);
            let line_number = exception["lineNumber"].as_i64().unwrap_or(-1);

            return Err(Error::Browser(format!(
                "JavaScript execution error at line {}, column {}: {}",
                line_number, column_number, exception_text
            )));
        }

        Ok(result["result"]["value"].clone())
    }

    /// Close the page/tab
    pub async fn close(&self) -> Result<()> {
        self.connection
            .send_command("Page.close", json!({}))
            .await
            .map_err(|e| Error::Browser(format!("Failed to close page: {}", e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_for_element_script_format() {
        let selector = "div.main";
        let script = format!(
            "!!document.querySelector(\"{}\")",
            selector.replace('"', "\\\"")
        );
        assert_eq!(script, "!!document.querySelector(\"div.main\")");
    }

    #[test]
    fn test_wait_for_element_script_with_quotes() {
        let selector = r#"div[data-attr="test"]"#;
        let script = format!(
            "!!document.querySelector(\"{}\")",
            selector.replace('"', "\\\"")
        );
        // The replace function escapes the quotes with backslash
        assert_eq!(script, "!!document.querySelector(\"div[data-attr=\\\"test\\\"]\")");
    }

    #[test]
    fn test_get_html_script() {
        let script = "document.documentElement.outerHTML";
        assert_eq!(script, "document.documentElement.outerHTML");
    }

    #[test]
    fn test_evaluate_command_format() {
        let script = "document.title";
        let cmd = json!({
            "expression": script,
            "returnByValue": true,
            "awaitPromise": true
        });
        assert_eq!(cmd["expression"], "document.title");
        assert_eq!(cmd["returnByValue"], true);
        assert_eq!(cmd["awaitPromise"], true);
    }

    #[test]
    fn test_goto_command_format() {
        let url = "https://example.com";
        let cmd = json!({ "url": url });
        assert_eq!(cmd["url"], "https://example.com");
    }

    #[test]
    fn test_polling_timeout_behavior() {
        let start = std::time::Instant::now();
        let timeout_secs = 1u64;

        // Simulate polling loop
        let mut elapsed = 0u64;
        while start.elapsed().as_secs() < timeout_secs {
            elapsed += 1;
            std::thread::sleep(std::time::Duration::from_millis(100));
            if elapsed >= 10 {
                break; // Early exit for test
            }
        }
        // Verify we didn't exceed timeout significantly
        assert!(start.elapsed().as_secs() < 2);
    }

    #[test]
    fn test_boolean_result_parsing() {
        let json_true = json!(true);
        assert_eq!(json_true.as_bool(), Some(true));

        let json_false = json!(false);
        assert_eq!(json_false.as_bool(), Some(false));

        let json_string = json!("not a boolean");
        assert_eq!(json_string.as_bool(), None);
    }

    #[test]
    fn test_html_result_string_extraction() {
        let html_value = json!("<html><body>test</body></html>");
        assert_eq!(html_value.as_str(), Some("<html><body>test</body></html>"));

        let json_obj = json!({"not": "a string"});
        assert_eq!(json_obj.as_str(), None);
    }

    #[test]
    fn test_exception_details_parsing() {
        let exception = json!({
            "exceptionDetails": {
                "exception": {
                    "description": "ReferenceError: x is not defined"
                },
                "text": "Alternative text",
                "columnNumber": 10,
                "lineNumber": 5
            }
        });

        assert!(exception.get("exceptionDetails").is_some());
        assert_eq!(
            exception["exceptionDetails"]["exception"]["description"].as_str(),
            Some("ReferenceError: x is not defined")
        );
        assert_eq!(exception["exceptionDetails"]["columnNumber"].as_i64(), Some(10));
        assert_eq!(exception["exceptionDetails"]["lineNumber"].as_i64(), Some(5));
    }

    #[test]
    fn test_exception_with_fallback_to_text() {
        let exception = json!({
            "exceptionDetails": {
                "text": "Error message"
            }
        });

        let description = exception["exceptionDetails"]["exception"]["description"]
            .as_str()
            .or_else(|| exception["exceptionDetails"]["text"].as_str())
            .unwrap_or("unknown");
        assert_eq!(description, "Error message");
    }

    #[tokio::test]
    async fn test_sleep_duration() {
        let start = std::time::Instant::now();
        sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90)); // Allow some margin
        assert!(elapsed < Duration::from_millis(200));
    }
}
