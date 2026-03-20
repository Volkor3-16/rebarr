use rocket::{State, get, put, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::provider_scores;
use super::errors::{ApiResult, bad_request, internal};

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct GlobalScoreResponse {
    /// Current global score override. None means no override is set (YAML default applies).
    pub score: Option<i32>,
    /// Whether this provider is globally enabled (default: true).
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct SeriesScoreResponse {
    /// Current per-series score override. None means global/YAML default applies.
    pub score: Option<i32>,
    /// Whether this provider is enabled for this series (default: true).
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct SetGlobalScoreRequest {
    pub score: i32,
    pub enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct SetSeriesScoreRequest {
    pub score: Option<i32>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// GET /api/providers/<name>/score
// ---------------------------------------------------------------------------

#[get("/api/providers/<name>/score")]
pub async fn get_global_score(
    pool: &State<SqlitePool>,
    name: &str,
) -> ApiResult<GlobalScoreResponse> {
    let score = provider_scores::get_global_score(pool.inner(), name)
        .await
        .map_err(internal)?;
    let enabled = provider_scores::get_global_enabled(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Json(GlobalScoreResponse { score, enabled }))
}

// ---------------------------------------------------------------------------
// PUT /api/providers/<name>/score
// ---------------------------------------------------------------------------

#[put("/api/providers/<name>/score", data = "<body>")]
pub async fn set_global_score(
    pool: &State<SqlitePool>,
    name: &str,
    body: Json<SetGlobalScoreRequest>,
) -> ApiResult<GlobalScoreResponse> {
    let enabled = body.enabled.unwrap_or(true);
    provider_scores::upsert_global_score(pool.inner(), name, body.score, enabled)
        .await
        .map_err(internal)?;
    Ok(Json(GlobalScoreResponse {
        score: Some(body.score),
        enabled,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers/<name>/score
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>/providers/<name>/score")]
pub async fn get_series_score(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
) -> ApiResult<SeriesScoreResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let score = provider_scores::get_series_score(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;
    let enabled = provider_scores::get_enabled(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;
    Ok(Json(SeriesScoreResponse { score, enabled }))
}

// ---------------------------------------------------------------------------
// PUT /api/manga/<id>/providers/<name>/score
// ---------------------------------------------------------------------------

#[put("/api/manga/<id>/providers/<name>/score", data = "<body>")]
pub async fn set_series_score(
    pool: &State<SqlitePool>,
    id: &str,
    name: &str,
    body: Json<SetSeriesScoreRequest>,
) -> ApiResult<SeriesScoreResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let score = body.score.unwrap_or(0);
    let enabled = body.enabled.unwrap_or(true);
    provider_scores::upsert_series(pool.inner(), name, manga_id, score, enabled)
        .await
        .map_err(internal)?;
    Ok(Json(SeriesScoreResponse {
        score: Some(score),
        enabled,
    }))
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        get_global_score,
        set_global_score,
        get_series_score,
        set_series_score,
    ]
}
