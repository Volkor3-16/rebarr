use chrono::Utc;
use log::{debug, info, warn};
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::task::JoinSet;

use crate::db::provider::MangaProvider;
use crate::db::task::TaskType;
use crate::db::{
    chapter as db_chapter, provider as db_provider, provider_scores as db_scores,
    settings as db_settings, task as db_task,
};
use crate::manga::manga::{DownloadStatus, Manga};
use crate::scraper::{ProviderRegistry, ProviderSearchResult, ScraperCtx};

/// Error for scary times
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("scraper error: {0}")]
    Scraper(#[from] crate::scraper::error::ScraperError),
}

/// A sucessful scan return!
#[derive(Debug)]
pub struct ScanResult {
    /// Number of providers that have a URL cached for this manga after the scan.
    pub providers_found: usize,
    /// Number of net-new chapters inserted with status `Missing`.
    pub new_chapters: usize,
}

/// Check only already-known providers for new chapters (skips provider search).
/// Used for periodic/automatic checks once a manga has been scanned once.
pub async fn check_new_chapters(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    task_id: uuid::Uuid,
) -> Result<ScanResult, ScanError> {
    scrape_known_providers(pool, registry, ctx, manga, task_id).await
}

/// Scan all providers for a manga:
/// 1. Search for provider URLs not yet cached in `MangaProvider`.
/// 2. Scrape chapter lists for every cached provider URL.
/// 3. Upsert new chapters (existing `Downloaded` chapters are untouched).
/// 4. Recompute CanonicalChapters and update chapter_count/downloaded_count.
pub async fn scan_manga(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    task_id: uuid::Uuid,
) -> Result<ScanResult, ScanError> {
    // Fallback to the other titles in case of emergency.
    let mut search_titles: Vec<String> = vec![manga.metadata.title.clone()];

    // Add all other_titles (synonyms, romaji, native, etc.)
    if let Some(ref other) = manga.metadata.other_titles {
        for synonym in other {
            // Skip if hidden (user manually hid this synonym)
            if synonym.hidden {
                continue;
            }

            if !synonym.title.trim().is_empty() {
                search_titles.push(synonym.title.clone());
            }
        }
    }

    search_titles.retain(|t| !t.trim().is_empty());
    search_titles.sort();
    search_titles.dedup();

    debug!(
        "[scan] Search titles for '{}': {:?}",
        manga.metadata.title, search_titles
    );

    let globally_disabled = db_scores::get_globally_disabled(pool).await?;

    let mut providers_to_search = Vec::new();
    for provider in registry.all() {
        // Skip globally disabled providers.
        if globally_disabled.contains(provider.name()) {
            debug!(
                "[scan] {} is globally disabled, skipping search.",
                provider.name()
            );
            continue;
        }

        // Skip providers that have already searched for the series.
        if db_provider::has_url(pool, manga.id, provider.name()).await? {
            debug!(
                "[scan] {} already has a URL for '{}', skipping search.",
                provider.name(),
                manga.metadata.title
            );
            continue;
        }

        providers_to_search.push((*provider).clone());
    }

    let total_search_providers = providers_to_search.len() as i64;
    if total_search_providers > 0 {
        let mut join_set = JoinSet::new();
        for provider in providers_to_search {
            let ctx = ctx.clone();
            let search_titles = search_titles.clone();
            join_set.spawn(async move {
                let mut results = Vec::new();
                let mut last_error = None;

                for title in &search_titles {
                    match ctx.executor.search(&ctx, &provider, title).await {
                        Ok(r) if !r.is_empty() => {
                            let best = best_match(&search_titles, &r);
                            if let Some((score, _)) = best {
                                if score >= 0.85 {
                                    results = r;
                                    break;
                                }
                                debug!(
                                    "[scan] Search for '{}' on {} returned results but best score ({:.2}) below threshold, trying next synonym.",
                                    title,
                                    provider.name(),
                                    score
                                );
                            }
                        }
                        Ok(_) => continue,
                        Err(e) => {
                            last_error = Some(e.to_string());
                            break;
                        }
                    }
                }

                (provider, results, last_error, search_titles)
            });
        }

        let mut completed = 0i64;
        while let Some(joined) = join_set.join_next().await {
            let (provider, results, last_error, search_titles) =
                joined.map_err(|e| ScanError::Scraper(crate::scraper::error::ScraperError::Browser(e.to_string())))?;
            completed += 1;

            let _ = db_task::set_progress(
                pool,
                task_id,
                &db_task::TaskProgress {
                    step: Some("provider-search".to_owned()),
                    label: Some(format!(
                        "Searched {completed} of {total_search_providers} providers"
                    )),
                    detail: Some(format!("Finished provider search on {}", provider.name())),
                    provider: Some(provider.name().to_owned()),
                    current: Some(completed),
                    total: Some(total_search_providers),
                    unit: Some("provider".to_owned()),
                    ..Default::default()
                },
            )
            .await;

            if let Some(err) = last_error {
                warn!("[scan] Search error on {}: {err}", provider.name());
            }

            if results.is_empty() {
                info!("[scan] No results on {} for this manga.", provider.name());
                db_provider::upsert_not_found(pool, manga.id, provider.name()).await?;
                continue;
            }

            if let Some((score, result)) = best_match(&search_titles, &results) {
                info!(
                    "[scan] Matched '{}' → '{}' on {} (score {:.2})",
                    manga.metadata.title,
                    result.title,
                    provider.name(),
                    score
                );
                db_provider::upsert(
                    pool,
                    &MangaProvider {
                        manga_id: manga.id,
                        enabled: true,
                        provider_name: provider.name().to_owned(),
                        provider_url: Some(result.url.clone()),
                        last_synced_at: None,
                        search_attempted_at: Some(Utc::now().timestamp()),
                    },
                )
                .await?;
            } else {
                db_provider::upsert_not_found(pool, manga.id, provider.name()).await?;
            }
        }
    }

    scrape_known_providers(pool, registry, ctx, manga, task_id).await
}

