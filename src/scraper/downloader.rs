use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

use crate::db::{
    chapter as db_chapter, quality_rules as db_quality_rules, settings as db_settings,
    task as db_task,
};
use crate::manga::core::{Chapter, DownloadStatus, Manga};
use crate::manga::scoring::{compute_score, rank_entries_scored};
use crate::manga::{comicinfo, files};
use crate::scraper::def::DownloadMethod;
use crate::scraper::engine::{is_cf_challenge, try_cf_checkbox_click};
use crate::scraper::{ProviderRegistry, ScraperCtx, browser::close_page_tab};

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
    #[error("invalid image data (likely HTML or error page)")]
    InvalidImage,
    #[error("no valid images downloaded")]
    NoValidImages,
    #[error("failed to download all pages: got {ok}/{total}")]
    IncompleteDownload { ok: usize, total: usize },
    #[error("too many duplicate identical images (likely placeholder/404 failure)")]
    DuplicateImages,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
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

    let quality_rules = db_quality_rules::get_all(pool).await?;

    // Read download mode setting
    let download_mode = db_settings::get(pool, "download_mode", "must_have").await?;
    let best_only = download_mode == "best_only";

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

    // Rank: language filter → quality score sort
    let lang_raw = db_settings::get(pool, "preferred_language", "").await?;
    let lang_filter = if lang_raw.is_empty() {
        None
    } else {
        Some(lang_raw.as_str())
    };
    let ranked = rank_entries_scored(all_entries, lang_filter, &quality_rules);

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

    // Batch pre-pass: identify entries missing chapter_url, group by provider,
    // and re-scrape each provider's chapter list once instead of once per entry.
    {
        let mut providers_to_scrape: std::collections::HashSet<&str> =
            std::collections::HashSet::new();
        for entry in &entries {
            if entry.provider_name.is_none() {
                continue;
            }
            let has_url = entry.chapter_url.as_ref().is_some_and(|u| !u.is_empty());
            if !has_url {
                let pname = entry.provider_name.as_deref().unwrap();
                if provider_map.contains_key(pname) {
                    providers_to_scrape.insert(pname);
                }
            }
        }

        if !providers_to_scrape.is_empty() {
            info!(
                "[dl] Batch re-scraping {} provider(s) with missing chapter URLs for '{}'",
                providers_to_scrape.len(),
                manga.metadata.title
            );
            for provider_name in &providers_to_scrape {
                let provider = provider_map.get(provider_name).unwrap();
                let Some(manga_provider) =
                    crate::db::provider::get_for_manga_provider(pool, manga.id, provider_name)
                        .await
                        .ok()
                        .flatten()
                else {
                    continue;
                };
                let Some(manga_url) = manga_provider.provider_url.as_deref() else {
                    continue;
                };

                debug!(
                    "[dl] Batch re-scraping chapter list from {provider_name} for '{}'",
                    manga.metadata.title
                );
                if let Ok(infos) = ctx
                    .executor
                    .chapters(ctx, provider, manga_url, &manga_provider.provider_data)
                    .await
                {
                    let _ =
                        db_chapter::upsert_from_scrape(pool, manga.id, provider_name, &infos).await;
                }
            }

            // Re-fetch entries from DB so the loop below sees updated chapter_url values.
            let fresh_all = db_chapter::get_all_for_chapter(
                pool,
                manga.id,
                chapter.chapter_base,
                chapter.chapter_variant,
            )
            .await?;
            let fresh_ranked = rank_entries_scored(
                fresh_all,
                if lang_raw.is_empty() {
                    None
                } else {
                    Some(lang_raw.as_str())
                },
                &quality_rules,
            );

            // Rebuild entries preserving canonical-first ordering.
            let mut fresh_entries: Vec<Chapter> = Vec::with_capacity(fresh_ranked.len());
            let mut fresh_fallbacks: Vec<Chapter> = Vec::with_capacity(fresh_ranked.len());
            for entry in fresh_ranked {
                if entry.id == chapter.id {
                    fresh_entries.push(entry);
                } else {
                    fresh_fallbacks.push(entry);
                }
            }
            fresh_entries.extend(fresh_fallbacks);
            entries = fresh_entries;
        }
    }

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
                db_chapter::set_status(pool, entry.id, DownloadStatus::Failed, None).await?;
                if best_only {
                    break;
                }
                continue;
            }
        };

        let pages = match ctx.executor.pages(ctx, provider, &chapter_url).await {
            Ok(p) => p,
            Err(e) => {
                warn!("[dl] pages() failed on {}: {e}", provider.name());
                last_err = e.to_string();
                db_chapter::set_status(pool, entry.id, DownloadStatus::Failed, None).await?;
                if best_only {
                    break;
                }
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
            db_chapter::set_status(pool, entry.id, DownloadStatus::Failed, None).await?;
            if best_only {
                break;
            }
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
            &chapter_url,
            provider.pages_download_method(),
            cancel_token.clone(),
        )
        .await
        {
            Ok(image_data) => {
                // File is always named after the canonical chapter's sort number.
                let cbz_path =
                    files::chapter_cbz_path(&files::series_dir(lib_root, manga), chapter);

                // Use the entry that actually served the content for ComicInfo metadata.
                let cbz_chapter = if entry.id == chapter.id {
                    chapter
                } else {
                    entry
                };

                if let Err(e) = write_cbz(&cbz_path, manga, cbz_chapter, image_data).await {
                    warn!("[dl] CBZ write failed: {e}");
                    last_err = e.to_string();
                    db_chapter::set_status(pool, entry.id, DownloadStatus::Failed, None).await?;
                    if best_only {
                        break;
                    }
                    continue;
                }

                if entry.id == chapter.id {
                    // Canonical chapter downloaded successfully — normal path.
                    db_chapter::set_status(
                        pool,
                        chapter.id,
                        DownloadStatus::Downloaded,
                        Some(Utc::now()),
                    )
                    .await?;

                    if let Ok(meta) = tokio::fs::metadata(&cbz_path).await {
                        let _ =
                            db_chapter::set_file_size(pool, chapter.id, meta.len() as i64).await;
                    }

                    cleanup_superseded_downloads(pool, manga, chapter, lib_root, &quality_rules)
                        .await;
                } else {
                    // A fallback provider succeeded — the originally-requested canonical failed.
                    // Mark the canonical as Failed and promote the fallback to Downloaded + canonical.
                    db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
                    db_chapter::set_status(
                        pool,
                        entry.id,
                        DownloadStatus::Downloaded,
                        Some(Utc::now()),
                    )
                    .await?;

                    if let Ok(meta) = tokio::fs::metadata(&cbz_path).await {
                        let _ = db_chapter::set_file_size(pool, entry.id, meta.len() as i64).await;
                    }

                    if let Err(e) = db_chapter::set_canonical_override(
                        pool,
                        manga.id,
                        entry.chapter_base,
                        entry.chapter_variant,
                        entry.id,
                    )
                    .await
                    {
                        warn!(
                            "[dl] Failed to update canonical to fallback {}: {e}",
                            entry.id
                        );
                    }

                    cleanup_superseded_downloads(pool, manga, entry, lib_root, &quality_rules)
                        .await;
                }

                db_chapter::update_manga_counts(pool, manga.id).await?;

                info!(
                    "[dl] Chapter {} of '{}' saved to {} (provider: {})",
                    chapter.number_sort(),
                    manga.metadata.title,
                    cbz_path.display(),
                    cbz_chapter.provider_name.as_deref().unwrap_or("unknown"),
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
                db_chapter::set_status(pool, entry.id, DownloadStatus::Failed, None).await?;
                if best_only {
                    break;
                }
                continue;
            }
        }
    }

    // If the canonical itself was never tried (all entries were skipped due to unloaded providers),
    // ensure it ends up as Failed rather than stuck in Downloading.
    let canonical_still_downloading = db_chapter::get_by_id(pool, chapter.id)
        .await
        .ok()
        .flatten()
        .map(|ch| ch.download_status == DownloadStatus::Downloading)
        .unwrap_or(false);
    if canonical_still_downloading {
        db_chapter::set_status(pool, chapter.id, DownloadStatus::Failed, None).await?;
    }

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

    let infos = ctx
        .executor
        .chapters(ctx, provider, manga_url, &manga_provider.provider_data)
        .await
        .ok()?;

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

/// Download page images using a tiered strategy:
///
/// - Tier 1 (`Auto` mode): reqwest with cookies extracted from the browser session.
///   Fast and parallel-friendly; works for most providers after the browser has
///   established session cookies by navigating to the chapter page.
/// - Tier 2 (fallback, or `Browser` mode): CDP `Network.loadNetworkResource`.
///   Routes through Chrome's browser-process network stack, bypassing renderer-level
///   content filters (ERR_BLOCKED_BY_CLIENT) and Service Worker interception.
///
/// `chapter_url` is navigated once to establish cookies / auth state, and is used as
/// the `Referer` header for image requests.
#[allow(clippy::too_many_arguments)]
pub async fn download_pages_via_browser(
    pool: Option<&sqlx::SqlitePool>,
    task_id: Option<uuid::Uuid>,
    ctx: &ScraperCtx,
    provider_name: Option<&str>,
    pages: &[crate::scraper::PageUrl],
    chapter_url: &str,
    download_method: DownloadMethod,
    cancel_token: CancellationToken,
) -> Result<Vec<(u32, Vec<u8>)>, DownloadError> {
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
    ctx.browser.register_page(page.target_id());

    let result = async {
        let mut results = Vec::with_capacity(pages.len());

        // Enable Network domain (required before setExtraHTTPHeaders) and inject
        // Referer at the CDP layer so it applies to subrequests in the page context.
        page.enable_request_capture()
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut extra_headers = HashMap::new();
        extra_headers.insert("Referer".to_string(), chapter_url.to_string());
        debug!("Referer: {chapter_url}");
        page.set_extra_headers(extra_headers)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Navigate to the chapter page to establish session cookies and any
        // JavaScript-driven auth state the provider may need for image access.
        if let Err(e) = page.goto(chapter_url).await {
            warn!("[dl] could not navigate to chapter URL {chapter_url}: {e}");
        } else {
            page.wait_for_network_idle(500, 10_000).await.ok();
        }

        // Wait for any Cloudflare challenge to be bypassed before proceeding.
        // Mirrors the logic in engine.rs so the downloader path also handles CF-gated CDNs.
        if let Ok(html) = page.content().await {
            if is_cf_challenge(&html) {
                info!("[dl] Cloudflare challenge detected at {chapter_url}, waiting for bypass...");
                let cf_timeout = Duration::from_secs(30);
                let poll_interval = Duration::from_millis(500);
                let start = std::time::Instant::now();
                let mut last_cf_click: Option<std::time::Instant> =
                    Some(std::time::Instant::now() - Duration::from_millis(1_000));
                loop {
                    if start.elapsed() >= cf_timeout {
                        break;
                    }
                    tokio::time::sleep(poll_interval).await;
                    if let Ok(html) = page.content().await {
                        if !is_cf_challenge(&html) {
                            info!("[dl] Cloudflare challenge bypassed");
                            break;
                        }
                        // Attempt human-like checkbox click every ~2 seconds.
                        if last_cf_click
                            .map(|t| t.elapsed() >= Duration::from_secs(2))
                            .unwrap_or(true)
                        {
                            try_cf_checkbox_click(&page).await;
                            last_cf_click = Some(std::time::Instant::now());
                        }
                    }
                }
            }
        }

        // Log Chrome UA and current page context.
        #[derive(serde::Deserialize)]
        struct PageCtx {
            ua: String,
            url: String,
            cookies: u32,
            sw_scope: Option<String>,
        }
        let browser_ua = match page
            .evaluate::<PageCtx>(
                r#"(async () => {
                    let sw_scope = null;
                    try {
                        const reg = await navigator.serviceWorker.getRegistration();
                        if (reg) sw_scope = reg.scope;
                    } catch (_) {}
                    return {
                        ua: navigator.userAgent,
                        url: document.URL,
                        cookies: document.cookie.split(';').filter(Boolean).length,
                        sw_scope,
                    };
                })()"#,
            )
            .await
        {
            Ok(page_ctx) => {
                info!(
                    "[dl] browser context — UA: {} | page: {} | cookies: {} | sw: {}",
                    page_ctx.ua,
                    page_ctx.url,
                    page_ctx.cookies,
                    page_ctx.sw_scope.as_deref().unwrap_or("none"),
                );
                // Unregister any Service Worker — it can intercept image fetches and
                // return stale offline fallbacks instead of the real images.
                if page_ctx.sw_scope.is_some() {
                    match page
                        .evaluate::<bool>(
                            r#"(async () => {
                                const reg = await navigator.serviceWorker.getRegistration();
                                return reg ? reg.unregister() : false;
                            })()"#,
                        )
                        .await
                    {
                        Ok(true) => info!("[dl] service worker unregistered"),
                        Ok(false) => warn!("[dl] service worker unregister() returned false"),
                        Err(e) => warn!("[dl] service worker unregister failed: {e}"),
                    }
                    if let Err(e) = page.goto("about:blank").await {
                        warn!("[dl] could not navigate to about:blank to shed SW: {e}");
                    } else {
                        info!("[dl] navigated to about:blank — SW no longer controls this tab");
                    }
                }
                Some(page_ctx.ua)
            }
            Err(e) => {
                warn!("[dl] could not read browser context: {e}");
                None
            }
        };

        // Close any popup/ad tabs the chapter page may have spawned.
        ctx.browser.close_popup_tabs(browser.as_ref(), &page).await;

        // Extract browser cookies so reqwest (Tier 1) can present them on image requests.
        // CDP Network.getCookies returns all cookies visible to the current page URL.
        let browser_cookies = extract_browser_cookies(&page, chapter_url).await;
        debug!(
            "[dl] extracted {} cookies from browser session",
            browser_cookies.len()
        );

        // Build a reqwest client that impersonates the browser session:
        // same User-Agent, Referer, and cookies extracted above.
        let referer = pages
            .first()
            .and_then(|p| p.referrer.as_deref())
            .unwrap_or(chapter_url);
        let reqwest_client =
            build_browser_reqwest(&browser_cookies, referer, browser_ua.as_deref());

        // If all pages share a referrer URL, navigate there so loadNetworkResource
        // uses it as the Referer via the frame's document URL (Chrome's default
        // referrer policy sends origin for cross-origin requests).
        if let Some(ref_url) = pages.first().and_then(|p| p.referrer.as_deref()) {
            info!("[dl] navigating to referrer context: {ref_url}");
            if let Err(e) = page.goto(ref_url).await {
                warn!("[dl] could not navigate to referrer URL {ref_url}: {e}");
            }
        }

        // Get the frame ID — used by loadNetworkResource to scope credential access.
        let frame_id = page
            .session()
            .get_frame_tree()
            .await
            .map(|ft| ft.frame.id)
            .unwrap_or_default();
        debug!("[dl] frame_id for loadNetworkResource: {frame_id}");

        let total_pages = pages.len() as i64;

        for (idx, page_url) in pages.iter().enumerate() {
            if cancel_token.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            let url = &page_url.url;
            info!("[dl] page {}/{} → {url}", idx + 1, pages.len());

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

            // Fetch image data using the tiered strategy.
            // Each tier is retried up to 3 times with exponential backoff.
            let image_data = fetch_image_tiered(
                &page,
                url,
                page_url.referrer.as_deref().unwrap_or(referer),
                &reqwest_client,
                &frame_id,
                &download_method,
                idx + 1,
                pages.len(),
            )
            .await?;

            // Validate that the downloaded data is actually an image
            if !is_valid_image(&image_data) {
                warn!(
                    "[dl] Page {} from {} is not a valid image (got {} bytes, magic: {:?})",
                    page_url.index,
                    provider_name.unwrap_or("unknown"),
                    image_data.len(),
                    &image_data[..image_data.len().min(16)]
                );
                debug!(
                    "[dl] Invalid image data for page {} (url: {}):\n{}",
                    page_url.index,
                    url,
                    String::from_utf8_lossy(&image_data)
                );
                return Err(DownloadError::InvalidImage);
            }

            results.push((page_url.index, image_data));
        }

        page.clear_extra_headers()
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        results.sort_by_key(|(idx, _)| *idx);

        // VALIDATION CHECKS
        if results.is_empty() {
            warn!(
                "[dl] No valid images downloaded for chapter {}",
                chapter_url
            );
            return Err(DownloadError::NoValidImages);
        }

        if results.len() != pages.len() {
            warn!(
                "[dl] Incomplete download: got {}/{} pages for {}",
                results.len(),
                pages.len(),
                chapter_url
            );
            return Err(DownloadError::IncompleteDownload {
                ok: results.len(),
                total: pages.len(),
            });
        }

        // Check for duplicate identical images (404/placeholder failure)
        use std::collections::HashSet;
        let mut hashes = HashSet::new();
        for (_, data) in &results {
            hashes.insert(blake3::hash(data));
        }
        let unique_count = hashes.len();
        let total_count = results.len();
        if unique_count <= (total_count / 2) {
            warn!(
                "[dl] Duplicate image failure: only {} unique images out of {} for {}",
                unique_count, total_count, chapter_url
            );
            return Err(DownloadError::DuplicateImages);
        }

        Ok(results)
    }
    .await;

    // Always close the Chrome tab, including on cancellation and provider errors.
    ctx.browser.unregister_page(page.target_id());
    close_page_tab(browser.as_ref(), &page).await;
    drop(page);

    result
}

