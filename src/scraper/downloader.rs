use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use tracing::{debug, info, warn, instrument};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::db::{
    chapter as db_chapter, provider as db_provider, settings as db_settings, task as db_task,
};
use crate::manga::{comicinfo, files};
use crate::manga::core::{Chapter, DownloadStatus, Manga};
use crate::manga::scoring::{ChapterFilter, compute_tier, rank_entries};
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
#[instrument(skip(pool, registry, ctx, cancel_token),
    fields(manga = %manga.metadata.title, chapter = chapter.number_sort()))]
#[allow(clippy::too_many_arguments)]
pub async fn download_chapter(
    pool: &sqlx::SqlitePool,
    task_id: uuid::Uuid,
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
    cancel_token: CancellationToken,
) -> Result<(), DownloadError> {
    info!(
        "[dl] Starting download: manga='{}', ch={}.{}, canonical_id={}",
        manga.metadata.title, chapter.chapter_base, chapter.chapter_variant, chapter.id
    );

    db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloading, None).await?;

    let trusted_groups = db_provider::get_trusted_groups(pool).await?;

    // Get all Chapters rows for this chapter number (all providers = all alternatives)
    let all_entries = db_chapter::get_all_for_chapter(
        pool,
        manga.id,
        chapter.chapter_base,
        chapter.chapter_variant,
    )
    .await?;

    if all_entries.is_empty() {
        db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
        return Err(DownloadError::NoProviders);
    }

    // Rank: language filter → tier sort
    let lang_raw = db_settings::get(pool, "preferred_language", "").await?;
    let ranked = rank_entries(
        all_entries,
        &ChapterFilter {
            language: if lang_raw.is_empty() {
                None
            } else {
                Some(lang_raw)
            },
        },
        &trusted_groups,
    );

    // Always try the user-selected canonical chapter first, then fall back to ranked order.
    let mut entries: Vec<Chapter> = Vec::with_capacity(ranked.len());
    let mut fallbacks: Vec<Chapter> = Vec::with_capacity(ranked.len());
    for entry in ranked {
        if entry.id == chapter.id {
            entries.push(entry);
        } else {
            fallbacks.push(entry);
        }
    }
    entries.extend(fallbacks);

    let provider_map: std::collections::HashMap<&str, &Arc<dyn crate::scraper::Provider>> =
        registry.all().into_iter().map(|p| (p.name(), p)).collect();

    let mut last_err = String::new();
    let total_providers = entries.len() as i64;

    for (provider_idx, entry) in entries.iter().enumerate() {
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
            warn!("[dl] Provider '{provider_name}' is in DB but not loaded.");
            continue;
        };

        info!(
            "[dl] Trying {} for chapter {} of '{}'…",
            provider.name(),
            chapter.number_sort(),
            manga.metadata.title
        );

        let _ = db_task::set_progress(
            pool,
            task_id,
            &db_task::TaskProgress {
                step: Some("download-provider".to_owned()),
                label: Some(format!(
                    "Trying provider {} of {}",
                    provider_idx + 1,
                    entries.len()
                )),
                detail: Some(format!(
                    "Resolving pages from {} for chapter {}",
                    provider.name(),
                    chapter.number_sort()
                )),
                provider: Some(provider.name().to_owned()),
                current: Some((provider_idx + 1) as i64),
                total: Some(total_providers),
                unit: Some("provider".to_owned()),
                ..Default::default()
            },
        )
        .await;

        let chapter_url = match ensure_chapter_url(pool, ctx, provider, manga.id, entry).await {
            Some(url) => url,
            None => {
                warn!(
                    "[dl] Chapter {} not found on {}.",
                    chapter.number_sort(),
                    provider.name()
                );
                last_err = format!(
                    "chapter {} not found on {}",
                    chapter.number_sort(),
                    provider.name()
                );
                continue;
            }
        };

        let pages = match ctx.executor.pages(ctx, provider, &chapter_url).await {
            Ok(p) => p,
            Err(e) => {
                warn!("[dl] pages() failed on {}: {e}", provider.name());
                last_err = e.to_string();
                continue;
            }
        };

        if pages.is_empty() {
            warn!(
                "[dl] {} returned 0 pages for chapter {}.",
                provider.name(),
                chapter.number_sort()
            );
            last_err = format!("0 pages returned by {}", provider.name());
            continue;
        }

        let _ = db_task::set_progress(
            pool,
            task_id,
            &db_task::TaskProgress {
                step: Some("download-pages".to_owned()),
                label: Some(format!("Downloading {} page(s)", pages.len())),
                detail: Some(format!(
                    "{} returned {} page(s) for chapter {}",
                    provider.name(),
                    pages.len(),
                    chapter.number_sort()
                )),
                provider: Some(provider.name().to_owned()),
                target: Some(chapter_url.clone()),
                current: Some(0),
                total: Some(pages.len() as i64),
                unit: Some("page".to_owned()),
            },
        )
        .await;

        match download_pages_via_browser(
            Some(pool),
            Some(task_id),
            ctx,
            Some(provider.name()),
            &pages,
            provider.page_delay_ms(),
            &chapter_url,
            cancel_token.clone(),
        )
        .await
        {
            Ok(image_data) => {
                let cbz_path =
                    files::chapter_cbz_path(&files::series_dir(lib_root, manga), chapter);

                if let Err(e) = write_cbz(&cbz_path, manga, chapter, image_data).await {
                    warn!("[dl] CBZ write failed: {e}");
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

                // Record file size (best-effort; ignore errors)
                if let Ok(meta) = tokio::fs::metadata(&cbz_path).await {
                    let _ = db_chapter::set_file_size(pool, chapter.id, meta.len() as i64).await;
                }

                // Remove any previously-downloaded lower-tier variants for this chapter slot.
                cleanup_superseded_downloads(pool, manga, chapter, lib_root, &trusted_groups).await;

                db_chapter::update_manga_counts(pool, manga.id).await?;

                info!(
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
                warn!("[dl] Image download failed on {}: {e}", provider.name());
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
            debug!(
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

    debug!(
        "[dl] Cache miss for chapter {} on {}; re-scraping.",
        entry.number_sort(),
        provider.name()
    );

    let infos = ctx.executor.chapters(ctx, provider, manga_url).await.ok()?;

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

/// Download page images using a two-tier strategy:
/// 1. JS fetch() from the chapter page context (avoids Chrome's image-URL navigation blocking).
/// 2. reqwest with spoofed headers + browser session cookies (fallback for CORS-restricted CDNs).
///
/// `page_delay_ms` — sleep this many ms between images to avoid rate-limiting.
/// `chapter_url` — navigated once to establish page context, and used as the `Referer` header.
#[allow(clippy::too_many_arguments)]
pub async fn download_pages_via_browser(
    pool: Option<&sqlx::SqlitePool>,
    task_id: Option<uuid::Uuid>,
    ctx: &ScraperCtx,
    provider_name: Option<&str>,
    pages: &[crate::scraper::PageUrl],
    page_delay_ms: u64,
    chapter_url: &str,
    cancel_token: CancellationToken,
) -> Result<Vec<(u32, Vec<u8>)>, DownloadError> {
    let mut results = Vec::with_capacity(pages.len());
    let _browser_slot = ctx.executor.acquire_browser_slot().await;

    let browser = ctx
        .browser
        .get()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let page = browser
        .new_blank_page()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Enable Network domain (required before setExtraHTTPHeaders) and inject
    // Referer at the CDP layer so it applies to every JS fetch() call below.
    page.enable_request_capture()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut extra_headers = HashMap::new();
    extra_headers.insert("Referer".to_string(), chapter_url.to_string());
    debug!("Referer: {chapter_url}");
    page.set_extra_headers(extra_headers)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Navigate to the chapter page once to establish a trusted origin with
    // valid session cookies. JS fetch() calls from this context won't be
    // blocked by Chrome's ERR_BLOCKED_BY_CLIENT (unlike direct image URL navigation).
    if let Err(e) = page.goto(chapter_url).await {
        warn!("[dl] could not navigate to chapter URL {chapter_url}: {e}");
    } else {
        page.wait_for_network_idle(500, 10_000).await.ok();
    }

    // Snapshot browser cookies for the reqwest fallback so session state carries over.
    let browser_cookies = page.cookies().await.unwrap_or_default();
    let cookie_header = browser_cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");

    // Once Tier 1 fails for any page (e.g. CORS), all pages in this chapter share the same
    // CDN domain, so skip Tier 1 for all remaining pages.
    let mut tier1_failed = false;

    let total_pages = pages.len() as i64;

    for (idx, page_url) in pages.iter().enumerate() {
        if cancel_token.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        if page_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(page_delay_ms)).await;
        }
        let url = &page_url.url;
        let referrer = page_url.referrer.as_deref().unwrap_or(chapter_url);
        tracing::trace!(page = idx + 1, total = pages.len(), %url, "downloading page");

        if let (Some(pool), Some(task_id)) = (pool, task_id) {
            let _ = db_task::set_progress(
                pool,
                task_id,
                &db_task::TaskProgress {
                    step: Some("download-pages".to_owned()),
                    label: Some(format!("Downloading page {} of {}", idx + 1, pages.len())),
                    detail: Some(format!("Fetching page {}", page_url.index)),
                    provider: provider_name.map(|name| name.to_owned()),
                    target: Some(url.clone()),
                    current: Some((idx + 1) as i64),
                    total: Some(total_pages),
                    unit: Some("page".to_owned()),
                },
            )
            .await;
        }

        // Tier 1: JS fetch() from the chapter page context.
        // Subrequests are not subject to ERR_BLOCKED_BY_CLIENT. CDP extra headers
        // inject the Referer automatically. Works for CDNs with CORS headers.
        let image_data = if tier1_failed {
            None
        } else {
            let escaped_url = url.replace('\\', "\\\\").replace('"', "\\\"");
            let js = format!(
                r#"(async () => {{
                    const r = await fetch("{escaped_url}", {{ cache: 'reload' }});
                    if (!r.ok) return null;
                    const buf = await r.arrayBuffer();
                    const bytes = new Uint8Array(buf);
                    let binary = "";
                    for (let i = 0; i < bytes.length; i += 8192) {{
                        binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
                    }}
                    return btoa(binary);
                }})()"#
            );

            match page.evaluate::<Option<String>>(&js).await {
                Ok(Some(b64)) => match BASE64.decode(b64.trim()) {
                    Ok(data) if !data.is_empty() => Some(data),
                    Ok(_) => {
                        debug!(
                            "[dl] tier1 empty bytes for {url}, switching to reqwest for remaining pages"
                        );
                        tier1_failed = true;
                        None
                    }
                    Err(e) => {
                        debug!(
                            "[dl] tier1 base64 decode failed for {url}: {e}, switching to reqwest"
                        );
                        tier1_failed = true;
                        None
                    }
                },
                Ok(None) => {
                    debug!(
                        "[dl] tier1 null for {url} (likely CORS), switching to reqwest for remaining pages"
                    );
                    tier1_failed = true;
                    None
                }
                Err(e) => {
                    debug!(
                        "[dl] tier1 eval error for {url}: {e}, switching to reqwest for remaining pages"
                    );
                    tier1_failed = true;
                    None
                }
            }
        };

        let image_data = if let Some(data) = image_data {
            data
        } else {
            // Tier 2: reqwest with spoofed browser headers + forwarded session cookies.
            // Works for CDNs with simple Referer-based hotlink protection.
            // Some CDNs (e.g. AllManga) reject the full chapter path and only accept the
            // site origin as Referer, so strip the path here.
            let referer_origin = url_origin(referrer);
            let mut req = ctx
                .http
                .get(url.as_str())
                .header("Referer", &referer_origin)
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36",
                )
                .header("Accept", "image/webp,image/apng,image/*,*/*;q=0.8");
            if !cookie_header.is_empty() {
                req = req.header("Cookie", &cookie_header);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(bytes) if !bytes.is_empty() => bytes.to_vec(),
                    Ok(_) => {
                        warn!("[dl] tier2 empty body for {url}");
                        continue;
                    }
                    Err(e) => {
                        warn!("[dl] tier2 body read failed for {url}: {e}");
                        continue;
                    }
                },
                Ok(resp) => {
                    warn!("[dl] tier2 HTTP {} for {url}", resp.status());
                    continue;
                }
                Err(e) => {
                    warn!("[dl] tier2 request failed for {url}: {e}");
                    continue;
                }
            }
        };

        results.push((page_url.index, image_data));
    }

    page.clear_extra_headers()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Close the Chrome tab explicitly — otherwise it stays open and leaks memory.
    let _ = browser.close_tab(page.target_id()).await;

    results.sort_by_key(|(idx, _)| *idx);
    Ok(results)
}

