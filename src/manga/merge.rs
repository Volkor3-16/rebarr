use chrono::Utc;
use log::{debug, info, warn};
use sqlx::SqlitePool;
use thiserror::Error;

use crate::db::provider::MangaProvider;
use crate::db::task::TaskType;
use crate::db::{
    chapter as db_chapter, provider as db_provider, provider_scores as db_scores,
    settings as db_settings, task as db_task,
};
use crate::manga::manga::{DownloadStatus, Manga};
use crate::scraper::{Provider, ProviderRegistry, ScraperCtx};

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
    let mut search_titles: Vec<&str> = vec![manga.metadata.title.as_str()];

    // Add all other_titles (synonyms, romaji, native, etc.)
    if let Some(ref other) = manga.metadata.other_titles {
        for synonym in other {
            // Skip if hidden (user manually hid this synonym)
            if synonym.hidden {
                continue;
            }

            if !synonym.title.trim().is_empty() {
                search_titles.push(synonym.title.as_str());
            }
        }
    }

    // Deduplicate using HashSet
    search_titles = search_titles
        .into_iter()
        .filter(|t| !t.trim().is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    debug!(
        "[scan] Search titles for '{}': {:?}",
        manga.metadata.title, search_titles
    );

    let globally_disabled = db_scores::get_globally_disabled(pool).await?;

    // For every provider...
    let search_provider_count = registry.all().len() as i64;
    let search_title_count = search_titles.len() as i64;
    let mut searched_providers = 0i64;

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

        info!(
            "[scan] Searching '{}' for '{}'…",
            provider.name(),
            manga.metadata.title
        );

        searched_providers += 1;

        let mut results = Vec::new();

        // For each title, search for any results
        // Continue to next synonym if results don't meet threshold
        for (title_idx, title) in search_titles.iter().enumerate() {
            let _ = db_task::set_progress(
                pool,
                task_id,
                &db_task::TaskProgress {
                    step: Some("provider-search".to_owned()),
                    label: Some(format!(
                        "Searching provider {searched_providers}/{search_provider_count}"
                    )),
                    detail: Some(format!("{} searching for \"{}\"", provider.name(), title)),
                    provider: Some(provider.name().to_owned()),
                    target: Some(title.to_string()),
                    current: Some((title_idx + 1) as i64),
                    total: Some(search_title_count),
                    unit: Some("title".to_owned()),
                },
            )
            .await;

            match provider.search(ctx, title).await {
                Ok(r) if !r.is_empty() => {
                    // Score results against all synonyms to find best match
                    let best = r
                        .iter()
                        .map(|res| {
                            let score = search_titles
                                .iter()
                                .map(|t| {
                                    strsim::jaro_winkler(
                                        &res.title.to_lowercase(),
                                        &t.to_lowercase(),
                                    )
                                })
                                .fold(0.0f64, f64::max);
                            (score, res)
                        })
                        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                    if let Some((score, _)) = best {
                        if score >= 0.85 {
                            // Good match found! Use these results and break early
                            results = r;
                            break;
                        }
                        // Below threshold - continue to next synonym
                        debug!(
                            "[scan] Search for '{}' on {} returned results but best score ({:.2}) below threshold, trying next synonym.",
                            title,
                            provider.name(),
                            score
                        );
                    }
                    // If no best score, continue to next synonym
                }
                Ok(_) => continue,
                Err(e) => {
                    warn!("[scan] Search error on {}: {e}", provider.name());
                    break;
                }
            }
        }

        // If theres no good results after scanning all synonyms, give up!
        if results.is_empty() {
            info!("[scan] No results on {} for this manga.", provider.name());
            db_provider::upsert_not_found(pool, manga.id, provider.name()).await?;
            continue;
        }

        // We have good results (score >= 0.85), extract best match for logging/db
        let best = results
            .iter()
            .map(|r| {
                let score = search_titles
                    .iter()
                    .map(|t| strsim::jaro_winkler(&r.title.to_lowercase(), &t.to_lowercase()))
                    .fold(0.0f64, f64::max);
                (score, r)
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((score, result)) = best {
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
    let provider_map: std::collections::HashMap<&str, &std::sync::Arc<dyn Provider>> =
        registry.all().into_iter().map(|p| (p.name(), p)).collect();

    let mut total_new = 0usize;
    let mut new_ids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    let total_providers = provider_entries.len() as i64;

    for (idx, entry) in provider_entries.iter().enumerate() {
        // Track whether this provider has been synced before so we can skip
        // back-catalogue auto-downloads on first sync.
        let was_previously_synced = entry.last_synced_at.is_some();

        let Some(provider) = provider_map.get(entry.provider_name.as_str()) else {
            warn!(
                "[scan] Provider '{}' is in DB but not loaded — skipping.",
                entry.provider_name
            );
            continue;
        };

        info!(
            "[scan] Fetching chapters from {} for '{}'…",
            entry.provider_name, manga.metadata.title
        );

        let _ = db_task::set_progress(
            pool,
            task_id,
            &db_task::TaskProgress {
                step: Some("chapter-sync".to_owned()),
                label: Some(format!(
                    "Syncing provider {} of {}",
                    idx + 1,
                    provider_entries.len()
                )),
                detail: Some(format!(
                    "Fetching chapter list from {}",
                    entry.provider_name
                )),
                provider: Some(entry.provider_name.clone()),
                target: entry.provider_url.clone(),
                current: Some((idx + 1) as i64),
                total: Some(total_providers),
                unit: Some("provider".to_owned()),
            },
        )
        .await;

        let provider_url = entry.provider_url.as_deref().unwrap();
        match provider.chapters(ctx, provider_url).await {
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
                warn!(
                    "[scan] Chapter fetch failed on {}: {e}",
                    entry.provider_name
                );
            }
        }
    }

    Ok((total_new, new_ids))
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
    let mut enqueued = 0usize;
    for ch in &canonical {
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
