use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::{QueryBuilder, Sqlite};
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
    /// Refresh cached library suggestions from AniList
    RefreshSuggestions,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
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

pub const DEFAULT_QUEUE_TERMINAL_LIMIT: i64 = 200;
// Provider syncs must outrank provider downloads so rebarr can discover
// available chapters before draining background download backlog.
pub const PRIORITY_PROVIDER_SYNC: i64 = 1;
pub const PRIORITY_DOWNLOAD_CHAPTER: i64 = 10;
pub const PRIORITY_QUEUE_FRONT_SEED: i64 = 0;

#[derive(Debug, Clone, Default)]
pub struct RecentTaskQuery {
    pub manga_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub before: Option<DateTime<Utc>>,
    pub statuses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct QueueTaskSnapshot {
    pub tasks: Vec<RecentTask>,
    pub terminal_limit: i64,
    pub has_more_history: bool,
    pub next_before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub enabled: bool,
    pub days: u64,
    pub min_keep: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRun {
    pub deleted_count: u64,
    pub cutoff: DateTime<Utc>,
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
        TaskType::RefreshSuggestions => "RefreshSuggestions",
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
        "RefreshSuggestions" => TaskType::RefreshSuggestions,
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

fn progress_from_payload(payload: Option<&str>) -> Option<TaskProgress> {
    payload.and_then(|json| serde_json::from_str::<TaskProgress>(json).ok())
}

fn chapter_number_raw(base: Option<i64>, variant: Option<i64>) -> Option<String> {
    base.map(|base| {
        let variant = variant.unwrap_or(0);
        if variant == 0 {
            base.to_string()
        } else {
            format!("{base}.{variant}")
        }
    })
}

fn recent_task_from_row(row: RecentTaskRow) -> RecentTask {
    RecentTask {
        id: row.uuid,
        task_type: row.task_type,
        status: row.status,
        manga_id: row.manga_id,
        chapter_id: row.chapter_id,
        priority: row.priority,
        attempt: row.attempt,
        max_attempts: row.max_attempts,
        last_error: row.last_error,
        progress: progress_from_payload(row.payload.as_deref()),
        manga_title: row.manga_title,
        chapter_number_raw: chapter_number_raw(row.chapter_base, row.chapter_variant),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn apply_recent_task_filters<'a>(
    mut qb: QueryBuilder<'a, Sqlite>,
    query: &'a RecentTaskQuery,
) -> QueryBuilder<'a, Sqlite> {
    let mut has_where = false;
    if let Some(manga_id) = query.manga_id {
        qb.push(" WHERE t.manga_id = ")
            .push_bind(manga_id.to_string());
        has_where = true;
    }
    if let Some(before) = query.before {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("t.created_at < ").push_bind(before);
        has_where = true;
    }
    if !query.statuses.is_empty() {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("t.status IN (");
        {
            let mut separated = qb.separated(", ");
            for status in &query.statuses {
                separated.push_bind(status);
            }
        }
        qb.push(")");
    }
    qb
}

async fn fetch_recent_task_rows(
    pool: &SqlitePool,
    query: &RecentTaskQuery,
) -> Result<Vec<RecentTaskRow>, sqlx::Error> {
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT t.uuid, t.task_type, t.status, t.manga_id, t.chapter_id,
                t.priority, t.attempt, t.max_attempts, t.last_error, t.payload,
                t.created_at, t.updated_at,
                m.title AS manga_title,
                c.chapter_base, c.chapter_variant
         FROM Task t
         LEFT JOIN Manga m ON t.manga_id = m.uuid
         LEFT JOIN Chapters c ON t.chapter_id = c.uuid",
    );
    qb = apply_recent_task_filters(qb, query);
    qb.push(" ORDER BY t.created_at DESC");
    if let Some(limit) = query.limit.filter(|limit| *limit > 0) {
        qb.push(" LIMIT ").push_bind(limit);
    }
    qb.build_query_as::<RecentTaskRow>().fetch_all(pool).await
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
    let details: Option<(
        Option<String>,
        Option<String>,
        Option<TaskProgress>,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT t.manga_id, t.chapter_id, t.payload, t.updated_at,
                m.title, c.chapter_base, c.chapter_variant
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
    .map(
        |(manga_id, chapter_id, payload, updated_at, title, base, variant): (
            Option<String>,
            Option<String>,
            Option<String>,
            DateTime<Utc>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        )| {
            (
                manga_id,
                chapter_id,
                progress_from_payload(payload.as_deref()),
                updated_at,
                title,
                chapter_number_raw(base, variant),
            )
        },
    );

    TaskUpdate {
        id: task_id.to_string(),
        task_type: task_type.to_string(),
        status: status.to_string(),
        manga_id: details
            .as_ref()
            .and_then(|(manga_id, _, _, _, _, _)| manga_id.clone()),
        chapter_id: details
            .as_ref()
            .and_then(|(_, chapter_id, _, _, _, _)| chapter_id.clone()),
        manga_title: details
            .as_ref()
            .and_then(|(_, _, _, _, title, _)| title.clone()),
        chapter_number_raw: details
            .as_ref()
            .and_then(|(_, _, _, _, _, chapter)| chapter.clone()),
        last_error,
        progress: details
            .as_ref()
            .and_then(|(_, _, progress, _, _, _)| progress.clone()),
        updated_at: details.map(|(_, _, _, updated_at, _, _)| updated_at),
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
        TaskType::RefreshSuggestions => "system",
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

/// Insert a new Pending task scoped to a library.
pub async fn enqueue_for_library(
    pool: &SqlitePool,
    task_type: TaskType,
    library_id: Uuid,
    priority: i64,
) -> Result<Uuid, sqlx::Error> {
    enqueue_library_with_payload(pool, task_type, library_id, priority, None, None).await
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
    events::emit_task_update(
        &task_event_details(
            pool,
            task.id,
            task_type_str(&task.task_type),
            "Running",
            None,
        )
        .await,
    );
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
    events::emit_task_update(
        &task_event_details(
            pool,
            task.id,
            task_type_str(&task.task_type),
            "Running",
            None,
        )
        .await,
    );
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
        events::emit_task_update(
            &task_event_details(
                pool,
                task.id,
                task_type_str(&task.task_type),
                "Completed",
                None,
            )
            .await,
        );
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
            events::emit_task_update(
                &task_event_details(
                    pool,
                    task.id,
                    task_type_str(&task.task_type),
                    "Pending",
                    Some(error.to_string()),
                )
                .await,
            );
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
            events::emit_task_update(
                &task_event_details(
                    pool,
                    task.id,
                    task_type_str(&task.task_type),
                    "Failed",
                    Some(error.to_string()),
                )
                .await,
            );
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
        events::emit_task_update(
            &task_event_details(
                pool,
                task.id,
                task_type_str(&task.task_type),
                "Cancelled",
                None,
            )
            .await,
        );
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
                manga_id: None,
                chapter_id: None,
                manga_title: None,
                chapter_number_raw: None,
                last_error: None,
                progress: None,
                updated_at: None,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recent tasks for the API / queue page
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, JsonSchema)]
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

pub async fn is_pending_for_library(
    pool: &SqlitePool,
    library_id: Uuid,
    task_type: TaskType,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM Task
         WHERE library_id = ? AND task_type = ? AND status IN ('Pending', 'Running')",
    )
    .bind(library_id.to_string())
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
    enqueue_full(
        pool, task_type, None, manga_id, chapter_id, priority, queue, payload,
    )
    .await
}

/// Insert a new Pending task at the front of a specific queue.
pub async fn enqueue_at_front(
    pool: &SqlitePool,
    task_type: TaskType,
    manga_id: Option<Uuid>,
    chapter_id: Option<Uuid>,
    queue: String,
    payload: Option<String>,
) -> Result<Uuid, sqlx::Error> {
    let priority = next_front_priority(pool, &queue).await?;
    enqueue_full(
        pool,
        task_type,
        None,
        manga_id,
        chapter_id,
        priority,
        Some(queue),
        payload,
    )
    .await
}

/// Insert a new Pending library task with optional queue and payload.
pub async fn enqueue_library_with_payload(
    pool: &SqlitePool,
    task_type: TaskType,
    library_id: Uuid,
    priority: i64,
    queue: Option<String>,
    payload: Option<String>,
) -> Result<Uuid, sqlx::Error> {
    enqueue_full(
        pool,
        task_type,
        Some(library_id),
        None,
        None,
        priority,
        queue,
        payload,
    )
    .await
}

async fn enqueue_full(
    pool: &SqlitePool,
    task_type: TaskType,
    library_id: Option<Uuid>,
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
            (uuid, task_type, status, queue, library_id, manga_id, chapter_id, priority, payload,
             attempt, max_attempts, created_at, updated_at, run_after)
         VALUES (?, ?, 'Pending', ?, ?, ?, ?, ?, ?, 0, 3, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(task_type_str(&task_type))
    .bind(queue)
    .bind(library_id.map(|v| v.to_string()))
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
    events::emit_task_update(
        &task_event_details(pool, id, task_type_str(&task_type), "Pending", None).await,
    );
    Ok(id)
}

async fn next_front_priority(pool: &SqlitePool, queue: &str) -> Result<i64, sqlx::Error> {
    let min_priority: Option<i64> =
        sqlx::query_scalar("SELECT MIN(priority) FROM Task WHERE queue = ? AND status = 'Pending'")
            .bind(queue)
            .fetch_one(pool)
            .await?;

    Ok(min_priority
        .unwrap_or(PRIORITY_QUEUE_FRONT_SEED)
        .saturating_sub(1))
}

/// Promote a Pending task so it will be claimed before all other Pending tasks in its queue.
/// Returns false if the task does not exist or is not currently Pending.
pub async fn prioritise_task(pool: &SqlitePool, id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT queue, task_type FROM Task WHERE uuid = ? AND status = 'Pending'")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;

    let Some((queue, task_type)) = row else {
        tx.commit().await?;
        return Ok(false);
    };

    let min_priority: Option<i64> =
        sqlx::query_scalar("SELECT MIN(priority) FROM Task WHERE queue = ? AND status = 'Pending'")
            .bind(&queue)
            .fetch_one(&mut *tx)
            .await?;

    let now = Utc::now();
    let next_priority = min_priority
        .unwrap_or(PRIORITY_QUEUE_FRONT_SEED)
        .saturating_sub(1);

    sqlx::query("UPDATE Task SET priority = ?, updated_at = ? WHERE uuid = ?")
        .bind(next_priority)
        .bind(now)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    events::emit_task_update(&task_event_details(pool, id, &task_type, "Pending", None).await);
    Ok(true)
}

/// Promote an existing Pending chapter download to the front of its queue.
/// Returns false if no Pending task exists for the chapter.
pub async fn prioritise_pending_download_for_chapter(
    pool: &SqlitePool,
    chapter_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let task_id: Option<String> = sqlx::query_scalar(
        "SELECT uuid
         FROM Task
         WHERE chapter_id = ? AND task_type = 'DownloadChapter' AND status = 'Pending'
         ORDER BY priority ASC, created_at ASC
         LIMIT 1",
    )
    .bind(chapter_id.to_string())
    .fetch_optional(pool)
    .await?;

    let Some(task_id) = task_id else {
        return Ok(false);
    };

    let task_id = Uuid::parse_str(&task_id)
        .map_err(|e| sqlx::Error::Protocol(format!("invalid task uuid: {e}")))?;
    prioritise_task(pool, task_id).await
}

/// Fetch recent tasks ordered by created_at DESC. Optionally filter by manga_id.
/// Includes manga title via LEFT JOIN for display purposes.
/// Pass `limit <= 0` to return all tasks (no limit).
pub async fn get_recent(
    pool: &SqlitePool,
    manga_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<RecentTask>, sqlx::Error> {
    let query = RecentTaskQuery {
        manga_id,
        limit: (limit > 0).then_some(limit),
        before: None,
        statuses: Vec::new(),
    };
    list_recent(pool, &query).await
}

pub async fn list_recent(
    pool: &SqlitePool,
    query: &RecentTaskQuery,
) -> Result<Vec<RecentTask>, sqlx::Error> {
    fetch_recent_task_rows(pool, query)
        .await
        .map(|rows| rows.into_iter().map(recent_task_from_row).collect())
}

pub async fn get_queue_snapshot(
    pool: &SqlitePool,
    manga_id: Option<Uuid>,
    terminal_limit: i64,
) -> Result<QueueTaskSnapshot, sqlx::Error> {
    let terminal_limit = terminal_limit.max(1);
    let active_query = RecentTaskQuery {
        manga_id,
        limit: None,
        before: None,
        statuses: vec!["Pending".to_string(), "Running".to_string()],
    };
    let terminal_query = RecentTaskQuery {
        manga_id,
        limit: Some(terminal_limit + 1),
        before: None,
        statuses: vec![
            "Completed".to_string(),
            "Failed".to_string(),
            "Cancelled".to_string(),
        ],
    };

    let active_rows = fetch_recent_task_rows(pool, &active_query).await?;
    let mut terminal_rows = fetch_recent_task_rows(pool, &terminal_query).await?;
    let has_more_history = terminal_rows.len() as i64 > terminal_limit;
    if has_more_history {
        terminal_rows.truncate(terminal_limit as usize);
    }
    let next_before = terminal_rows.last().map(|task| task.created_at);

    let mut tasks: Vec<RecentTask> = active_rows
        .into_iter()
        .chain(terminal_rows)
        .map(recent_task_from_row)
        .collect();
    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(QueueTaskSnapshot {
        tasks,
        terminal_limit,
        has_more_history,
        next_before,
    })
}

pub async fn prune_terminal_history(
    pool: &SqlitePool,
    days: u64,
    min_keep: u64,
) -> Result<RetentionRun, sqlx::Error> {
    let cutoff = Utc::now() - Duration::days(days.min(i64::MAX as u64) as i64);
    let min_keep = min_keep.min(i64::MAX as u64) as i64;
    let result = sqlx::query(
        "DELETE FROM Task
         WHERE status IN ('Completed', 'Cancelled')
           AND created_at < ?
           AND uuid NOT IN (
               SELECT uuid FROM (
                   SELECT uuid
                   FROM Task
                   WHERE status IN ('Completed', 'Cancelled')
                   ORDER BY created_at DESC, uuid DESC
                   LIMIT ?
               )
           )",
    )
    .bind(cutoff)
    .bind(min_keep)
    .execute(pool)
    .await?;

    Ok(RetentionRun {
        deleted_count: result.rows_affected(),
        cutoff,
    })
}

pub async fn maybe_run_daily_retention(
    pool: &SqlitePool,
    policy: RetentionPolicy,
    now: DateTime<Utc>,
) -> Result<Option<RetentionRun>, sqlx::Error> {
    if !policy.enabled {
        return Ok(None);
    }

    let last_run: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT value FROM Settings WHERE key = 'task_retention_last_run_at'")
            .fetch_optional(pool)
            .await?
            .and_then(|raw: String| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|ts| ts.with_timezone(&Utc));

    if last_run.is_some_and(|ts| now.signed_duration_since(ts) < Duration::days(1)) {
        return Ok(None);
    }

    let run = prune_terminal_history(pool, policy.days, policy.min_keep).await?;
    sqlx::query(
        "INSERT INTO Settings (key, value) VALUES ('task_retention_last_run_at', ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(Some(run))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use sqlx::Row;

    async fn insert_task(
        pool: &SqlitePool,
        status: &str,
        created_at: DateTime<Utc>,
    ) -> Result<String, sqlx::Error> {
        let id = enqueue(pool, TaskType::Backup, None, None, 0).await?;
        sqlx::query(
            "UPDATE Task
             SET status = ?, created_at = ?, updated_at = ?
             WHERE uuid = ?",
        )
        .bind(status)
        .bind(created_at)
        .bind(created_at)
        .bind(id.to_string())
        .execute(pool)
        .await?;
        Ok(id.to_string())
    }

    #[tokio::test]
    async fn prune_terminal_history_only_deletes_completed_and_cancelled() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let now = Utc::now();

        insert_task(&pool, "Completed", now - Duration::days(40))
            .await
            .expect("completed");
        insert_task(&pool, "Cancelled", now - Duration::days(35))
            .await
            .expect("cancelled");
        insert_task(&pool, "Failed", now - Duration::days(50))
            .await
            .expect("failed");
        insert_task(&pool, "Pending", now - Duration::days(60))
            .await
            .expect("pending");
        insert_task(&pool, "Running", now - Duration::days(70))
            .await
            .expect("running");

        let result = prune_terminal_history(&pool, 30, 0).await.expect("prune");
        assert_eq!(result.deleted_count, 2);

        let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM Task ORDER BY status")
            .fetch_all(&pool)
            .await
            .expect("statuses");
        assert_eq!(statuses, vec!["Failed", "Pending", "Running"]);
    }

    #[tokio::test]
    async fn prune_terminal_history_respects_min_keep_floor() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let now = Utc::now();

        let mut ids = Vec::new();
        for offset in [50, 40, 30] {
            ids.push(
                insert_task(&pool, "Completed", now - Duration::days(offset))
                    .await
                    .expect("insert"),
            );
        }

        let result = prune_terminal_history(&pool, 1, 2).await.expect("prune");
        assert_eq!(result.deleted_count, 1);

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT uuid FROM Task ORDER BY created_at DESC, uuid DESC")
                .fetch_all(&pool)
                .await
                .expect("remaining");
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&ids[1]));
        assert!(remaining.contains(&ids[2]));
    }

    #[tokio::test]
    async fn maybe_run_daily_retention_skips_when_disabled_or_already_ran() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let now = Utc::now();

        insert_task(&pool, "Completed", now - Duration::days(40))
            .await
            .expect("completed");

        let disabled = RetentionPolicy {
            enabled: false,
            days: 30,
            min_keep: 0,
        };
        assert!(
            maybe_run_daily_retention(&pool, disabled, now)
                .await
                .expect("disabled")
                .is_none()
        );

        let enabled = RetentionPolicy {
            enabled: true,
            days: 30,
            min_keep: 0,
        };
        let first = maybe_run_daily_retention(&pool, enabled, now)
            .await
            .expect("first");
        assert_eq!(first.expect("run").deleted_count, 1);

        insert_task(&pool, "Completed", now - Duration::days(60))
            .await
            .expect("second completed");
        let second = maybe_run_daily_retention(&pool, enabled, now + Duration::hours(12))
            .await
            .expect("second");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn claim_next_for_queue_prioritises_provider_sync_over_older_download() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let queue = "provider:test-provider";
        let older = Utc::now() - Duration::minutes(5);
        let newer = Utc::now();

        let download_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.to_string()),
        )
        .await
        .expect("download task");
        sqlx::query("UPDATE Task SET created_at = ?, updated_at = ? WHERE uuid = ?")
            .bind(older)
            .bind(older)
            .bind(download_id.to_string())
            .execute(&pool)
            .await
            .expect("age download task");

