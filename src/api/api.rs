use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rocket::{State, delete, get, http::Status, patch, post, put, routes, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db::{self, task::{RecentTask, TaskType}}, http::anilist::ALClient, manga::{comicinfo, covers, manga::{DownloadStatus, Library, Manga, MangaMetadata, MangaSource, MangaType, PublishingStatus}, scoring::compute_tier}, scraper::ProviderRegistry, scheduler::worker::CancelMap,
};


// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ApiError {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (Status, Json<ApiError>)>;

fn err(status: Status, msg: impl ToString) -> (Status, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: msg.to_string(),
        }),
    )
}

fn internal(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::InternalServerError, msg)
}

fn bad_request(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::BadRequest, msg)
}

fn not_found(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::NotFound, msg)
}

// ---------------------------------------------------------------------------
// GET /api/libraries
// ---------------------------------------------------------------------------

#[get("/api/libraries")]
async fn list_libraries(pool: &State<SqlitePool>) -> ApiResult<Vec<Library>> {
    db::library::get_all(pool.inner())
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/libraries
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NewLibraryRequest {
    library_type: String,
    root_path: String,
}

#[post("/api/libraries", data = "<body>")]
async fn create_library(
    pool: &State<SqlitePool>,
    body: Json<NewLibraryRequest>,
) -> ApiResult<Library> {
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }

    let r#type = match body.library_type.as_str() {
        "Comics" => MangaType::Comics,
        _ => MangaType::Manga,
    };

    let lib = Library {
        uuid: Uuid::new_v4(),
        r#type,
        root_path: PathBuf::from(body.root_path.trim()),
    };

    db::library::insert(pool.inner(), &lib)
        .await
        .map_err(internal)?;
    Ok(Json(lib))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>")]
async fn get_library(pool: &State<SqlitePool>, id: &str) -> ApiResult<Library> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("library not found"))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>/manga
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>/manga")]
async fn list_library_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<Manga>> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_all_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// GET /api/manga/search?q=
// ---------------------------------------------------------------------------

