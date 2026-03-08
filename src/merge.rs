use chrono::Utc;
use sqlx::SqlitePool;
use thiserror::Error;

use crate::db::provider::MangaProvider;
use crate::db::{chapter as db_chapter, provider as db_provider, task as db_task};
use crate::db::task::TaskType;
use crate::manga::Manga;
use crate::scraper::{Provider, ProviderRegistry, ScraperCtx};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("scraper error: {0}")]
    Scraper(#[from] crate::scraper::error::ScraperError),
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ScanResult {
    /// Number of providers that have a URL cached for this manga after the scan.
    pub providers_found: usize,
    /// Number of net-new chapters inserted with status `Missing`.
    pub new_chapters: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all providers for a manga:
/// 1. Search for provider URLs not yet cached in `MangaProvider`.
/// 2. Scrape chapter lists for every cached provider URL.
/// 3. Upsert new chapters (existing `Downloaded` chapters are untouched).
/// 4. Refresh `chapter_count` and `downloaded_count` on the Manga row.
pub async fn scan_manga(
    pool: &SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
) -> Result<ScanResult, ScanError> {
    // Titles to try when searching (skip empty strings)
    let search_titles: Vec<&str> = [
        manga.metadata.title.as_str(),
        manga.metadata.title_roman.as_str(),
        manga.metadata.title_og.as_str(),
    ]
    .into_iter()
    .filter(|t| !t.trim().is_empty())
    .collect::<std::collections::HashSet<_>>()
    .into_iter()
    .collect();

    // --- Phase 1: Find provider URLs ---
    for provider in registry.all() {
        if db_provider::exists(pool, manga.id, provider.name()).await? {
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

        // Try each title variant; stop at the first non-empty result set
        let mut results = Vec::new();
        for title in &search_titles {
            match provider.search(ctx, title).await {
                Ok(r) if !r.is_empty() => {
                    results = r;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    log::warn!("[scan] Search error on {}: {e}", provider.name());
                    break;
                }
            }
        }

        if results.is_empty() {
            log::info!("[scan] No results on {} for this manga.", provider.name());
            continue;
        }

        // Score each result against all title variants; take best match
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
            if score >= 0.85 {
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
                        provider_name: provider.name().to_owned(),
                        provider_url: result.url.clone(),
                        last_synced_at: None,
                        provider_score: 0.0,
                        score_override: None,
                    },
                )
                .await?;
            } else {
                log::info!(
                    "[scan] Best match on {} was '{}' (score {:.2}) — below threshold, skipping.",
                    provider.name(),
                    result.title,
                    score
                );
            }
        }
    }

    // --- Phase 2: Scrape chapter lists ---
    let provider_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    let mut total_new = 0usize;
    // Chapters new on providers that were already synced before — candidates for auto-download.
    let mut auto_download_ids: Vec<uuid::Uuid> = Vec::new();

    // Build a map from name → Arc<dyn Provider> for quick lookup
    let provider_map: std::collections::HashMap<&str, &std::sync::Arc<dyn Provider>> = registry
        .all()
        .into_iter()
        .map(|p| (p.name(), p))
        .collect();

    for entry in &provider_entries {
        // Track whether this provider had a prior successful sync.
        // If it did, any new chapters found this time are genuinely new (not first-population).
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

        match provider.chapters(ctx, &entry.provider_url).await {
            Ok(infos) => {
                let new_ids = db_chapter::upsert_from_scrape(pool, manga.id, &entry.provider_name, &infos).await?;
                let inserted = new_ids.len();
                total_new += inserted;
                log::info!(
                    "[scan] {} returned {} chapters ({inserted} new).",
                    entry.provider_name,
                    infos.len()
                );

                // Collect new chapter IDs for auto-download (only if provider was already synced)
                if manga.monitored && was_previously_synced {
                    auto_download_ids.extend_from_slice(&new_ids);
                }

                // Update last_synced_at
                db_provider::upsert(
                    pool,
                    &MangaProvider {
                        manga_id: manga.id,
                        provider_name: entry.provider_name.clone(),
                        provider_url: entry.provider_url.clone(),
                        last_synced_at: Some(Utc::now()),
                        provider_score: 0.0,
                        score_override: None,
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

    // --- Auto-download new chapters for monitored manga ---
    if !auto_download_ids.is_empty() {
        log::info!(
            "[scan] Queueing {} auto-download(s) for monitored manga '{}'.",
            auto_download_ids.len(),
            manga.metadata.title
        );
        for chapter_id in &auto_download_ids {
            if let Err(e) = db_task::enqueue(pool, TaskType::DownloadChapter, Some(manga.id), Some(*chapter_id), 8).await {
                log::warn!("[scan] Failed to enqueue auto-download for chapter {chapter_id}: {e}");
            }
        }
    }

    // --- Phase 3: Refresh manga counts ---
    db_chapter::update_manga_counts(pool, manga.id).await?;

    let final_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    Ok(ScanResult {
        providers_found: final_entries.len(),
        new_chapters: total_new,
    })
}
