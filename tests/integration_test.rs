//! Integration tests with real Chrome/Chromium browser
//!
//! These tests require Chrome/Chromium to be installed and are feature-gated
//! with the "integration-tests" feature.

#[cfg(feature = "integration-tests")]
mod chrome_tests {
    use chrome_cdp::{BrowserManager, CdpPage};
    use std::path::PathBuf;

    /// Create a BrowserManager with flags for containerized environments
    fn create_manager() -> BrowserManager {
        let args = vec![
            "--no-sandbox".to_string(),
            "--disable-gpu".to_string(),
            "--disable-setuid-sandbox".to_string(),
        ];
        BrowserManager::new(Some(PathBuf::from("/usr/bin/chromium")), true, false, args)
    }

    #[tokio::test]
    async fn test_browser_launch_and_close() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();
        // Browser should be alive - if we got here, launch succeeded
        drop(browser);
    }

    #[tokio::test]
    async fn test_page_create_and_close() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        assert!(ws_url.starts_with("ws://"));

        let page = CdpPage::new(&ws_url).await.unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_page_navigate_and_evaluate() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        // Navigate to a data URL (no network needed)
        page.goto("data:text/html,<html><body>Hello</body></html>")
            .await
            .unwrap();

        // Evaluate JavaScript
        let result = page.evaluate("document.body.innerText").await.unwrap();
        assert_eq!(result.as_str(), Some("Hello"));

        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_page_get_html() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        page.goto(
            "data:text/html,<html><head><title>Test</title></head><body>Content</body></html>",
        )
        .await
        .unwrap();

        let html = page.get_html().await.unwrap();
        assert!(html.contains("<html>"));
        assert!(html.contains("Content"));

        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_page_wait_for_element_timeout() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        page.goto("data:text/html,<html><body></body></html>")
            .await
            .unwrap();

        // Wait for non-existent element - should timeout and return false
        let found = page.wait_for_element("#non-existent", 2).await.unwrap();
        assert!(!found);

        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_page_wait_for_element_found() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        page.goto("data:text/html,<html><body><div id='target'>Found</div></body></html>")
            .await
            .unwrap();

        // Wait for existing element - should return true
        let found = page.wait_for_element("#target", 2).await.unwrap();
        assert!(found);

        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_evaluate_error_handling() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        page.goto("data:text/html,<html><body></body></html>")
            .await
            .unwrap();

        // Evaluate invalid JavaScript - should return an error
        let result = page.evaluate("throw new Error('test error')").await;
        assert!(result.is_err());

        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_page_evaluate_complex_js() {
        let manager = create_manager();
        let browser = manager.get_browser().await.unwrap();

        let ws_url = browser.new_page().await.unwrap();
        let page = CdpPage::new(&ws_url).await.unwrap();

        page.goto("data:text/html,<html><body></body></html>")
            .await
            .unwrap();

        // Create element via JavaScript
        page.evaluate("document.body.appendChild(document.createElement('div'))")
            .await
            .unwrap();

        // Verify element exists
        let result = page
            .evaluate("document.querySelector('div') !== null")
            .await
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));

        page.close().await.unwrap();
    }
}

// Non-feature-gated test that always runs but skips if feature not enabled
#[cfg(not(feature = "integration-tests"))]
mod integration_disabled {
    #[tokio::test]
    async fn test_integration_feature_not_enabled() {
        // This test runs when the integration-tests feature is not enabled
        // It's a placeholder to show that integration tests exist but are disabled
    }
}
