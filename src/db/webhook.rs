use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    pub id: Uuid,
    pub target_url: String,
    pub enabled: bool,
    pub task_types: Vec<String>,
    pub task_statuses: Vec<String>,
    pub body_template: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewWebhookEndpoint {
    pub target_url: String,
    pub enabled: bool,
    pub task_types: Vec<String>,
    pub task_statuses: Vec<String>,
    pub body_template: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebhookDeliveryTarget {
    pub id: Uuid,
    pub target_url: String,
    pub body_template: Option<String>,
}

#[derive(sqlx::FromRow)]
struct WebhookRow {
    uuid: String,
    target_url: String,
    enabled: i64,
    body_template: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn ts_to_dt(ts: i64) -> Result<DateTime<Utc>, sqlx::Error> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .ok_or_else(|| sqlx::Error::Decode(format!("invalid timestamp: {ts}").into()))
}

fn parse_uuid(s: &str) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(s).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

fn normalise_distinct(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values
        .iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

async fn replace_filters(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    webhook_id: Uuid,
    task_types: &[String],
    task_statuses: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM WebhookEventFilter WHERE webhook_id = ?")
        .bind(webhook_id.to_string())
        .execute(&mut **tx)
        .await?;

    for task_type in task_types {
        for task_status in task_statuses {
            sqlx::query(
                "INSERT INTO WebhookEventFilter (webhook_id, task_type, task_status)
                 VALUES (?, ?, ?)",
            )
            .bind(webhook_id.to_string())
            .bind(task_type)
            .bind(task_status)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

async fn load_filter_map(
    pool: &SqlitePool,
) -> Result<std::collections::HashMap<String, (Vec<String>, Vec<String>)>, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT webhook_id, task_type, task_status
         FROM WebhookEventFilter
         ORDER BY task_type ASC, task_status ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut map: std::collections::HashMap<String, (Vec<String>, Vec<String>)> =
        std::collections::HashMap::new();
    for (webhook_id, task_type, task_status) in rows {
        let entry = map.entry(webhook_id).or_default();
        entry.0.push(task_type);
        entry.1.push(task_status);
    }

    for (task_types, task_statuses) in map.values_mut() {
        task_types.sort();
        task_types.dedup();
        task_statuses.sort();
        task_statuses.dedup();
    }

    Ok(map)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<WebhookEndpoint>, sqlx::Error> {
    let rows = sqlx::query_as::<_, WebhookRow>(
        "SELECT uuid, target_url, enabled, body_template, created_at, updated_at
         FROM WebhookEndpoint
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    let filters = load_filter_map(pool).await?;

    rows.into_iter()
        .map(|row| {
            let key = row.uuid.clone();
            let (task_types, task_statuses) = filters.get(&key).cloned().unwrap_or_default();
            Ok(WebhookEndpoint {
                id: parse_uuid(&row.uuid)?,
                target_url: row.target_url,
                enabled: row.enabled != 0,
                task_types,
                task_statuses,
                body_template: row.body_template,
                created_at: ts_to_dt(row.created_at)?,
                updated_at: ts_to_dt(row.updated_at)?,
            })
        })
        .collect()
}

pub async fn create(
    pool: &SqlitePool,
    input: NewWebhookEndpoint,
) -> Result<WebhookEndpoint, sqlx::Error> {
    let now = Utc::now().timestamp();
    let id = Uuid::new_v4();
    let task_types = normalise_distinct(&input.task_types);
    let task_statuses = normalise_distinct(&input.task_statuses);

    let mut tx = pool.begin().await?;
    let body_template = input.body_template.as_deref().and_then(|t| {
        let t = t.trim();
        if t.is_empty() { None } else { Some(t.to_owned()) }
    });

    sqlx::query(
        "INSERT INTO WebhookEndpoint (uuid, target_url, enabled, body_template, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(input.target_url.trim())
    .bind(input.enabled as i64)
    .bind(&body_template)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    replace_filters(&mut tx, id, &task_types, &task_statuses).await?;
    tx.commit().await?;

    Ok(WebhookEndpoint {
        id,
        target_url: input.target_url.trim().to_owned(),
        enabled: input.enabled,
        task_types,
        task_statuses,
        body_template,
        created_at: ts_to_dt(now)?,
        updated_at: ts_to_dt(now)?,
    })
}

pub async fn update(
    pool: &SqlitePool,
    id: Uuid,
    input: NewWebhookEndpoint,
) -> Result<Option<WebhookEndpoint>, sqlx::Error> {
    let now = Utc::now().timestamp();
    let task_types = normalise_distinct(&input.task_types);
    let task_statuses = normalise_distinct(&input.task_statuses);
    let mut tx = pool.begin().await?;

    let body_template = input.body_template.as_deref().and_then(|t| {
        let t = t.trim();
        if t.is_empty() { None } else { Some(t.to_owned()) }
    });

    let updated = sqlx::query(
        "UPDATE WebhookEndpoint
         SET target_url = ?, enabled = ?, body_template = ?, updated_at = ?
         WHERE uuid = ?",
    )
    .bind(input.target_url.trim())
    .bind(input.enabled as i64)
    .bind(&body_template)
    .bind(now)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if updated == 0 {
        tx.rollback().await?;
        return Ok(None);
    }

    replace_filters(&mut tx, id, &task_types, &task_statuses).await?;
    tx.commit().await?;

    let created_at_raw: i64 =
        sqlx::query_scalar("SELECT created_at FROM WebhookEndpoint WHERE uuid = ?")
            .bind(id.to_string())
            .fetch_one(pool)
            .await?;

    Ok(Some(WebhookEndpoint {
        id,
        target_url: input.target_url.trim().to_owned(),
        enabled: input.enabled,
        task_types,
        task_statuses,
        body_template,
        created_at: ts_to_dt(created_at_raw)?,
        updated_at: ts_to_dt(now)?,
    }))
}

pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query("DELETE FROM WebhookEndpoint WHERE uuid = ?")
        .bind(id.to_string())
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

pub async fn matching_targets(
    pool: &SqlitePool,
    task_type: &str,
    task_status: &str,
) -> Result<Vec<WebhookDeliveryTarget>, sqlx::Error> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT w.uuid, w.target_url, w.body_template
         FROM WebhookEndpoint w
         JOIN WebhookEventFilter f ON f.webhook_id = w.uuid
         WHERE w.enabled = 1 AND f.task_type = ? AND f.task_status = ?
         ORDER BY w.created_at ASC",
    )
    .bind(task_type)
    .bind(task_status)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(id, target_url, body_template)| {
            Ok(WebhookDeliveryTarget {
                id: parse_uuid(&id)?,
                target_url,
                body_template,
            })
        })
        .collect()
}
