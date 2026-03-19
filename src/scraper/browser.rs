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

        let config = StealthConfig {
            headless: true,
            // patch_binary modifies the Chrome binary on disk (~400 MB copy).
            // Disabled to avoid issues in environments with read-only Chrome installs.
            // eoka's CDP command filtering provides substantial evasion without it.
            patch_binary: false,
            ..StealthConfig::default()
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
        *guard = None;
    }
}

impl Default for BrowserPool {
    fn default() -> Self {
        Self::new()
    }
}
