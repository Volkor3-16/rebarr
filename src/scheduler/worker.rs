// This file handles:
// - All potential tasks that can be loaded into the queue
// - Multi-queue worker dispatch for per-provider queues

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::{debug, error, info, warn};
use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::task::{Task, TaskType};
use crate::db::{
    chapter as db_chapter, library as db_library, manga as db_manga, provider as db_provider,
    provider_failure as db_provider_failure, settings as db_settings, task as db_task,
};
use crate::http::metadata::AniListMetadata;
use crate::library::scanner::scan_existing_chapters;
use crate::manga::{comicinfo, covers, files};
use crate::manga::merge;
use crate::manga::core::{DownloadStatus, Manga, PublishingStatus};
use crate::scheduler::optimiser;
use crate::scraper::downloader;
use crate::scraper::{ProviderRegistry, ScraperCtx};

/// Shared map of task UUID → CancellationToken for in-flight tasks.
/// The cancel API endpoint signals the token; the running task checks it.
pub type CancelMap = Arc<Mutex<HashMap<Uuid, CancellationToken>>>;

/// Queue name prefix for provider-specific queues.
pub fn provider_queue_name(provider_name: &str) -> String {
    format!("provider:{provider_name}")
}

async fn rewrite_downloaded_comicinfo(
    pool: &SqlitePool,
    manga: &Manga,
    library_root: &std::path::Path,
) -> Result<(), String> {
    let series_dir = files::series_dir(library_root, manga);
    let chapters = db_chapter::get_all_for_manga(pool, manga.id)
        .await
        .map_err(|e| e.to_string())?;

    for chapter in chapters
        .iter()
        .filter(|chapter| chapter.download_status == DownloadStatus::Downloaded)
    {
        let cbz_path = files::chapter_cbz_path(&series_dir, chapter);
        if !cbz_path.exists() {
            continue;
        }

        let expected_xml = comicinfo::generate_chapter_xml(
            manga,
            chapter,
            comicinfo::read_cbz_page_count(&cbz_path).unwrap_or(0),
            chapter.provider_name.as_deref(),
        );
        let current_xml = comicinfo::read_cbz_comicinfo_xml(&cbz_path).unwrap_or_default();
        if current_xml != expected_xml {
            comicinfo::rewrite_chapter_comicinfo(&cbz_path, &expected_xml)
                .await
                .map_err(|e| format!("failed to rewrite {}: {e}", cbz_path.display()))?;
        }
    }

    Ok(())
}

// Workers
// This file handles all the queue and tasks.

/// Spawn the multi-queue worker system as a detached tokio task.
/// Creates one worker pool per provider (concurrency from YAML max_concurrency)
/// plus a system worker for non-provider tasks.
pub fn start(
    pool: SqlitePool,
    registry: Arc<ProviderRegistry>,
    ctx: ScraperCtx,
    cancel_map: CancelMap,
    shutdown_token: CancellationToken,
) -> JoinHandle<()> {
    let mut handles = Vec::new();

    // Spawn the periodic scheduler as a separate task
    let scheduler_pool = pool.clone();
    let scheduler_registry = registry.clone();
    let scheduler_shutdown = shutdown_token.clone();
    handles.push(tokio::spawn(async move {
        run_scheduler(scheduler_pool.clone(), scheduler_registry, scheduler_shutdown).await;
    }));

    // Spawn system queue workers (1-2 workers for system tasks)
    let system_workers = 2;
    for i in 0..system_workers {
        let pool = pool.clone();
        let registry = registry.clone();
        let ctx = ctx.clone();
        let cancel_map = cancel_map.clone();
        let worker_shutdown = shutdown_token.clone();
        handles.push(tokio::spawn(async move {
            queue_worker(
                &pool,
                &registry,
                &ctx,
                &cancel_map,
                "system",
                format!("system-worker-{i}"),
                worker_shutdown,
            )
            .await;
        }));
    }

    // Spawn provider-specific queue workers
    for provider in registry.all() {
        let concurrency = provider.max_concurrency() as usize;
        for i in 0..concurrency {
            let pool = pool.clone();
            let registry = registry.clone();
            let ctx = ctx.clone();
            let cancel_map = cancel_map.clone();
            let queue_name = provider_queue_name(provider.name());
            let worker_name = format!("{}-{i}", provider.name());
            let worker_shutdown = shutdown_token.clone();
            handles.push(tokio::spawn(async move {
                queue_worker(&pool, &registry, &ctx, &cancel_map, &queue_name, worker_name, worker_shutdown).await;
            }));
        }
    }

    // Return a handle that waits for all workers to finish
    tokio::spawn(async move {
        // Wait for shutdown signal
        shutdown_token.cancelled().await;
        // Wait for all worker tasks to complete
        for handle in handles {
            let _ = handle.await;
        }
    })
}

