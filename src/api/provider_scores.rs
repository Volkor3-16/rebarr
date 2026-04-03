use rocket::{State, delete, get, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::scraper::ProviderRegistry;

use super::errors::{ApiResult, bad_request, internal};
use crate::db::{chapter as db_chapter, provider as db_provider, provider_scores, settings as db_settings};

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct GlobalScoreResponse {
    /// Current global score override. None means no override is set (YAML default applies).
    pub score: Option<i32>,
    /// Whether this provider is globally enabled (default: true).
    pub enabled: bool,
    /// Default score from the provider YAML config (used when no override is set).
    pub default_score: i32,
}

#[derive(Serialize, JsonSchema)]
pub struct SeriesScoreResponse {
    /// Current per-series score override. None means global/YAML default applies.
    pub score: Option<i32>,
    /// Whether this provider is enabled for this series (default: true).
    pub enabled: bool,
    /// Effective score for this series (series > global > yaml default).
    pub effective_score: i32,
    /// Default score from the provider YAML config.
    pub default_score: i32,
    /// Where the effective score comes from: "series", "global", or "default".
    pub score_source: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetGlobalScoreRequest {
    pub score: i32,
    pub enabled: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetSeriesScoreRequest {
    pub score: Option<i32>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// GET /api/providers/<name>/score
// ---------------------------------------------------------------------------

/// Get global score override for a provider.
#[openapi(tag = "Provider Scores")]
#[get("/api/providers/<name>/score")]
pub async fn get_global_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
    name: &str,
) -> ApiResult<GlobalScoreResponse> {
    let score = provider_scores::get_global_score(pool.inner(), name)
        .await
        .map_err(internal)?;
    let enabled = provider_scores::get_global_enabled(pool.inner(), name)
        .await
        .map_err(internal)?;
    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);
    Ok(Json(GlobalScoreResponse { score, enabled, default_score }))
}

// ---------------------------------------------------------------------------
// PUT /api/providers/<name>/score
// ---------------------------------------------------------------------------

/// Set global score override for a provider.
#[openapi(tag = "Provider Scores")]
#[put("/api/providers/<name>/score", data = "<body>")]
pub async fn set_global_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
    name: &str,
    body: Json<SetGlobalScoreRequest>,
) -> ApiResult<GlobalScoreResponse> {
    let enabled = body.enabled.unwrap_or(true);
    provider_scores::upsert_global_score(pool.inner(), name, body.score, enabled)
        .await
        .map_err(internal)?;

    // Regenerate canonical chapters for all manga that use this provider.
    let affected_manga_ids = get_manga_ids_for_provider(pool.inner(), name).await.map_err(internal)?;
    let trusted_groups = db_provider::get_trusted_groups(pool.inner()).await.map_err(internal)?;
    let preferred_language = db_settings::get(pool.inner(), "preferred_language", "").await.map_err(internal)?;
    let yaml_defaults = std::collections::HashMap::new();
    for manga_id in affected_manga_ids {
        let scores = provider_scores::load_effective_scores(pool.inner(), manga_id, &yaml_defaults).await.map_err(internal)?;
        db_chapter::update_canonical(pool.inner(), manga_id, &trusted_groups, &preferred_language, &scores)
            .await
            .map_err(internal)?;
    }

    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);
    Ok(Json(GlobalScoreResponse {
        score: Some(body.score),
        enabled,
        default_score,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>/providers/<name>/score
// ---------------------------------------------------------------------------

/// Get per-series score override for a provider.
#[openapi(tag = "Provider Scores")]
#[get("/api/manga/<id>/providers/<name>/score")]
pub async fn get_series_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
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

    // Determine effective score and its source
    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);

    let (effective_score, score_source) = if let Some(s) = score {
        (s, "series".to_string())
    } else if let Some(s) = provider_scores::get_global_score(pool.inner(), name)
        .await
        .map_err(internal)?
    {
        (s, "global".to_string())
    } else {
        (default_score, "default".to_string())
    };

    Ok(Json(SeriesScoreResponse {
        score,
        enabled,
        effective_score,
        default_score,
        score_source,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/manga/<id>/providers/<name>/score
// ---------------------------------------------------------------------------

/// Set per-series score override for a provider.
#[openapi(tag = "Provider Scores")]
#[put("/api/manga/<id>/providers/<name>/score", data = "<body>")]
pub async fn set_series_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
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

    // Regenerate canonical chapters for this manga.
    let trusted_groups = db_provider::get_trusted_groups(pool.inner()).await.map_err(internal)?;
    let preferred_language = db_settings::get(pool.inner(), "preferred_language", "").await.map_err(internal)?;
    let yaml_defaults = std::collections::HashMap::new();
    let scores = provider_scores::load_effective_scores(pool.inner(), manga_id, &yaml_defaults).await.map_err(internal)?;
    db_chapter::update_canonical(pool.inner(), manga_id, &trusted_groups, &preferred_language, &scores)
        .await
        .map_err(internal)?;

    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);

    Ok(Json(SeriesScoreResponse {
        score: Some(score),
        enabled,
        effective_score: score,
        default_score,
        score_source: "series".to_string(),
    }))
}

// ---------------------------------------------------------------------------
// DELETE /api/providers/<name>/score — remove global override
// ---------------------------------------------------------------------------

/// Remove global score override for a provider.
#[openapi(tag = "Provider Scores")]
#[delete("/api/providers/<name>/score")]
pub async fn delete_global_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
    name: &str,
) -> ApiResult<GlobalScoreResponse> {
    sqlx::query("DELETE FROM Providers WHERE provider_name = ? AND manga_id IS NULL")
        .bind(name)
        .execute(pool.inner())
        .await
        .map_err(internal)?;

    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);

    Ok(Json(GlobalScoreResponse {
        score: None,
        enabled: true,
        default_score,
    }))
}

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>/providers/<name>/score — remove series override
// ---------------------------------------------------------------------------

/// Remove per-series score override for a provider.
#[openapi(tag = "Provider Scores")]
#[delete("/api/manga/<id>/providers/<name>/score")]
pub async fn delete_series_score(
    pool: &State<SqlitePool>,
    registry: &State<std::sync::Arc<ProviderRegistry>>,
    id: &str,
    name: &str,
) -> ApiResult<SeriesScoreResponse> {
    let manga_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    sqlx::query("DELETE FROM Providers WHERE provider_name = ? AND manga_id = ?")
        .bind(name)
        .bind(manga_id.to_string())
        .execute(pool.inner())
        .await
        .map_err(internal)?;

    let default_score = registry
        .all()
        .iter()
        .find(|p| p.name() == name)
        .map(|p| p.default_score())
        .unwrap_or(0);

    // Re-calculate effective score after deletion
    let (effective_score, score_source) = if let Some(s) =
        provider_scores::get_global_score(pool.inner(), name)
            .await
            .map_err(internal)?
    {
        (s, "global".to_string())
    } else {
        (default_score, "default".to_string())
    };

    let enabled = provider_scores::get_enabled(pool.inner(), name, manga_id)
        .await
        .map_err(internal)?;

    Ok(Json(SeriesScoreResponse {
        score: None,
        enabled,
        effective_score,
        default_score,
        score_source,
    }))
}

/// Helper: find all manga IDs that have chapters from a given provider.
async fn get_manga_ids_for_provider(
    pool: &SqlitePool,
    provider_name: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT manga_id FROM Chapters WHERE provider_name = ?",
    )
    .bind(provider_name)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .filter_map(|(s,)| Uuid::parse_str(&s).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .map(Ok)
        .collect()
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        get_global_score,
        set_global_score,
        delete_global_score,
        get_series_score,
        set_series_score,
        delete_series_score,
    ]
}
