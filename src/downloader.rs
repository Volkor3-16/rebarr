use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::comicinfo;
use crate::db::{chapter as db_chapter, chapter_url as db_chapter_url, provider as db_provider};
use crate::manga::{Chapter, DownloadStatus, Manga};
use crate::scraper::{ProviderRegistry, ScraperCtx};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("no provider URLs found for this manga — run a scan first")]
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
/// 1. Look up cached provider URLs for this manga.
/// 2. For each provider (highest score first):
///    a. Re-scrape chapter list to find the URL for this chapter.
///    b. Scrape page image URLs.
///    c. Download all images concurrently (max 4 in parallel).
///    d. Write a CBZ to `lib_root/manga.relative_path/Chapter <num>.cbz`.
///    e. Mark chapter Downloaded and update manga counts.
/// 3. If all providers fail, mark as Failed and return Err.
pub async fn download_chapter(
    pool: &sqlx::SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
) -> Result<(), DownloadError> {
    // Mark as Downloading immediately so the UI can reflect in-progress state
    db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloading, None).await?;

    let provider_entries = db_provider::get_all_for_manga(pool, manga.id).await?;
    if provider_entries.is_empty() {
        db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
        return Err(DownloadError::NoProviders);
    }

    // Build a lookup from provider name → Arc<dyn Provider>
    let provider_map: std::collections::HashMap<&str, &Arc<dyn crate::scraper::Provider>> =
        registry
            .all()
            .into_iter()
            .map(|p| (p.name(), p))
            .collect();

    let mut ordered: Vec<_> = provider_entries
        .iter()
        .filter_map(|e| provider_map.get(e.provider_name.as_str()).map(|p| (e, *p)))
        .collect();
    // Sort by effective score descending:
    //   score_override (user-set) > provider_score (auto-computed)
    ordered.sort_by(|a, b| {
        let eff_a = a.0.effective_score();
        let eff_b = b.0.effective_score();
        eff_b.partial_cmp(&eff_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut last_err = String::new();

    for (entry, provider) in &ordered {
        log::info!(
            "[dl] Trying {} for chapter {} of '{}'…",
            provider.name(),
            chapter.number_raw,
            manga.metadata.title
        );

        // Find chapter URL — check cache first, re-scrape only if missing
        let chapter_url = match find_chapter_url(pool, ctx, provider, manga.id, &entry.provider_url, chapter).await
        {
            Some(url) => url,
            None => {
                log::warn!(
                    "[dl] Chapter {} not found on {}.",
                    chapter.number_raw,
                    provider.name()
                );
                last_err = format!(
                    "chapter {} not found on {}",
                    chapter.number_raw,
                    provider.name()
                );
                continue;
            }
        };

        // Get page URLs
        let pages = match provider.pages(ctx, &chapter_url).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("[dl] pages() failed on {}: {e}", provider.name());
                last_err = e.to_string();
                continue;
            }
        };

        if pages.is_empty() {
            log::warn!(
                "[dl] {} returned 0 pages for chapter {}.",
                provider.name(),
                chapter.number_raw
            );
            last_err = format!("0 pages returned by {}", provider.name());
            continue;
        }

        // Download images concurrently (max 4 at once)
        match download_pages(ctx, &pages).await {
            Ok(image_data) => {
                // Build CBZ filename matching the web UI label format.
                let mut cbz_name = format!("Chapter {}", chapter.number_sort);
                if let Some(ref t) = chapter.title {
                    cbz_name.push_str(&format!(" - {t}"));
                }
                if let Some(ref g) = chapter.scanlator_group {
                    cbz_name.push_str(&format!(" [{g}]"));
                }
                // Sanitize: strip characters that are illegal in filenames.
                let cbz_name: String = cbz_name
                    .chars()
                    .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
                    .collect();
                let cbz_path = lib_root
                    .join(&manga.relative_path)
                    .join(format!("{cbz_name}.cbz"));

                if let Err(e) = write_cbz(&cbz_path, manga, chapter, image_data).await
                {
                    log::warn!("[dl] CBZ write failed: {e}");
                    last_err = e.to_string();
                    continue;
                }

                // Mark downloaded and update counts
                db_chapter::set_status(
                    pool,
                    chapter.id,
                    DownloadStatus::Downloaded,
                    Some(Utc::now()),
                )
                .await?;
                db_chapter::update_manga_counts(pool, manga.id).await?;

                log::info!(
                    "[dl] Chapter {} of '{}' saved to {}",
                    chapter.number_raw,
                    manga.metadata.title,
                    cbz_path.display()
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

/// Find the chapter page URL, checking the DB cache before re-scraping.
async fn find_chapter_url(
    pool: &sqlx::SqlitePool,
    ctx: &ScraperCtx,
    provider: &Arc<dyn crate::scraper::Provider>,
    manga_id: uuid::Uuid,
    manga_url: &str,
    chapter: &Chapter,
) -> Option<String> {
    // Check the cache first — avoids re-scraping the full chapter list
    if let Ok(Some(url)) = db_chapter_url::get(pool, manga_id, provider.name(), chapter.number_sort).await {
        log::debug!("[dl] Using cached URL for chapter {} from {}.", chapter.number_raw, provider.name());
        return Some(url);
    }

    // Cache miss — scrape the full chapter list and cache all URLs for future use
    log::debug!("[dl] Cache miss for chapter {} on {}; re-scraping chapter list.", chapter.number_raw, provider.name());
    let infos = provider.chapters(ctx, manga_url).await.ok()?;

    // Cache every URL we got back
    for info in &infos {
        if let Some(url) = &info.url {
            let _ = db_chapter_url::upsert(pool, manga_id, provider.name(), info.number, url).await;
        }
    }

    infos
        .into_iter()
        .find(|info| (info.number - chapter.number_sort).abs() < 0.01)
        .and_then(|info| info.url)
}

/// Download all page images concurrently (max 4 parallel).
/// Returns Vec<(index, bytes)> sorted by index.
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

    // Spawn blocking because zip I/O is synchronous
    let path = path.to_owned();
    let comic_info = comicinfo::generate_chapter_xml(manga, chapter, images.len());

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("ComicInfo.xml", options)?;
        zip.write_all(comic_info.as_bytes())?;

        // Image files
        for (index, data) in images {
            // Guess extension from magic bytes
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
        _ => "jpg", // fallback
    }
}