/// Run a worker loop for a specific queue.
/// Claims tasks from the queue and dispatches them for execution.
async fn queue_worker(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    cancel_map: &CancelMap,
    queue: &str,
    worker_name: String,
    shutdown_token: CancellationToken,
) {
    info!("[{worker_name}] Starting worker for queue '{queue}'");

    loop {
        // Check for shutdown signal
        if shutdown_token.is_cancelled() {
            info!("[{worker_name}] Shutting down.");
            return;
        }

        // Honour the global queue-pause setting
        let paused = db_settings::get(pool, "queue_paused", "false")
            .await
            .map(|v| v == "true")
            .unwrap_or(false);
        if paused {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                _ = shutdown_token.cancelled() => {
                    info!("[{worker_name}] Shutting down.");
                    return;
                }
            }
            continue;
        }

        match db_task::claim_next_for_queue(pool, queue).await {
            Ok(Some(task)) => {
                debug!(
                    "[{}] Claimed task {:?} (id={}) from queue '{queue}'",
                    worker_name, task.task_type, task.id
                );

                // Register a cancellation token for this task
                let token = CancellationToken::new();
                cancel_map.lock().unwrap().insert(task.id, token.clone());

                let result = dispatch(pool, registry, ctx, &task, token).await;

                // Remove the token from the map — task is no longer in-flight
                cancel_map.lock().unwrap().remove(&task.id);

                match result {
                    Ok(()) => {
                        if let Err(e) = db_task::complete(pool, task.id).await {
                            error!("[{}] Failed to mark task complete: {e}", worker_name);
                        }
                        debug!("[{}] Task {} completed.", worker_name, task.id);
                    }
                    Err(e) if e == "cancelled" => {
                        info!("[{}] Task {} was cancelled.", worker_name, task.id);
                    }
                    Err(e) => {
                        warn!("[{}] Task {} failed: {e}", worker_name, task.id);
                        if let Err(db_err) = db_task::fail(pool, task.id, &e).await {
                            error!("[{}] Failed to record task failure: {db_err}", worker_name);
                        }
                    }
                }
            }
            Ok(None) => {
                // Nothing ready — sleep before polling again
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    _ = shutdown_token.cancelled() => {
                        info!("[{worker_name}] Shutting down.");
                        return;
                    }
                }
            }
            Err(e) => {
                error!("[{}] Error polling task queue: {e}", worker_name);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                    _ = shutdown_token.cancelled() => {
                        info!("[{worker_name}] Shutting down.");
                        return;
                    }
                }
            }
        }
    }
}

