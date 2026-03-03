use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::task::JoinHandle;

use crate::db::{library as db_library, manga as db_manga, chapter as db_chapter, task as db_task};
use crate::db::task::{Task, TaskType};
use crate::downloader;
use crate::merge;
use crate::scraper::{ProviderRegistry, ScraperCtx};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn the background worker as a detached tokio task.
/// The worker loops indefinitely, polling for pending tasks every 5 seconds.
pub fn start(
    pool: SqlitePool,
    registry: Arc<ProviderRegistry>,
    ctx: ScraperCtx,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rate_limiter = RateLimiter::new(&registry);
        loop {
            match db_task::claim_next(&pool).await {
                Ok(Some(task)) => {
                    log::info!("[worker] Claimed task {:?} (id={})", task.task_type, task.id);

                    // Throttle based on provider(s) involved
                    throttle_for_task(&pool, &registry, &task, &mut rate_limiter).await;

                    let result = dispatch(&pool, &registry, &ctx, &task).await;
                    match result {
                        Ok(()) => {
                            if let Err(e) = db_task::complete(&pool, task.id).await {
                                log::error!("[worker] Failed to mark task complete: {e}");
                            }
                            log::info!("[worker] Task {} completed.", task.id);
                        }
                        Err(e) => {
                            log::warn!("[worker] Task {} failed: {e}", task.id);
                            if let Err(db_err) = db_task::fail(&pool, task.id, &e).await {
                                log::error!("[worker] Failed to record task failure: {db_err}");
                            }
                        }
                    }
                }
                Ok(None) => {
                    // Nothing ready — sleep before polling again
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    log::error!("[worker] Error polling task queue: {e}");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

async fn dispatch(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    task: &Task,
) -> Result<(), String> {
    match task.task_type {
        TaskType::ScanLibrary | TaskType::CheckNewChapter => {
            let manga_id = task.manga_id.ok_or("ScanLibrary task missing manga_id")?;
            let manga = db_manga::get_by_id(pool, manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {manga_id} not found"))?;

            let result = merge::scan_manga(pool, registry, ctx, &manga)
                .await
                .map_err(|e| e.to_string())?;

            log::info!(
                "[worker] Scan complete for '{}': {} providers, {} new chapters.",
                manga.metadata.title,
                result.providers_found,
                result.new_chapters
            );
            Ok(())
        }

        TaskType::DownloadChapter => {
            let chapter_id = task.chapter_id.ok_or("DownloadChapter task missing chapter_id")?;

            let chapter = db_chapter::get_by_id(pool, chapter_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("chapter {chapter_id} not found"))?;

            let manga = db_manga::get_by_id(pool, chapter.manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {} not found", chapter.manga_id))?;

            let library = db_library::get_by_id(pool, manga.library_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("library {} not found", manga.library_id))?;

            downloader::download_chapter(pool, registry, ctx, &manga, &chapter, &library.root_path)
                .await
                .map_err(|e| e.to_string())
        }

        // Not yet implemented task types — log and succeed silently
        TaskType::RefreshAniList | TaskType::Backup => {
            log::info!("[worker] Task type {:?} not yet implemented, skipping.", task.task_type);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Per-provider rate limiter. Tracks the last time we dispatched a task that
/// used each provider, and enforces the minimum inter-request interval derived
/// from `requests_per_minute`.
struct RateLimiter {
    /// provider_name → minimum interval between tasks
    intervals: HashMap<String, Duration>,
    /// provider_name → when we last dispatched a task for it
    last_used: HashMap<String, Instant>,
}

impl RateLimiter {
    fn new(registry: &ProviderRegistry) -> Self {
        let mut intervals = HashMap::new();
        for provider in registry.by_score() {
            // Minimum interval = 60_000ms / rpm  (floor at 100ms)
            let rpm = provider.rate_limit_rpm();
            let millis = if rpm > 0 { (60_000u64 / rpm as u64).max(100) } else { 2_000 };
            intervals.insert(provider.name().to_owned(), Duration::from_millis(millis));
        }
        Self {
            intervals,
            last_used: HashMap::new(),
        }
    }

    /// Sleep if we need to wait before making another request to `provider_name`.
    async fn throttle(&mut self, provider_name: &str) {
        let Some(&interval) = self.intervals.get(provider_name) else {
            return;
        };
        if let Some(&last) = self.last_used.get(provider_name) {
            let elapsed = last.elapsed();
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
        self.last_used.insert(provider_name.to_owned(), Instant::now());
    }
}

/// Throttle for all provider names associated with the target manga of a task.
async fn throttle_for_task(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    task: &Task,
    limiter: &mut RateLimiter,
) {
    // If task has a manga_id, throttle based on its cached providers
    if let Some(manga_id) = task.manga_id {
        if let Ok(entries) = crate::db::provider::get_all_for_manga(pool, manga_id).await {
            for entry in entries {
                // Only throttle if this provider is actually loaded
                if registry.by_score().iter().any(|p| p.name() == entry.provider_name) {
                    limiter.throttle(&entry.provider_name).await;
                }
            }
            return;
        }
    }
    // For chapter tasks, look up manga_id via chapter
    if let Some(chapter_id) = task.chapter_id {
        if let Ok(Some(chapter)) = db_chapter::get_by_id(pool, chapter_id).await {
            if let Ok(entries) =
                crate::db::provider::get_all_for_manga(pool, chapter.manga_id).await
            {
                for entry in entries {
                    if registry.by_score().iter().any(|p| p.name() == entry.provider_name) {
                        limiter.throttle(&entry.provider_name).await;
                    }
                }
            }
        }
    }
}
