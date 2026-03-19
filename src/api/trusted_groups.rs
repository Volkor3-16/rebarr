use rocket::{State, delete, get, post, http::Status, serde::json::Json};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db;

use super::errors::{bad_request, internal, ApiError, ApiResult};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddTrustedGroupRequest {
    pub name: String,
}

// ---------------------------------------------------------------------------
// GET /api/trusted-groups
// ---------------------------------------------------------------------------

#[get("/api/trusted-groups")]
pub async fn list_trusted_groups(pool: &State<SqlitePool>) -> ApiResult<Vec<String>> {
    let groups = db::provider::get_trusted_groups(pool.inner())
        .await
        .map_err(internal)?;
    Ok(Json(groups))
}

// ---------------------------------------------------------------------------
// POST /api/trusted-groups
// ---------------------------------------------------------------------------

#[post("/api/trusted-groups", data = "<body>")]
pub async fn add_trusted_group(
    pool: &State<SqlitePool>,
    body: Json<AddTrustedGroupRequest>,
) -> Result<Status, (Status, Json<ApiError>)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(bad_request("name must not be empty"));
    }
    db::provider::add_trusted_group(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Status::Created)
}

// ---------------------------------------------------------------------------
// DELETE /api/trusted-groups/<name>
// ---------------------------------------------------------------------------

#[delete("/api/trusted-groups/<name>")]
pub async fn remove_trusted_group(
    pool: &State<SqlitePool>,
    name: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    db::provider::remove_trusted_group(pool.inner(), name)
        .await
        .map_err(internal)?;
    Ok(Status::Ok)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_trusted_groups,
        add_trusted_group,
        remove_trusted_group,
    ]
}
