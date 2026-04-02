use rocket::{State, get, http::Status, post, serde::json::Json};
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{db, db::task::RecentTask, scraper::ProviderRegistry, scheduler::worker::CancelMap};
use crate::manga::core::DownloadStatus;

use super::errors::{ApiError, ApiResult, bad_request, internal};

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
    let effective_limit = limit.unwrap_or(0);
    db::task::get_recent(pool.inner(), mid, effective_limit)
        .await
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// GET /api/tasks/grouped
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct QueuedTask {
    pub id: String,
    pub task_type: String,
    pub status: String,
    pub manga_id: Option<String>,
    pub chapter_id: Option<String>,
    pub priority: i64,
    pub attempt: i64,
    pub max_attempts: i64,
    pub last_error: Option<String>,
    pub progress: Option<db::task::TaskProgress>,
    pub manga_title: Option<String>,
    pub chapter_number_raw: Option<String>,
    pub queue: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueInfo {
    /// Display name: provider name for provider queues, "System" for system
    pub display_name: String,
    pub is_provider: bool,
    pub provider_name: Option<String>,
    pub tasks: Vec<QueuedTask>,
    pub running_count: usize,
    pub pending_count: usize,
    pub total_count: usize,
    /// Number of workers for this queue (from provider max_concurrency)
    pub worker_count: usize,
}

#[derive(sqlx::FromRow)]
struct QueuedTaskRow {
    uuid: String,
    task_type: String,
    status: String,
    queue: String,
    manga_id: Option<String>,
    chapter_id: Option<String>,
    priority: i64,
    attempt: i64,
    max_attempts: i64,
    last_error: Option<String>,
    payload: Option<String>,
    manga_title: Option<String>,
    chapter_base: Option<i64>,
    chapter_variant: Option<i64>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[get("/api/tasks/grouped")]
pub async fn list_tasks_grouped(
    pool: &State<SqlitePool>,
    registry: &State<Arc<ProviderRegistry>>,
) -> ApiResult<Vec<QueueInfo>> {
    let rows: Vec<QueuedTaskRow> = sqlx::query_as(
        "SELECT t.uuid, t.task_type, t.status, t.queue, t.manga_id, t.chapter_id,
                t.priority, t.attempt, t.max_attempts, t.last_error, t.payload,
                t.created_at, t.updated_at,
                m.title AS manga_title,
                c.chapter_base, c.chapter_variant
         FROM Task t
         LEFT JOIN Manga m ON t.manga_id = m.uuid
         LEFT JOIN Chapters c ON t.chapter_id = c.uuid
         WHERE t.status IN ('Pending', 'Running')
         ORDER BY t.queue, t.priority ASC, t.created_at ASC
         LIMIT 500",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(internal)?;

    // Group by queue
    let mut queues: std::collections::HashMap<String, Vec<QueuedTask>> = std::collections::HashMap::new();
    for row in rows {
        let chapter_number_raw = row.chapter_base.map(|base| {
            let variant = row.chapter_variant.unwrap_or(0);
            if variant == 0 {
                base.to_string()
            } else {
                format!("{base}.{variant}")
            }
        });
        let progress = row.payload.as_deref()
            .and_then(|json| serde_json::from_str::<db::task::TaskProgress>(json).ok());

        let task = QueuedTask {
            id: row.uuid.clone(),
            task_type: row.task_type,
            status: row.status,
            manga_id: row.manga_id,
            chapter_id: row.chapter_id,
            priority: row.priority,
            attempt: row.attempt,
            max_attempts: row.max_attempts,
            last_error: row.last_error,
            progress,
            manga_title: row.manga_title,
            chapter_number_raw,
            queue: row.queue.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        };
        queues.entry(row.queue).or_default().push(task);
    }

    // Build queue info list - always include system + all providers
    let mut result: Vec<QueueInfo> = Vec::new();

    // System queue
    let sys_tasks = queues.remove("system").unwrap_or_default();
    result.push(QueueInfo {
        display_name: "System".to_owned(),
        is_provider: false,
        provider_name: None,
        tasks: sys_tasks.clone(),
        running_count: sys_tasks.iter().filter(|t| t.status == "Running").count(),
        pending_count: sys_tasks.iter().filter(|t| t.status == "Pending").count(),
        total_count: sys_tasks.len(),
        worker_count: 2,
    });

    // Provider queues (always show all providers)
    for provider in registry.as_ref().all() {
        let pname = provider.name();
        let qname = format!("provider:{pname}");
        let tasks = queues.remove(&qname).unwrap_or_default();
        let running = tasks.iter().filter(|t| t.status == "Running").count();
        let pending = tasks.iter().filter(|t| t.status == "Pending").count();

        // Sort: running first (oldest first), then pending (oldest first)
        let mut tasks = tasks;
        tasks.sort_by(|a, b| {
            let ar = a.status == "Running"; let br = b.status == "Running";
            if ar && !br { std::cmp::Ordering::Less }
            else if !ar && br { std::cmp::Ordering::Greater }
            else { a.created_at.cmp(&b.created_at) }
        });

        result.push(QueueInfo {
            display_name: pname.to_owned(),
            is_provider: true,
            provider_name: Some(pname.to_owned()),
            tasks,
            running_count: running,
            pending_count: pending,
            total_count: running + pending,
            worker_count: provider.max_concurrency() as usize,
        });
    }

    Ok(Json(result))
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

    let task = db::task::get_by_id(pool.inner(), uuid)
        .await
        .map_err(internal)?;

    db::task::cancel(pool.inner(), uuid)
        .await
        .map_err(internal)?;

    if let Some(task) = task {
        if task.task_type == db::task::TaskType::DownloadChapter {
            if let Some(chapter_id) = task.chapter_id {
                let _ = db::chapter::set_status(
                    pool.inner(),
                    chapter_id,
                    DownloadStatus::Missing,
                    None,
                )
                .await;
            }
        }
    }

    if let Some(token) = cancel_map.lock().unwrap().get(&uuid) {
        token.cancel();
    }
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes aggregation
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![list_tasks, list_tasks_grouped, cancel_task]
}