/// Runs the periodic "check for new chapters" scheduler.
/// Reads scan_interval_hours from Settings at the start of each cycle and
/// enqueues per-provider SyncProviderChapters tasks for all monitored manga that are due.
/// Each provider gets its own task in its own queue (provider:{name}).
async fn run_scheduler(pool: SqlitePool, _registry: Arc<ProviderRegistry>, shutdown_token: CancellationToken) {
    loop {
        // Check for shutdown signal
        if shutdown_token.is_cancelled() {
            info!("[scheduler] Shutting down.");
            return;
        }
        // Read interval from settings (re-read each cycle so config changes take effect)
        let hours = db_settings::get(&pool, "scan_interval_hours", "6")
            .await
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(6);

        // Check every minute for manga that are due
        match db_manga::get_due_for_check(&pool, hours).await {
            Ok(manga_list) => {
                for manga in manga_list {
                    // Get known providers for this manga
                    match db_provider::get_all_for_manga(&pool, manga.id).await {
                        Ok(providers) => {
                            let mut enqueued = 0;
                            for p in &providers {
                                if !p.found() {
                                    continue; // Skip providers that haven't found the manga
                                }

                                // Skip auto-disabled providers
                                if crate::db::provider_failure::is_auto_disabled(
                                    &pool, &p.provider_name, manga.id,
                                )
                                .await
                                .unwrap_or(false)
                                {
                                    debug!(
                                        "[scheduler] Skipping disabled provider {} for '{}'",
                                        p.provider_name, manga.metadata.title
                                    );
                                    continue;
                                }

                                let queue = provider_queue_name(&p.provider_name);

                                // Dedupe: skip if already pending/running for this queue+provider
                                if db_task::is_pending_in_queue(&pool, &queue, manga.id, TaskType::SyncProviderChapters)
                                    .await
                                    .unwrap_or(false)
                                {
                                    continue;
                                }

                                // Enqueue per-provider task with provider name in payload
                                let payload = serde_json::json!({
                                    "provider": p.provider_name
                                })
                                .to_string();

                                if let Err(e) = db_task::enqueue_with_payload(
                                    &pool,
                                    TaskType::SyncProviderChapters,
                                    Some(manga.id),
                                    None,
                                    10,
                                    Some(queue.clone()),
                                    Some(payload),
                                )
                                .await
                                {
                                    error!(
                                        "[scheduler] Failed to enqueue SyncProviderChapters for '{}': on {}' {e}",
                                        manga.metadata.title, p.provider_name
                                    );
                                } else {
                                    enqueued += 1;
                                }
                            }
                            if enqueued > 0 {
                                debug!(
                                    "[scheduler] Enqueued {enqueued} provider check(s) for '{}'",
                                    manga.metadata.title
                                );
                            }
                        }
                        Err(e) => {
                            error!(
                                "[scheduler] Failed to get providers for '{}': {e}",
                                manga.metadata.title
                            );
                        }
                    }
                }
            }
            Err(e) => {
                error!("[scheduler] Failed to fetch manga due for check: {e}");
            }
        }

        // Sleep for 1 minute before checking again
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => {},
            _ = shutdown_token.cancelled() => {
                info!("[scheduler] Shutting down.");
                return;
            }
        }
    }
}

