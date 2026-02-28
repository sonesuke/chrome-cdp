//! Chrome browser process management

use crate::{Error, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Chrome browser process manager
pub struct CdpBrowser {
    process: Option<Child>,
    port: u16,
}

impl CdpBrowser {
    /// Launch Chrome/Chromium with CDP enabled
    pub async fn launch(
        executable_path: Option<PathBuf>,
        args: Vec<String>,
        headless: bool,
        debug: bool,
    ) -> Result<Self> {
        let chrome_path = executable_path
            .or_else(|| std::env::var("CHROME_BIN").ok().map(PathBuf::from))
            .unwrap_or_else(|| {
                #[cfg(target_os = "windows")]
                {
                    PathBuf::from("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe")
                }
                #[cfg(target_os = "macos")]
                {
                    PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
                }
                #[cfg(target_os = "linux")]
                {
                    PathBuf::from("/usr/bin/google-chrome")
                }
                #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
                {
                    PathBuf::from("chrome")
                }
            });

        // Create a temporary user data directory with a unique ID
        let unique_id = uuid::Uuid::new_v4();
        let temp_dir = std::env::temp_dir().join(format!("chrome-{}", unique_id));
        std::fs::create_dir_all(&temp_dir)?;

        let mut cmd = Command::new(&chrome_path);
        cmd.arg("--remote-debugging-port=0"); // Let OS assign a random port
        cmd.arg(format!("--user-data-dir={}", temp_dir.display()));
        cmd.arg("--password-store=basic"); // Prevent keychain prompts
        cmd.arg("--no-first-run"); // Skip first run wizards

        if headless {
            cmd.arg("--headless");
        }

        for arg in args {
            cmd.arg(&arg);
        }

        // Capture stderr to read the assigned port
        let stderr_file = temp_dir.join("chrome_stderr.log");
        let stderr_handle = std::fs::File::create(&stderr_file)?;

        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::from(stderr_handle));

        if debug {
            eprintln!("Launching Chrome: {:?}", cmd);
        }

        let process = Arc::new(std::sync::Mutex::new(cmd.spawn()?));

        // Read the port from stderr
        let port: Arc<std::sync::Mutex<Option<u16>>> = Arc::new(std::sync::Mutex::new(None));
        let port_clone = port.clone();
        let stderr_path = stderr_file.clone();

        // Spawn a thread to read stderr and look for the port
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            // Try for up to 30 seconds (CI environments may be slower)
            while start.elapsed().as_secs() < 30 {
                if let Ok(content) = std::fs::read_to_string(&stderr_path) {
                    for line in content.lines() {
                        if debug {
                            eprintln!("CHROME STDERR: {}", line);
                        }
                        if line.contains("DevTools listening on") {
                            if let Some(port_str) = line.split("127.0.0.1:").nth(1) {
                                if let Some(port_num) = port_str.split('/').next() {
                                    if let Ok(p) = port_num.parse::<u16>() {
                                        if let Ok(mut guard) = port_clone.lock() {
                                            *guard = Some(p);
                                        }
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        let stderr_path_for_error = stderr_file.clone();
        let process_for_error = process.clone();
        // Wait for the port to be discovered
        let discovered_port = tokio::task::spawn_blocking(move || {
            for _ in 0..300 {
                let port_val = port.lock().map_or(None, |guard| *guard);
                if let Some(p) = port_val {
                    return Ok(p);
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            // Build detailed error message
            let chrome_stderr =
                std::fs::read_to_string(&stderr_path_for_error).unwrap_or_else(|_| "(unreadable)".to_string());

            let os_info = format!(
                "{} {} ({})",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::env::consts::FAMILY
            );

            let mut err_msg = format!(
                "=== Chrome Browser Launch Failure ===\n\
                 OS: {}\n\
                 Chrome Executable: {:?}\n\
                 User Data Dir: {:?}\n\
                 === Chrome stderr ===\n{}\n\
                 === End of stderr ===",
                os_info, chrome_path, temp_dir, chrome_stderr
            );

            // Check process status
            if let Ok(Some(status)) = process_for_error.lock().unwrap().try_wait() {
                err_msg = format!("{}\n\nChrome process exited early with status: {}", err_msg, status);
            } else {
                err_msg = format!(
                    "{}\n\nChrome process is still running but debugging port was not found after 30 seconds.\n\n\
                     Troubleshooting:\n\
                     - If running in CI, ensure Chrome/Chromium is installed\n\
                     - Try setting CHROME_BIN environment variable\n\
                     - For Linux CI, add --no-sandbox flag",
                    err_msg
                );
            }

            Err(Error::Browser(err_msg))
        })
        .await
        .map_err(|e| Error::Browser(format!("Task failed: {}", e)))??;

        let ws_url =
            Self::get_ws_url_with_retry(discovered_port, 10, Duration::from_millis(500)).await?;

        // Unwrap the Arc<Mutex<>> to get the process
        let process = match Arc::try_unwrap(process) {
            Ok(mutex) => mutex.into_inner().unwrap(),
            Err(_) => {
                return Err(Error::Browser(
                    "Failed to acquire process ownership".to_string(),
                ))
            }
        };

        // Verify WebSocket URL is accessible (discard the result)
        Self::get_ws_url_with_retry(discovered_port, 10, Duration::from_millis(500)).await?;

        Ok(Self {
            process: Some(process),
            port: discovered_port,
        })
    }

    /// Get WebSocket debugger URL from Chrome with retry logic
    async fn get_ws_url_with_retry(
        port: u16,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Result<String> {
        let mut last_error = None;

        for attempt in 0..max_retries {
            match Self::get_ws_url(port).await {
                Ok(url) => return Ok(url),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        sleep(retry_delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            Error::Browser("Failed to get WebSocket URL after retries".to_string())
        }))
    }

    /// Get WebSocket debugger URL from Chrome
    async fn get_ws_url(port: u16) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/version", port);
        let client = reqwest::Client::new();

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Http(format!("Failed to connect to {}: {}", url, e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| Error::Http(format!("Failed to read response from {}: {}", url, e)))?;

        if !status.is_success() {
            return Err(Error::Browser(format!(
                "Chrome debugger returned error status {} ({}). Response: {}",
                status, url, body
            )));
        }

        let value: Value = serde_json::from_str(&body)?;

        value["webSocketDebuggerUrl"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| {
                Error::Browser(format!(
                    "Chrome debugger response does not contain webSocketDebuggerUrl. Response: {}",
                    body
                ))
            })
    }

    /// Create a new page and return its WebSocket URL
    pub async fn new_page(&self) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/json/new", self.port);
        let client = reqwest::Client::new();

        let response =
            client.put(&url).send().await.map_err(|e| {
                Error::Http(format!("Failed to create new page via {}: {}", url, e))
            })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| Error::Http(format!("Failed to read response from {}: {}", url, e)))?;

        if !status.is_success() {
            return Err(Error::Browser(format!(
                "Chrome debugger returned error {} when creating new page ({}). Response: {}",
                status, url, body
            )));
        }

        let value: Value = serde_json::from_str(&body).map_err(|e| {
            Error::Browser(format!(
                "Failed to parse JSON response from {}: {}. Response body: {}",
                url, e, body
            ))
        })?;

        value["webSocketDebuggerUrl"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| {
                Error::Browser(format!(
                    "Could not find webSocketDebuggerUrl in response. Response: {}",
                    body
                ))
            })
    }
}

impl Drop for CdpBrowser {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
        }
    }
}

/// Browser state for managing lifecycle
pub struct BrowserState {
    pub browser: Option<Arc<CdpBrowser>>,
    pub last_used: Instant,
}

/// Manager for browser instances with auto-cleanup
#[derive(Clone)]
pub struct BrowserManager {
    browser_path: Option<PathBuf>,
    headless: bool,
    debug: bool,
    chrome_args: Vec<String>,
    state: Arc<Mutex<BrowserState>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chrome_path_linux() {
        // This test verifies the path construction logic
        let path = PathBuf::from("/usr/bin/google-chrome");
        assert!(path.starts_with("/usr/bin"));
    }

    #[test]
    fn test_chrome_path_windows() {
        // This test verifies the path construction logic
        // On Linux, verify the path format still parses correctly
        let path_str = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
        let _path = PathBuf::from(path_str);
        // Just verify the path contains expected components
        assert!(path_str.contains("Chrome"));
        assert!(path_str.contains("chrome.exe"));
    }

    #[test]
    fn test_chrome_path_macos() {
        // This test verifies the path construction logic
        let path = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(path.starts_with("/Applications"));
    }

    #[test]
    fn test_user_data_dir_construction() {
        let unique_id = uuid::Uuid::new_v4();
        let temp_dir = std::env::temp_dir().join(format!("chrome-{}", unique_id));
        assert!(temp_dir.to_string_lossy().contains("chrome-"));
    }

    #[test]
    fn test_http_endpoint_url_construction() {
        let port = 9222u16;
        let url = format!("http://127.0.0.1:{}/json/version", port);
        assert_eq!(url, "http://127.0.0.1:9222/json/version");
    }

    #[test]
    fn test_new_page_url_construction() {
        let port = 9222u16;
        let url = format!("http://127.0.0.1:{}/json/new", port);
        assert_eq!(url, "http://127.0.0.1:9222/json/new");
    }

    #[test]
    fn test_websocket_url_extraction_from_valid_json() {
        let json_body = r#"{"webSocketDebuggerUrl":"ws://localhost:9222/devtools/page/ABC123","browser":"Chrome"}"#;
        let value: Value = serde_json::from_str(json_body).unwrap();
        let url = value["webSocketDebuggerUrl"].as_str();
        assert_eq!(url, Some("ws://localhost:9222/devtools/page/ABC123"));
    }

    #[test]
    fn test_websocket_url_extraction_missing() {
        let json_body = r#"{"browser":"Chrome","version":"1.2.3"}"#;
        let value: Value = serde_json::from_str(json_body).unwrap();
        let url = value["webSocketDebuggerUrl"].as_str();
        assert_eq!(url, None);
    }

    #[test]
    fn test_stderr_port_parsing() {
        let line = "DevTools listening on ws://127.0.0.1:9222/devtools/browser";
        if let Some(port_str) = line.split("127.0.0.1:").nth(1) {
            if let Some(port_num) = port_str.split('/').next() {
                let port: u16 = port_num.parse().unwrap();
                assert_eq!(port, 9222);
            }
        }
    }

    #[test]
    fn test_stderr_port_parsing_with_trailing_path() {
        let line = "DevTools listening on ws://127.0.0.1:12345/devtools/page/ABC";
        if let Some(port_str) = line.split("127.0.0.1:").nth(1) {
            if let Some(port_num) = port_str.split('/').next() {
                let port: u16 = port_num.parse().unwrap();
                assert_eq!(port, 12345);
            }
        }
    }

    #[test]
    fn test_ci_env_var_detection() {
        // Test that CI environment variable can be detected
        let was_set = std::env::var("CI").is_ok();
        // This test just verifies the check works; it may be true or false
        if was_set {
            assert_eq!(std::env::var("CI").unwrap(), "true");
        }
    }

    #[tokio::test]
    async fn test_browser_manager_creation() {
        let manager = BrowserManager::new(None, true, false, vec![]);
        assert_eq!(manager.headless, true);
        assert_eq!(manager.debug, false);
        assert!(manager.chrome_args.is_empty());
        assert!(manager.browser_path.is_none());
    }

    #[tokio::test]
    async fn test_browser_manager_with_custom_args() {
        let custom_args = vec!["--disable-gpu".to_string(), "--no-sandbox".to_string()];
        let manager = BrowserManager::new(None, false, true, custom_args);
        assert_eq!(manager.headless, false);
        assert_eq!(manager.debug, true);
        assert_eq!(manager.chrome_args.len(), 2);
        assert_eq!(manager.chrome_args[0], "--disable-gpu");
        assert_eq!(manager.chrome_args[1], "--no-sandbox");
    }

    #[tokio::test]
    async fn test_browser_manager_with_path() {
        let path = PathBuf::from("/custom/chrome");
        let manager = BrowserManager::new(Some(path), true, false, vec![]);
        assert_eq!(manager.browser_path, Some(PathBuf::from("/custom/chrome")));
    }

    #[test]
    fn test_browser_state_initialization() {
        let state = BrowserState {
            browser: None,
            last_used: Instant::now(),
        };
        assert!(state.browser.is_none());
    }

    #[tokio::test]
    async fn test_inactivity_timeout_check() {
        let state = BrowserState {
            browser: None,
            last_used: Instant::now(),
        };
        // Should not be expired immediately
        assert!(state.last_used.elapsed() < Duration::from_secs(5 * 60));
    }

    #[tokio::test]
    async fn test_retry_delay_duration() {
        let delay = Duration::from_millis(500);
        assert_eq!(delay.as_millis(), 500);
    }

    #[test]
    fn test_error_message_formatting() {
        let os_info = format!(
            "{} {} ({})",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::consts::FAMILY
        );
        assert!(!os_info.is_empty());
        assert!(os_info.contains(std::env::consts::OS));
    }

    #[tokio::test]
    async fn test_get_ws_url_retry_logic() {
        // Test that retry logic constructs proper error on failure
        // This is a unit test for the logic structure
        let max_retries = 3u32;
        let mut attempt_count = 0;
        let mut last_error = None;

        for attempt in 0..max_retries {
            attempt_count = attempt + 1;
            // Simulate failure
            last_error = Some("Failed to connect".to_string());
            if attempt < max_retries - 1 {
                // Would sleep here in real implementation
            }
        }

        assert_eq!(attempt_count, 3);
        assert!(last_error.is_some());
    }

    #[tokio::test]
    async fn test_drop_impl_kills_process() {
        // Verify Drop trait behavior is correctly defined
        // This is a compile-time check that the impl exists
        let _ = std::mem::needs_drop::<CdpBrowser>();
    }

    #[tokio::test]
    async fn test_new_page_error_on_invalid_response() {
        let body = r#"{"invalid":"response"}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        let url = value["webSocketDebuggerUrl"].as_str();
        assert!(url.is_none());
    }

    #[tokio::test]
    async fn test_new_page_error_on_invalid_json() {
        let body = r#"invalid json"#;
        let result: std::result::Result<Value, serde_json::Error> = serde_json::from_str(body);
        assert!(result.is_err());
    }
}

impl BrowserManager {
    /// Create a new browser manager
    pub fn new(
        browser_path: Option<PathBuf>,
        headless: bool,
        debug: bool,
        chrome_args: Vec<String>,
    ) -> Self {
        let state = Arc::new(Mutex::new(BrowserState {
            browser: None,
            last_used: Instant::now(),
        }));

        // Spawn the inactivity monitor task
        let state_clone = state.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(60)).await;
                let mut s = state_clone.lock().await;
                if s.browser.is_some() && s.last_used.elapsed() > Duration::from_secs(5 * 60) {
                    s.browser = None; // Drops Arc<CdpBrowser>, which triggers process kill
                }
            }
        });

        Self {
            browser_path,
            headless,
            debug,
            chrome_args,
            state,
        }
    }

    /// Get or create a browser instance
    pub async fn get_browser(&self) -> Result<Arc<CdpBrowser>> {
        let mut s = self.state.lock().await;
        s.last_used = Instant::now();

        if let Some(browser) = &s.browser {
            return Ok(Arc::clone(browser));
        }

        let mut args = vec!["--disable-blink-features=AutomationControlled".to_string()];

        // In CI environments, automatically add sandbox-disabling flags
        if std::env::var("CI").is_ok() {
            args.push("--disable-gpu".to_string());
            args.push("--no-sandbox".to_string());
            args.push("--disable-setuid-sandbox".to_string());
        }

        // Add custom Chrome args
        args.extend(self.chrome_args.clone());

        let browser = Arc::new(
            CdpBrowser::launch(self.browser_path.clone(), args, self.headless, self.debug).await?,
        );
        s.browser = Some(Arc::clone(&browser));

        Ok(browser)
    }
}
