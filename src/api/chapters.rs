use chrono::Utc;
use rocket::{State, delete, get, http::Status, post, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    db,
    manga::{core::DownloadStatus, files, metadata_rules, scoring},
    scheduler::worker::CancelMap,
};

use super::errors::{ApiError, ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

/// Response struct for a single chapter row (all providers included, not just canonical).
#[derive(Serialize, JsonSchema)]
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
    /// True if the canonical for this slot was set by the user (not auto-scored).
    pub has_canonical_override: bool,
    /// Quality score computed from quality rules (higher = preferred source).
    pub score: i32,
    /// List of quality rules that matched this chapter, with their individual score values.
    pub matched_rules: Vec<(String, i32)>,
    /// Size of the CBZ file on disk in bytes (None if not yet downloaded or not measured).
    pub file_size_bytes: Option<i64>,
    /// User-applied tags (e.g. "hidden", "low_quality").
    pub tags: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetCanonicalRequest {
    pub chapter_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddTagRequest {
    pub tag: String,
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/chapters
// ---------------------------------------------------------------------------

/// Returns a list of chapters for a given manga.
#[openapi(tag = "Chapters")]
#[get("/api/manga/<id>/chapters")]
pub async fn list_chapters(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<ChapterListItem>> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), manga_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("manga not found"))?;
    build_chapter_list(pool.inner(), manga_id).await
}

/// Load and shape the full chapter list for a manga:
/// applies preferred-language filtering, annotates each row with its canonical flag
/// and tier score, and returns the API response shape.
/// Metadata (title, released_at) is merged from all providers for each canonical slot:
/// the highest-scored source's title is used, earliest release date wins.
async fn build_chapter_list(pool: &SqlitePool, manga_id: Uuid) -> ApiResult<Vec<ChapterListItem>> {
    let all_rows = db::chapter::get_all_for_manga(pool, manga_id)
        .await
        .map_err(internal)?;

    let preferred_language = db::settings::get(pool, "preferred_language", "")
        .await
        .map_err(internal)?;

    // Filter by preferred language; chapters with no language set are always included.
    let filtered_rows: Vec<_> = if preferred_language.is_empty() {
        all_rows.clone()
    } else {
        all_rows
            .iter()
            .filter(|ch| {
                ch.language.eq_ignore_ascii_case(&preferred_language) || ch.language.is_empty()
            })
            .cloned()
            .collect()
    };

    let canonical_uuids: std::collections::HashSet<String> =
        db::chapter::get_canonical_uuids(pool, manga_id)
            .await
            .map_err(internal)?
            .into_iter()
            .collect();

    // Load canonical overrides so we can expose has_canonical_override per row.
    let canonical_overrides = db::chapter::get_canonical_overrides_map(pool, manga_id)
        .await
        .map_err(internal)?;

    // Load quality rules for metadata merging score comparisons.
    let quality_rules = db::quality_rules::get_all(pool).await.map_err(internal)?;

    // Load metadata filtering rules to clean up provider metadata before merging.
    let meta_rules = metadata_rules::load(pool).await.map_err(internal)?;

    // Build a per-slot metadata merge map: (base, variant) -> (best_title, earliest_released_at).
    // Only for canonical slots — non-canonical rows never need merged metadata.
    // We look across ALL provider rows for the same (base, variant, language) slot.
    let mut slot_meta: std::collections::HashMap<(i32, i32), (Option<String>, Option<i64>)> =
        std::collections::HashMap::new();

    for canonical_uuid in &canonical_uuids {
        // Find the canonical chapter row.
        let Some(canonical_ch) = filtered_rows
            .iter()
            .find(|ch| &ch.id.to_string() == canonical_uuid)
        else {
            continue;
        };

        let key = (canonical_ch.chapter_base, canonical_ch.chapter_variant);
        let lang = &canonical_ch.language;

        // Collect all provider rows for this (base, variant, language) slot.
        let slot_rows: Vec<_> = all_rows
            .iter()
            .filter(|ch| {
                ch.chapter_base == canonical_ch.chapter_base
                    && ch.chapter_variant == canonical_ch.chapter_variant
                    && ch.language.eq_ignore_ascii_case(lang)
            })
            .collect();

        // Best title: score-ordered, first non-empty title wins after applying metadata rules.
        let mut scored: Vec<_> = slot_rows
            .iter()
            .map(|ch| {
                let score = scoring::compute_score(ch, &quality_rules);
                (score, ch)
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        let best_title = scored
            .iter()
            .filter_map(|(_, ch)| {
                let provider = ch.provider_name.as_deref();
                let filtered = metadata_rules::apply_rules(
                    &meta_rules,
                    provider,
                    "title",
                    ch.title.as_deref(),
                );
                filtered.filter(|t| !t.is_empty())
            })
            .next();

        // Earliest released_at across all providers.
        let earliest_released_at = slot_rows
            .iter()
            .filter_map(|ch| ch.released_at)
            .map(|dt| dt.timestamp())
            .min();

        slot_meta.insert(key, (best_title, earliest_released_at));
    }

    let items = filtered_rows
        .into_iter()
        .map(|ch| {
            let is_canonical = canonical_uuids.contains(&ch.id.to_string());
            let slot_key_str = format!("{}:{}", ch.chapter_base, ch.chapter_variant);
            let has_canonical_override = is_canonical
                && canonical_overrides
                    .get(&slot_key_str)
                    .map(|ov| ov == &ch.id.to_string())
                    .unwrap_or(false);
            let score = scoring::compute_score(&ch, &quality_rules);
            let matched_rules = scoring::compute_matched_rules(&ch, &quality_rules);

            // For canonical rows, use merged metadata; non-canonical rows use their own data.
            let (display_title, display_released_at) = if is_canonical {
                let slot_key = (ch.chapter_base, ch.chapter_variant);
                if let Some((merged_title, merged_released)) = slot_meta.get(&slot_key) {
                    (
                        merged_title.clone(),
                        merged_released.or_else(|| ch.released_at.map(|dt| dt.timestamp())),
                    )
                } else {
                    (ch.title.clone(), ch.released_at.map(|dt| dt.timestamp()))
                }
            } else {
                let filtered_title = metadata_rules::apply_rules(
                    &meta_rules,
                    ch.provider_name.as_deref(),
                    "title",
                    ch.title.as_deref(),
                );
                (filtered_title, ch.released_at.map(|dt| dt.timestamp()))
            };

            let display_scanlator_group = metadata_rules::apply_rules(
                &meta_rules,
                ch.provider_name.as_deref(),
                "scanlator_group",
                ch.scanlator_group.as_deref(),
            );

            ChapterListItem {
                id: ch.id.to_string(),
                manga_id: ch.manga_id.to_string(),
                chapter_base: ch.chapter_base,
                chapter_variant: ch.chapter_variant,
                title: display_title,
                language: ch.language,
                scanlator_group: display_scanlator_group,
                provider_name: ch.provider_name,
                chapter_url: ch.chapter_url,
                download_status: ch.download_status.as_str().to_string(),
                released_at: display_released_at,
                downloaded_at: ch.downloaded_at.map(|dt| dt.timestamp()),
                scraped_at: ch.scraped_at.map(|dt| dt.timestamp()),
                is_extra: ch.is_extra,
                is_canonical,
                has_canonical_override,
                score,
                matched_rules,
                file_size_bytes: ch.file_size_bytes,
                tags: ch.tags,
            }
        })
        .collect();

    Ok(Json(items))
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/download
// ---------------------------------------------------------------------------

/// Tells rebarr to download a specific (canonical) chapter.
#[openapi(tag = "Chapters")]
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

    // Assign to provider-specific queue if the chapter has a provider name
    let queue = chapter
        .provider_name
        .as_ref()
        .map(|name| format!("provider:{name}"));

    db::task::enqueue_with_queue(
        pool.inner(),
        crate::db::task::TaskType::DownloadChapter,
        Some(manga_id),
        Some(chapter.id),
        10,
        queue,
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

/// Deletes the downloaded cbz from disk and keeps the chapter entry in the database.
#[openapi(tag = "Chapters")]
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

        let chapter_path =
            files::chapter_cbz_path(&files::series_dir(&library.root_path, &manga), &chapter);

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

    db::chapter::set_status(pool.inner(), chapter.id, DownloadStatus::Missing, None)
        .await
        .map_err(internal)?;
    db::chapter::clear_download_artifacts(pool.inner(), chapter.id)
        .await
        .map_err(internal)?;
    db::chapter::update_manga_counts(pool.inner(), manga_id)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

/// Deletes the chapter entry from the database.
#[openapi(tag = "Chapters")]
#[delete("/api/manga/<id>/chapters/<base>/<variant>/entry")]
pub async fn delete_chapter_entry_api(
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

    db::chapter::delete(pool.inner(), chapter.id)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/tags
// DELETE /api/manga/<id>/chapters/<base>/<variant>/tags/<tag>
// ---------------------------------------------------------------------------

/// Add a tag to a chapter (e.g. "hidden", "low_quality").
#[openapi(tag = "Chapters")]
#[post(
    "/api/manga/<id>/chapters/<base>/<variant>/tags",
    data = "<body>"
)]
pub async fn add_chapter_tag_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
    body: Json<AddTagRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::chapter::add_tag(pool.inner(), chapter.id, &body.tag)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

/// Remove a tag from a chapter.
#[openapi(tag = "Chapters")]
#[delete("/api/manga/<id>/chapters/<base>/<variant>/tags/<tag>")]
pub async fn remove_chapter_tag_api(
    pool: &State<SqlitePool>,
    id: &str,
    base: i32,
    variant: i32,
    tag: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let chapter = db::chapter::get_canonical_by_number(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("chapter not found"))?;

    db::chapter::remove_tag(pool.inner(), chapter.id, tag)
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
        delete_chapter_entry_api,
        mark_chapter_downloaded,
        reset_chapter_api,
        toggle_extra_api,
        optimise_chapter_api,
        set_canonical_api,
        clear_canonical_override_api,
        add_chapter_tag_api,
        remove_chapter_tag_api,
    ]
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/mark-downloaded
// ---------------------------------------------------------------------------

/// Marks the given chapter as downloaded
#[openapi(tag = "Chapters")]
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

/// Reset the chapter back to missing, cancelling any running or queued tasks for the chapter.
#[openapi(tag = "Chapters")]
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

/// Marks a given chapter as an 'extra' / .5 special.
#[openapi(tag = "Chapters")]
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

    db::chapter::set_is_extra_manual(pool.inner(), chapter.id, !chapter.is_extra)
        .await
        .map_err(internal)?;

    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/manga/<id>/chapters/<base>/<variant>/optimise
// ---------------------------------------------------------------------------

/// Triggers a optimise task for a given chapter (optimise re-encodes images to webp)
#[openapi(tag = "Chapters")]
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

/// Sets a given chapter as canonical (as in, the 'best' release of a chapter, one that will be downloaded.)
#[openapi(tag = "Chapters")]
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

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>/chapters/<base>/<variant>/canonical-override
// ---------------------------------------------------------------------------

/// Clears a user-set canonical override, returning the slot to auto-scored selection.
#[openapi(tag = "Chapters")]
#[delete("/api/manga/<id>/chapters/<base>/<variant>/canonical-override")]
pub async fn clear_canonical_override_api(
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

    db::chapter::remove_canonical_override(pool.inner(), manga_id, base, variant)
        .await
        .map_err(internal)?;

    // Re-score so the auto-selected winner takes effect immediately.
    let preferred_language = db::settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    db::chapter::update_canonical(pool.inner(), manga_id, &preferred_language)
        .await
        .map_err(internal)?;

    info!("[api] Canonical override cleared: manga={manga_id}, ch={base}.{variant}");
    Ok(Status::NoContent)
}