/// Phase 2+: scrape chapter lists from all cached provider URLs, upsert chapters,
/// and auto-download new chapters for monitored manga.
async fn scrape_known_providers(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    task_id: uuid::Uuid,
) -> Result<ScanResult, ScanError> {
    let globally_disabled = db_scores::get_globally_disabled(pool).await?;
    let all_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    let provider_entries: Vec<_> = all_entries
        .into_iter()
        .filter(|e| e.found() && !globally_disabled.contains(&e.provider_name))
        .collect();

    let (total_new, new_ids_for_download) =
        scrape_chapters(pool, registry, ctx, manga, &provider_entries, task_id).await?;

    let trusted_groups = db_provider::get_trusted_groups(pool).await?;
    let preferred_language = db_settings::get(pool, "preferred_language", "").await?;
    let yaml_defaults = registry.yaml_default_scores();
    let provider_scores = db_scores::load_effective_scores(pool, manga.id, &yaml_defaults).await?;
    db_chapter::update_canonical(
        pool,
        manga.id,
        &trusted_groups,
        &preferred_language,
        &provider_scores,
    )
    .await?;

    if manga.monitored {
        enqueue_auto_downloads(pool, manga, &new_ids_for_download).await;
        enqueue_upgrades(pool, manga, &trusted_groups).await;
    }

    let final_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    Ok(ScanResult {
        providers_found: final_entries.iter().filter(|e| e.found()).count(),
        new_chapters: total_new,
    })
}

/// Fetch chapter lists from every cached provider entry, upsert results into DB,
/// and return (total_new_count, new_chapter_ids_eligible_for_auto_download).
///
/// `new_chapter_ids` is populated only for providers that have been synced before —
/// first-time syncs are excluded to avoid mass-downloading a manga's entire back-catalogue
/// the first time it is added.
async fn scrape_chapters(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    provider_entries: &[crate::db::provider::MangaProvider],
    task_id: uuid::Uuid,
) -> Result<(usize, std::collections::HashSet<uuid::Uuid>), ScanError> {
    let provider_map: std::collections::HashMap<String, std::sync::Arc<dyn crate::scraper::Provider>> =
        registry
            .all()
            .into_iter()
            .map(|p| (p.name().to_owned(), (*p).clone()))
            .collect();

    let mut total_new = 0usize;
    let mut new_ids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    let mut join_set = JoinSet::new();
    let total_providers = provider_entries.len() as i64;

    for entry in provider_entries.iter().cloned() {
        let Some(provider) = provider_map.get(entry.provider_name.as_str()).cloned() else {
            warn!(
                "[scan] Provider '{}' is in DB but not loaded — skipping.",
                entry.provider_name
            );
            continue;
        };

        let ctx = ctx.clone();
        join_set.spawn(async move {
            let provider_url = entry.provider_url.clone().unwrap_or_default();
            let result = ctx.executor.chapters(&ctx, &provider, &provider_url).await;
            (entry, result)
        });
    }

    let mut completed = 0i64;
    while let Some(joined) = join_set.join_next().await {
        let (entry, result) =
            joined.map_err(|e| ScanError::Scraper(crate::scraper::error::ScraperError::Browser(e.to_string())))?;
        completed += 1;

        let _ = db_task::set_progress(
            pool,
            task_id,
            &db_task::TaskProgress {
                step: Some("chapter-sync".to_owned()),
                label: Some(format!("Synced {completed} of {total_providers} providers")),
                detail: Some(format!("Finished chapter sync for {}", entry.provider_name)),
                provider: Some(entry.provider_name.clone()),
                target: entry.provider_url.clone(),
                current: Some(completed),
                total: Some(total_providers),
                unit: Some("provider".to_owned()),
            },
        )
        .await;

        let was_previously_synced = entry.last_synced_at.is_some();
        match result {
            Ok(infos) => {
                let inserted_ids =
                    db_chapter::upsert_from_scrape(pool, manga.id, &entry.provider_name, &infos)
                        .await?;
                let inserted = inserted_ids.len();
                total_new += inserted;
                info!(
                    "[scan] {} returned {} chapters ({inserted} new).",
                    entry.provider_name,
                    infos.len()
                );

                if was_previously_synced {
                    new_ids.extend(inserted_ids);
                }

                db_provider::upsert(
                    pool,
                    &MangaProvider {
                        manga_id: manga.id,
                        enabled: true,
                        provider_name: entry.provider_name.clone(),
                        provider_url: entry.provider_url.clone(),
                        last_synced_at: Some(Utc::now().timestamp()),
                        search_attempted_at: entry.search_attempted_at,
                    },
                )
                .await?;
            }
            Err(e) => {
                warn!("[scan] Chapter fetch failed on {}: {e}", entry.provider_name);
            }
        }
    }

    Ok((total_new, new_ids))
}

