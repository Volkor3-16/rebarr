use std::path::PathBuf;

use rocket::{State, delete, get, patch, post, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::Deserialize;
use sqlx::SqlitePool;
use tracing::debug;
use uuid::Uuid;

use crate::{
    db,
    db::suggestions::LibrarySuggestionList,
    http::metadata::AniListMetadata,
    library::suggestions as library_suggestions,
    manga::core::{Manga, MangaType},
};

use super::errors::{ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct NewLibraryRequest {
    pub library_type: String,
    pub root_path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateLibraryRequest {
    pub root_path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SuggestionVisibilityRequest {
    pub hidden: bool,
}

// ---------------------------------------------------------------------------
// GET /api/libraries
// ---------------------------------------------------------------------------

/// Returns all libraries in rebarr.
#[openapi(tag = "Libraries")]
#[get("/api/libraries")]
pub async fn list_libraries(
    pool: &State<SqlitePool>,
) -> ApiResult<Vec<crate::manga::core::Library>> {
    debug!("Listing Libraries (GET /api/libraries)");
    db::library::get_all(pool.inner())
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/libraries
// ---------------------------------------------------------------------------

/// Creates a new library
#[openapi(tag = "Libraries")]
#[post("/api/libraries", data = "<body>")]
pub async fn create_library(
    pool: &State<SqlitePool>,
    body: Json<NewLibraryRequest>,
) -> ApiResult<crate::manga::core::Library> {
    debug!("Creating new library: {}", body.root_path);
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }

    let r#type = match body.library_type.as_str() {
        "Comics" => MangaType::Comics,
        _ => MangaType::Manga,
    };

    let root_path = PathBuf::from(body.root_path.trim());
    let lib = crate::manga::core::Library {
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

/// Returns the info about a specific library.
#[openapi(tag = "Libraries")]
#[get("/api/libraries/<id>")]
pub async fn get_library(
    pool: &State<SqlitePool>,
    id: &str,
) -> ApiResult<crate::manga::core::Library> {
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

/// Returns all manga series in a specific library.
#[openapi(tag = "Libraries")]
#[get("/api/libraries/<id>/manga")]
pub async fn list_library_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<Manga>> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    debug!("Getting manga list by library id: {uuid}");
    db::manga::get_all_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

/// Returns cached suggestions for a specific library.
#[openapi(tag = "Libraries")]
#[get("/api/libraries/<id>/suggestions")]
pub async fn list_library_suggestions(
    pool: &State<SqlitePool>,
    id: &str,
) -> ApiResult<LibrarySuggestionList> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;
    db::suggestions::get_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

/// Enqueue a suggestions refresh for a specific library.
#[openapi(tag = "Libraries")]
#[post("/api/libraries/<id>/suggestions/refresh")]
pub async fn refresh_library_suggestions(
    pool: &State<SqlitePool>,
    al: &State<AniListMetadata>,
    id: &str,
) -> ApiResult<LibrarySuggestionList> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("library not found"))?;
    library_suggestions::refresh_library_suggestions(pool.inner(), al.inner(), uuid)
        .await
        .map_err(internal)?;
    db::suggestions::get_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

/// Hide or unhide a suggestion for a library.
#[openapi(tag = "Libraries")]
#[patch("/api/libraries/<id>/suggestions/<anilist_id>", data = "<body>")]
pub async fn set_suggestion_visibility(
    pool: &State<SqlitePool>,
    id: &str,
    anilist_id: u32,
    body: Json<SuggestionVisibilityRequest>,
) -> ApiResult<LibrarySuggestionList> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let updated = db::suggestions::set_hidden(pool.inner(), uuid, anilist_id, body.hidden)
        .await
        .map_err(internal)?;
    if !updated {
        return Err(not_found("suggestion not found"));
    }
    db::suggestions::get_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// PUT /api/libraries/<id>
// ---------------------------------------------------------------------------

/// Updates/changes the root path of a library
#[openapi(tag = "Libraries")]
#[put("/api/libraries/<id>", data = "<body>")]
pub async fn update_library(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<UpdateLibraryRequest>,
) -> ApiResult<crate::manga::core::Library> {
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

/// Deletes a library (and cleans up all leftovers)
#[openapi(tag = "Libraries")]
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
        list_library_suggestions,
        refresh_library_suggestions,
        set_suggestion_visibility,
    ]
}
