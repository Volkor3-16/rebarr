use rocket::{State, get, put, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};

use crate::db;

use super::errors::{bad_request, internal, ApiError, ApiResult};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SettingsResponse {
    pub scan_interval_hours: u64,
    pub queue_paused: bool,
    /// BCP 47 language code to prefer when selecting a provider (e.g. "en"). `null` = accept any.
    pub preferred_language: Option<String>,
    /// Comma-separated list of language codes to filter from synonym searches
    pub synonym_filter_languages: String,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub scan_interval_hours: Option<u64>,
    pub queue_paused: Option<bool>,
    /// Set to a BCP 47 code (e.g. "en") to filter downloads to that language, or "" to clear.
    pub preferred_language: Option<String>,
    /// Comma-separated list of language codes to filter from synonym searches (e.g. "zh,vi,ru")
    pub synonym_filter_languages: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /api/settings
// ---------------------------------------------------------------------------

#[get("/api/settings")]
pub async fn get_settings(pool: &State<sqlx::SqlitePool>) -> ApiResult<SettingsResponse> {
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
    // Default to empty - user must explicitly configure filters
    let filter_langs = db::settings::get(pool.inner(), "synonym_filter_languages", "")
        .await
        .map_err(internal)?;
    Ok(Json(SettingsResponse {
        scan_interval_hours: hours,
        queue_paused,
        preferred_language,
        synonym_filter_languages: filter_langs,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/settings
// ---------------------------------------------------------------------------

#[put("/api/settings", data = "<body>")]
pub async fn update_settings(
    pool: &State<sqlx::SqlitePool>,
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
    if let Some(ref langs) = body.synonym_filter_languages {
        db::settings::set(pool.inner(), "synonym_filter_languages", langs.trim())
            .await
            .map_err(internal)?;
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        get_settings,
        update_settings,
    ]
}
