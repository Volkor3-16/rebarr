use rocket::{State, get, http::Status, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::scraper::ScraperCtx;

use super::errors::{ApiError, ApiResult, bad_request, internal};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct SettingsResponse {
    pub scan_interval_hours: u64,
    pub queue_paused: bool,
    pub browser_worker_count: u64,
    /// BCP 47 language code to prefer when selecting a provider (e.g. "en"). `null` = accept any.
    pub preferred_language: Option<String>,
    /// Whether the first-run setup wizard has been completed.
    pub wizard_completed: bool,
    /// Whether newly-added manga should be monitored by default.
    pub default_monitored: bool,
    /// Whether AniList-completed series should be unmonitored on add/refresh.
    pub auto_unmonitor_completed: bool,
    /// Whether already-downloaded chapters should be preserved as canonical winners.
    pub disable_chapter_upgrades: bool,
    /// Download mode: "best_only" (try best release, fail immediately) or "must_have" (fallback on failure).
    pub download_mode: String,
    pub task_retention_enabled: bool,
    pub task_retention_days: u64,
    pub task_retention_min_keep: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateSettingsRequest {
    pub scan_interval_hours: Option<u64>,
    pub queue_paused: Option<bool>,
    pub browser_worker_count: Option<u64>,
    /// Set to a BCP 47 code (e.g. "en") to filter downloads to that language, or "" to clear.
    pub preferred_language: Option<String>,
    pub wizard_completed: Option<bool>,
    pub default_monitored: Option<bool>,
    pub auto_unmonitor_completed: Option<bool>,
    pub disable_chapter_upgrades: Option<bool>,
    /// "best_only" or "must_have".
    pub download_mode: Option<String>,
    pub task_retention_enabled: Option<bool>,
    pub task_retention_days: Option<u64>,
    pub task_retention_min_keep: Option<u64>,
}

// ---------------------------------------------------------------------------
// GET /api/settings
// ---------------------------------------------------------------------------

/// Get current application settings.
#[openapi(tag = "Settings")]
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
    // Absence of wizard_completed key means the wizard has not been run.
    let wizard_completed = db::settings::get(pool.inner(), "wizard_completed", "false")
        .await
        .unwrap_or_else(|_| "false".to_string())
        == "true";
    let default_monitored = db::settings::get(pool.inner(), "default_monitored", "true")
        .await
        .unwrap_or_else(|_| "true".to_string())
        != "false";
    let auto_unmonitor_completed =
        db::settings::get(pool.inner(), "auto_unmonitor_completed", "false")
            .await
            .unwrap_or_else(|_| "false".to_string())
            == "true";
    let disable_chapter_upgrades =
        db::settings::get(pool.inner(), "disable_chapter_upgrades", "false")
            .await
            .unwrap_or_else(|_| "false".to_string())
            == "true";
    let download_mode_raw = db::settings::get(pool.inner(), "download_mode", "must_have")
        .await
        .unwrap_or_else(|_| "must_have".to_string());
    let download_mode = if download_mode_raw == "best_only" {
        "best_only".to_string()
    } else {
        "must_have".to_string()
    };
    let task_retention_enabled = db::settings::get(pool.inner(), "task_retention_enabled", "false")
        .await
        .unwrap_or_else(|_| "false".to_string())
        == "true";
    let task_retention_days = db::settings::get(pool.inner(), "task_retention_days", "30")
        .await
        .unwrap_or_else(|_| "30".to_string())
        .parse::<u64>()
        .unwrap_or(30)
        .clamp(1, 3650);
    let task_retention_min_keep = db::settings::get(pool.inner(), "task_retention_min_keep", "200")
        .await
        .unwrap_or_else(|_| "200".to_string())
        .parse::<u64>()
        .unwrap_or(200)
        .clamp(0, 100_000);
    Ok(Json(SettingsResponse {
        scan_interval_hours: hours,
        queue_paused,
        browser_worker_count,
        preferred_language,
        wizard_completed,
        default_monitored,
        auto_unmonitor_completed,
        disable_chapter_upgrades,
        download_mode,
        task_retention_enabled,
        task_retention_days,
        task_retention_min_keep,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/settings
// ---------------------------------------------------------------------------

/// Update application settings.
#[openapi(tag = "Settings")]
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
    if let Some(enabled) = body.auto_unmonitor_completed {
        db::settings::set(
            pool.inner(),
            "auto_unmonitor_completed",
            if enabled { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(enabled) = body.disable_chapter_upgrades {
        db::settings::set(
            pool.inner(),
            "disable_chapter_upgrades",
            if enabled { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(ref mode) = body.download_mode {
        let validated = if mode == "best_only" {
            "best_only"
        } else {
            "must_have"
        };
        db::settings::set(pool.inner(), "download_mode", validated)
            .await
            .map_err(internal)?;
    }
    if let Some(enabled) = body.task_retention_enabled {
        db::settings::set(
            pool.inner(),
            "task_retention_enabled",
            if enabled { "true" } else { "false" },
        )
        .await
        .map_err(internal)?;
    }
    if let Some(days) = body.task_retention_days {
        if !(1..=3650).contains(&days) {
            return Err(bad_request("task_retention_days must be 1–3650"));
        }
        db::settings::set(pool.inner(), "task_retention_days", &days.to_string())
            .await
            .map_err(internal)?;
    }
    if let Some(min_keep) = body.task_retention_min_keep {
        if min_keep > 100_000 {
            return Err(bad_request("task_retention_min_keep must be 0–100000"));
        }
        db::settings::set(
            pool.inner(),
            "task_retention_min_keep",
            &min_keep.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scraper::{ProviderRegistry, browser::BrowserPool, executor::ProviderExecutor};
    use rocket::local::asynchronous::Client;
    use std::sync::Arc;

    async fn test_client() -> Client {
        let pool = crate::db::init("sqlite::memory:").await.expect("init db");
        let registry = ProviderRegistry::from_providers_for_tests(vec![]);
        let ctx = ScraperCtx::new(
            reqwest::Client::new(),
            BrowserPool::new(),
            Arc::new(ProviderExecutor::new(&registry, 2)),
        );
        let rocket = rocket::build()
            .manage(pool)
            .manage(ctx)
            .mount("/", rocket::routes![get_settings, update_settings]);
        Client::tracked(rocket).await.expect("client")
    }

    #[tokio::test]
    async fn retention_settings_round_trip() {
        let client = test_client().await;

        let response = client
            .put("/api/settings")
            .header(rocket::http::ContentType::JSON)
            .body(
                r#"{
                    "task_retention_enabled": true,
                    "task_retention_days": 45,
                    "task_retention_min_keep": 321
                }"#,
            )
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::NoContent);

        let response = client.get("/api/settings").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let json: serde_json::Value = response.into_json().await.expect("json");
        assert_eq!(json["task_retention_enabled"], true);
        assert_eq!(json["task_retention_days"], 45);
        assert_eq!(json["task_retention_min_keep"], 321);
    }

    #[tokio::test]
    async fn retention_settings_validate_bounds() {
        let client = test_client().await;

        let response = client
            .put("/api/settings")
            .header(rocket::http::ContentType::JSON)
            .body(r#"{ "task_retention_days": 0 }"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest);

        let response = client
            .put("/api/settings")
            .header(rocket::http::ContentType::JSON)
            .body(r#"{ "task_retention_min_keep": 100001 }"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest);
    }
}