/// This is where each task in the queue is processed.
#[tracing::instrument(
    skip(pool, registry, ctx, cancel_token),
    fields(task_id = %task.id, task_type = ?task.task_type)
)]
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

            // Phase 1: Search for provider URLs (fast, runs in this system task)
            merge::search_providers(pool, registry, ctx, &manga, task.id)
                .await
                .map_err(|e| e.to_string())?;

            // Phase 2: Enqueue per-provider chapter sync tasks
            let globally_disabled = crate::db::provider_scores::get_globally_disabled(pool)
                .await
                .unwrap_or_default();
            let all_entries = db_provider::get_all_for_manga(pool, manga.id)
                .await
                .map_err(|e| e.to_string())?;

            let mut enqueued = 0;
            for entry in &all_entries {
                if !entry.found() || globally_disabled.contains(&entry.provider_name) {
                    continue;
                }

                let queue = provider_queue_name(&entry.provider_name);

                // Dedupe: skip if already pending/running for this queue+provider
                if db_task::is_pending_in_queue(pool, &queue, manga.id, TaskType::SyncProviderChapters)
                    .await
                    .unwrap_or(false)
                {
                    continue;
                }

                let payload = serde_json::json!({
                    "provider": entry.provider_name
                })
                .to_string();

                if let Err(e) = db_task::enqueue_with_payload(
                    pool,
                    TaskType::SyncProviderChapters,
                    Some(manga.id),
                    None,
                    5,
                    Some(queue.clone()),
                    Some(payload),
                )
                .await
                {
                    warn!(
                        "[worker] Failed to enqueue SyncProviderChapters for '{}' on {}: {e}",
                        manga.metadata.title, entry.provider_name
                    );
                } else {
                    enqueued += 1;
                }
            }

            info!(
                "[worker] BuildFullChapterList search complete for '{}': enqueued {} provider sync task(s).",
                manga.metadata.title, enqueued
            );

            Ok(())
        }

        TaskType::SyncProviderChapters => {
            let manga_id = task
                .manga_id
                .ok_or("SyncProviderChapters task missing manga_id")?;
            let manga = db_manga::get_by_id(pool, manga_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("manga {manga_id} not found"))?;

            // Check if this is a per-provider task (has provider in payload)
            if let Some(ref payload) = task.payload {
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(payload) {
                    if let Some(provider_name) = info.get("provider").and_then(|v| v.as_str()) {
                        // Per-provider task: check only this provider
                        let result = merge::check_provider_chapters(
                            pool, registry, ctx, &manga, task.id, provider_name,
                        )
                        .await
                        .map_err(|e| e.to_string())?;

                        // Clear failure records on success
                        let _ = db_provider_failure::clear_for_manga(pool, provider_name, manga_id).await;

                        // Update last_checked_at to spread out future checks
                        if let Err(e) = db_manga::update_last_checked(pool, manga_id).await {
                            warn!("[worker] Failed to update last_checked_at: {e}");
                        }

                        info!(
                            "[worker] Provider check complete for '{}' on {}': {} new chapters.",
                            manga.metadata.title, provider_name, result.new_chapters
                        );
                        return Ok(());
                    }
                }
            }

            // Fallback: check all providers (legacy behaviour)
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
                crate::manga::core::MangaSource::AniList => {
                    let Some(anilist_id) = manga.anilist_id else {
                        info!(
                            "[worker] Manga '{}' has no AniList ID — skipping refresh.",
                            manga.metadata.title
                        );
                        return Ok(());
                    };

                    let al = AniListMetadata::new();
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
                    if db_settings::get(pool, "auto_unmonitor_completed", "false")
                        .await
                        .map(|v| v == "true")
                        .unwrap_or(false)
                        && matches!(fresh.metadata.publishing_status, PublishingStatus::Completed)
                    {
                        fresh.monitored = false;
                    }

                    // Re-download cover if the URL changed
                    let library = db_library::get_by_id(pool, manga.library_id)
                        .await
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("library {} not found", manga.library_id))?;
                    if let Some(url) = fresh.thumbnail_url.take() {
                        let series_dir = files::series_dir(&library.root_path, &manga);
                        fresh.thumbnail_url =
                            covers::download_cover(&ctx.http, &url, manga.id, &series_dir)
                                .await
                                .or(Some(url));
                    }

                    db_manga::update_metadata(pool, &fresh)
                        .await
                        .map_err(|e| e.to_string())?;
                    let series_dir = files::series_dir(&library.root_path, &fresh);
                    comicinfo::write_series_comicinfo(&series_dir, &fresh)
                        .await
                        .map_err(|e| e.to_string())?;
                    rewrite_downloaded_comicinfo(pool, &fresh, &library.root_path).await?;

                    info!(
                        "[worker] Refreshed AniList metadata for '{}'.",
                        fresh.metadata.title
                    );
                }
                crate::manga::core::MangaSource::Local => {
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