/// After a successful download, remove any previously-downloaded lower-tier variants
/// for the same (chapter_base, chapter_variant) slot.
///
/// Non-fatal: logs errors and continues. Only removes variants with a strictly worse
/// tier than `chapter` — same-tier variants are left untouched.
async fn cleanup_superseded_downloads(
    pool: &sqlx::SqlitePool,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
    trusted_groups: &[String],
) {
    let all_variants = match db_chapter::get_all_for_chapter(
        pool,
        manga.id,
        chapter.chapter_base,
        chapter.chapter_variant,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!("[dl] cleanup: could not load chapter variants: {e}");
            return;
        }
    };

    let new_tier = compute_tier(chapter.scanlator_group.as_deref(), trusted_groups);
    let series_dir = lib_root.join(&manga.relative_path);
    let number_prefix = format!("Chapter {}", chapter.number_sort());

    for variant in &all_variants {
        if variant.id == chapter.id {
            continue;
        }
        if variant.download_status != DownloadStatus::Downloaded {
            continue;
        }
        let old_tier = compute_tier(variant.scanlator_group.as_deref(), trusted_groups);
        if old_tier <= new_tier {
            continue; // Same or better tier — don't touch
        }

        // Find the CBZ file: prefix-match "Chapter {number}*.cbz" in series dir.
        let cbz_path = std::fs::read_dir(&series_dir).ok().and_then(|entries| {
            entries.flatten().find_map(|e| {
                let fname = e.file_name();
                let name = fname.to_string_lossy();
                if name.starts_with(&number_prefix) && name.ends_with(".cbz") {
                    Some(e.path())
                } else {
                    None
                }
            })
        });

        if let Some(path) = cbz_path {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("[dl] cleanup: failed to remove {}: {e}", path.display());
            } else {
                info!("[dl] cleanup: removed superseded {}", path.display());
            }
        }

        if let Err(e) =
            db_chapter::set_status(pool, variant.id, DownloadStatus::Missing, None).await
        {
            warn!(
                "[dl] cleanup: failed to mark variant {} as Missing: {e}",
                variant.id
            );
        }
    }
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
    let comic_info = comicinfo::generate_chapter_xml(
        manga,
        chapter,
        images.len(),
        chapter.provider_name.as_deref(),
    );

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

/// Extract the origin (scheme + host + "/") from a URL.
/// "https://allmanga.to/manga/abc/ch-1" → "https://allmanga.to/"
fn url_origin(url: &str) -> String {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = url[after_scheme..]
        .find('/')
        .map(|i| i + after_scheme)
        .unwrap_or(url.len());
    format!("{}/", &url[..host_end])
}

/// Guess image extension from magic bytes.
pub fn image_ext(data: &[u8]) -> &'static str {
    match data {
        d if d.starts_with(b"\xFF\xD8\xFF") => "jpg",
        d if d.starts_with(b"\x89PNG") => "png",
        d if d.starts_with(b"GIF8") => "gif",
        d if d.starts_with(b"RIFF") && d.len() >= 12 && &d[8..12] == b"WEBP" => "webp",
        _ => "jpg",
    }
}
