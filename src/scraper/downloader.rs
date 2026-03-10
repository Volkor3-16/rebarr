use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::db::{chapter as db_chapter, chapter_url as db_chapter_url, provider as db_provider};
use crate::manga::comicinfo;
use crate::manga::manga::{Chapter, DownloadStatus, Manga};
use crate::manga::scoring::compute_tier;
use crate::scraper::{ProviderRegistry, ScraperCtx};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("no provider URLs found for this chapter — run a scan first")]
    NoProviders,
    #[error("all providers failed to download chapter {0}")]
    AllProvidersFailed(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Download a chapter from the best available provider:
/// 1. If the chapter has a preferred_provider, use that provider exclusively.
/// 2. Otherwise look up all cached provider URLs for this chapter, rank by scanlation tier,
///    and try each in order (Tier 1 = Official first, Tier 4 = No group last).
/// 3. For each provider: scrape page URLs, download images, write CBZ.
/// 4. If all providers fail, mark as Failed and return Err.
pub async fn download_chapter(
    pool: &sqlx::SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
) -> Result<(), DownloadError> {
    db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloading, None).await?;

    // Load trusted groups for tier computation
    let trusted_groups = db_provider::get_trusted_groups(pool).await?;

    // Get all providers that have a cached URL for this chapter
    let mut entries = db_chapter_url::get_for_chapter(pool, manga.id, chapter.number_sort).await?;

    if entries.is_empty() {
        db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
        return Err(DownloadError::NoProviders);
    }

    // If chapter has a preferred provider, restrict to just that one
    if let Some(ref preferred) = chapter.preferred_provider {
        let filtered: Vec<_> = entries.iter().filter(|e| &e.provider_name == preferred).cloned().collect();
        if !filtered.is_empty() {
            entries = filtered;
        } else {
            log::warn!(
                "[dl] Preferred provider '{}' has no cached URL for chapter {}; falling back to all providers.",
                preferred, chapter.number_raw
            );
        }
    }

    // Sort by tier (ascending: tier 1 = Official first, tier 4 = No group last)
    entries.sort_by_key(|e| compute_tier(e.scanlator_group.as_deref(), &trusted_groups));

    // Build provider lookup map
    let provider_map: std::collections::HashMap<&str, &Arc<dyn crate::scraper::Provider>> =
        registry.all().into_iter().map(|p| (p.name(), p)).collect();

    let mut last_err = String::new();

    for entry in &entries {
        let Some(provider) = provider_map.get(entry.provider_name.as_str()) else {
            log::warn!("[dl] Provider '{}' is in DB but not loaded.", entry.provider_name);
            continue;
        };

        log::info!(
            "[dl] Trying {} (tier {}) for chapter {} of '{}'…",
            provider.name(),
            compute_tier(entry.scanlator_group.as_deref(), &trusted_groups),
            chapter.number_raw,
            manga.metadata.title
        );

        let chapter_url = match ensure_chapter_url(pool, ctx, provider, manga.id, entry, chapter).await {
            Some(url) => url,
            None => {
                log::warn!("[dl] Chapter {} not found on {}.", chapter.number_raw, provider.name());
                last_err = format!("chapter {} not found on {}", chapter.number_raw, provider.name());
                continue;
            }
        };

        let pages = match provider.pages(ctx, &chapter_url).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("[dl] pages() failed on {}: {e}", provider.name());
                last_err = e.to_string();
                continue;
            }
        };

        if pages.is_empty() {
            log::warn!("[dl] {} returned 0 pages for chapter {}.", provider.name(), chapter.number_raw);
            last_err = format!("0 pages returned by {}", provider.name());
            continue;
        }

        match download_pages(ctx, &pages).await {
            Ok(image_data) => {
                let mut cbz_name = format!("Chapter {}", chapter.number_sort);
                if let Some(ref t) = chapter.title {
                    cbz_name.push_str(&format!(" - {t}"));
                }
                if let Some(ref g) = chapter.scanlator_group {
                    cbz_name.push_str(&format!(" [{g}]"));
                }
                let cbz_name: String = cbz_name
                    .chars()
                    .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
                    .collect();
                let cbz_path = lib_root
                    .join(&manga.relative_path)
                    .join(format!("{cbz_name}.cbz"));

                if let Err(e) = write_cbz(&cbz_path, manga, chapter, image_data).await {
                    log::warn!("[dl] CBZ write failed: {e}");
                    last_err = e.to_string();
                    continue;
                }

                db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloaded, Some(Utc::now())).await?;
                db_chapter::update_manga_counts(pool, manga.id).await?;

                log::info!(
                    "[dl] Chapter {} of '{}' saved to {}",
                    chapter.number_raw, manga.metadata.title, cbz_path.display()
                );
                return Ok(());
            }
            Err(e) => {
                log::warn!("[dl] Image download failed on {}: {e}", provider.name());
                last_err = e.to_string();
                continue;
            }
        }
    }

    db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
    Err(DownloadError::AllProvidersFailed(last_err))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Use the cached chapter URL from the entry; fall back to re-scraping if the URL is missing.
