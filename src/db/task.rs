use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::events::{self, TaskUpdate};
use crate::http::webhook;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TaskType {
    /// Build a full chapter list from all enabled providers
    BuildFullChapterList,
    /// Refresh metadata from the source (AniList, local, etc.)
    /// TODO: This also creates/updates ComicInfo.xml files
    RefreshMetadata,
    /// Sync chapters from a single provider (used for both initial build and periodic checks)
    SyncProviderChapters,
    /// Download a chapter
    DownloadChapter,
    /// Scan disk for existing chapter files
    ScanDisk,
    /// Optimise chapter images
    OptimiseChapter,
    /// Backup database
    Backup,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: Uuid,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub queue: String,
    pub library_id: Option<Uuid>,
    pub manga_id: Option<Uuid>,
    pub chapter_id: Option<Uuid>,
    pub priority: i64,
    pub payload: Option<String>,
    pub attempt: i64,
    pub max_attempts: i64,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub run_after: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskProgress {
    pub step: Option<String>,
    pub label: Option<String>,
    pub detail: Option<String>,
    pub provider: Option<String>,
    pub target: Option<String>,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub unit: Option<String>,
}

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct TaskRow {
    uuid: String,
    task_type: String,
    status: String,
    queue: String,
    library_id: Option<String>,
    manga_id: Option<String>,
    chapter_id: Option<String>,
    priority: i64,
    payload: Option<String>,
    attempt: i64,
    max_attempts: i64,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    run_after: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub fn task_type_str(t: &TaskType) -> &'static str {
    match t {
        TaskType::BuildFullChapterList => "BuildFullChapterList",
        TaskType::RefreshMetadata => "RefreshMetadata",
        TaskType::SyncProviderChapters => "SyncProviderChapters",
        TaskType::DownloadChapter => "DownloadChapter",
        TaskType::ScanDisk => "ScanDisk",
        TaskType::OptimiseChapter => "OptimiseChapter",
        TaskType::Backup => "Backup",
    }
}

fn task_status_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "Pending",
        TaskStatus::Running => "Running",
        TaskStatus::Completed => "Completed",
        TaskStatus::Failed => "Failed",
        TaskStatus::Cancelled => "Cancelled",
    }
}

fn parse_uuid_opt(s: Option<String>) -> Result<Option<Uuid>, sqlx::Error> {
    s.map(|v| Uuid::parse_str(&v).map_err(|e| sqlx::Error::Decode(Box::new(e))))
        .transpose()
}

fn task_from_row(row: TaskRow) -> Result<Task, sqlx::Error> {
    let id = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    let task_type = match row.task_type.as_str() {
        // Old names for backwards compatibility
        "ScanLibrary" => TaskType::BuildFullChapterList,
        "RefreshAniList" => TaskType::RefreshMetadata,
        // New names
        "BuildFullChapterList" => TaskType::BuildFullChapterList,
        "RefreshMetadata" => TaskType::RefreshMetadata,
        "CheckNewChapter" | "SyncProviderChapters" => TaskType::SyncProviderChapters,
        "DownloadChapter" => TaskType::DownloadChapter,
        "ScanDisk" => TaskType::ScanDisk,
        "OptimiseChapter" => TaskType::OptimiseChapter,
        "Backup" => TaskType::Backup,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown task_type: {other}").into(),
            ));
        }
    };

    let status = match row.status.as_str() {
        "Running" => TaskStatus::Running,
        "Completed" => TaskStatus::Completed,
        "Failed" => TaskStatus::Failed,
        "Cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    };

    Ok(Task {
        id,
        task_type,
        status,
        queue: row.queue,
        library_id: parse_uuid_opt(row.library_id)?,
        manga_id: parse_uuid_opt(row.manga_id)?,
        chapter_id: parse_uuid_opt(row.chapter_id)?,
        priority: row.priority,
        payload: row.payload,
        attempt: row.attempt,
        max_attempts: row.max_attempts,
        last_error: row.last_error,
        created_at: row.created_at,
        updated_at: row.updated_at,
        run_after: row.run_after,
    })
}

