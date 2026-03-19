use std::path::PathBuf;

use chrono::Utc;
use log::{debug, trace};
use rocket::{State, delete, get, patch, post, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db,
    http::anilist::ALClient,
    manga::{
        comicinfo, covers,
        manga::{
            Manga, MangaMetadata, MangaSource, PublishingStatus,
            Synonym, SynonymSource,
        },
    },
    scraper::ProviderRegistry,
};

use super::errors::{bad_request, err, internal, not_found, ApiError, ApiResult};

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

#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    needs_browser: bool,
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
    if let Some(existing) = db::manga::exists_by_external_ids(
        pool.inner(),
        library_id,
        manga.anilist_id,
        manga.mal_id,
    )
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

    manga.id = manga.anilist_id
        .map(db::manga::manga_uuid)
        .unwrap_or_else(|| db::manga::manual_manga_uuid(body.relative_path.trim()));
    manga.library_id = library_id;
    manga.relative_path = PathBuf::from(body.relative_path.trim());
    manga.created_at = Utc::now().timestamp();
    manga.metadata_updated_at = Utc::now().timestamp();

    // Download cover into the manga's series folder; fall back to original URL on failure
    if let Some(url) = manga.thumbnail_url.take() {
        let series_dir = library.root_path.join(&manga.relative_path);
        manga.thumbnail_url = covers::download_cover(http.inner(), &url, manga.id, &series_dir)
            .await
            .or(Some(url));
    }

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    // Write series-level ComicInfo.xml into the series folder
    let series_dir = library.root_path.join(&manga.relative_path);
    if let Err(e) = comicinfo::write_series_comicinfo(&series_dir, &manga).await {
        log::warn!("[api] Failed to write ComicInfo.xml for '{}': {e}", manga.metadata.title);
    }

    // Only queue RefreshMetadata - user must configure providers/aliases before scanning
    db::task::enqueue(pool.inner(), crate::db::task::TaskType::RefreshMetadata, Some(manga.id), None, 5)
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
        synopsis: body.synopsis.as_deref().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
        publishing_status: body.publishing_status.clone().unwrap_or(PublishingStatus::Unknown),
        tags: body.tags.clone().unwrap_or_default(),
        start_year: body.start_year,
        end_year: body.end_year,
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

    // Download cover if a URL was provided
    if let Some(url) = manga.thumbnail_url.take() {
        let series_dir = library.root_path.join(&manga.relative_path);
        manga.thumbnail_url = covers::download_cover(http.inner(), &url, manga.id, &series_dir)
            .await
            .or(Some(url));
    }

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(internal)?;

    // Write series-level ComicInfo.xml
    let series_dir = library.root_path.join(&manga.relative_path);
    if let Err(e) = comicinfo::write_series_comicinfo(&series_dir, &manga).await {
        log::warn!("[api] Failed to write ComicInfo.xml for '{}': {e}", manga.metadata.title);
    }

    // For manual manga, queue RefreshMetadata (checks local source) and BuildFullChapterList
    // but only after user configures providers/aliases
    db::task::enqueue(pool.inner(), crate::db::task::TaskType::RefreshMetadata, Some(manga.id), None, 5)
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

#[delete("/api/manga/<id>")]
pub async fn delete_manga(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
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
pub async fn list_providers(registry: &State<std::sync::Arc<ProviderRegistry>>) -> Json<Vec<ProviderInfo>> {
    let providers = registry
        .all()
        .into_iter()
        .map(|p| ProviderInfo {
            name: p.name().to_owned(),
            needs_browser: p.needs_browser(),
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

    db::task::enqueue(pool.inner(), crate::db::task::TaskType::BuildFullChapterList, Some(manga_id), None, 5)
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

    db::task::enqueue(pool.inner(), crate::db::task::TaskType::CheckNewChapter, Some(manga_id), None, 5)
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

    db::task::enqueue(pool.inner(), crate::db::task::TaskType::RefreshMetadata, Some(manga_id), None, 5)
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

    db::task::enqueue(pool.inner(), crate::db::task::TaskType::ScanDisk, Some(manga_id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/cover
// Serve cover.<ext> from the manga's series folder on disk.
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>/cover")]
pub async fn serve_cover(
    pool: &State<SqlitePool>,
    id: &str,
) -> Option<rocket::fs::NamedFile> {
    let manga_id = Uuid::parse_str(id).ok()?;
    let manga = db::manga::get_by_id(pool.inner(), manga_id).await.ok()??;
    let library = db::library::get_by_id(pool.inner(), manga.library_id).await.ok()??;
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
    
    log::debug!("[synonyms] update_synonyms called for manga={manga_id}, body={body:?}");
    
    // Get existing manga
    let mut manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    // Get current synonyms or create empty list
    let mut synonyms = manga.metadata.other_titles.take().unwrap_or_default();
    
    log::debug!("[synonyms] current synonyms: {synonyms:?}");
    
    // Process "add" - add new Manual synonyms
    if let Some(ref add_titles) = body.add {
        log::debug!("[synonyms] processing add: {add_titles:?}");
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
        log::debug!("[synonyms] processing hide: {hide_titles:?}");
        for title in hide_titles {
            let title = title.trim().to_owned();
            if title.is_empty() {
                continue;
            }
            // Find and hide AniList synonyms (compare both trimmed and untrimmed)
            for syn in synonyms.iter_mut() {
                if syn.source == SynonymSource::AniList
                    && (syn.title == title || syn.title.trim() == title) {
                        log::debug!("[synonyms] hiding synonym: {} (source={:?})", syn.title, syn.source);
                        syn.hidden = true;
                        syn.filter_reason = Some("manual".to_owned());
                    }
            }
        }
    }
    
    // Process "remove" - remove Manual synonyms only (can't fully delete AniList ones)
    if let Some(ref remove_titles) = body.remove {
        log::debug!("[synonyms] processing remove: {remove_titles:?}");
        let titles_to_remove: std::collections::HashSet<_> = remove_titles
            .iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        
        log::debug!("[synonyms] titles_to_remove: {titles_to_remove:?}");
        
        // Build new list instead of using retain with mutation
        let mut new_synonyms = Vec::new();
        for s in &synonyms {
            log::debug!("[synonyms] checking synonym: {} (source={:?}, hidden={})", s.title, s.source, s.hidden);
        }
        
        for s in synonyms {
            if titles_to_remove.contains(&s.title) {
                if s.source == SynonymSource::AniList {
                    // For AniList synonyms in remove list, just unhide them
                    log::debug!("[synonyms] removing AniList synonym (will unhide): {}", s.title);
                    let mut updated = s;
                    updated.hidden = false;
                    new_synonyms.push(updated);
                } else {
                    // Manual synonyms in remove list are dropped (removed)
                    log::debug!("[synonyms] removing Manual synonym: {}", s.title);
                }
                // Manual synonyms in remove list are dropped (removed)
            } else {
                new_synonyms.push(s);
            }
        }
        synonyms = new_synonyms;
    }
    
    log::debug!("[synonyms] final synonyms: {synonyms:?}");
    
    // Update manga with new synonyms
    manga.metadata.other_titles = Some(synonyms);
    manga.metadata_updated_at = Utc::now().timestamp();
    
    // Save to database
    log::debug!("[synonyms] saving to database...");
    db::manga::update_metadata(pool.inner(), &manga)
        .await
        .map_err(internal)?;
    log::debug!("[synonyms] saved successfully");

    // Re-fetch to get updated data
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
    ]
}