// ---------------------------------------------------------------------------
// Tiered image fetch
// ---------------------------------------------------------------------------

/// Attempt to fetch one image using the configured tier strategy, with retries.
///
/// - `Auto`: Try reqwest (Tier 1). On non-success status or invalid image,
///   fall through to CDP (Tier 2).
/// - `Browser`: Skip Tier 1, go straight to CDP.
///
/// Each tier retries up to 3 times with exponential backoff (500ms, 1s, 2s).
#[allow(clippy::too_many_arguments)]
async fn fetch_image_tiered(
    page: &eoka::Page,
    url: &str,
    referer: &str,
    reqwest_client: &reqwest::Client,
    frame_id: &str,
    method: &DownloadMethod,
    page_num: usize,
    total_pages: usize,
) -> Result<Vec<u8>, DownloadError> {
    // Tier 1: reqwest with browser cookies (fast path)
    if *method == DownloadMethod::Auto {
        match fetch_via_reqwest(reqwest_client, url, referer, page_num, total_pages).await {
            Ok(bytes) if is_valid_image(&bytes) => {
                debug!("[dl] page {page_num}/{total_pages} — Tier 1 (reqwest) succeeded");
                return Ok(bytes);
            }
            Ok(bytes) => {
                warn!(
                    "[dl] page {page_num}/{total_pages} — Tier 1 got {} bytes but not a valid image, falling back to CDP",
                    bytes.len()
                );
            }
            Err(e) => {
                warn!(
                    "[dl] page {page_num}/{total_pages} — Tier 1 (reqwest) failed: {e}, falling back to CDP"
                );
            }
        }
    }

    // Tier 2: CDP Network.loadNetworkResource (browser-process network stack)
    fetch_via_cdp(page, url, frame_id, page_num, total_pages).await
}

