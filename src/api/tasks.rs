use chrono::{DateTime, Utc};
use rocket::{FromForm, State, get, http::Status, post, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::manga::core::DownloadStatus;
use crate::{db, db::task::RecentTask, scheduler::worker::CancelMap, scraper::ProviderRegistry};

use super::errors::{ApiError, ApiResult, bad_request, internal};

#[derive(Debug, FromForm, JsonSchema)]
pub struct TaskListQuery {
    manga_id: Option<String>,
    limit: Option<i64>,
    before: Option<String>,
    status: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct QueueTasksResponse {
    pub tasks: Vec<RecentTask>,
    pub terminal_limit: i64,
    pub has_more_history: bool,
    pub next_before: Option<DateTime<Utc>>,
}

fn parse_manga_id(raw: Option<&str>) -> Result<Option<Uuid>, (Status, Json<ApiError>)> {
    raw.map(|id| Uuid::parse_str(id).map_err(|_| bad_request("invalid manga_id")))
        .transpose()
}

fn parse_before(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, (Status, Json<ApiError>)> {
    raw.map(|value| {
        DateTime::parse_from_rfc3339(value)
            .map(|ts| ts.with_timezone(&Utc))
            .map_err(|_| bad_request("invalid before timestamp"))
    })
    .transpose()
}

fn parse_status_filters(values: &[String]) -> Result<Vec<String>, (Status, Json<ApiError>)> {
    let mut result = Vec::new();
    for value in values {
        for raw in value.split(',') {
            let status = raw.trim();
            if status.is_empty() {
                continue;
            }
            match status {
                "Pending" | "Running" | "Completed" | "Failed" | "Cancelled" => {
                    result.push(status.to_string());
                }
                _ => return Err(bad_request("invalid status filter")),
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// GET /api/tasks
// ---------------------------------------------------------------------------

/// List recent tasks with optional filtering by manga.
#[openapi(tag = "Tasks")]
#[get("/api/tasks?<query..>")]
pub async fn list_tasks(
    pool: &State<SqlitePool>,
    query: TaskListQuery,
) -> ApiResult<Vec<RecentTask>> {
    let recent_query = db::task::RecentTaskQuery {
        manga_id: parse_manga_id(query.manga_id.as_deref())?,
        limit: query.limit.filter(|limit| *limit > 0),
        before: parse_before(query.before.as_deref())?,
        statuses: parse_status_filters(&query.status)?,
    };

    db::task::list_recent(pool.inner(), &recent_query)
        .await
        .map(Json)
        .map_err(internal)
}

/// List tasks for the queue page: all active tasks plus a bounded recent history.
#[openapi(tag = "Tasks")]
#[get("/api/tasks/queue?<manga_id>&<terminal_limit>")]
pub async fn list_queue_tasks(
    pool: &State<SqlitePool>,
    manga_id: Option<&str>,
    terminal_limit: Option<i64>,
) -> ApiResult<QueueTasksResponse> {
    let snapshot = db::task::get_queue_snapshot(
        pool.inner(),
        parse_manga_id(manga_id)?,
        terminal_limit.unwrap_or(db::task::DEFAULT_QUEUE_TERMINAL_LIMIT),
    )
    .await
    .map_err(internal)?;

    Ok(Json(QueueTasksResponse {
        tasks: snapshot.tasks,
        terminal_limit: snapshot.terminal_limit,
        has_more_history: snapshot.has_more_history,
        next_before: snapshot.next_before,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/tasks/grouped
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, JsonSchema)]
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

/// List tasks grouped by queue with status information.
#[openapi(tag = "Tasks")]
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
    let mut queues: std::collections::HashMap<String, Vec<QueuedTask>> =
        std::collections::HashMap::new();
    for row in rows {
        let chapter_number_raw = row.chapter_base.map(|base| {
            let variant = row.chapter_variant.unwrap_or(0);
            if variant == 0 {
                base.to_string()
            } else {
                format!("{base}.{variant}")
            }
        });
        let progress = row
            .payload
            .as_deref()
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
            let ar = a.status == "Running";
            let br = b.status == "Running";
            if ar && !br {
                std::cmp::Ordering::Less
            } else if !ar && br {
                std::cmp::Ordering::Greater
            } else {
                a.created_at.cmp(&b.created_at)
            }
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

/// Cancel a running or pending task.
#[openapi(tag = "Tasks")]
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
    rocket::routes![
        list_tasks,
        list_queue_tasks,
        list_tasks_grouped,
        cancel_task
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::local::asynchronous::Client;
    use serde_json::Value;

    async fn test_client() -> Client {
        let pool = crate::db::init("sqlite::memory:").await.expect("init db");
        let rocket = rocket::build()
            .manage(pool)
            .mount("/", rocket::routes![list_tasks, list_queue_tasks]);
        Client::tracked(rocket).await.expect("client")
    }

    async fn insert_task(
        pool: &SqlitePool,
        task_type: db::task::TaskType,
        status: &str,
        created_at: DateTime<Utc>,
    ) -> String {
        let id = db::task::enqueue(pool, task_type, None, None, 0)
            .await
            .expect("enqueue task");
        let payload = serde_json::to_string(&db::task::TaskProgress {
            current: Some(1),
            total: Some(10),
            unit: Some("page".to_string()),
            ..Default::default()
        })
        .expect("progress");
        sqlx::query(
            "UPDATE Task
             SET status = ?, payload = ?, created_at = ?, updated_at = ?
             WHERE uuid = ?",
        )
        .bind(status)
        .bind(payload)
        .bind(created_at)
        .bind(created_at)
        .bind(id.to_string())
        .execute(pool)
        .await
        .expect("update task");
        id.to_string()
    }

    #[rocket::async_test]
    async fn queue_endpoint_returns_active_tasks_plus_bounded_terminal_history() {
        let client = test_client().await;
        let pool = client.rocket().state::<SqlitePool>().expect("pool");
        let now = Utc::now();

        insert_task(pool, db::task::TaskType::DownloadChapter, "Pending", now).await;
        insert_task(
            pool,
            db::task::TaskType::RefreshMetadata,
            "Running",
            now - chrono::Duration::seconds(1),
        )
        .await;
        insert_task(
            pool,
            db::task::TaskType::Backup,
            "Completed",
            now - chrono::Duration::seconds(2),
        )
        .await;
        insert_task(
            pool,
            db::task::TaskType::ScanDisk,
            "Failed",
            now - chrono::Duration::seconds(3),
        )
        .await;
        insert_task(
            pool,
            db::task::TaskType::OptimiseChapter,
            "Cancelled",
            now - chrono::Duration::seconds(4),
        )
        .await;

        let response = client
            .get("/api/tasks/queue?terminal_limit=2")
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);

        let json: Value = response.into_json().await.expect("json body");
        let tasks = json["tasks"].as_array().expect("tasks array");
        assert_eq!(tasks.len(), 4);
        assert_eq!(json["terminal_limit"].as_i64(), Some(2));
        assert_eq!(json["has_more_history"].as_bool(), Some(true));

        let statuses: Vec<&str> = tasks
            .iter()
            .map(|task| task["status"].as_str().expect("status"))
            .collect();
        assert_eq!(statuses, vec!["Pending", "Running", "Completed", "Failed"]);
        assert!(tasks.iter().all(|task| {
            task.get("progress")
                .and_then(|value| value.as_object())
                .is_some()
        }));
    }

    #[rocket::async_test]
    async fn list_tasks_supports_status_filter_and_before_cursor() {
        let client = test_client().await;
        let pool = client.rocket().state::<SqlitePool>().expect("pool");
        let now = Utc::now();

        insert_task(
            pool,
            db::task::TaskType::Backup,
            "Completed",
            now - chrono::Duration::seconds(1),
        )
        .await;
        insert_task(
            pool,
            db::task::TaskType::ScanDisk,
            "Failed",
            now - chrono::Duration::seconds(2),
        )
        .await;
        insert_task(
            pool,
            db::task::TaskType::RefreshMetadata,
            "Pending",
            now - chrono::Duration::seconds(3),
        )
        .await;

        let before =
            urlencoding::encode(&(now - chrono::Duration::milliseconds(1500)).to_rfc3339())
                .into_owned();
        let response = client
            .get(format!(
                "/api/tasks?status=Completed,Failed&before={before}&limit=5"
            ))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);

        let json: Value = response.into_json().await.expect("json body");
        let tasks = json.as_array().expect("array");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["status"].as_str(), Some("Failed"));
    }

    #[rocket::async_test]
    async fn list_tasks_limit_zero_keeps_explicit_all_history_behavior() {
        let client = test_client().await;
        let pool = client.rocket().state::<SqlitePool>().expect("pool");
        let now = Utc::now();

        insert_task(pool, db::task::TaskType::Backup, "Completed", now).await;
        insert_task(
            pool,
            db::task::TaskType::ScanDisk,
            "Failed",
            now - chrono::Duration::seconds(1),
        )
        .await;

        let response = client.get("/api/tasks?limit=0").dispatch().await;
        assert_eq!(response.status(), Status::Ok);

        let json: Value = response.into_json().await.expect("json body");
        assert_eq!(json.as_array().expect("array").len(), 2);
    }
}
