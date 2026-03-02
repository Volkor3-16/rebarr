use std::path::PathBuf;

use chrono::Utc;
use rocket::{State, delete, get, http::Status, post, routes, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    covers, db,
    manga::{Library, Manga, MangaType},
    metadata::anilist::ALClient,
};

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ApiError {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (Status, Json<ApiError>)>;

fn err(status: Status, msg: impl ToString) -> (Status, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: msg.to_string(),
        }),
    )
}

fn internal(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::InternalServerError, msg)
}

fn bad_request(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::BadRequest, msg)
}

fn not_found(msg: impl ToString) -> (Status, Json<ApiError>) {
    err(Status::NotFound, msg)
}

// ---------------------------------------------------------------------------
// GET /api/libraries
// ---------------------------------------------------------------------------

#[get("/api/libraries")]
async fn list_libraries(pool: &State<SqlitePool>) -> ApiResult<Vec<Library>> {
    db::library::get_all(pool.inner())
        .await
        .map(Json)
        .map_err(|e| internal(e))
}

// ---------------------------------------------------------------------------
// POST /api/libraries
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NewLibraryRequest {
    library_type: String,
    root_path: String,
}

#[post("/api/libraries", data = "<body>")]
async fn create_library(
    pool: &State<SqlitePool>,
    body: Json<NewLibraryRequest>,
) -> ApiResult<Library> {
    if body.root_path.trim().is_empty() {
        return Err(bad_request("root_path cannot be empty"));
    }

    let r#type = match body.library_type.as_str() {
        "Comics" => MangaType::Comics,
        _ => MangaType::Manga,
    };

    let lib = Library {
        uuid: Uuid::new_v4(),
        r#type,
        root_path: PathBuf::from(body.root_path.trim()),
    };

    db::library::insert(pool.inner(), &lib)
        .await
        .map_err(|e| internal(e))?;
    Ok(Json(lib))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>")]
async fn get_library(pool: &State<SqlitePool>, id: &str) -> ApiResult<Library> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::library::get_by_id(pool.inner(), uuid)
        .await
        .map_err(|e| internal(e))?
        .map(Json)
        .ok_or_else(|| not_found("library not found"))
}

// ---------------------------------------------------------------------------
// GET /api/libraries/<id>/manga
// ---------------------------------------------------------------------------

#[get("/api/libraries/<id>/manga")]
async fn list_library_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Vec<Manga>> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_all_for_library(pool.inner(), uuid)
        .await
        .map(Json)
        .map_err(|e| internal(e))
}

// ---------------------------------------------------------------------------
// GET /api/manga/search?q=
// ---------------------------------------------------------------------------

#[get("/api/manga/search?<q>")]
async fn search_manga(al: &State<ALClient>, q: &str) -> ApiResult<Vec<Manga>> {
    if q.trim().is_empty() {
        return Ok(Json(vec![]));
    }
    al.search_manga_as_manga(q.trim())
        .await
        .map(Json)
        .map_err(|e| internal(e))
}

// ---------------------------------------------------------------------------
// POST /api/manga
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddMangaRequest {
    anilist_id: i32,
    library_id: String,
    relative_path: String,
}

#[post("/api/manga", data = "<body>")]
async fn add_manga(
    pool: &State<SqlitePool>,
    al: &State<ALClient>,
    http: &State<reqwest::Client>,
    body: Json<AddMangaRequest>,
) -> ApiResult<Manga> {
    if body.relative_path.trim().is_empty() {
        return Err(bad_request("relative_path cannot be empty"));
    }

    let library_id =
        Uuid::parse_str(&body.library_id).map_err(|_| bad_request("invalid library_id UUID"))?;

    // Verify library exists
    db::library::get_by_id(pool.inner(), library_id)
        .await
        .map_err(|e| internal(e))?
        .ok_or_else(|| not_found("library not found"))?;

    // Fetch full manga from AniList — includes tags
    let mut manga = al
        .grab_manga(body.anilist_id)
        .await
        .map_err(|e| err(Status::BadGateway, format!("AniList lookup failed: {e}")))?;

    manga.id = Uuid::new_v4();
    manga.library_id = library_id;
    manga.relative_path = PathBuf::from(body.relative_path.trim());
    manga.created_at = Utc::now();
    manga.metadata_updated_at = Utc::now();

    // Download high-res cover to disk; fall back to original URL on failure
    if let Some(url) = &manga.thumbnail_url.clone() {
        manga.thumbnail_url = covers::download_cover(http.inner(), url, manga.id)
            .await
            .or(manga.thumbnail_url);
    }

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(|e| internal(e))?;

    Ok(Json(manga))
}

// ---------------------------------------------------------------------------
// GET /api/manga/<id>
// ---------------------------------------------------------------------------

#[get("/api/manga/<id>")]
async fn get_manga(pool: &State<SqlitePool>, id: &str) -> ApiResult<Manga> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::get_by_id(pool.inner(), uuid)
        .await
        .map_err(|e| internal(e))?
        .map(Json)
        .ok_or_else(|| not_found("manga not found"))
}

// ---------------------------------------------------------------------------
// DELETE /api/manga/<id>
// ---------------------------------------------------------------------------

#[delete("/api/manga/<id>")]
async fn delete_manga(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::manga::delete(pool.inner(), uuid)
        .await
        .map_err(|e| internal(e))?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    routes![
        list_libraries,
        create_library,
        get_library,
        list_library_manga,
        search_manga,
        add_manga,
        get_manga,
        delete_manga,
    ]
}