fn best_match<'a>(
    search_titles: &[String],
    results: &'a [ProviderSearchResult],
) -> Option<(f64, &'a ProviderSearchResult)> {
    results
        .iter()
        .map(|res| {
            let score = search_titles
                .iter()
                .map(|t| strsim::jaro_winkler(&res.title.to_lowercase(), &t.to_lowercase()))
                .fold(0.0f64, f64::max);
            (score, res)
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
}

/// Enqueue DownloadChapter tasks for canonical chapters that were newly discovered.
/// Only chapters present in `new_ids` and confirmed canonical are queued.
async fn enqueue_auto_downloads(
    pool: &SqlitePool,
    manga: &Manga,
    new_ids: &std::collections::HashSet<uuid::Uuid>,
) {
    if new_ids.is_empty() {
        return;
    }
    let canonical = match db_chapter::get_canonical_for_manga(pool, manga.id).await {
        Ok(v) => v,
        Err(e) => {
            warn!("[scan] Failed to load canonicals for auto-download: {e}");
            return;
        }
    };
    // If a whole chapter (variant=0) is canonical for a given base, skip its split
    // sub-parts (variant>0) — they cover the same content.
    let whole_chapter_bases: std::collections::HashSet<i32> = canonical
        .iter()
        .filter(|ch| ch.chapter_variant == 0)
        .map(|ch| ch.chapter_base)
        .collect();

    let mut enqueued = 0usize;
    for ch in &canonical {
        if ch.chapter_variant > 0 && whole_chapter_bases.contains(&ch.chapter_base) {
            continue;
        }
        if new_ids.contains(&ch.id) {
            match db_task::enqueue(
                pool,
                TaskType::DownloadChapter,
                Some(manga.id),
                Some(ch.id),
                8,
            )
            .await
            {
                Ok(_) => enqueued += 1,
                Err(e) => warn!(
                    "[scan] Failed to enqueue auto-download for chapter {}: {e}",
                    ch.id
                ),
            }
        }
    }
    if enqueued > 0 {
        info!(
            "[scan] Queued {enqueued} auto-download(s) for monitored manga '{}'.",
            manga.metadata.title
        );
    }
}

/// Enqueue DownloadChapter tasks for chapters where a better-tier source became canonical
/// since the last download (e.g. official release now available for a chapter we got from
/// an unknown scanlator). Skips chapters already Queued or Downloading.
async fn enqueue_upgrades(pool: &SqlitePool, manga: &Manga, trusted_groups: &[String]) {
    let candidates = match db_chapter::find_upgrade_candidates(pool, manga.id, trusted_groups).await
    {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => return,
        Err(e) => {
            warn!("[scan] Upgrade candidate check failed: {e}");
            return;
        }
    };

    let mut upgrade_count = 0usize;
    for candidate in &candidates {
        let already_active = db_chapter::get_by_id(pool, candidate.new_canonical_id)
            .await
            .ok()
            .flatten()
            .map(|ch| {
                matches!(
                    ch.download_status,
                    DownloadStatus::Queued | DownloadStatus::Downloading
                )
            })
            .unwrap_or(false);

        if already_active {
            continue;
        }

        match db_task::enqueue(
            pool,
            TaskType::DownloadChapter,
            Some(manga.id),
            Some(candidate.new_canonical_id),
            7,
        )
        .await
        {
            Ok(_) => upgrade_count += 1,
            Err(e) => warn!(
                "[scan] Failed to enqueue upgrade for {}: {e}",
                candidate.new_canonical_id
            ),
        }
    }
    if upgrade_count > 0 {
        info!(
            "[scan] Queued {upgrade_count} chapter upgrade(s) for '{}'.",
            manga.metadata.title
        );
    }
}
