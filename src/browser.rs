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
    #[allow(dead_code)]
    ws_url: String,
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

        Ok(Self {
            process: Some(process),
            port: discovered_port,
            ws_url,
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
