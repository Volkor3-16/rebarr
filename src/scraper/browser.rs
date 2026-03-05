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
                // Use a per-process user-data-dir so concurrent or successive
                // runs never hit a stale SingletonLock from a previous crash.
                // Use user_data_dir() on the builder (not a raw --user-data-dir
                // arg) so chromiumoxide doesn't also create /tmp/chromiumoxide-runner.
                let user_data_dir =
                    format!("/tmp/rebarr-chrome-{}", std::process::id());

                // Remove any stale SingletonLock left by a previous crash with
                // the same PID (rare, but possible on PID reuse).
                let _ = std::fs::remove_file(format!("{user_data_dir}/SingletonLock"));

                let config = BrowserConfig::builder()
                    .no_sandbox()
                    .user_data_dir(&user_data_dir)
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