// ---------------------------------------------------------------------------
// Queue helpers
// ---------------------------------------------------------------------------

/// Look up task details needed for an SSE event (manga title, chapter number).
async fn task_event_details(
    pool: &SqlitePool,
    task_id: Uuid,
    task_type: &str,
    status: &str,
    last_error: Option<String>,
) -> TaskUpdate {
    let (manga_title, chapter_number_raw): (Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT m.title, c.chapter_base, c.chapter_variant
             FROM Task t
             LEFT JOIN Manga m ON t.manga_id = m.uuid
             LEFT JOIN Chapters c ON t.chapter_id = c.uuid
             WHERE t.uuid = ?",
        )
        .bind(task_id.to_string())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|(title, base, variant): (Option<String>, Option<i64>, Option<i64>)| {
            let chapter = base.map(|b| {
                let v = variant.unwrap_or(0);
                if v == 0 {
                    b.to_string()
                } else {
                    format!("{b}.{v}")
                }
            });
            (title, chapter)
        })
        .unwrap_or((None, None));

    TaskUpdate {
        id: task_id.to_string(),
        task_type: task_type.to_string(),
        status: status.to_string(),
        manga_title,
        chapter_number_raw,
        last_error,
    }
}

/// Determine which queue a task type belongs to.
/// System tasks go to 'system', provider-specific tasks go to the provider name.
pub fn task_queue(task_type: &TaskType) -> &'static str {
    match task_type {
        // System tasks - handled by system worker
        TaskType::BuildFullChapterList => "system",
        TaskType::RefreshMetadata => "system",
        TaskType::SyncProviderChapters => "system",
        TaskType::ScanDisk => "system",
        TaskType::OptimiseChapter => "system",
        TaskType::Backup => "system",
        // Download tasks - will be assigned to specific provider queues based on the chapter
        TaskType::DownloadChapter => "system", // Will be overridden when we know the provider
    }
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Insert a new Pending task. Returns the new task UUID.
pub async fn enqueue(
    pool: &SqlitePool,
    task_type: TaskType,
    manga_id: Option<Uuid>,
    chapter_id: Option<Uuid>,
    priority: i64,
) -> Result<Uuid, sqlx::Error> {
    enqueue_with_queue(pool, task_type, manga_id, chapter_id, priority, None).await
}

/// Insert a new Pending task with a specific queue.
pub async fn enqueue_with_queue(
    pool: &SqlitePool,
    task_type: TaskType,
    manga_id: Option<Uuid>,
    chapter_id: Option<Uuid>,
    priority: i64,
    queue: Option<String>,
) -> Result<Uuid, sqlx::Error> {
    enqueue_with_payload(pool, task_type, manga_id, chapter_id, priority, queue, None).await
}

/// Claim the next task from a specific queue.
pub async fn claim_next_for_queue(
    pool: &SqlitePool,
    queue: &str,
) -> Result<Option<Task>, sqlx::Error> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, TaskRow>(
        "SELECT uuid, task_type, status, queue, library_id, manga_id, chapter_id,
                priority, payload, attempt, max_attempts, last_error,
                created_at, updated_at, run_after
         FROM Task
         WHERE queue = ? AND status = 'Pending' AND run_after <= ?
         ORDER BY priority ASC, created_at ASC
         LIMIT 1",
    )
    .bind(queue)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };

    sqlx::query("UPDATE Task SET status = 'Running', updated_at = ? WHERE uuid = ?")
        .bind(now)
        .bind(&row.uuid)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let task = task_from_row(row)?;
    webhook::dispatch_task_event(task.id, task_type_str(&task.task_type), "Running");
    events::emit_task_update(&task_event_details(
        pool,
        task.id,
        task_type_str(&task.task_type),
        "Running",
        None,
    ).await);
    Ok(Some(task))
}