        let sync_id = enqueue_with_queue(
            &pool,
            TaskType::SyncProviderChapters,
            None,
            None,
            PRIORITY_PROVIDER_SYNC,
            Some(queue.to_string()),
        )
        .await
        .expect("sync task");
        sqlx::query("UPDATE Task SET created_at = ?, updated_at = ? WHERE uuid = ?")
            .bind(newer)
            .bind(newer)
            .bind(sync_id.to_string())
            .execute(&pool)
            .await
            .expect("freshen sync task");

        let claimed = claim_next_for_queue(&pool, queue)
            .await
            .expect("claim task")
            .expect("task exists");

        assert_eq!(claimed.id, sync_id);
        assert_eq!(claimed.task_type, TaskType::SyncProviderChapters);
    }

    #[tokio::test]
    async fn claim_next_for_queue_uses_fifo_when_priorities_match() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let queue = "provider:test-provider";
        let older = Utc::now() - Duration::minutes(5);
        let newer = Utc::now();

        let first_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.to_string()),
        )
        .await
        .expect("first task");
        sqlx::query("UPDATE Task SET created_at = ?, updated_at = ? WHERE uuid = ?")
            .bind(older)
            .bind(older)
            .bind(first_id.to_string())
            .execute(&pool)
            .await
            .expect("age first task");

        let second_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.to_string()),
        )
        .await
        .expect("second task");
        sqlx::query("UPDATE Task SET created_at = ?, updated_at = ? WHERE uuid = ?")
            .bind(newer)
            .bind(newer)
            .bind(second_id.to_string())
            .execute(&pool)
            .await
            .expect("freshen second task");

        let claimed = claim_next_for_queue(&pool, queue)
            .await
            .expect("claim task")
            .expect("task exists");

        assert_eq!(claimed.id, first_id);
    }

    #[tokio::test]
    async fn provider_sync_priority_constant_stays_above_download_priority() {
        assert!(PRIORITY_PROVIDER_SYNC < PRIORITY_DOWNLOAD_CHAPTER);
    }

    #[tokio::test]
    async fn enqueue_with_queue_persists_explicit_priority() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let queue = "provider:test-provider".to_string();

        let sync_id = enqueue_with_queue(
            &pool,
            TaskType::SyncProviderChapters,
            None,
            None,
            PRIORITY_PROVIDER_SYNC,
            Some(queue.clone()),
        )
        .await
        .expect("enqueue sync");
        let download_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.clone()),
        )
        .await
        .expect("enqueue download");

        let sync_priority: i64 = sqlx::query("SELECT priority FROM Task WHERE uuid = ?")
            .bind(sync_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch sync")
            .get(0);
        let download_priority: i64 = sqlx::query("SELECT priority FROM Task WHERE uuid = ?")
            .bind(download_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch download")
            .get(0);

        assert_eq!(sync_priority, PRIORITY_PROVIDER_SYNC);
        assert_eq!(download_priority, PRIORITY_DOWNLOAD_CHAPTER);
    }

    #[tokio::test]
    async fn prioritise_task_moves_pending_download_ahead_of_provider_sync() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let queue = "provider:test-provider".to_string();

        let sync_id = enqueue_with_queue(
            &pool,
            TaskType::SyncProviderChapters,
            None,
            None,
            PRIORITY_PROVIDER_SYNC,
            Some(queue.clone()),
        )
        .await
        .expect("enqueue sync");
        let download_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.clone()),
        )
        .await
        .expect("enqueue download");

        let changed = prioritise_task(&pool, download_id)
            .await
            .expect("prioritise");
        assert!(changed);

        let claimed = claim_next_for_queue(&pool, &queue)
            .await
            .expect("claim task")
            .expect("task exists");

        assert_eq!(claimed.id, download_id);
        assert_ne!(claimed.id, sync_id);
    }

    #[tokio::test]
    async fn enqueue_at_front_places_new_download_ahead_of_existing_pending_tasks() {
        let pool = db::init("sqlite::memory:").await.expect("init db");
        let queue = "provider:test-provider".to_string();

        let existing_id = enqueue_with_queue(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            PRIORITY_DOWNLOAD_CHAPTER,
            Some(queue.clone()),
        )
        .await
        .expect("enqueue existing");

        let urgent_id = enqueue_at_front(
            &pool,
            TaskType::DownloadChapter,
            None,
            None,
            queue.clone(),
            None,
        )
        .await
        .expect("enqueue urgent");

        let claimed = claim_next_for_queue(&pool, &queue)
            .await
            .expect("claim task")
            .expect("task exists");

        assert_eq!(claimed.id, urgent_id);
        assert_ne!(claimed.id, existing_id);
    }
}
