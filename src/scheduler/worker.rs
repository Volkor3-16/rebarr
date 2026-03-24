// This file handles:
// - All potential tasks that can be loaded into the queue

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::task::{Task, TaskType};
use crate::db::{
    chapter as db_chapter, library as db_library, manga as db_manga, settings as db_settings,
    task as db_task,
};
use crate::http::anilist::ALClient;
use crate::library::scanner::scan_existing_chapters;
use crate::manga::covers;
use crate::manga::merge;
use crate::scheduler::optimiser;
use crate::scraper::downloader;
use crate::scraper::{ProviderRegistry, ScraperCtx};

/// Shared map of task UUID → CancellationToken for in-flight tasks.
/// The cancel API endpoint signals the token; the running task checks it.
pub type CancelMap = Arc<Mutex<HashMap<Uuid, CancellationToken>>>;

// Workers
// This file handles all the queue and tasks.

/// Spawn the background worker as a detached tokio task.
/// The worker loops indefinitely, polling for pending tasks every 5 seconds.
/// Also spawns a separate scheduler task that enqueues periodic chapter checks.
pub fn start(
    pool: SqlitePool,
    registry: Arc<ProviderRegistry>,
    ctx: ScraperCtx,
    cancel_map: CancelMap,
) -> JoinHandle<()> {
    // Spawn the periodic scheduler as a separate task
    let scheduler_pool = pool.clone();
    tokio::spawn(async move {
        run_scheduler(scheduler_pool).await;
    });

    tokio::spawn(async move {
        let mut rate_limiter = RateLimiter::new(&registry);
        loop {
            // Honour the global queue-pause setting
            let paused = db_settings::get(&pool, "queue_paused", "false")
                .await
                .map(|v| v == "true")
                .unwrap_or(false);
            if paused {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            match db_task::claim_next(&pool).await {
                Ok(Some(task)) => {
                    info!(
                        "[worker] Claimed task {:?} (id={})",
                        task.task_type, task.id
                    );

                    // Throttle based on provider(s) involved
                    throttle_for_task(&pool, &registry, &task, &mut rate_limiter).await;

                    // Register a cancellation token for this task
                    let token = CancellationToken::new();
                    cancel_map.lock().unwrap().insert(task.id, token.clone());

                    let result = dispatch(&pool, &registry, &ctx, &task, token).await;

                    // Remove the token from the map — task is no longer in-flight
                    cancel_map.lock().unwrap().remove(&task.id);

                    match result {
                        Ok(()) => {
                            if let Err(e) = db_task::complete(&pool, task.id).await {
                                error!("[worker] Failed to mark task complete: {e}");
                            }
                            info!("[worker] Task {} completed.", task.id);
                        }
                        Err(e) if e == "cancelled" => {
                            info!("[worker] Task {} was cancelled.", task.id);
                            // Status already set to Cancelled by the cancel endpoint
                        }
                        Err(e) => {
                            warn!("[worker] Task {} failed: {e}", task.id);
                            if let Err(db_err) = db_task::fail(&pool, task.id, &e).await {
                                error!("[worker] Failed to record task failure: {db_err}");
                            }
                        }
                    }
                }
                Ok(None) => {
                    // Nothing ready — sleep before polling again
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!("[worker] Error polling task queue: {e}");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    })
}

/// Runs the periodic "check for new chapters" scheduler.
/// Reads scan_interval_hours from Settings at the start of each cycle and
/// enqueues CheckNewChapter for all monitored manga that are due for a check.
/// Uses offset-based scheduling: checks happen N hours after the LAST check,
/// not at absolute intervals. This naturally spreads out checks.
async fn run_scheduler(pool: SqlitePool) {
    loop {
        // Read interval from settings (re-read each cycle so config changes take effect)
        let hours = db_settings::get(&pool, "scan_interval_hours", "6")
            .await
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(6);

        // Check every minute for manga that are due
        // This way we don't wait hours if a manga's check is due shortly after server start
        match db_manga::get_due_for_check(&pool, hours).await {
            Ok(manga_list) => {
                for manga in manga_list {
                    // Dedupe: skip if already pending/running
                    match db_task::is_pending_for_manga(&pool, manga.id, TaskType::CheckNewChapter)
                        .await
                    {
                        Ok(false) => {
                            if let Err(e) = db_task::enqueue(
                                &pool,
                                TaskType::CheckNewChapter,
                                Some(manga.id),
                                None,
                                10,
                            )
                            .await
                            {
                                error!(
                                    "[scheduler] Failed to enqueue CheckNewChapter for '{}': {e}",
                                    manga.metadata.title
                                );
                            } else {
                                debug!(
                                    "[scheduler] Enqueued CheckNewChapter for '{}'",
                                    manga.metadata.title
                                );
                            }
                        }
                        Ok(true) => {} // already queued — skip
                        Err(e) => {
                            error!("[scheduler] Error checking pending tasks: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                error!("[scheduler] Failed to fetch manga due for check: {e}");
            }
        }

        // Sleep for 1 minute before checking again
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// This is where each task in the queue is processed.
async fn dispatch(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    task: &Task,
    cancel_token: CancellationToken,
) -> Result<(), String> {
    match task.task_type {
        TaskType::BuildFullChapterList => {
            let manga_id = task
                .manga_id
                .ok_or("BuildFullChapterList task missing manga_id")?;
            let manga = db_manga::get_by_id(pool, manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {manga_id} not found"))?;

            let result = merge::scan_manga(pool, registry, ctx, &manga, task.id)
                .await
                .map_err(|e| e.to_string())?;

            info!(
                "[worker] Full scan complete for '{}': {} providers, {} new chapters.",
                manga.metadata.title, result.providers_found, result.new_chapters
            );

            Ok(())
        }

        TaskType::CheckNewChapter => {
            let manga_id = task
                .manga_id
                .ok_or("CheckNewChapter task missing manga_id")?;
            let manga = db_manga::get_by_id(pool, manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {manga_id} not found"))?;

            let result = merge::check_new_chapters(pool, registry, ctx, &manga, task.id)
                .await
                .map_err(|e| e.to_string())?;

            // Update last_checked_at to spread out future checks
            if let Err(e) = db_manga::update_last_checked(pool, manga_id).await {
                warn!("[worker] Failed to update last_checked_at: {e}");
            }

            info!(
                "[worker] Chapter check complete for '{}': {} new chapters.",
                manga.metadata.title, result.new_chapters
            );

            Ok(())
        }

        TaskType::DownloadChapter => {
            let chapter_id = task
                .chapter_id
                .ok_or("DownloadChapter task missing chapter_id")?;

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

            downloader::download_chapter(
                pool,
                task.id,
                registry,
                ctx,
                &manga,
                &chapter,
                &library.root_path,
                cancel_token,
            )
            .await
            .map_err(|e| e.to_string())
        }

        TaskType::RefreshMetadata => {
            let manga_id = task
                .manga_id
                .ok_or("RefreshMetadata task missing manga_id")?;
            let manga = db_manga::get_by_id(pool, manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {manga_id} not found"))?;

            // Refresh based on metadata_source
            match manga.metadata_source {
                crate::manga::manga::MangaSource::AniList => {
                    let Some(anilist_id) = manga.anilist_id else {
                        info!(
                            "[worker] Manga '{}' has no AniList ID — skipping refresh.",
                            manga.metadata.title
                        );
                        return Ok(());
                    };

                    let al = ALClient::new();
                    let mut fresh = al
                        .grab_manga(anilist_id as i32)
                        .await
                        .map_err(|e| format!("AniList fetch failed: {e}"))?;

                    // Preserve internal identity fields from the stored record
                    fresh.id = manga.id;
                    fresh.library_id = manga.library_id;
                    fresh.relative_path = manga.relative_path.clone();
                    fresh.downloaded_count = manga.downloaded_count;
                    fresh.chapter_count = manga.chapter_count;
                    fresh.monitored = manga.monitored;
                    fresh.created_at = manga.created_at;
                    fresh.metadata_updated_at = chrono::Utc::now().timestamp();

                    // Re-download cover if the URL changed
                    if let Some(url) = fresh.thumbnail_url.take() {
                        let library = db_library::get_by_id(pool, manga.library_id)
                            .await
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| format!("library {} not found", manga.library_id))?;
                        let series_dir = library.root_path.join(&manga.relative_path);
                        fresh.thumbnail_url =
                            covers::download_cover(&ctx.http, &url, manga.id, &series_dir)
                                .await
                                .or(Some(url));
                    }

                    db_manga::update_metadata(pool, &fresh)
                        .await
                        .map_err(|e| e.to_string())?;

                    info!(
                        "[worker] Refreshed AniList metadata for '{}'.",
                        fresh.metadata.title
                    );
                }
                crate::manga::manga::MangaSource::Local => {
                    info!(
                        "[worker] Manga '{}' has Local metadata source — nothing to refresh.",
                        manga.metadata.title
                    );
                }
            }

            Ok(())
        }

        TaskType::ScanDisk => {
            let manga_id = task.manga_id.ok_or("ScanDisk task missing manga_id")?;
            scan_existing_chapters(pool, manga_id, task.id).await
        }

        TaskType::OptimiseChapter => {
            let chapter_id = task
                .chapter_id
                .ok_or("OptimiseChapter task missing chapter_id")?;
            optimiser::optimise_chapter(pool, chapter_id).await
        }

        // Not yet implemented task types — log and succeed silently
        TaskType::Backup => {
            info!(
                "[worker] Task type {:?} not yet implemented, skipping.",
                task.task_type
            );
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
        for provider in registry.all() {
            // Minimum interval = 60_000ms / rpm  (floor at 100ms)
            let rpm = provider.rate_limit_rpm();
            let millis = if rpm > 0 {
                (60_000u64 / rpm as u64).max(100)
            } else {
                2_000
            };
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
        self.last_used
            .insert(provider_name.to_owned(), Instant::now());
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
                if registry
                    .all()
                    .iter()
                    .any(|p| p.name() == entry.provider_name)
                {
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
                    if registry
                        .all()
                        .iter()
                        .any(|p| p.name() == entry.provider_name)
                    {
                        limiter.throttle(&entry.provider_name).await;
                    }
                }
            }
        }
    }
}
