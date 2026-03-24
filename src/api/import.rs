use rocket::{State, post, serde::json::Json};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::importer::{self, ConfirmedImport, ImportCandidate, ImportSummary};

use super::errors::{ApiResult, bad_request, internal};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ScanRequest {
    pub source_dir: String,
}

#[derive(Deserialize)]
pub struct ExecuteRequest {
    pub imports: Vec<ConfirmedImport>,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// Scan a directory for CBZ files and return import candidates with suggested manga matches.
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
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![scan_api, execute_api]
}
