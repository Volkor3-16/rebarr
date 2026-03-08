use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use log::debug;
use rocket::{State, delete, get, http::Status, patch, post, put, routes, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    comicinfo, covers, db,
    db::task::{RecentTask, TaskType},
    manga::{Chapter, DownloadStatus, Library, Manga, MangaMetadata, MangaSource, MangaType, PublishingStatus},
    metadata::anilist::ALClient,
    scraper::ProviderRegistry,
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

    manga.id = Uuid::new_v4();
    manga.library_id = library_id;
    manga.relative_path = PathBuf::from(body.relative_path.trim());
    manga.created_at = Utc::now();
    manga.metadata_updated_at = Utc::now();

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

    // Auto-trigger a scan so provider URLs and chapters are populated immediately
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
    title_og: Option<String>,
    title_roman: Option<String>,
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
        title_og: body.title_og.as_deref().unwrap_or("").trim().to_owned(),
        title_roman: body.title_roman.as_deref().unwrap_or("").trim().to_owned(),
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
        created_at: Utc::now(),
        metadata_updated_at: Utc::now(),
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
// POST /api/manga/<id>/provider
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetProviderRequest {
    provider_name: String,
    provider_url: String,
}

#[post("/api/manga/<id>/provider", data = "<body>")]
async fn set_provider(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<SetProviderRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    db::provider::upsert(
        pool.inner(),
        &db::provider::MangaProvider {
            manga_id,
            provider_name: body.provider_name.clone(),
            provider_url: body.provider_url.clone(),
            last_synced_at: None,
            provider_score: 0.0,
            score_override: None,
        },
    )
    .await
    .map_err(internal)?;

    Ok(Status::Ok)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MangaProviderResponse {
    provider_name: String,
    provider_url: String,
    auto_score: f64,
    score_override: Option<f64>,
    effective_score: f64,
    last_synced_at: Option<chrono::DateTime<chrono::Utc>>,
}

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
        .map(|e| {
            let effective = e.effective_score();
            MangaProviderResponse {
                provider_name: e.provider_name,
                provider_url: e.provider_url,
                auto_score: e.provider_score,
                score_override: e.score_override,
                effective_score: effective,
                last_synced_at: e.last_synced_at,
            }
        })
        .collect();

    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// PATCH /api/manga/<id>/providers/<provider_name>
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PatchProviderRequest {
    /// Set to a number to override, or null/absent to reset to auto scoring.
    score_override: Option<f64>,
}

#[patch("/api/manga/<id>/providers/<provider_name>", data = "<body>")]
async fn patch_manga_provider(
    pool: &State<SqlitePool>,
    id: &str,
    provider_name: &str,
    body: Json<PatchProviderRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::provider::set_score_override(pool.inner(), manga_id, provider_name, body.score_override)
        .await
        .map_err(internal)?;
    Ok(Status::Ok)
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/chapters
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChapterListItem {
    #[serde(flatten)]
    chapter: Chapter,
    /// Human-readable age, e.g. "3days" or "2months". Use as "{found_ago} ago".
    found_ago: String,
}

fn format_found_ago(created_at: chrono::DateTime<chrono::Utc>) -> String {
    let age = chrono::Utc::now().signed_duration_since(created_at);
    let secs = age.num_seconds().max(0) as u64;
    let std_dur = std::time::Duration::from_secs(secs);
    // Take only the most significant unit from humantime output.
    humantime::format_duration(std_dur)
        .to_string()
        .split_whitespace()
        .next()
        .unwrap_or("just now")
        .to_string()
}

#[get("/api/manga/<id>/chapters")]
async fn list_chapters(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<ChapterListItem>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let chapters = db::chapter::get_all_for_manga(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    let items = chapters
        .into_iter()
        .map(|ch| {
            let found_ago = format_found_ago(ch.created_at);
            ChapterListItem { chapter: ch, found_ago }
        })
        .collect();

    Ok(Json(items))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<num>/download
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<num>/download")]
async fn download_chapter_api(
    pool: &State<SqlitePool>,
    id: &str,
    num: f32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let chapter = db::chapter::get_by_number(pool.inner(), manga_id, num)
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

    Ok(Status::Accepted)
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
// POST /api/manga/<id>/chapters/<num>/mark-downloaded
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<num>/mark-downloaded")]
async fn mark_chapter_downloaded(
    pool: &State<SqlitePool>,
    id: &str,
    num: f32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_by_number(pool.inner(), manga_id, num)
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
// POST /api/manga/<id>/chapters/<num>/optimise
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<num>/optimise")]
async fn optimise_chapter_api(
    pool: &State<SqlitePool>,
    id: &str,
    num: f32,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let chapter = db::chapter::get_by_number(pool.inner(), manga_id, num)
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
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::task::cancel(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// GET /api/settings  +  PUT /api/settings
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SettingsResponse {
    scan_interval_hours: u64,
    queue_paused: bool,
}

#[derive(Deserialize)]
struct UpdateSettingsRequest {
    scan_interval_hours: Option<u64>,
    queue_paused: Option<bool>,
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
    Ok(Json(SettingsResponse {
        scan_interval_hours: hours,
        queue_paused,
    }))
}

#[put("/api/settings", data = "<body>")]
async fn update_settings(
    pool: &State<SqlitePool>,
    body: Json<UpdateSettingsRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    if let Some(hours) = body.scan_interval_hours {
        if hours < 1 || hours > 168 {
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

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

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
        set_provider,
        list_manga_providers,
        patch_manga_provider,
        list_chapters,
        download_chapter_api,
        refresh_manga_api,
        scan_disk_api,
        mark_chapter_downloaded,
        optimise_chapter_api,
        list_tasks,
        cancel_task,
        get_settings,
        update_settings,
        serve_cover,
    ]
}
