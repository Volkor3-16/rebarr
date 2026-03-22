use chrono::Utc;
use log::{info, warn};
use rocket::{State, delete, get, http::Status, post, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db,
    manga::{manga::DownloadStatus, scoring},
    scheduler::worker::CancelMap,
};

use super::errors::{ApiError, ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

/// Response struct for a single chapter row (all providers included, not just canonical).
#[derive(Serialize)]
pub struct ChapterListItem {
    pub id: String,
    pub manga_id: String,
    pub chapter_base: i32,
    pub chapter_variant: i32,
    pub title: Option<String>,
    pub language: String,
    pub scanlator_group: Option<String>,
    pub provider_name: Option<String>,
    pub chapter_url: Option<String>,
    pub download_status: String,
    /// Unix timestamp in seconds
    pub released_at: Option<i64>,
    /// Unix timestamp in seconds
    pub downloaded_at: Option<i64>,
    /// Unix timestamp in seconds
    pub scraped_at: Option<i64>,
    /// True if this chapter is an extra/bonus.
    pub is_extra: bool,
    /// True if this row is the current canonical winner for its (chapter_base, chapter_variant) slot.
    pub is_canonical: bool,
    /// Scanlation tier: 1=Official, 2=Known Scanner, 3=Unknown Scanner, 4=No Group.
    pub tier: u8,
    /// Size of the CBZ file on disk in bytes (None if not yet downloaded or not measured).
    pub file_size_bytes: Option<i64>,
}

#[derive(Deserialize)]
pub struct SetCanonicalRequest {
    pub chapter_id: String,
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/chapters
// ---------------------------------------------------------------------------

/// Returns a list of chapters for a given manga.
#[get("/api/manga/<id>/chapters")]
pub async fn list_chapters(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<ChapterListItem>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;

    let all_rows = db::chapter::get_all_for_manga(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    // Get preferred language setting
    let preferred_language = db::settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;

    // Filter chapters by preferred language if set
    let filtered_rows: Vec<_> = if !preferred_language.is_empty() {
        all_rows
            .into_iter()
            .filter(|ch| {
                // Allow chapters with matching language or no language specified
                ch.language.eq_ignore_ascii_case(&preferred_language) || ch.language.is_empty()
            })
            .collect()
    } else {
        all_rows
    };

    let canonical_uuids: std::collections::HashSet<String> =
        db::chapter::get_canonical_uuids(pool.inner(), manga_id)
            .await
            .map_err(internal)?
            .into_iter()
            .collect();

    let trusted = db::provider::get_trusted_groups(pool.inner())
        .await
        .map_err(internal)?;

    let items = filtered_rows
        .into_iter()
        .map(|ch| {
            let is_canonical = canonical_uuids.contains(&ch.id.to_string());
            let tier = scoring::compute_tier(ch.scanlator_group.as_deref(), &trusted);
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
                file_size_bytes: ch.file_size_bytes,
            }
        })
        .collect();

    Ok(Json(items))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/download
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/download")]
pub async fn download_chapter_api(
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

    info!(
        "[api] Enqueuing download: manga={manga_id}, ch={base}.{variant}, canonical={}",
        chapter.id
    );

    db::task::enqueue(
        pool.inner(),
        crate::db::task::TaskType::DownloadChapter,
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
pub async fn delete_chapter_api(
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
            format!(
                "Chapter {}.{}",
                chapter.chapter_base, chapter.chapter_variant
            )
        };

        // Add title if present and non-empty
        if let Some(ref title) = chapter.title {
            if !title.is_empty() {
                cbz_name.push_str(&format!(" - {title}"));
            }
        }

        // Add scanlator group if present and non-empty
        if let Some(ref group) = chapter.scanlator_group {
            if !group.is_empty() {
                cbz_name.push_str(&format!(" [{group}]"));
            }
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

        let chapter_path = library
            .root_path
            .join(&manga.relative_path)
            .join(format!("{cbz_name}.cbz"));

        if chapter_path.exists() {
            if let Err(e) = std::fs::remove_file(&chapter_path) {
                warn!(
                    "[api] Failed to delete chapter file '{}': {}",
                    chapter_path.display(),
                    e
                );
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
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_chapters,
        download_chapter_api,
        delete_chapter_api,
        mark_chapter_downloaded,
        reset_chapter_api,
        toggle_extra_api,
        optimise_chapter_api,
        set_canonical_api,
    ]
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/mark-downloaded
// ---------------------------------------------------------------------------

#[post("/api/manga/<id>/chapters/<base>/<variant>/mark-downloaded")]
pub async fn mark_chapter_downloaded(
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
pub async fn reset_chapter_api(
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
pub async fn toggle_extra_api(
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
pub async fn optimise_chapter_api(
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
        crate::db::task::TaskType::OptimiseChapter,
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

#[post(
    "/api/manga/<id>/chapters/<base>/<variant>/set-canonical",
    data = "<body>"
)]
pub async fn set_canonical_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
    body: Json<SetCanonicalRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter_id =
        Uuid::parse_str(&body.chapter_id).map_err(|_| bad_request("invalid chapter UUID"))?;

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

    info!("[api] Canonical override set: manga={manga_id}, ch={base}.{variant} → {chapter_id}");

    Ok(Status::NoContent)
}
