use rocket::{State, get, post, http::Status, serde::json::Json};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{db, db::task::RecentTask, scheduler::worker::CancelMap};

use super::errors::{bad_request, internal, ApiError, ApiResult};

// ---------------------------------------------------------------------------
// GET /api/tasks
// ---------------------------------------------------------------------------

#[get("/api/tasks?<manga_id>&<limit>")]
pub async fn list_tasks(
    pool: &State<SqlitePool>,
    manga_id: Option<&str>,
    limit: Option<i64>,
) -> ApiResult<Vec<RecentTask>> {
    let mid = manga_id.and_then(|s| Uuid::parse_str(s).ok());
    // limit=0 (or omitted when no manga_id filter) means "all tasks"
    let effective_limit = limit.unwrap_or(0);
    db::task::get_recent(pool.inner(), mid, effective_limit)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// POST /api/tasks/<id>/cancel
// ---------------------------------------------------------------------------

#[post("/api/tasks/<id>/cancel")]
pub async fn cancel_task(
    pool: &State<SqlitePool>,
    cancel_map: &State<CancelMap>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let uuid = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    db::task::cancel(pool.inner(), uuid)
        .await
        .map_err(internal)?;
    // Signal the running task to stop
    if let Some(token) = cancel_map.lock().unwrap().get(&uuid) {
        token.cancel();
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_tasks,
        cancel_task,
    ]
}
