use rocket::{State, delete, get, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::errors::{ApiResult, bad_request, internal};
use crate::db::{chapter as db_chapter, provider_settings, settings as db_settings};

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct GlobalSettingsResponse {
    /// Whether this provider is globally enabled (default: true).
    pub enabled: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SeriesSettingsResponse {
    /// Per-series override. None means no override is set (global setting applies).
    pub enabled: Option<bool>,
    /// Effective enabled value after applying global fallback.
    pub effective_enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetGlobalSettingsRequest {
    pub enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetSeriesSettingsRequest {
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// GET /api/providers/<name>/settings
// ---------------------------------------------------------------------------

/// Get global enabled setting for a provider.
#[openapi(tag = "Provider Settings")]
#[get("/api/providers/<name>/settings")]
pub async fn get_global_settings(
    pool: &State<SqlitePool>,
    name: &str,
) -> ApiResult<GlobalSettingsResponse> {
    let enabled = provider_settings::get_global_enabled(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Json(GlobalSettingsResponse { enabled }))
}

// ---------------------------------------------------------------------------
// PUT /api/providers/<name>/settings
// ---------------------------------------------------------------------------

/// Set global enabled setting for a provider.
/// Also regenerates canonical chapters for all manga that use this provider.
#[openapi(tag = "Provider Settings")]
#[put("/api/providers/<name>/settings", data = "<body>")]
pub async fn set_global_settings(
    pool: &State<SqlitePool>,
    name: &str,
    body: Json<SetGlobalSettingsRequest>,
) -> ApiResult<GlobalSettingsResponse> {
    provider_settings::set_global_enabled(pool.inner(), name, body.enabled)
        .await
        .map_err(internal)?;

    // Regenerate canonical chapters for all manga that use this provider.
    let preferred_language = db_settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    for manga_id in manga_ids_for_provider(pool.inner(), name)
        .await
        .map_err(internal)?
    {
        db_chapter::update_canonical(pool.inner(), manga_id, &preferred_language)
            .await
            .map_err(internal)?;
    }

    Ok(Json(GlobalSettingsResponse { enabled: body.enabled }))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers/<name>/settings
// ---------------------------------------------------------------------------

/// Get per-series enabled setting for a provider.
#[openapi(tag = "Provider Settings")]
#[get("/api/manga/<id>/providers/<name>/settings")]
pub async fn get_series_settings(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
) -> ApiResult<SeriesSettingsResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let enabled = provider_settings::get_series_enabled(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;
    let effective_enabled = provider_settings::get_effective_enabled(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;
    Ok(Json(SeriesSettingsResponse { enabled, effective_enabled }))
}

// ---------------------------------------------------------------------------
// PUT /api/manga/<id>/providers/<name>/settings
// ---------------------------------------------------------------------------

/// Set per-series enabled setting for a provider.
/// Also regenerates canonical chapters for this manga.
#[openapi(tag = "Provider Settings")]
#[put("/api/manga/<id>/providers/<name>/settings", data = "<body>")]
pub async fn set_series_settings(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
    body: Json<SetSeriesSettingsRequest>,
) -> ApiResult<SeriesSettingsResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    provider_settings::set_series_enabled(pool.inner(), name, manga_id, body.enabled)
        .await
        .map_err(internal)?;

    // When disabling a provider for this series, purge all missing chapters from that provider.
    if !body.enabled {
        db_chapter::delete_missing_for_provider(pool.inner(), manga_id, name)
            .await
            .map_err(internal)?;
    }

    let preferred_language = db_settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    db_chapter::update_canonical(pool.inner(), manga_id, &preferred_language)
        .await
        .map_err(internal)?;

    Ok(Json(SeriesSettingsResponse {
        enabled: Some(body.enabled),
        effective_enabled: body.enabled,
    }))
}

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>/providers/<name>/settings
// ---------------------------------------------------------------------------

/// Clear the per-series enabled override, reverting to the global setting.
#[openapi(tag = "Provider Settings")]
#[delete("/api/manga/<id>/providers/<name>/settings")]
pub async fn delete_series_settings(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
) -> ApiResult<SeriesSettingsResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    provider_settings::delete_series_setting(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;

    let preferred_language = db_settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    db_chapter::update_canonical(pool.inner(), manga_id, &preferred_language)
        .await
        .map_err(internal)?;

    let effective_enabled =
        provider_settings::get_effective_enabled(pool.inner(), name, manga_id)
            .await
            .map_err(internal)?;
    Ok(Json(SeriesSettingsResponse { enabled: None, effective_enabled }))
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        get_global_settings,
        set_global_settings,
        get_series_settings,
        set_series_settings,
        delete_series_settings,
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn manga_ids_for_provider(
    pool: &SqlitePool,
    provider_name: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT manga_id FROM Chapters WHERE provider_name = ?")
            .bind(provider_name)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(s,)| Uuid::parse_str(&s).ok())
        .collect())
}
