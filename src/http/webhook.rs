use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use log::warn;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::{self, webhook as db_webhook};

static DISPATCHER: OnceLock<WebhookDispatcher> = OnceLock::new();

#[derive(Clone)]
pub struct WebhookDispatcher {
    pool: SqlitePool,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
pub struct TaskWebhookPayload {
    pub task_id: String,
    pub task_type: String,
    pub status: String,
    pub queue: String,
    pub priority: i64,
    pub attempt: i64,
    pub max_attempts: i64,
    pub last_error: Option<String>,
    pub manga_id: Option<String>,
    pub manga_title: Option<String>,
    pub chapter_id: Option<String>,
    pub chapter_number_raw: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WebhookDispatcher {
    pub fn new(pool: SqlitePool, http: reqwest::Client) -> Self {
        Self { pool, http }
    }

    pub fn install(self) {
        let _ = DISPATCHER.set(self);
    }

    pub fn dispatch_task_event(&self, task_id: Uuid, task_type: &str, task_status: &str) {
        let pool = self.pool.clone();
        let http = self.http.clone();
        let task_type = task_type.to_owned();
        let task_status = task_status.to_owned();

        tokio::spawn(async move {
            let targets = match db_webhook::matching_targets(&pool, &task_type, &task_status).await
            {
                Ok(targets) => targets,
                Err(e) => {
                    warn!(
                        "[webhook] failed to load targets for {task_type} {task_status}: {e}"
                    );
                    return;
                }
            };

            if targets.is_empty() {
                return;
            }

            let payload = match build_payload(&pool, task_id).await {
                Ok(Some(payload)) => payload,
                Ok(None) => return,
                Err(e) => {
                    warn!(
                        "[webhook] failed to build payload for task {task_id}: {e}"
                    );
                    return;
                }
            };

            for target in targets {
                let body = match target.body_template.as_deref() {
                    Some(tmpl) => render_body(tmpl, &payload).into_bytes(),
                    None => match serde_json::to_vec(&payload) {
                        Ok(b) => b,
                        Err(e) => {
                            warn!("[webhook] failed to serialize payload for {task_id}: {e}");
                            return;
                        }
                    },
                };
                let res = http
                    .post(&target.target_url)
                    .timeout(Duration::from_secs(5))
                    .header("Content-Type", "application/json")
                    .body(body)
                    .send()
                    .await;

                match res {
                    Ok(resp) if resp.status().is_success() => {}
                    Ok(resp) => warn!(
                        "[webhook] delivery to {} failed with HTTP {}",
                        target.target_url,
                        resp.status()
                    ),
                    Err(e) => warn!(
                        "[webhook] delivery to {} failed: {}",
                        target.target_url, e
                    ),
                }
            }
        });
    }
}

pub fn dispatch_task_event(task_id: Uuid, task_type: &str, task_status: &str) {
    if let Some(dispatcher) = DISPATCHER.get() {
        dispatcher.dispatch_task_event(task_id, task_type, task_status);
    }
}

/// Substitute `{{variable}}` placeholders in a template string using payload fields.
/// Missing or null fields render as an empty string.
fn render_body(template: &str, payload: &TaskWebhookPayload) -> String {
    let vars: &[(&str, String)] = &[
        ("task_id", payload.task_id.clone()),
        ("task_type", payload.task_type.clone()),
        ("status", payload.status.clone()),
        ("queue", payload.queue.clone()),
        ("priority", payload.priority.to_string()),
        ("attempt", payload.attempt.to_string()),
        ("max_attempts", payload.max_attempts.to_string()),
        ("last_error", payload.last_error.clone().unwrap_or_default()),
        ("manga_id", payload.manga_id.clone().unwrap_or_default()),
        ("manga_title", payload.manga_title.clone().unwrap_or_default()),
        ("chapter_id", payload.chapter_id.clone().unwrap_or_default()),
        ("chapter_number_raw", payload.chapter_number_raw.clone().unwrap_or_default()),
        ("created_at", payload.created_at.to_rfc3339()),
        ("updated_at", payload.updated_at.to_rfc3339()),
    ];
    let mut out = template.to_owned();
    for (key, val) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), val);
    }
    out
}

async fn build_payload(
    pool: &SqlitePool,
    task_id: Uuid,
) -> Result<Option<TaskWebhookPayload>, sqlx::Error> {
    let recent = db::task::get_recent(pool, None, 200).await?;
    let Some(task) = recent.into_iter().find(|task| task.id == task_id.to_string()) else {
        return Ok(None);
    };

    let task_row = db::task::get_by_id(pool, task_id).await?;
    let Some(task_row) = task_row else {
        return Ok(None);
    };

    Ok(Some(TaskWebhookPayload {
        task_id: task.id,
        task_type: task.task_type,
        status: task.status,
        queue: task_row.queue,
        priority: task.priority,
        attempt: task.attempt,
        max_attempts: task.max_attempts,
        last_error: task.last_error,
        manga_id: task.manga_id,
        manga_title: task.manga_title,
        chapter_id: task.chapter_id,
        chapter_number_raw: task.chapter_number_raw,
        created_at: task.created_at,
        updated_at: task.updated_at,
    }))
}
