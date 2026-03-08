use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TaskType {
    ScanLibrary,
    RefreshAniList,
    CheckNewChapter,
    DownloadChapter,
    ScanDisk,
    OptimiseChapter,
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

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct TaskRow {
    uuid: String,
    task_type: String,
    status: String,
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

fn task_type_str(t: &TaskType) -> &'static str {
    match t {
        TaskType::ScanLibrary => "ScanLibrary",
        TaskType::RefreshAniList => "RefreshAniList",
        TaskType::CheckNewChapter => "CheckNewChapter",
        TaskType::DownloadChapter => "DownloadChapter",
        TaskType::ScanDisk => "ScanDisk",
        TaskType::OptimiseChapter => "OptimiseChapter",
        TaskType::Backup => "Backup",
    }
}

fn parse_uuid_opt(s: Option<String>) -> Result<Option<Uuid>, sqlx::Error> {
    s.map(|v| Uuid::parse_str(&v).map_err(|e| sqlx::Error::Decode(Box::new(e))))
        .transpose()
}

fn task_from_row(row: TaskRow) -> Result<Task, sqlx::Error> {
    let id = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    let task_type = match row.task_type.as_str() {
        "ScanLibrary" => TaskType::ScanLibrary,
        "RefreshAniList" => TaskType::RefreshAniList,
        "CheckNewChapter" => TaskType::CheckNewChapter,
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
    let id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO Task
            (uuid, task_type, status, manga_id, chapter_id, priority,
             attempt, max_attempts, created_at, updated_at, run_after)
         VALUES (?, ?, 'Pending', ?, ?, ?, 0, 3, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(task_type_str(&task_type))
    .bind(manga_id.map(|v| v.to_string()))
    .bind(chapter_id.map(|v| v.to_string()))
    .bind(priority)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Atomically claim the next runnable Pending task (lowest priority value,
/// oldest created_at, run_after <= now). Returns None if nothing is ready.
pub async fn claim_next(pool: &SqlitePool) -> Result<Option<Task>, sqlx::Error> {
    let now = Utc::now();

    // SQLite doesn't support UPDATE ... RETURNING with sqlx easily in one shot,
    // so we use a transaction: SELECT then UPDATE.
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, TaskRow>(
        "SELECT uuid, task_type, status, library_id, manga_id, chapter_id,
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

    task_from_row(row).map(Some)
}

/// Mark a task as Completed.
pub async fn complete(pool: &SqlitePool, task_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Task SET status = 'Completed', updated_at = ? WHERE uuid = ?")
        .bind(Utc::now())
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
    sqlx::query(
        "UPDATE Task SET status = 'Cancelled', updated_at = ? WHERE uuid = ? AND status IN ('Pending', 'Running')",
    )
    .bind(Utc::now())
    .bind(task_id.to_string())
    .execute(pool)
    .await?;
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
    pub manga_title: Option<String>,
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
    manga_title: Option<String>,
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
                t.priority, t.attempt, t.max_attempts, t.last_error,
                t.created_at, t.updated_at,
                m.title AS manga_title
         FROM Task t
         LEFT JOIN Manga m ON t.manga_id = m.uuid
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
            .map(|r| RecentTask {
                id: r.uuid,
                task_type: r.task_type,
                status: r.status,
                manga_id: r.manga_id,
                chapter_id: r.chapter_id,
                priority: r.priority,
                attempt: r.attempt,
                max_attempts: r.max_attempts,
                last_error: r.last_error,
                manga_title: r.manga_title,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect()
    })
}
