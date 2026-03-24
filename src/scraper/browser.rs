use std::sync::Arc;

use eoka::{Browser, StealthConfig};
use tokio::sync::Mutex;

use crate::scraper::error::ScraperError;

/// A shared, lazily-started headless Chromium instance.
///
/// Clone this freely — the inner `Arc` keeps one browser alive for the process
/// lifetime. The browser is only launched on the first call to `get()`, so
/// startup cost is zero if no provider ever needs JavaScript rendering.
///
/// Unlike a `OnceCell`, this can be `reset()` so that the next `get()` call
/// re-launches Chromium after a CDP transport failure.
#[derive(Clone)]
pub struct BrowserPool {
    inner: Arc<Mutex<Option<Arc<Browser>>>>,
}

impl BrowserPool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the shared browser, starting Chromium if it is not running.
    pub async fn get(&self) -> Result<Arc<Browser>, ScraperError> {
        let mut guard = self.inner.lock().await;
        if let Some(ref browser) = *guard {
            return Ok(Arc::clone(browser));
        }

        let headless = std::env::var("CHROME_HEADLESS")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true);

        let config = StealthConfig {
            headless,
            webgl_spoof: true,
            canvas_spoof: true,
            audio_spoof: true,
            human_mouse: true,
            human_typing: true,
            user_agent: Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.7680.153 Safari/537.36".to_string()),
            chrome_path: None,
            patch_binary: false, // This might break on non-writable installs
            viewport_width: 1366,
            viewport_height: 768,
            debug: false,
            debug_dir: None,
            proxy: None,
            proxy_username: None,
            proxy_password: None,
            cdp_timeout: 30,
            timezone: Some("Australia/Melbourne".to_string()),
        };

        let browser = Browser::launch_with_config(config)
            .await
            .map_err(|e| ScraperError::Browser(e.to_string()))?;

        let browser = Arc::new(browser);
        *guard = Some(Arc::clone(&browser));
        Ok(browser)
    }

    /// Discard the current browser instance.
    ///
    /// The next call to `get()` will launch a fresh Chromium process. Call
    /// this when a CDP transport error indicates the connection is dead.
    pub async fn reset(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(browser_arc) = guard.take() {
            // If we're the sole owner, call close() which sends a graceful
            // CDP shutdown and waits for the Chrome process to exit (no zombies).
            // If other code still holds the Arc, just drop our reference and
            // rely on Transport::drop() for cleanup.
            match Arc::try_unwrap(browser_arc) {
                Ok(browser) => {
                    let _ = browser.close().await;
                }
                Err(_arc) => {
                    // Arc still shared — drop it; Transport::drop will kill Chrome
                    // when the last reference goes away.
                }
            }
        }
    }
}

impl Default for BrowserPool {
    fn default() -> Self {
        Self::new()
    }
}
