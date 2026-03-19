use std::path::PathBuf;

use log::debug;
use rocket::{State, delete, get, post, put, serde::json::Json};
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db,
    manga::manga::{Manga, MangaType},
};

use super::errors::{ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NewLibraryRequest {
    pub library_type: String,
    pub root_path: String,
}

#[derive(Deserialize)]
pub struct UpdateLibraryRequest {
    pub root_path: String,
}

// ---------------------------------------------------------------------------
// GET /api/libraries
// ---------------------------------------------------------------------------

#[get("/api/libraries")]
pub async fn list_libraries(
    pool: &State<SqlitePool>,
) -> ApiResult<Vec<crate::manga::manga::Library>> {
    debug!("Listing Libraries (GET /api/libraries)");
    db::library::get_all(pool.inner())
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/libraries
// ---------------------------------------------------------------------------

#[post("/api/libraries", data = "<body>")]
pub async fn create_library(
    pool: &State<SqlitePool>,
    body: Json<NewLibraryRequest>,
) -> ApiResult<crate::manga::manga::Library> {
    debug!("Creating new library: {}", body.root_path);
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }

    let r#type = match body.library_type.as_str() {
        "Comics" => MangaType::Comics,
        _ => MangaType::Manga,
    };

    let root_path = PathBuf::from(body.root_path.trim());
    let lib = crate::manga::manga::Library {
        uuid: db::library::library_uuid(
            body.library_type.as_str(),
            root_path.to_string_lossy().as_ref(),
        ),
        r#type,
        root_path,
    };

    db::library::insert(pool.inner(), &lib)
        .await
        .map_err(internal)?;
    Ok(Json(lib))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>")]
pub async fn get_library(
    pool: &State<SqlitePool>,
    id: &str,
) -> ApiResult<crate::manga::manga::Library> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    debug!("Getting library by id: {uuid}");
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("library not found"))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>/manga
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>/manga")]
pub async fn list_library_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<Manga>> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    debug!("Getting manga list by library id: {uuid}");
    db::manga::get_all_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// PUT /api/libraries/<id>
// ---------------------------------------------------------------------------

#[put("/api/libraries/<id>", data = "<body>")]
pub async fn update_library(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<UpdateLibraryRequest>,
) -> ApiResult<crate::manga::manga::Library> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }
    db::library::update_root_path(pool.inner(), uuid, body.root_path.trim())
        .await
        .map_err(internal)?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or_else(|| not_found("library not found"))
}

// ---------------------------------------------------------------------------
// DELETE /api/libraries/<id>
// ---------------------------------------------------------------------------

#[delete("/api/libraries/<id>")]
pub async fn delete_library(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<rocket::http::Status, (rocket::http::Status, Json<super::errors::ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::delete(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    Ok(rocket::http::Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_libraries,
        create_library,
        get_library,
        update_library,
        delete_library,
        list_library_manga,
    ]
}
