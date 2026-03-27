use std::path::PathBuf;

use chrono::Utc;
use tracing::{debug, trace, warn};
use rocket::{State, delete, get, http::Status, patch, post, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use strsim;
use uuid::Uuid;

use crate::{
    db,
    http::anilist::ALClient,
    manga::{
        comicinfo, covers, files,
        manga::{Manga, MangaMetadata, MangaSource, PublishingStatus, Synonym, SynonymSource},
    },
    scraper::{ProviderRegistry, ScraperCtx},
};

use super::errors::{ApiError, ApiResult, bad_request, err, internal, not_found};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddMangaRequest {
    pub anilist_id: i32,
    pub library_id: String,
    pub relative_path: String,
}

#[derive(Deserialize)]
pub struct AddMangaManualRequest {
    pub library_id: String,
    pub relative_path: String,
    pub title: String,
    /// Simple string titles that will be converted to Synonym with Manual source
    pub other_titles: Option<Vec<String>>,
    pub synopsis: Option<String>,
    pub publishing_status: Option<PublishingStatus>,
    pub tags: Option<Vec<String>>,
    pub start_year: Option<i32>,
    pub end_year: Option<i32>,
    pub cover_url: Option<String>,
}

#[derive(Deserialize)]
struct PatchMangaRequest {
    monitored: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct DeleteMangaRequest {
    delete_files: Option<bool>,
}

#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    needs_browser: bool,
    version: Option<String>,
    tags: Vec<crate::scraper::def::ProviderTag>,
}

#[derive(Serialize)]
pub struct MangaProviderResponse {
    pub provider_name: String,
    /// `None` means this provider was searched but the manga was not found.
    pub provider_url: Option<String>,
    pub found: bool,
    pub last_synced_at: Option<i64>,
    pub search_attempted_at: Option<i64>,
}

#[derive(Serialize)]
pub struct ProviderCandidate {
    pub title: String,
    pub url: String,
    pub cover: Option<String>,
    /// Best Jaro-Winkler score across all manga synonyms (0.0–1.0)
    pub score: f64,
}

#[derive(Deserialize)]
struct SetProviderUrlRequest {
    /// Pass `null` to clear the mapping for this provider.
    url: Option<String>,
}

#[derive(Deserialize)]
pub struct SetCoverUrlRequest {
    pub url: String,
}

/// Request to add/remove/hide synonyms for a manga
#[derive(Deserialize, Debug)]
pub struct UpdateSynonymsRequest {
    /// Synonyms to add (will be marked as Manual source)
    pub add: Option<Vec<String>>,
    /// Synonym titles to hide (AniList synonyms will be marked hidden)
    pub hide: Option<Vec<String>>,
    /// Synonym titles to remove (only Manual synonyms can be fully removed)
    pub remove: Option<Vec<String>>,
}

async fn auto_unmonitor_completed_if_enabled(
    pool: &SqlitePool,
    manga: &mut Manga,
) -> Result<(), sqlx::Error> {
    let enabled = db::settings::get(pool, "auto_unmonitor_completed", "false").await? == "true";
    if enabled && matches!(manga.metadata.publishing_status, PublishingStatus::Completed) {
        manga.monitored = false;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// GET /api/manga/search?q=
// ---------------------------------------------------------------------------

#[get("/api/manga/search?<q>")]
pub async fn search_manga(al: &State<ALClient>, q: &str) -> ApiResult<Vec<Manga>> {
    if q.trim().is_empty() {
        return Ok(Json(vec![]));
    }
    debug!("Searching for manga: {q}");
    al.search_manga_as_manga(q.trim())
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/manga
// ---------------------------------------------------------------------------

#[post("/api/manga", data = "<body>")]
pub async fn add_manga(
    pool: &State<SqlitePool>,
    al: &State<ALClient>,
    http: &State<reqwest::Client>,
    body: Json<AddMangaRequest>,
) -> ApiResult<Manga> {
    if body.relative_path.trim().is_empty() {
        return Err(bad_request("relative_path cannot be empty"));
    }

    let library_id =
        Uuid::parse_str(&body.library_id).map_err(|_| bad_request("invalid library_id UUID"))?;

    // Verify library exists
    let library = db::library::get_by_id(pool.inner(), library_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;

    // Fetch full manga from AniList — includes tags
    let mut manga = al
        .grab_manga(body.anilist_id)
        .await
        .map_err(|e| err(Status::BadGateway, format!("AniList lookup failed: {e}")))?;

    trace!("Full Manga Object: {manga:?}");

    // Check for duplicates in this library
    if let Some(existing) =
        db::manga::exists_by_external_ids(pool.inner(), library_id, manga.anilist_id, manga.mal_id)
            .await
            .map_err(internal)?
    {
        return Err((
            Status::Conflict,
            Json(ApiError {
                error: format!(
                    "This manga ({}) already exists in this library with ID: {}",
                    existing.metadata.title, existing.id
                ),
            }),
        ));
    }

    manga.id = manga
        .anilist_id
        .map(db::manga::manga_uuid)
        .unwrap_or_else(|| db::manga::manual_manga_uuid(body.relative_path.trim()));
    manga.library_id = library_id;
    manga.relative_path = PathBuf::from(body.relative_path.trim());
    manga.created_at = Utc::now().timestamp();
    manga.metadata_updated_at = Utc::now().timestamp();
    auto_unmonitor_completed_if_enabled(pool.inner(), &mut manga)
        .await
        .map_err(internal)?;

    // Download cover into the manga's series folder; fall back to original URL on failure
    if let Some(url) = manga.thumbnail_url.take() {
        let series_dir = files::series_dir(&library.root_path, &manga);
        manga.thumbnail_url = covers::download_cover(http.inner(), &url, manga.id, &series_dir)
            .await
            .or(Some(url));
    }

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    // Write series-level ComicInfo.xml into the series folder
    let series_dir = files::series_dir(&library.root_path, &manga);
    if let Err(e) = comicinfo::write_series_comicinfo(&series_dir, &manga).await {
        warn!(
            "[api] Failed to write ComicInfo.xml for '{}': {e}",
            manga.metadata.title
        );
    }

    // Only queue RefreshMetadata - user must configure providers/aliases before scanning
    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::RefreshMetadata,
        Some(manga.id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Json(manga))
}

// ---------------------------------------------------------------------------
// POST /api/manga/manual
// ---------------------------------------------------------------------------

#[post("/api/manga/manual", data = "<body>")]
pub async fn add_manga_manual(
    pool: &State<SqlitePool>,
    http: &State<reqwest::Client>,
    body: Json<AddMangaManualRequest>,
) -> ApiResult<Manga> {
    if body.title.trim().is_empty() {
        return Err(bad_request("title cannot be empty"));
    }
    if body.relative_path.trim().is_empty() {
        return Err(bad_request("relative_path cannot be empty"));
    }

    let library_id =
        Uuid::parse_str(&body.library_id).map_err(|_| bad_request("invalid library_id UUID"))?;

    let library = db::library::get_by_id(pool.inner(), library_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;

    // Convert string titles to Synonym objects with Manual source
    let other_titles = body.other_titles.as_ref().map(|titles| {
        titles
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| Synonym {
                title: s.trim().to_owned(),
                source: SynonymSource::Manual,
                hidden: false,
                filter_reason: None,
            })
            .collect()
    });

    let metadata = MangaMetadata {
        title: body.title.trim().to_owned(),
        other_titles,
        synopsis: body
            .synopsis
            .as_deref()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        publishing_status: body
            .publishing_status
            .clone()
            .unwrap_or(PublishingStatus::Unknown),
        tags: body.tags.clone().unwrap_or_default(),
        start_year: body.start_year,
        start_month: None,
        start_day: None,
        end_year: body.end_year,
        // ComicInfo fields
        writer: None,
        penciller: None,
        inker: None,
        colorist: None,
        letterer: None,
        editor: None,
        translator: None,
        genre: None,
        community_rating: None,
    };

    let mut manga = Manga {
        id: db::manga::manual_manga_uuid(body.relative_path.trim()),
        library_id,
        anilist_id: None,
        mal_id: None,
        metadata,
        relative_path: PathBuf::from(body.relative_path.trim()),
        downloaded_count: None,
        chapter_count: None,
        metadata_source: MangaSource::Local,
        thumbnail_url: body.cover_url.clone().filter(|s| !s.is_empty()),
        monitored: true,
        created_at: Utc::now().timestamp(),
        metadata_updated_at: Utc::now().timestamp(),
        last_checked_at: None,
    };
    auto_unmonitor_completed_if_enabled(pool.inner(), &mut manga)
        .await
        .map_err(internal)?;

    // Download cover if a URL was provided
    if let Some(url) = manga.thumbnail_url.take() {
        let series_dir = files::series_dir(&library.root_path, &manga);
        manga.thumbnail_url = covers::download_cover(http.inner(), &url, manga.id, &series_dir)
            .await
            .or(Some(url));
    }

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    // Write series-level ComicInfo.xml
    let series_dir = files::series_dir(&library.root_path, &manga);
    if let Err(e) = comicinfo::write_series_comicinfo(&series_dir, &manga).await {
        warn!(
            "[api] Failed to write ComicInfo.xml for '{}': {e}",
            manga.metadata.title
        );
    }

    // For manual manga, queue RefreshMetadata (checks local source) and BuildFullChapterList
    // but only after user configures providers/aliases
    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::RefreshMetadata,
        Some(manga.id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Json(manga))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>")]
pub async fn get_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Manga> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("manga not found"))
}

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>
// ---------------------------------------------------------------------------

#[delete("/api/manga/<id>", data = "<body>")]
pub async fn delete_manga(
    pool: &State<SqlitePool>,
    id: &str,
    body: Option<Json<DeleteMangaRequest>>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let delete_files = body
        .as_ref()
        .and_then(|body| body.delete_files)
        .unwrap_or(false);

    let manga = db::manga::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    if delete_files {
        let library = db::library::get_by_id(pool.inner(), manga.library_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| not_found("library not found"))?;

        let series_dir = library.root_path.join(&manga.relative_path);
        if series_dir.exists() {
            tokio::fs::remove_dir_all(&series_dir)
                .await
                .map_err(|e| internal(format!("failed to delete series files: {e}")))?;
        }
    }

    db::manga::delete(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// PATCH /api/manga/<id>
// ---------------------------------------------------------------------------

#[patch("/api/manga/<id>", data = "<body>")]
pub async fn patch_manga(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<PatchMangaRequest>,
) -> ApiResult<Manga> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    if let Some(monitored) = body.monitored {
        db::manga::set_monitored(pool.inner(), uuid, monitored)
            .await
            .map_err(internal)?;
    }
    db::manga::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("manga not found"))
}

// ---------------------------------------------------------------------------
// GET /api/providers
// ---------------------------------------------------------------------------

#[get("/api/providers")]
pub async fn list_providers(
    registry: &State<std::sync::Arc<ProviderRegistry>>,
) -> Json<Vec<ProviderInfo>> {
    let providers = registry
        .all()
        .into_iter()
        .map(|p| ProviderInfo {
            name: p.name().to_owned(),
            needs_browser: p.needs_browser(),
            version: p.version().map(str::to_owned),
            tags: p.tags().to_vec(),
        })
        .collect();
    Json(providers)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/scan
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/scan")]
pub async fn scan_manga_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::BuildFullChapterList,
        Some(manga_id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/check-new
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/check-new")]
pub async fn check_new_chapters_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::CheckNewChapter,
        Some(manga_id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers
// ---------------------------------------------------------------------------

/// Returns a list of providers for a given manga series id.
#[get("/api/manga/<id>/providers")]
pub async fn list_manga_providers(
    pool: &State<SqlitePool>,
    id: &str,
) -> ApiResult<Vec<MangaProviderResponse>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let entries = db::provider::get_all_for_manga(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    let resp = entries
        .into_iter()
        .map(|e| MangaProviderResponse {
            found: e.found(),
            provider_name: e.provider_name,
            provider_url: e.provider_url,
            last_synced_at: e.last_synced_at,
            search_attempted_at: e.search_attempted_at,
        })
        .collect();

    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/refresh
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/refresh")]
pub async fn refresh_manga_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::RefreshMetadata,
        Some(manga_id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/scan-disk
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/scan-disk")]
pub async fn scan_disk_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::ScanDisk,
        Some(manga_id),
        None,
        5,
    )
    .await
    .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/cover
// Serve cover.<ext> from the manga's series folder on disk.
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>/cover")]
pub async fn serve_cover(pool: &State<SqlitePool>, id: &str) -> Option<rocket::fs::NamedFile> {
    let manga_id = Uuid::parse_str(id).ok()?;
    let manga = db::manga::get_by_id(pool.inner(), manga_id).await.ok()??;
    let library = db::library::get_by_id(pool.inner(), manga.library_id)
        .await
        .ok()??;
    let series_dir = library.root_path.join(&manga.relative_path);

    for ext in &["jpg", "jpeg", "png", "webp", "avif"] {
        let path = series_dir.join(format!("cover.{ext}"));
        if path.exists() {
            return rocket::fs::NamedFile::open(path).await.ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// PATCH /api/manga/<id>/synonyms
// ---------------------------------------------------------------------------

/// Update synonyms for a manga - add new ones, hide existing AniList ones, or remove Manual ones
#[patch("/api/manga/<id>/synonyms", data = "<body>")]
pub async fn update_synonyms(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<UpdateSynonymsRequest>,
) -> ApiResult<Manga> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;

    debug!("[synonyms] update_synonyms called for manga={manga_id}, body={body:?}");

    // Get existing manga
    let mut manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    // Get current synonyms or create empty list
    let mut synonyms = manga.metadata.other_titles.take().unwrap_or_default();

    debug!("[synonyms] current synonyms: {synonyms:?}");

    // Process "add" - add new Manual synonyms
    if let Some(ref add_titles) = body.add {
        debug!("[synonyms] processing add: {add_titles:?}");
        for title in add_titles {
            let title = title.trim().to_owned();
            if title.is_empty() {
                continue;
            }
            // Check if already exists
            if !synonyms.iter().any(|s| s.title == title) {
                synonyms.push(Synonym {
                    title,
                    source: SynonymSource::Manual,
                    hidden: false,
                    filter_reason: None,
                });
            }
        }
    }

    // Process "hide" - hide AniList synonyms (mark as hidden with manual reason)
    if let Some(ref hide_titles) = body.hide {
        debug!("[synonyms] processing hide: {hide_titles:?}");
        for title in hide_titles {
            let title = title.trim().to_owned();
            if title.is_empty() {
                continue;
            }
            // Find and hide AniList synonyms (compare both trimmed and untrimmed)
            for syn in synonyms.iter_mut() {
                if syn.source == SynonymSource::AniList
                    && (syn.title == title || syn.title.trim() == title)
                {
                    debug!(
                        "[synonyms] hiding synonym: {} (source={:?})",
                        syn.title, syn.source
                    );
                    syn.hidden = true;
                    syn.filter_reason = Some("manual".to_owned());
                }
            }
        }
    }

    // Process "remove" - remove Manual synonyms only (can't fully delete AniList ones)
    if let Some(ref remove_titles) = body.remove {
        debug!("[synonyms] processing remove: {remove_titles:?}");
        let titles_to_remove: std::collections::HashSet<_> = remove_titles
            .iter()
            .filter(|s| !s.trim().is_empty())
            .collect();

        debug!("[synonyms] titles_to_remove: {titles_to_remove:?}");

        // Build new list instead of using retain with mutation
        let mut new_synonyms = Vec::new();
        for s in &synonyms {
            debug!(
                "[synonyms] checking synonym: {} (source={:?}, hidden={})",
                s.title, s.source, s.hidden
            );
        }

        for s in synonyms {
            if titles_to_remove.contains(&s.title) {
                if s.source == SynonymSource::AniList {
                    // For AniList synonyms in remove list, just unhide them
                    debug!(
                        "[synonyms] removing AniList synonym (will unhide): {}",
                        s.title
                    );
                    let mut updated = s;
                    updated.hidden = false;
                    new_synonyms.push(updated);
                } else {
                    // Manual synonyms in remove list are dropped (removed)
                    debug!("[synonyms] removing Manual synonym: {}", s.title);
                }
                // Manual synonyms in remove list are dropped (removed)
            } else {
                new_synonyms.push(s);
            }
        }
        synonyms = new_synonyms;
    }

    debug!("[synonyms] final synonyms: {synonyms:?}");

    // Update manga with new synonyms
    manga.metadata.other_titles = Some(synonyms);
    manga.metadata_updated_at = Utc::now().timestamp();

    // Save to database
    debug!("[synonyms] saving to database...");
    db::manga::update_metadata(pool.inner(), &manga)
        .await
        .map_err(internal)?;
    debug!("[synonyms] saved successfully");

    // Re-fetch to get updated data
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("manga not found after update"))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers/<name>/candidates
// ---------------------------------------------------------------------------

/// Search a specific provider for this manga and return all results with scores,
/// so the user can manually pick the correct series when there are ambiguous matches.
#[get("/api/manga/<id>/providers/<name>/candidates")]
pub async fn provider_candidates(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
    ctx: &State<ScraperCtx>,
    id: &str,
    name: &str,
) -> ApiResult<Vec<ProviderCandidate>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let provider = registry
        .all()
        .into_iter()
        .find(|p| p.name() == name)
        .ok_or_else(|| not_found("provider not found"))?;

    // Build the same search title list as merge.rs
    let mut search_titles: Vec<String> = vec![manga.metadata.title.clone()];
    if let Some(ref other) = manga.metadata.other_titles {
        for syn in other {
            if !syn.hidden && !syn.title.trim().is_empty() {
                search_titles.push(syn.title.clone());
            }
        }
    }
    // Deduplicate
    search_titles.sort();
    search_titles.dedup();

    // Try each title until we get some results
    let mut raw_results = Vec::new();
    for title in &search_titles {
        match ctx.executor.search(ctx.inner(), provider, title).await {
            Ok(r) if !r.is_empty() => {
                raw_results = r;
                break;
            }
            Ok(_) => continue,
            Err(e) => {
                return Err(err(
                    rocket::http::Status::BadGateway,
                    format!("provider search failed: {e}"),
                ));
            }
        }
    }

    // Score every result against all synonyms and return sorted descending
    let mut candidates: Vec<ProviderCandidate> = raw_results
        .into_iter()
        .map(|r| {
            let score = search_titles
                .iter()
                .map(|t| strsim::jaro_winkler(&r.title.to_lowercase(), &t.to_lowercase()))
                .fold(0.0f64, f64::max);
            ProviderCandidate {
                title: r.title,
                url: r.url,
                cover: r.cover_url,
                score,
            }
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Json(candidates))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/providers/<name>/url
// ---------------------------------------------------------------------------

/// Manually set (or clear) the provider URL for a manga/provider pair.
/// Clears `last_synced_at` so the next scan re-scrapes from the new URL.
#[post("/api/manga/<id>/providers/<name>/url", data = "<body>")]
pub async fn set_provider_url(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
    body: Json<SetProviderUrlRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    use crate::db::provider::MangaProvider;
    use chrono::Utc;

    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::provider::upsert(
        pool.inner(),
        &MangaProvider {
            manga_id,
            enabled: true,
            provider_name: name.to_owned(),
            provider_url: body.url.clone(),
            last_synced_at: None,
            search_attempted_at: Some(Utc::now().timestamp()),
        },
    )
    .await
    .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/cover — download cover from URL
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/cover", data = "<body>")]
pub async fn upload_cover_url(
    pool: &State<SqlitePool>,
    http: &State<reqwest::Client>,
    id: &str,
    body: Json<SetCoverUrlRequest>,
) -> ApiResult<Manga> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let mut manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let library = db::library::get_by_id(pool.inner(), manga.library_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;

    let series_dir = library.root_path.join(&manga.relative_path);
    manga.thumbnail_url = covers::download_cover(http.inner(), &body.url, manga.id, &series_dir)
        .await
        .or(Some(body.url.clone()));

    manga.metadata_updated_at = Utc::now().timestamp();
    db::manga::update_metadata(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("manga not found after update"))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/cover/upload — upload cover file directly
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/cover/upload", data = "<data>")]
pub async fn upload_cover_file(
    pool: &State<SqlitePool>,
    id: &str,
    data: rocket::data::Data<'_>,
) -> ApiResult<Manga> {
    use rocket::data::ToByteUnit;

    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let mut manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let library = db::library::get_by_id(pool.inner(), manga.library_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;

    // Read up to 10MB
    let bytes = data
        .open(10.mebibytes())
        .into_bytes()
        .await
        .map_err(|e| bad_request(&format!("failed to read upload: {e}")))?;

    if !bytes.is_complete() {
        return Err(bad_request("file too large (max 10MB)"));
    }

    let bytes = bytes.into_inner();

    // Detect image type from magic bytes
    let ext = if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(&[0x52, 0x49, 0x46, 0x46]) && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        return Err(bad_request("unsupported image format (use jpg, png, or webp)"));
    };

    let series_dir = library.root_path.join(&manga.relative_path);
    tokio::fs::create_dir_all(&series_dir)
        .await
        .map_err(|e| internal(format!("failed to create series dir: {e}")))?;

    // Remove old cover files
    for old_ext in &["jpg", "jpeg", "png", "webp", "avif"] {
        let old_path = series_dir.join(format!("cover.{old_ext}"));
        if old_path.exists() {
            let _ = tokio::fs::remove_file(&old_path).await;
        }
    }

    let dest = series_dir.join(format!("cover.{ext}"));
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| internal(format!("failed to write cover: {e}")))?;

    manga.thumbnail_url = Some(format!("/api/manga/{manga_id}/cover"));
    manga.metadata_updated_at = Utc::now().timestamp();
    db::manga::update_metadata(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("manga not found after update"))
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        upload_cover_url,
        upload_cover_file,
        search_manga,
        add_manga,
        add_manga_manual,
        get_manga,
        delete_manga,
        patch_manga,
        list_providers,
        scan_manga_api,
        check_new_chapters_api,
        list_manga_providers,
        refresh_manga_api,
        scan_disk_api,
        serve_cover,
        update_synonyms,
        provider_candidates,
        set_provider_url,
    ]
}
