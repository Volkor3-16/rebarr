use rocket::{State, post, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::http::metadata::AniListMetadata;
use crate::importer::{self, ConfirmedImport, ConfirmedSeriesImport, FolderEntry,
                      ImportCandidate, ImportSummary, SeriesImportSummary};

use super::errors::{ApiResult, bad_request, internal};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ScanRequest {
    pub source_dir: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExecuteRequest {
    pub imports: Vec<ConfirmedImport>,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// Scan a directory for CBZ files and return import candidates with suggested manga matches.
#[openapi(tag = "Import")]
#[post("/api/import/scan", data = "<body>")]
pub async fn scan_api(
    pool: &State<SqlitePool>,
    body: Json<ScanRequest>,
) -> ApiResult<Vec<ImportCandidate>> {
    let dir = PathBuf::from(&body.source_dir);
    if !dir.exists() {
        return Err(bad_request(format!(
            "directory does not exist: {}",
            body.source_dir
        )));
    }
    if !dir.is_dir() {
        return Err(bad_request(format!(
            "path is not a directory: {}",
            body.source_dir
        )));
    }

    let candidates = importer::scan_directory(dir, pool.inner())
        .await
        .map_err(internal)?;

    Ok(Json(candidates))
}

/// Execute confirmed imports: rewrite CBZs with fresh ComicInfo.xml, move to library structure,
/// and enqueue ScanDisk for each affected manga.
#[openapi(tag = "Import")]
#[post("/api/import/execute", data = "<body>")]
pub async fn execute_api(
    pool: &State<SqlitePool>,
    body: Json<ExecuteRequest>,
) -> ApiResult<ImportSummary> {
    if body.imports.is_empty() {
        return Err(bad_request("no imports provided"));
    }

    let summary = importer::execute_imports(body.into_inner().imports, pool.inner()).await;
    Ok(Json(summary))
}

// ---------------------------------------------------------------------------
// Series import request types
// ---------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct SeriesScanRequest {
    pub source_dir: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SeriesExecuteRequest {
    pub imports: Vec<ConfirmedSeriesImport>,
    #[serde(default)]
    pub queue_chapter_scan: bool,
}

// ---------------------------------------------------------------------------
// Series import routes
// ---------------------------------------------------------------------------

/// Scan a directory for immediate subdirectories (one per manga series).
/// Returns folder names + shallow CBZ count. Does not touch AniList.
#[openapi(tag = "Import")]
#[post("/api/import/series-scan", data = "<body>")]
pub async fn series_scan_api(
    body: Json<SeriesScanRequest>,
) -> ApiResult<Vec<FolderEntry>> {
    let dir = PathBuf::from(&body.source_dir);
    if !dir.exists() {
        return Err(bad_request(format!(
            "directory does not exist: {}",
            body.source_dir
        )));
    }
    if !dir.is_dir() {
        return Err(bad_request(format!(
            "path is not a directory: {}",
            body.source_dir
        )));
    }

    let entries = importer::scan_series_dir(dir).await.map_err(internal)?;
    Ok(Json(entries))
}

/// Bulk-create manga entries from confirmed series matches.
/// Always queues ScanDisk for each added series; optionally queues BuildFullChapterList.
#[openapi(tag = "Import")]
#[post("/api/import/series-execute", data = "<body>")]
pub async fn series_execute_api(
    pool: &State<SqlitePool>,
    al: &State<AniListMetadata>,
    http: &State<reqwest::Client>,
    body: Json<SeriesExecuteRequest>,
) -> ApiResult<SeriesImportSummary> {
    if body.imports.is_empty() {
        return Err(bad_request("no imports provided"));
    }
    let inner = body.into_inner();
    let summary = importer::execute_series_imports(
        inner.imports,
        pool.inner(),
        al.inner(),
        http.inner(),
        inner.queue_chapter_scan,
    )
    .await;
    Ok(Json(summary))
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![scan_api, execute_api, series_scan_api, series_execute_api]
}