/// Fetch an image URL via reqwest, using the browser's cookies and UA.
/// Retries up to 3 times with exponential backoff on transient errors.
async fn fetch_via_reqwest(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    page_num: usize,
    total_pages: usize,
) -> Result<Vec<u8>, String> {
    let mut last_err = String::new();
    for attempt in 0u32..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
        }
        match client
            .get(url)
            .header(reqwest::header::REFERER, referer)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    match resp.bytes().await {
                        Ok(b) => return Ok(b.to_vec()),
                        Err(e) => {
                            last_err = format!("body read error: {e}");
                            continue;
                        }
                    }
                } else {
                    // Non-retriable client errors (403, 404) — stop immediately.
                    let msg = format!("HTTP {status}");
                    if status.as_u16() == 403 || status.as_u16() == 404 {
                        return Err(msg);
                    }
                    last_err = msg;
                }
            }
            Err(e) => {
                last_err = format!("request error: {e}");
            }
        }
    }
    debug!("[dl] page {page_num}/{total_pages} — reqwest gave up after 3 attempts: {last_err}");
    Err(last_err)
}

/// Fetch an image URL via CDP `Network.loadNetworkResource`.
/// Runs in Chrome's browser-process network stack — bypasses renderer content
/// filters and carries full browser credentials. Retries up to 3 times.
async fn fetch_via_cdp(
    page: &eoka::Page,
    url: &str,
    frame_id: &str,
    page_num: usize,
    total_pages: usize,
) -> Result<Vec<u8>, DownloadError> {
    #[derive(serde::Serialize)]
    struct LnrOptions {
        #[serde(rename = "disableCache")]
        disable_cache: bool,
        #[serde(rename = "includeCredentials")]
        include_credentials: bool,
    }
    #[derive(serde::Serialize)]
    struct LnrParams {
        url: String,
        options: LnrOptions,
        #[serde(rename = "frameId", skip_serializing_if = "str::is_empty")]
        frame_id: String,
    }
    #[derive(serde::Deserialize, Debug)]
    struct LnrResult {
        success: bool,
        #[serde(default, rename = "netErrorName")]
        net_error_name: Option<String>,
        #[serde(default, rename = "httpStatusCode")]
        http_status_code: Option<f64>,
        #[serde(default)]
        stream: Option<String>,
    }
    #[derive(serde::Deserialize, Debug)]
    struct LnrReturns {
        resource: LnrResult,
    }
    #[derive(serde::Serialize)]
    struct IoReadParams {
        handle: String,
    }
    #[derive(serde::Deserialize)]
    struct IoReadResult {
        #[serde(default, rename = "base64Encoded")]
        base64_encoded: Option<bool>,
        data: String,
        eof: bool,
    }

    let mut last_err = String::new();
    for attempt in 0u32..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
        }

        let lnr_params = LnrParams {
            url: url.to_owned(),
            options: LnrOptions {
                disable_cache: true,
                include_credentials: true,
            },
            frame_id: frame_id.to_owned(),
        };
        let lnr: LnrReturns = match page
            .session()
            .send("Network.loadNetworkResource", &lnr_params)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "[dl] page {page_num}/{total_pages} — CDP attempt {}: loadNetworkResource error: {e}",
                    attempt + 1
                );
                last_err = e.to_string();
                continue;
            }
        };

        if !lnr.resource.success {
            let msg = format!(
                "loadNetworkResource failed: netError={} HTTP={:?}",
                lnr.resource.net_error_name.as_deref().unwrap_or("—"),
                lnr.resource.http_status_code,
            );
            warn!(
                "[dl] page {page_num}/{total_pages} — CDP attempt {}: {msg}",
                attempt + 1
            );
            last_err = msg;
            continue;
        }

        let stream_handle = match lnr.resource.stream {
            Some(h) => h,
            None => {
                last_err = "loadNetworkResource: no stream handle".to_owned();
                warn!(
                    "[dl] page {page_num}/{total_pages} — CDP attempt {}: {last_err}",
                    attempt + 1
                );
                continue;
            }
        };

        // Read the stream in chunks until EOF.
        let mut image_data: Vec<u8> = Vec::new();
        let mut read_ok = true;
        loop {
            let read_result: IoReadResult = match page
                .session()
                .send(
                    "IO.read",
                    &IoReadParams {
                        handle: stream_handle.clone(),
                    },
                )
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("[dl] page {page_num}/{total_pages} — IO.read error: {e}");
                    last_err = e.to_string();
                    read_ok = false;
                    break;
                }
            };
            if read_result.base64_encoded == Some(true) {
                match BASE64.decode(read_result.data.trim()) {
                    Ok(bytes) => image_data.extend(bytes),
                    Err(e) => {
                        warn!(
                            "[dl] page {page_num}/{total_pages} — IO.read base64 decode error: {e}"
                        );
                        last_err = format!("IO.read base64 decode: {e}");
                        read_ok = false;
                        break;
                    }
                }
            } else {
                image_data.extend(read_result.data.as_bytes());
            }
            if read_result.eof {
                break;
            }
        }

        if read_ok {
            debug!(
                "[dl] page {page_num}/{total_pages} — Tier 2 (CDP) succeeded ({} bytes)",
                image_data.len()
            );
            return Ok(image_data);
        }
    }

    Err(DownloadError::Io(std::io::Error::other(format!(
        "CDP download failed after 3 attempts: {last_err}"
    ))))
}

