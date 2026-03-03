use std::sync::Arc;

use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
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
                let config = BrowserConfig::builder()
                    .no_sandbox()
                    .build()
                    .map_err(|e| ScraperError::Browser(e.to_string()))?;

                let (browser, mut handler) = Browser::launch(config)
                    .await
                    .map_err(|e| ScraperError::Browser(e.to_string()))?;

                // The CDP handler must be driven continuously. Spawn it for the
                // process lifetime — it exits when the browser process does.
                tokio::spawn(async move {
                    while let Some(event) = handler.next().await {
                        if event.is_err() {
                            break;
                        }
                    }
                });

                Ok(Arc::new(browser))
            })
            .await
            .cloned()
    }
}
