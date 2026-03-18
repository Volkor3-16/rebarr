use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::db::{chapter as db_chapter, provider as db_provider, settings as db_settings};
use crate::manga::comicinfo;
use crate::manga::manga::{Chapter, DownloadStatus, Manga};
use crate::manga::scoring::{ChapterFilter, rank_entries};
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
    #[error("cancelled")]
    Cancelled,
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
/// 1. Load all Chapters rows for this chapter number (all providers).
/// 2. Rank by language filter + scanlation tier.
/// 3. For each provider: use cached chapter_url (re-scrape if missing), get pages, write CBZ.
/// 4. If all providers fail, mark as Failed and return Err.
pub async fn download_chapter(
    pool: &sqlx::SqlitePool,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
    cancel_token: CancellationToken,
) -> Result<(), DownloadError> {
    db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloading, None).await?;

    let trusted_groups = db_provider::get_trusted_groups(pool).await?;

    // Get all Chapters rows for this chapter number (all providers = all alternatives)
    let all_entries =
        db_chapter::get_all_for_chapter(pool, manga.id, chapter.chapter_base, chapter.chapter_variant)
            .await?;

    if all_entries.is_empty() {
        db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
        return Err(DownloadError::NoProviders);
    }

    // Rank: language filter → tier sort
    let lang_raw = db_settings::get(pool, "preferred_language", "").await?;
    let entries = rank_entries(
        all_entries,
        &ChapterFilter {
            language: if lang_raw.is_empty() { None } else { Some(lang_raw) },
        },
        &trusted_groups,
    );

    let provider_map: std::collections::HashMap<&str, &Arc<dyn crate::scraper::Provider>> =
        registry.all().into_iter().map(|p| (p.name(), p)).collect();

    let mut last_err = String::new();

    for entry in &entries {
        // Check for cancellation before each provider attempt
        if cancel_token.is_cancelled() {
            db_chapter::set_status(pool, chapter.id, DownloadStatus::Missing, None).await?;
            return Err(DownloadError::Cancelled);
        }

        let provider_name = match &entry.provider_name {
            Some(n) => n.as_str(),
            None => {
                // Manually added file — skip
                continue;
            }
        };

        let Some(provider) = provider_map.get(provider_name) else {
            log::warn!("[dl] Provider '{provider_name}' is in DB but not loaded.");
            continue;
        };

        log::info!(
            "[dl] Trying {} for chapter {} of '{}'…",
            provider.name(),
            chapter.number_sort(),
            manga.metadata.title
        );

        let chapter_url = match ensure_chapter_url(pool, ctx, provider, manga.id, entry).await {
            Some(url) => url,
            None => {
                log::warn!("[dl] Chapter {} not found on {}.", chapter.number_sort(), provider.name());
                last_err = format!("chapter {} not found on {}", chapter.number_sort(), provider.name());
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
            log::warn!(
                "[dl] {} returned 0 pages for chapter {}.",
                provider.name(),
                chapter.number_sort()
            );
            last_err = format!("0 pages returned by {}", provider.name());
            continue;
        }

        match download_pages(ctx, &pages, provider.page_delay_ms(), cancel_token.clone()).await {
            Ok(image_data) => {
                let mut cbz_name = format!("Chapter {}", chapter.number_sort());
                if let Some(ref t) = chapter.title {
                    cbz_name.push_str(&format!(" - {t}"));
                }
                if let Some(g) = chapter.scanlator_group.as_deref().filter(|s| !s.is_empty()) {
                    cbz_name.push_str(&format!(" [{g}]"));
                }
                let cbz_name: String = cbz_name
                    .chars()
                    // TODO: surely there's a better path-safe conversion thing that already exists in rust.
                    .map(|c| {
                        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
                let cbz_path = lib_root
                    .join(&manga.relative_path)
                    .join(format!("{cbz_name}.cbz"));

                if let Err(e) = write_cbz(&cbz_path, manga, chapter, image_data).await {
                    log::warn!("[dl] CBZ write failed: {e}");
                    last_err = e.to_string();
                    continue;
                }

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
                    chapter.number_sort(),
                    manga.metadata.title,
                    cbz_path.display()
                );
                return Ok(());
            }
            Err(DownloadError::Cancelled) => {
                db_chapter::set_status(pool, chapter.id, DownloadStatus::Missing, None).await?;
                return Err(DownloadError::Cancelled);
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

/// Use the cached chapter_url from the entry; fall back to re-scraping if missing.
async fn ensure_chapter_url(
    pool: &sqlx::SqlitePool,
    ctx: &ScraperCtx,
    provider: &Arc<dyn crate::scraper::Provider>,
    manga_id: uuid::Uuid,
    entry: &Chapter,
) -> Option<String> {
    if let Some(url) = &entry.chapter_url {
        if !url.is_empty() {
            log::debug!(
                "[dl] Using cached URL for chapter {} from {}.",
                entry.number_sort(),
                provider.name()
            );
            return Some(url.clone());
        }
    }

    // No URL — re-scrape the full chapter list for this provider
    let manga_provider =
        crate::db::provider::get_for_manga_provider(pool, manga_id, provider.name())
            .await
            .ok()??;
    let manga_url = manga_provider.provider_url.as_deref()?;

    log::debug!(
        "[dl] Cache miss for chapter {} on {}; re-scraping.",
        entry.number_sort(),
        provider.name()
    );

    let infos = provider.chapters(ctx, manga_url).await.ok()?;

    // Write the re-scraped data back to Chapters
    let _ = db_chapter::upsert_from_scrape(pool, manga_id, provider.name(), &infos).await;

    infos
        .into_iter()
        .find(|info| {
            (info.chapter_base as i32 == entry.chapter_base)
                && (info.chapter_variant as i32 == entry.chapter_variant)
        })
        .and_then(|info| info.url)
}

/// Download all page images concurrently (max 4 parallel). Returns Vec<(index, bytes)> sorted.
/// `page_delay_ms` — if non-zero, sleep this many ms after acquiring the semaphore permit
/// before each request to avoid hammering the provider.
async fn download_pages(
    ctx: &ScraperCtx,
    pages: &[crate::scraper::PageUrl],
    page_delay_ms: u64,
    cancel_token: CancellationToken,
) -> Result<Vec<(u32, Vec<u8>)>, DownloadError> {
    let semaphore = Arc::new(Semaphore::new(4));
    let mut handles = Vec::with_capacity(pages.len());

    for page in pages {
        if cancel_token.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }

        let url = page.url.clone();
        let index = page.index;
        let http = ctx.http.clone();
        let sem = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if page_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(page_delay_ms)).await;
            }
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