async fn ensure_chapter_url(
    pool: &sqlx::SqlitePool,
    ctx: &ScraperCtx,
    provider: &Arc<dyn crate::scraper::Provider>,
    manga_id: uuid::Uuid,
    entry: &db_chapter_url::ChapterProviderEntry,
    chapter: &Chapter,
) -> Option<String> {
    if !entry.chapter_url.is_empty() {
        log::debug!("[dl] Using cached URL for chapter {} from {}.", chapter.number_raw, provider.name());
        return Some(entry.chapter_url.clone());
    }

    // No URL in cache — re-scrape the full chapter list for this provider
    let manga_url = entry.manga_provider_url.as_deref()?;
    log::debug!("[dl] Cache miss for chapter {} on {}; re-scraping.", chapter.number_raw, provider.name());
    let infos = provider.chapters(ctx, manga_url).await.ok()?;

    for info in &infos {
        if let Some(url) = &info.url {
            let _ = db_chapter_url::upsert(
                pool, manga_id, provider.name(), info.number, url,
                info.scanlator_group.as_deref(),
            ).await;
        }
    }

    infos
        .into_iter()
        .find(|info| (info.number - chapter.number_sort).abs() < 0.01)
        .and_then(|info| info.url)
}

/// Download all page images concurrently (max 4 parallel). Returns Vec<(index, bytes)> sorted by index.
async fn download_pages(
    ctx: &ScraperCtx,
    pages: &[crate::scraper::PageUrl],
) -> Result<Vec<(u32, Vec<u8>)>, DownloadError> {
    let semaphore = Arc::new(Semaphore::new(4));
    let mut handles = Vec::with_capacity(pages.len());

    for page in pages {
        let url = page.url.clone();
        let index = page.index;
        let http = ctx.http.clone();
        let sem = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let bytes = http.get(&url).send().await?.bytes().await?;
            Ok::<(u32, Vec<u8>), reqwest::Error>((index, bytes.to_vec()))
        }));
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        let (index, data) = handle
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .map_err(DownloadError::Http)?;
        results.push((index, data));
    }

    results.sort_by_key(|(idx, _)| *idx);
    Ok(results)
}

/// Write image data as a CBZ (ZIP) file with a rich ComicInfo.xml.
async fn write_cbz(
    path: &Path,
    manga: &Manga,
    chapter: &Chapter,
    images: Vec<(u32, Vec<u8>)>,
) -> Result<(), DownloadError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let path = path.to_owned();
    let comic_info = comicinfo::generate_chapter_xml(manga, chapter, images.len());

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("ComicInfo.xml", options)?;
        zip.write_all(comic_info.as_bytes())?;

        for (index, data) in images {
            let ext = image_ext(&data);
            let name = format!("{index:04}.{ext}");
            zip.start_file(name, options)?;
            zip.write_all(&data)?;
        }

        zip.finish()?;
        Ok::<(), DownloadError>(())
    })
    .await
    .map_err(|e| std::io::Error::other(e.to_string()))??;

    Ok(())
}

/// Guess image extension from magic bytes.
fn image_ext(data: &[u8]) -> &'static str {
    match data {
        d if d.starts_with(b"\xFF\xD8\xFF") => "jpg",
        d if d.starts_with(b"\x89PNG") => "png",
        d if d.starts_with(b"GIF8") => "gif",
        d if d.starts_with(b"RIFF") && d.len() >= 12 && &d[8..12] == b"WEBP" => "webp",
        _ => "jpg",
    }
}
