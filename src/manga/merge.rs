use chrono::Utc;
use sqlx::SqlitePool;
use thiserror::Error;

use crate::db::provider::MangaProvider;
use crate::db::{chapter as db_chapter, provider as db_provider, task as db_task, settings as db_settings};
use crate::db::task::TaskType;
use crate::manga::manga::Manga;
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
) -> Result<ScanResult, ScanError> {
    scrape_known_providers(pool, registry, ctx, manga).await
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
) -> Result<ScanResult, ScanError> {
    // Fallback to the other titles in case of emergency.
    let mut search_titles: Vec<&str> = vec![manga.metadata.title.as_str()];
    
    // Add all other_titles (synonyms, romaji, native, etc.)
    if let Some(ref other) = manga.metadata.other_titles {
        for title in other {
            if !title.trim().is_empty() {
                search_titles.push(title.as_str());
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

    // For every provider...
    for provider in registry.all() {
        // Skip providers that have already searched for the series.
        if db_provider::has_url(pool, manga.id, provider.name()).await? {
            log::debug!(
                "[scan] {} already has a URL for '{}', skipping search.",
                provider.name(),
                manga.metadata.title
            );
            continue;
        }

        log::info!(
            "[scan] Searching '{}' for '{}'…",
            provider.name(),
            manga.metadata.title
        );

        let mut results = Vec::new();

        // For each title, search for any results
        // Continue to next synonym if results don't meet threshold
        for title in &search_titles {
            match provider.search(ctx, title).await {
                Ok(r) if !r.is_empty() => {
                    // Score results against all synonyms to find best match
                    let best = r
                        .iter()
                        .map(|res| {
                            let score = search_titles
                                .iter()
                                .map(|t| strsim::jaro_winkler(&res.title.to_lowercase(), &t.to_lowercase()))
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
                        log::debug!(
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
                    log::warn!("[scan] Search error on {}: {e}", provider.name());
                    break;
                }
            }
        }

        // If theres no good results after scanning all synonyms, give up!
        if results.is_empty() {
            log::info!("[scan] No results on {} for this manga.", provider.name());
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
            log::info!(
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

    scrape_known_providers(pool, registry, ctx, manga).await
}

/// Phase 2+: scrape chapter lists from all cached provider URLs, upsert chapters,
/// and auto-download new chapters for monitored manga.
async fn scrape_known_providers(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
) -> Result<ScanResult, ScanError> {
    // Now lets start scraping chapter lists!
    let all_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    let provider_entries: Vec<_> = all_entries.into_iter().filter(|e| e.found()).collect();
    let mut total_new = 0usize;
    // UUIDs of newly inserted Chapters rows (for auto-download candidates).
    let mut all_new_ids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    let provider_map: std::collections::HashMap<&str, &std::sync::Arc<dyn Provider>> = registry
        .all()
        .into_iter()
        .map(|p| (p.name(), p))
        .collect();

    for entry in &provider_entries {
        let was_previously_synced = entry.last_synced_at.is_some();

        let Some(provider) = provider_map.get(entry.provider_name.as_str()) else {
            log::warn!(
                "[scan] Provider '{}' is in DB but not loaded — skipping.",
                entry.provider_name
            );
            continue;
        };

        log::info!(
            "[scan] Fetching chapters from {} for '{}'…",
            entry.provider_name,
            manga.metadata.title
        );

        let provider_url = entry.provider_url.as_deref().unwrap();
        match provider.chapters(ctx, provider_url).await {
            Ok(infos) => {
                let new_ids =
                    db_chapter::upsert_from_scrape(pool, manga.id, &entry.provider_name, &infos)
                        .await?;
                let inserted = new_ids.len();
                total_new += inserted;
                log::info!(
                    "[scan] {} returned {} chapters ({inserted} new).",
                    entry.provider_name,
                    infos.len()
                );

                if manga.monitored && was_previously_synced {
                    all_new_ids.extend(new_ids);
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
                log::warn!(
                    "[scan] Chapter fetch failed on {}: {e}",
                    entry.provider_name
                );
            }
        }
    }

    // --- Phase 3: CanonicalChapters ---
    let trusted_groups = db_provider::get_trusted_groups(pool).await?;
    let preferred_language = db_settings::get(pool, "preferred_language", "").await?;
    db_chapter::update_canonical(pool, manga.id, &trusted_groups, &preferred_language).await?;

    // --- Auto-download new chapters for monitored manga ---
    if !all_new_ids.is_empty() {
        let canonical = db_chapter::get_canonical_for_manga(pool, manga.id).await?;
        let mut enqueued = 0usize;
        for ch in &canonical {
            if all_new_ids.contains(&ch.id) {
                if let Err(e) =
                    db_task::enqueue(pool, TaskType::DownloadChapter, Some(manga.id), Some(ch.id), 8)
                        .await
                {
                    log::warn!("[scan] Failed to enqueue auto-download for chapter {}: {e}", ch.id);
                } else {
                    enqueued += 1;
                }
            }
        }
        if enqueued > 0 {
            log::info!(
                "[scan] Queued {enqueued} auto-download(s) for monitored manga '{}'.",
                manga.metadata.title
            );
        }
    }

    let final_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    Ok(ScanResult {
        providers_found: final_entries.iter().filter(|e| e.found()).count(),
        new_chapters: total_new,
    })
}
