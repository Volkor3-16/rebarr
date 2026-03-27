use rocket::{State, get, http::Status, put, serde::json::Json};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::scraper::ScraperCtx;

use super::errors::{ApiError, ApiResult, bad_request, internal};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SettingsResponse {
    pub scan_interval_hours: u64,
    pub queue_paused: bool,
    pub browser_worker_count: u64,
    /// BCP 47 language code to prefer when selecting a provider (e.g. "en"). `null` = accept any.
    pub preferred_language: Option<String>,
    /// Comma-separated list of language codes to filter from synonym searches
    pub synonym_filter_languages: String,
    /// Whether the first-run setup wizard has been completed.
    pub wizard_completed: bool,
    /// Whether newly-added manga should be monitored by default.
    pub default_monitored: bool,
    /// Minimum scanlator tier to consider when downloading (1=Official only, 4=all sources).
    pub min_tier: u64,
    /// Whether AniList-completed series should be unmonitored on add/refresh.
    pub auto_unmonitor_completed: bool,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub scan_interval_hours: Option<u64>,
    pub queue_paused: Option<bool>,
    pub browser_worker_count: Option<u64>,
    /// Set to a BCP 47 code (e.g. "en") to filter downloads to that language, or "" to clear.
    pub preferred_language: Option<String>,
    /// Comma-separated list of language codes to filter from synonym searches (e.g. "zh,vi,ru")
    pub synonym_filter_languages: Option<String>,
    pub wizard_completed: Option<bool>,
    pub default_monitored: Option<bool>,
    /// 1–4: minimum scanlator tier.
    pub min_tier: Option<u64>,
    pub auto_unmonitor_completed: Option<bool>,
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
    let browser_worker_count = db::settings::get(pool.inner(), "browser_worker_count", "3")
        .await
        .map_err(internal)?
        .parse::<u64>()
        .unwrap_or(3)
        .clamp(1, 16);
    let lang_raw = db::settings::get(pool.inner(), "preferred_language", "")
        .await
        .map_err(internal)?;
    let preferred_language = if lang_raw.is_empty() {
        None
    } else {
        Some(lang_raw)
    };
    // Default to empty - user must explicitly configure filters
    let filter_langs = db::settings::get(pool.inner(), "synonym_filter_languages", "")
        .await
        .map_err(internal)?;
    // Absence of wizard_completed key means the wizard has not been run.
    let wizard_completed = db::settings::get(pool.inner(), "wizard_completed", "false")
        .await
        .unwrap_or_else(|_| "false".to_string())
        == "true";
    let default_monitored = db::settings::get(pool.inner(), "default_monitored", "true")
        .await
        .unwrap_or_else(|_| "true".to_string())
        != "false";
    let min_tier = db::settings::get(pool.inner(), "min_tier", "4")
        .await
        .unwrap_or_else(|_| "4".to_string())
        .parse::<u64>()
        .unwrap_or(4)
        .clamp(1, 4);
    let auto_unmonitor_completed =
        db::settings::get(pool.inner(), "auto_unmonitor_completed", "false")
            .await
            .unwrap_or_else(|_| "false".to_string())
            == "true";
    Ok(Json(SettingsResponse {
        scan_interval_hours: hours,
        queue_paused,
        browser_worker_count,
        preferred_language,
        synonym_filter_languages: filter_langs,
        wizard_completed,
        default_monitored,
        min_tier,
        auto_unmonitor_completed,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/settings
// ---------------------------------------------------------------------------

#[put("/api/settings", data = "<body>")]
pub async fn update_settings(
    pool: &State<sqlx::SqlitePool>,
    ctx: &State<ScraperCtx>,
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
        db::settings::set(
            pool.inner(),
            "queue_paused",
            if paused { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(count) = body.browser_worker_count {
        if !(1..=16).contains(&count) {
            return Err(bad_request("browser_worker_count must be 1–16"));
        }
        db::settings::set(pool.inner(), "browser_worker_count", &count.to_string())
            .await
            .map_err(internal)?;
        ctx.executor.set_browser_worker_count(count as usize).await;
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
    if let Some(completed) = body.wizard_completed {
        db::settings::set(
            pool.inner(),
            "wizard_completed",
            if completed { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(monitored) = body.default_monitored {
        db::settings::set(
            pool.inner(),
            "default_monitored",
            if monitored { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(tier) = body.min_tier {
        if !(1..=4).contains(&tier) {
            return Err(bad_request("min_tier must be 1–4"));
        }
        db::settings::set(pool.inner(), "min_tier", &tier.to_string())
            .await
            .map_err(internal)?;
    }
    if let Some(enabled) = body.auto_unmonitor_completed {
        db::settings::set(
            pool.inner(),
            "auto_unmonitor_completed",
            if enabled { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![get_settings, update_settings,]
}