/// Atomically claim the next runnable Pending task (lowest priority value,
/// oldest created_at, run_after <= now). Returns None if nothing is ready.
pub async fn claim_next(pool: &SqlitePool) -> Result<Option<Task>, sqlx::Error> {
    let now = Utc::now();

    // SQLite doesn't support UPDATE ... RETURNING with sqlx easily in one shot,
    // so we use a transaction: SELECT then UPDATE.
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, TaskRow>(
        "SELECT uuid, task_type, status, queue, library_id, manga_id, chapter_id,
                priority, payload, attempt, max_attempts, last_error,
                created_at, updated_at, run_after
         FROM Task
         WHERE status = 'Pending' AND run_after <= ?
         ORDER BY priority ASC, created_at ASC
         LIMIT 1",
    )
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };

    sqlx::query("UPDATE Task SET status = 'Running', updated_at = ? WHERE uuid = ?")
        .bind(now)
        .bind(&row.uuid)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let task = task_from_row(row)?;
    webhook::dispatch_task_event(task.id, task_type_str(&task.task_type), "Running");
    events::emit_task_update(&task_event_details(
        pool,
        task.id,
        task_type_str(&task.task_type),
        "Running",
        None,
    ).await);
    Ok(Some(task))
}

/// Mark a task as Completed.
pub async fn complete(pool: &SqlitePool, task_id: Uuid) -> Result<(), sqlx::Error> {
    let task = get_by_id(pool, task_id).await?;
    sqlx::query("UPDATE Task SET status = 'Completed', updated_at = ? WHERE uuid = ?")
        .bind(Utc::now())
        .bind(task_id.to_string())
        .execute(pool)
        .await?;
    if let Some(task) = task {
        webhook::dispatch_task_event(task.id, task_type_str(&task.task_type), "Completed");
        events::emit_task_update(&task_event_details(
            pool,
            task.id,
            task_type_str(&task.task_type),
            "Completed",
            None,
        ).await);
    }
    Ok(())
}

