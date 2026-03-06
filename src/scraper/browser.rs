use std::sync::Arc;

use eoka::{Browser, StealthConfig};
use tokio::sync::OnceCell;

use crate::scraper::error::ScraperError;

/// A shared, lazily-started headless Chromium instance.
///
/// Clone this freely — the inner `Arc` keeps one browser alive for the process
/// lifetime. The browser is only launched on the first call to `get()`, so
/// startup cost is zero if no provider ever needs JavaScript rendering.
#[derive(Clone)]
pub struct BrowserPool {
    inner: Arc<OnceCell<Arc<Browser>>>,
}

impl BrowserPool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(OnceCell::new()),
        }
    }

    /// Returns the shared browser, starting Chromium on the first call.
    pub async fn get(&self) -> Result<Arc<Browser>, ScraperError> {
        self.inner
            .get_or_try_init(|| async {
                let config = StealthConfig {
                    headless: false,
                    // patch_binary modifies the Chrome binary on disk (~400 MB copy).
                    // Disabled to avoid issues in environments with read-only Chrome installs.
                    // eoka's CDP command filtering provides substantial evasion without it.
                    patch_binary: false,
                    ..StealthConfig::default()
                };

                let browser = Browser::launch_with_config(config)
                    .await
                    .map_err(|e| ScraperError::Browser(e.to_string()))?;

                Ok(Arc::new(browser))
            })
            .await
            .cloned()
    }
}