#[get("/api/manga/search?<q>")]
async fn search_manga(al: &State<ALClient>, q: &str) -> ApiResult<Vec<Manga>> {
    if q.trim().is_empty() {
        return Ok(Json(vec![]));
    }
    al.search_manga_as_manga(q.trim())
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/manga
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddMangaRequest {
    anilist_id: i32,
    library_id: String,
    relative_path: String,
}

/// API for adding new manga to a library.
/// This just does some checks, and 
#[post("/api/manga", data = "<body>")]
async fn add_manga(
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

    manga.id = Uuid::new_v4();
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

    // Auto-trigger a scan task
    db::task::enqueue(pool.inner(), TaskType::ScanLibrary, Some(manga.id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Json(manga))
}

// ---------------------------------------------------------------------------
// POST /api/manga/manual
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddMangaManualRequest {
    library_id: String,
    relative_path: String,
    title: String,
    other_titles: Option<Vec<String>>,
    synopsis: Option<String>,
    publishing_status: Option<PublishingStatus>,
    tags: Option<Vec<String>>,
    start_year: Option<i32>,
    end_year: Option<i32>,
    cover_url: Option<String>,
}

#[post("/api/manga/manual", data = "<body>")]
async fn add_manga_manual(
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

    let metadata = MangaMetadata {
        title: body.title.trim().to_owned(),
        other_titles: body.other_titles.clone(),
        synopsis: body.synopsis.as_deref().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
        publishing_status: body.publishing_status.clone().unwrap_or(PublishingStatus::Unknown),
        tags: body.tags.clone().unwrap_or_default(),
        start_year: body.start_year,
        end_year: body.end_year,
    };

    let mut manga = Manga {
        id: uuid::Uuid::new_v4(),
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

    // Auto-trigger a scan so providers can be searched immediately
    db::task::enqueue(pool.inner(), TaskType::ScanLibrary, Some(manga.id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Json(manga))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>")]
async fn get_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Manga> {
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
async fn delete_manga(
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

#[derive(Deserialize)]
struct PatchMangaRequest {
    monitored: Option<bool>,
}

#[patch("/api/manga/<id>", data = "<body>")]
async fn patch_manga(
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

#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    needs_browser: bool,
}

#[get("/api/providers")]
async fn list_providers(registry: &State<Arc<ProviderRegistry>>) -> Json<Vec<ProviderInfo>> {
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
async fn scan_manga_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(pool.inner(), TaskType::ScanLibrary, Some(manga_id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/check-new
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/check-new")]
async fn check_new_chapters_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(pool.inner(), TaskType::CheckNewChapter, Some(manga_id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/provider
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetProviderRequest {
    provider_name: String,
    provider_url: String,
}

/// I honestly have no idea what this does.
// #[post("/api/manga/<id>/provider", data = "<body>")]
// async fn set_provider(
//     pool: &State<SqlitePool>,
//     id: &str,
//     body: Json<SetProviderRequest>,
// ) -> Result<Status, (Status, Json<ApiError>)> {
//     let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
//     db::manga::get_by_id(pool.inner(), manga_id)
//         .await
//         .map_err(internal)?
//         .ok_or_else(|| not_found("manga not found"))?;

//     db::provider::upsert(
//         pool.inner(),
//         &db::provider::MangaProvider {
//             manga_id,
//             provider_name: body.provider_name.clone(),
//             provider_url: Some(body.provider_url.clone()),
//             last_synced_at: None,
//             search_attempted_at: Some(chrono::Utc::now()),
//         },
//     )
//     .await
//     .map_err(internal)?;

//     Ok(Status::Ok)
// }

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MangaProviderResponse {
    provider_name: String,
    /// `None` means this provider was searched but the manga was not found.
    provider_url: Option<String>,
    found: bool,
    last_synced_at: Option<i64>,
    search_attempted_at: Option<i64>,
}

/// Returns a list of providers for a given manga series id.
#[get("/api/manga/<id>/providers")]
async fn list_manga_providers(
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
// GET /api/manga/<id>/chapters
// ---------------------------------------------------------------------------

/// Response struct for a single chapter row (all providers included, not just canonical).
#[derive(Serialize)]
struct ChapterListItem {
    id: String,
    manga_id: String,
    chapter_base: i32,
    chapter_variant: i32,
    title: Option<String>,
    language: String,
    scanlator_group: Option<String>,
    provider_name: Option<String>,
    chapter_url: Option<String>,
    download_status: String,
    /// Unix timestamp in seconds
    released_at: Option<i64>,
    /// Unix timestamp in seconds
    downloaded_at: Option<i64>,
    /// Unix timestamp in seconds
    scraped_at: Option<i64>,
    /// True if this chapter is an extra/bonus.
    is_extra: bool,
    /// True if this row is the current canonical winner for its (chapter_base, chapter_variant) slot.
    is_canonical: bool,
    /// Scanlation tier: 1=Official, 2=Known Scanner, 3=Unknown Scanner, 4=No Group.
    tier: u8,
}

#[get("/api/manga/<id>/chapters")]
async fn list_chapters(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<ChapterListItem>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let all_rows = db::chapter::get_all_for_manga(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    let canonical_uuids: std::collections::HashSet<String> =
        db::chapter::get_canonical_uuids(pool.inner(), manga_id)
            .await
            .map_err(internal)?
            .into_iter()
            .collect();

    let trusted = db::provider::get_trusted_groups(pool.inner())
        .await
        .map_err(internal)?;

    let items = all_rows
        .into_iter()
        .map(|ch| {
            let is_canonical = canonical_uuids.contains(&ch.id.to_string());
            let tier = compute_tier(ch.scanlator_group.as_deref(), &trusted);
            ChapterListItem {
                id: ch.id.to_string(),
                manga_id: ch.manga_id.to_string(),
                chapter_base: ch.chapter_base,
                chapter_variant: ch.chapter_variant,
                title: ch.title,
                language: ch.language,
                scanlator_group: ch.scanlator_group,
                provider_name: ch.provider_name,
                chapter_url: ch.chapter_url,
                download_status: ch.download_status.as_str().to_string(),
                released_at: ch.released_at.map(|dt| dt.timestamp()),
                downloaded_at: ch.downloaded_at.map(|dt| dt.timestamp()),
                scraped_at: ch.scraped_at.map(|dt| dt.timestamp()),
                is_extra: ch.is_extra,
                is_canonical,
                tier,
            }
        })
        .collect();

    Ok(Json(items))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/download
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/download")]
async fn download_chapter_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::task::enqueue(
        pool.inner(),
        TaskType::DownloadChapter,
        Some(manga_id),
        Some(chapter.id),
        10,
    )
    .await
    .map_err(internal)?;

    db::chapter::set_status(pool.inner(), chapter.id, DownloadStatus::Queued, None)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>/chapters/<base>/<variant>
// ---------------------------------------------------------------------------

#[delete("/api/manga/<id>/chapters/<base>/<variant>")]
async fn delete_chapter_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    
    // Verify manga exists
    let manga = db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;
    
    // Find the canonical chapter
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;
    
    // Delete the downloaded files from disk if they exist
    if chapter.download_status == DownloadStatus::Downloaded {
        let library = db::library::get_by_id(pool.inner(), manga.library_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| not_found("library not found"))?;
        
        // Chapter file naming: "Chapter XX" or "Chapter XX.Y" + optional title + optional group
        let mut cbz_name = if chapter.chapter_variant == 0 {
            format!("Chapter {}", chapter.chapter_base)
        } else {
            format!("Chapter {}.{}", chapter.chapter_base, chapter.chapter_variant)
        };
        
        // Add title if present
        if let Some(ref title) = chapter.title {
            cbz_name.push_str(&format!(" - {}", title));
        }
        
        // Add scanlator group if present
        if let Some(ref group) = chapter.scanlator_group {
            cbz_name.push_str(&format!(" [{}]", group));
        }
        
        // Sanitize filename
        let cbz_name: String = cbz_name
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        
        let chapter_path = library.root_path.join(&manga.relative_path).join(format!("{}.cbz", cbz_name));
        
        if chapter_path.exists() {
            if let Err(e) = std::fs::remove_file(&chapter_path) {
                log::warn!("[api] Failed to delete chapter file '{}': {}", chapter_path.display(), e);
            }
        }
    }
    
    // Delete from database
    db::chapter::delete(pool.inner(), chapter.id)
        .await
        .map_err(internal)?;
    
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/refresh
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/refresh")]
async fn refresh_manga_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(pool.inner(), TaskType::RefreshAniList, Some(manga_id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/scan-disk
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/scan-disk")]
async fn scan_disk_api(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::task::enqueue(pool.inner(), TaskType::ScanDisk, Some(manga_id), None, 5)
        .await
        .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/mark-downloaded
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/mark-downloaded")]
async fn mark_chapter_downloaded(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::chapter::set_status(
        pool.inner(),
        chapter.id,
        DownloadStatus::Downloaded,
        Some(Utc::now()),
    )
    .await
    .map_err(internal)?;

    db::chapter::update_manga_counts(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/reset
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/reset")]
async fn reset_chapter_api(
    pool: &State<SqlitePool>,
    cancel_map: &State<CancelMap>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::chapter::set_status(pool.inner(), chapter.id, DownloadStatus::Missing, None)
        .await
        .map_err(internal)?;

    db::chapter::update_manga_counts(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    // Cancel any in-flight or pending DownloadChapter tasks for this chapter
    let running_tasks = db::task::get_running_for_chapter(pool.inner(), chapter.id)
        .await
        .map_err(internal)?;
    for task_id in running_tasks {
        if let Some(token) = cancel_map.lock().unwrap().get(&task_id) {
            token.cancel();
        }
    }
    db::task::cancel_by_chapter(pool.inner(), chapter.id)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/toggle-extra
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/toggle-extra")]
async fn toggle_extra_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::chapter::set_is_extra(pool.inner(), chapter.id, !chapter.is_extra)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/optimise
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/optimise")]
async fn optimise_chapter_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::task::enqueue(
        pool.inner(),
        TaskType::OptimiseChapter,
        Some(manga_id),
        Some(chapter.id),
        15,
    )
    .await
    .map_err(internal)?;

    Ok(Status::Accepted)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/set-canonical
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetCanonicalRequest {
    chapter_id: String,
}

#[post("/api/manga/<id>/chapters/<base>/<variant>/set-canonical", data = "<body>")]
async fn set_canonical_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
    body: Json<SetCanonicalRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter_id = Uuid::parse_str(&body.chapter_id).map_err(|_| bad_request("invalid chapter UUID"))?;

    let chapter = db::chapter::get_by_id(pool.inner(), chapter_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    if chapter.manga_id != manga_id {
        return Err(bad_request("chapter does not belong to this manga"));
    }
    if chapter.chapter_base != base || chapter.chapter_variant != variant {
        return Err(bad_request("chapter does not match the given base/variant"));
    }

    db::chapter::set_canonical_override(pool.inner(), manga_id, base, variant, chapter_id)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// PUT /api/libraries/<id>
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UpdateLibraryRequest {
    root_path: String,
}

#[put("/api/libraries/<id>", data = "<body>")]
async fn update_library(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<UpdateLibraryRequest>,
) -> ApiResult<Library> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }
    db::library::update_root_path(pool.inner(), uuid, body.root_path.trim())
        .await
        .map_err(internal)?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("library not found"))
}

// ---------------------------------------------------------------------------
// DELETE /api/libraries/<id>
// ---------------------------------------------------------------------------

#[delete("/api/libraries/<id>")]
async fn delete_library(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::delete(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// GET /api/tasks
// ---------------------------------------------------------------------------

#[get("/api/tasks?<manga_id>&<limit>")]
async fn list_tasks(
    pool: &State<SqlitePool>,
    manga_id: Option<&str>,
    limit: Option<i64>,
) -> ApiResult<Vec<RecentTask>> {
    let mid = manga_id.and_then(|s| Uuid::parse_str(s).ok());
    // limit=0 (or omitted when no manga_id filter) means "all tasks"
    let effective_limit = limit.unwrap_or(0);
    db::task::get_recent(pool.inner(), mid, effective_limit)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/tasks/<id>/cancel
// ---------------------------------------------------------------------------

#[post("/api/tasks/<id>/cancel")]
async fn cancel_task(
    pool: &State<SqlitePool>,
    cancel_map: &State<CancelMap>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::task::cancel(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    // Signal the running task to stop
    if let Some(token) = cancel_map.lock().unwrap().get(&uuid) {
        token.cancel();
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// GET /api/settings  +  PUT /api/settings
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SettingsResponse {
    scan_interval_hours: u64,
    queue_paused: bool,
    /// BCP 47 language code to prefer when selecting a provider (e.g. "en"). `null` = accept any.
    preferred_language: Option<String>,
}

#[derive(Deserialize)]
struct UpdateSettingsRequest {
    scan_interval_hours: Option<u64>,
    queue_paused: Option<bool>,
    /// Set to a BCP 47 code (e.g. "en") to filter downloads to that language, or "" to clear.
    preferred_language: Option<String>,
}

#[get("/api/settings")]
async fn get_settings(pool: &State<SqlitePool>) -> ApiResult<SettingsResponse> {
    let hours = db::settings::get(pool.inner(), "scan_interval_hours", "6")
        .await
        .map_err(internal)?
        .parse::<u64>()
        .unwrap_or(6);
    let queue_paused = db::settings::get(pool.inner(), "queue_paused", "false")
        .await
        .map_err(internal)?
        == "true";
    let lang_raw = db::settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    let preferred_language = if lang_raw.is_empty() { None } else { Some(lang_raw) };
    Ok(Json(SettingsResponse {
        scan_interval_hours: hours,
        queue_paused,
        preferred_language,
    }))
}

#[put("/api/settings", data = "<body>")]
async fn update_settings(
    pool: &State<SqlitePool>,
    body: Json<UpdateSettingsRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    if let Some(hours) = body.scan_interval_hours {
        if !(1..=168).contains(&hours) {
            return Err(bad_request("scan_interval_hours must be 1–168"));
        }
        db::settings::set(pool.inner(), "scan_interval_hours", &hours.to_string())
            .await
            .map_err(internal)?;
    }
    if let Some(paused) = body.queue_paused {
        db::settings::set(pool.inner(), "queue_paused", if paused { "true" } else { "false" })
            .await
            .map_err(internal)?;
    }
    if let Some(ref lang) = body.preferred_language {
        db::settings::set(pool.inner(), "preferred_language", lang.trim())
            .await
            .map_err(internal)?;
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/cover
// Serve cover.<ext> from the manga's series folder on disk.
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>/cover")]
async fn serve_cover(
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

// GET /api/trusted-groups
// POST /api/trusted-groups
// DELETE /api/trusted-groups/<name>

#[get("/api/trusted-groups")]
async fn list_trusted_groups(pool: &State<SqlitePool>) -> ApiResult<Vec<String>> {
    let groups = db::provider::get_trusted_groups(pool.inner())
        .await
        .map_err(internal)?;
    Ok(Json(groups))
}

#[derive(Deserialize)]
struct AddTrustedGroupRequest {
    name: String,
}

#[post("/api/trusted-groups", data = "<body>")]
async fn add_trusted_group(
    pool: &State<SqlitePool>,
    body: Json<AddTrustedGroupRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(bad_request("name must not be empty"));
    }
    db::provider::add_trusted_group(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Status::Created)
}

#[delete("/api/trusted-groups/<name>")]
async fn remove_trusted_group(
    pool: &State<SqlitePool>,
    name: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    db::provider::remove_trusted_group(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Status::Ok)
}

/// All the routes be here
pub fn routes() -> Vec<rocket::Route> {
    routes![
        list_libraries,
        create_library,
        get_library,
        update_library,
        delete_library,
        list_library_manga,
        search_manga,
        add_manga,
        add_manga_manual,
        get_manga,
        delete_manga,
        patch_manga,
        list_providers,
        scan_manga_api,
        check_new_chapters_api,
        // set_provider,
        list_manga_providers,
        list_chapters,
        download_chapter_api,
        delete_chapter_api,
        refresh_manga_api,
        scan_disk_api,
        mark_chapter_downloaded,
        reset_chapter_api,
        toggle_extra_api,
        optimise_chapter_api,
        set_canonical_api,
        list_tasks,
        cancel_task,
        get_settings,
        update_settings,
        serve_cover,
        list_trusted_groups,
        add_trusted_group,
        remove_trusted_group,
    ]
}