// ---------------------------------------------------------------------------
// Browser cookie / client helpers
// ---------------------------------------------------------------------------

/// Extract all cookies visible to `url` from the browser session via CDP.
/// Returns `(name, value)` pairs suitable for building a Cookie header.
async fn extract_browser_cookies(page: &eoka::Page, url: &str) -> Vec<(String, String)> {
    #[derive(serde::Serialize)]
    struct GetCookiesParams {
        urls: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct CookieEntry {
        name: String,
        value: String,
    }
    #[derive(serde::Deserialize)]
    struct GetCookiesResult {
        cookies: Vec<CookieEntry>,
    }

    let params = GetCookiesParams {
        urls: vec![url.to_owned()],
    };
    match page
        .session()
        .send::<GetCookiesParams, GetCookiesResult>("Network.getCookies", &params)
        .await
    {
        Ok(result) => result
            .cookies
            .into_iter()
            .map(|c| (c.name, c.value))
            .collect(),
        Err(e) => {
            warn!("[dl] could not extract browser cookies: {e}");
            Vec::new()
        }
    }
}

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.7680.153 Safari/537.36";

/// Build a reqwest Client configured to look like the browser session:
/// same User-Agent, a `Cookie` header from extracted browser cookies, and
/// no automatic redirect following (so we can detect soft-block redirects).
fn build_browser_reqwest(
    cookies: &[(String, String)],
    referer: &str,
    browser_ua: Option<&str>,
) -> reqwest::Client {
    let ua = browser_ua.unwrap_or(BROWSER_UA);
    let cookie_header = cookies
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let mut headers = reqwest::header::HeaderMap::new();
    if !cookie_header.is_empty() {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&cookie_header) {
            headers.insert(reqwest::header::COOKIE, v);
        }
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_str(referer) {
        headers.insert(reqwest::header::REFERER, v);
    }

    reqwest::Client::builder()
        .user_agent(ua)
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// After a successful download, remove any previously-downloaded lower-scored variants
/// for the same (chapter_base, chapter_variant) slot.
///
/// Non-fatal: logs errors and continues. Only removes variants with a strictly worse
/// quality score than `chapter` — same-or-better score variants are left untouched.
async fn cleanup_superseded_downloads(
    pool: &sqlx::SqlitePool,
    manga: &Manga,
    chapter: &Chapter,
    lib_root: &Path,
    quality_rules: &[crate::db::quality_rules::QualityRule],
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

    let new_score = compute_score(chapter, quality_rules);
    let series_dir = lib_root.join(&manga.relative_path);
    let number_prefix = format!("Chapter {}", chapter.number_sort());

    for variant in &all_variants {
        if variant.id == chapter.id {
            continue;
        }
        if variant.download_status != DownloadStatus::Downloaded {
            continue;
        }
        let old_score = compute_score(variant, quality_rules);
        if old_score >= new_score {
            continue; // Same or better quality — don't touch
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

/// Returns true if the data appears to be a valid image format.
/// Checks magic bytes for JPEG, PNG, GIF, WebP and AVIF.
pub fn is_valid_image(data: &[u8]) -> bool {
    matches!(
        data,
        d if d.starts_with(b"\xFF\xD8\xFF") ||  // JPEG
             d.starts_with(b"\x89PNG") ||        // PNG
             d.starts_with(b"GIF8") ||           // GIF
             (d.starts_with(b"RIFF") && d.len() >= 12 && &d[8..12] == b"WEBP") || // WebP
             (d.len() >= 12 && &d[4..12] == b"ftypavif") // AVIF
    )
}

/// Guess image extension from magic bytes.
pub fn image_ext(data: &[u8]) -> &'static str {
    match data {
        d if d.starts_with(b"\xFF\xD8\xFF") => "jpg",
        d if d.starts_with(b"\x89PNG") => "png",
        d if d.starts_with(b"GIF8") => "gif",
        d if d.starts_with(b"RIFF") && d.len() >= 12 && &d[8..12] == b"WEBP" => "webp",
        d if d.len() >= 12 && &d[4..12] == b"ftypavif" => "avif",
        _ => "jpg",
    }
}