/// Replace the task payload with a structured progress snapshot.
pub async fn set_progress(
    pool: &SqlitePool,
    task_id: Uuid,
    progress: &TaskProgress,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    let payload = serde_json::to_string(progress).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    sqlx::query("UPDATE Task SET payload = ?, updated_at = ? WHERE uuid = ?")
        .bind(payload)
        .bind(now)
        .bind(task_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark a task as Failed with an error message.
/// If `attempt < max_attempts`, re-queues as Pending with exponential backoff.
/// Otherwise leaves as Failed.
pub async fn fail(pool: &SqlitePool, task_id: Uuid, error: &str) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    let task = get_by_id(pool, task_id).await?;

    // Fetch current attempt / max_attempts
    let (attempt, max_attempts): (i64, i64) =
        sqlx::query_as("SELECT attempt, max_attempts FROM Task WHERE uuid = ?")
            .bind(task_id.to_string())
            .fetch_one(pool)
            .await?;

    let new_attempt = attempt + 1;

    if new_attempt < max_attempts {
        // Exponential backoff: 2^attempt minutes
        let backoff_minutes = 2i64.pow(attempt as u32);
        let run_after = now + Duration::minutes(backoff_minutes);
        sqlx::query(
            "UPDATE Task SET status = 'Pending', attempt = ?, last_error = ?,
                             run_after = ?, updated_at = ?
             WHERE uuid = ?",
        )
        .bind(new_attempt)
        .bind(error)
        .bind(run_after)
        .bind(now)
        .bind(task_id.to_string())
        .execute(pool)
        .await?;
        if let Some(task) = task {
            webhook::dispatch_task_event(
                task.id,
                task_type_str(&task.task_type),
                task_status_str(&TaskStatus::Pending),
            );
            events::emit_task_update(&task_event_details(
                pool,
                task.id,
                task_type_str(&task.task_type),
                "Pending",
                Some(error.to_string()),
            ).await);
        }
    } else {
        sqlx::query(
            "UPDATE Task SET status = 'Failed', attempt = ?, last_error = ?, updated_at = ?
             WHERE uuid = ?",
        )
        .bind(new_attempt)
        .bind(error)
        .bind(now)
        .bind(task_id.to_string())
        .execute(pool)
        .await?;
        if let Some(task) = task {
            webhook::dispatch_task_event(
                task.id,
                task_type_str(&task.task_type),
                task_status_str(&TaskStatus::Failed),
            );
            events::emit_task_update(&task_event_details(
                pool,
                task.id,
                task_type_str(&task.task_type),
                "Failed",
                Some(error.to_string()),
            ).await);
        }
    }
    Ok(())
}

/// On server startup, reset any tasks left stuck in `Running` state back to `Pending`
/// so they are retried. Returns the number of tasks reset.
pub async fn reset_running_tasks(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let now = Utc::now();
    let result = sqlx::query(
        "UPDATE Task SET status = 'Pending', run_after = ?, updated_at = ? WHERE status = 'Running'",
    )
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Cancel a Pending or Running task. Has no effect on Completed/Failed/Cancelled tasks.
pub async fn cancel(pool: &SqlitePool, task_id: Uuid) -> Result<(), sqlx::Error> {
    let task = get_by_id(pool, task_id).await?;
    sqlx::query(
        "UPDATE Task SET status = 'Cancelled', updated_at = ? WHERE uuid = ? AND status IN ('Pending', 'Running')",
    )
    .bind(Utc::now())
    .bind(task_id.to_string())
    .execute(pool)
    .await?;
    if let Some(task) = task {
        webhook::dispatch_task_event(task.id, task_type_str(&task.task_type), "Cancelled");
        events::emit_task_update(&task_event_details(
            pool,
            task.id,
            task_type_str(&task.task_type),
            "Cancelled",
            None,
        ).await);
    }
    Ok(())
}

pub async fn get_by_id(pool: &SqlitePool, task_id: Uuid) -> Result<Option<Task>, sqlx::Error> {
    let row = sqlx::query_as::<_, TaskRow>(
        "SELECT uuid, task_type, status, queue, library_id, manga_id, chapter_id,
                priority, payload, attempt, max_attempts, last_error,
                created_at, updated_at, run_after
         FROM Task
         WHERE uuid = ?",
    )
    .bind(task_id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(task_from_row).transpose()
}

/// Get UUIDs of all Running DownloadChapter tasks for a specific chapter (for cancellation signalling).
pub async fn get_running_for_chapter(
    pool: &SqlitePool,
    chapter_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT uuid FROM Task WHERE chapter_id = ? AND task_type = 'DownloadChapter' AND status = 'Running'",
    )
    .bind(chapter_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|s| Uuid::parse_str(&s).ok())
        .collect())
}

/// Cancel all Pending or Running DownloadChapter tasks for a specific chapter.
pub async fn cancel_by_chapter(pool: &SqlitePool, chapter_id: Uuid) -> Result<(), sqlx::Error> {
    let tasks: Vec<(String, String)> = sqlx::query_as(
        "SELECT uuid, task_type
         FROM Task
         WHERE chapter_id = ? AND task_type = 'DownloadChapter' AND status IN ('Pending', 'Running')",
    )
    .bind(chapter_id.to_string())
    .fetch_all(pool)
    .await?;

    sqlx::query(
        "UPDATE Task SET status = 'Cancelled', updated_at = ? WHERE chapter_id = ? AND task_type = 'DownloadChapter' AND status IN ('Pending', 'Running')",
    )
    .bind(Utc::now())
    .bind(chapter_id.to_string())
    .execute(pool)
    .await?;

    for (id, task_type) in &tasks {
        if let Ok(task_id) = Uuid::parse_str(id) {
            webhook::dispatch_task_event(task_id, task_type, "Cancelled");
            events::emit_task_update(&TaskUpdate {
                id: id.clone(),
                task_type: task_type.clone(),
                status: "Cancelled".to_string(),
                manga_title: None,
                chapter_number_raw: None,
                last_error: None,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recent tasks for the API / queue page
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RecentTask {
    pub id: String,
    pub task_type: String,
    pub status: String,
    pub manga_id: Option<String>,
    pub chapter_id: Option<String>,
    pub priority: i64,
    pub attempt: i64,
    pub max_attempts: i64,
    pub last_error: Option<String>,
    pub progress: Option<TaskProgress>,
    pub manga_title: Option<String>,
    pub chapter_number_raw: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RecentTaskRow {
    uuid: String,
    task_type: String,
    status: String,
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
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Check whether a Pending or Running task of the given type already exists for a manga.
pub async fn is_pending_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
    task_type: TaskType,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM Task WHERE manga_id = ? AND task_type = ? AND status IN ('Pending', 'Running')",
    )
    .bind(manga_id.to_string())
    .bind(task_type_str(&task_type))
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// Check whether a Pending or Running task of the given type already exists for a manga in a specific queue.
pub async fn is_pending_in_queue(
    pool: &SqlitePool,
    queue: &str,
    manga_id: Uuid,
    task_type: TaskType,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM Task WHERE queue = ? AND manga_id = ? AND task_type = ? AND status IN ('Pending', 'Running')",
    )
    .bind(queue)
    .bind(manga_id.to_string())
    .bind(task_type_str(&task_type))
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// Insert a new Pending task with optional queue and payload.
pub async fn enqueue_with_payload(
    pool: &SqlitePool,
    task_type: TaskType,
    manga_id: Option<Uuid>,
    chapter_id: Option<Uuid>,
    priority: i64,
    queue: Option<String>,
    payload: Option<String>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let queue = queue.unwrap_or_else(|| task_queue(&task_type).to_string());

    sqlx::query(
        "INSERT INTO Task
            (uuid, task_type, status, queue, manga_id, chapter_id, priority, payload,
             attempt, max_attempts, created_at, updated_at, run_after)
         VALUES (?, ?, 'Pending', ?, ?, ?, ?, ?, 0, 3, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(task_type_str(&task_type))
    .bind(queue)
    .bind(manga_id.map(|v| v.to_string()))
    .bind(chapter_id.map(|v| v.to_string()))
    .bind(priority)
    .bind(payload.as_deref())
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    webhook::dispatch_task_event(id, task_type_str(&task_type), "Pending");
    events::emit_task_update(&task_event_details(
        pool,
        id,
        task_type_str(&task_type),
        "Pending",
        None,
    ).await);
    Ok(id)
}

/// Fetch recent tasks ordered by created_at DESC. Optionally filter by manga_id.
/// Includes manga title via LEFT JOIN for display purposes.
/// Pass `limit <= 0` to return all tasks (no limit).
pub async fn get_recent(
    pool: &SqlitePool,
    manga_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<RecentTask>, sqlx::Error> {
    let effective_limit = if limit <= 0 { i64::MAX } else { limit };
    let manga_id_str = manga_id.map(|v| v.to_string());
    sqlx::query_as::<_, RecentTaskRow>(
        "SELECT t.uuid, t.task_type, t.status, t.manga_id, t.chapter_id,
                t.priority, t.attempt, t.max_attempts, t.last_error, t.payload,
                t.created_at, t.updated_at,
                m.title AS manga_title,
                c.chapter_base, c.chapter_variant
         FROM Task t
         LEFT JOIN Manga m ON t.manga_id = m.uuid
         LEFT JOIN Chapters c ON t.chapter_id = c.uuid
         WHERE (? IS NULL OR t.manga_id = ?)
         ORDER BY t.created_at DESC
         LIMIT ?",
    )
    .bind(&manga_id_str)
    .bind(&manga_id_str)
    .bind(effective_limit)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|r| {
                // Build a display string like "27" or "27.5" from base + variant
                let chapter_number_raw = r.chapter_base.map(|base| {
                    let variant = r.chapter_variant.unwrap_or(0);
                    if variant == 0 {
                        base.to_string()
                    } else {
                        format!("{base}.{variant}")
                    }
                });
                RecentTask {
                    id: r.uuid,
                    task_type: r.task_type,
                    status: r.status,
                    manga_id: r.manga_id,
                    chapter_id: r.chapter_id,
                    priority: r.priority,
                    attempt: r.attempt,
                    max_attempts: r.max_attempts,
                    last_error: r.last_error,
                    progress: r
                        .payload
                        .as_deref()
                        .and_then(|json| serde_json::from_str::<TaskProgress>(json).ok()),
                    manga_title: r.manga_title,
                    chapter_number_raw,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                }
            })
            .collect()
    })
}
