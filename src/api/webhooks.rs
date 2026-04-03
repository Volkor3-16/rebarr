use rocket::{State, delete, get, http::Status, post, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db;

use super::errors::{ApiError, ApiResult, bad_request, internal, not_found};

const VALID_TASK_TYPES: &[&str] = &[
    "BuildFullChapterList",
    "RefreshMetadata",
    "CheckNewChapter",
    "DownloadChapter",
    "ScanDisk",
    "OptimiseChapter",
    "Backup",
];

const VALID_TASK_STATUSES: &[&str] = &["Pending", "Running", "Completed", "Failed", "Cancelled"];

#[derive(Serialize, JsonSchema)]
pub struct WebhookEndpointResponse {
    pub id: String,
    pub target_url: String,
    pub enabled: bool,
    pub task_types: Vec<String>,
    pub task_statuses: Vec<String>,
    pub body_template: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WebhookEndpointRequest {
    pub target_url: String,
    pub enabled: Option<bool>,
    pub task_types: Vec<String>,
    pub task_statuses: Vec<String>,
    pub body_template: Option<String>,
}

fn validate_request(
    body: &WebhookEndpointRequest,
) -> Result<db::webhook::NewWebhookEndpoint, (Status, Json<ApiError>)> {
    let target_url = body.target_url.trim();
    if target_url.is_empty() {
        return Err(bad_request("target_url must not be empty"));
    }
    if !(target_url.starts_with("http://") || target_url.starts_with("https://")) {
        return Err(bad_request("target_url must start with http:// or https://"));
    }
    if body.task_types.is_empty() {
        return Err(bad_request("at least one task_type is required"));
    }
    if body.task_statuses.is_empty() {
        return Err(bad_request("at least one task_status is required"));
    }
    if body
        .task_types
        .iter()
        .any(|task_type| !VALID_TASK_TYPES.contains(&task_type.as_str()))
    {
        return Err(bad_request("one or more task_types are invalid"));
    }
    if body
        .task_statuses
        .iter()
        .any(|status| !VALID_TASK_STATUSES.contains(&status.as_str()))
    {
        return Err(bad_request("one or more task_statuses are invalid"));
    }

    Ok(db::webhook::NewWebhookEndpoint {
        target_url: target_url.to_owned(),
        enabled: body.enabled.unwrap_or(true),
        task_types: body.task_types.clone(),
        task_statuses: body.task_statuses.clone(),
        body_template: body.body_template.clone(),
    })
}

fn to_response(hook: db::webhook::WebhookEndpoint) -> WebhookEndpointResponse {
    WebhookEndpointResponse {
        id: hook.id.to_string(),
        target_url: hook.target_url,
        enabled: hook.enabled,
        task_types: hook.task_types,
        task_statuses: hook.task_statuses,
        body_template: hook.body_template,
        created_at: hook.created_at,
        updated_at: hook.updated_at,
    }
}

/// List all webhook endpoints.
#[openapi(tag = "Webhooks")]
#[get("/api/webhooks")]
pub async fn list_webhooks(pool: &State<SqlitePool>) -> ApiResult<Vec<WebhookEndpointResponse>> {
    let hooks = db::webhook::list(pool.inner()).await.map_err(internal)?;
    Ok(Json(hooks.into_iter().map(to_response).collect()))
}

/// Create a new webhook endpoint.
#[openapi(tag = "Webhooks")]
#[post("/api/webhooks", data = "<body>")]
pub async fn create_webhook(
    pool: &State<SqlitePool>,
    body: Json<WebhookEndpointRequest>,
) -> ApiResult<WebhookEndpointResponse> {
    let hook = validate_request(&body)?;
    let created = db::webhook::create(pool.inner(), hook)
        .await
        .map_err(internal)?;
    Ok(Json(to_response(created)))
}

/// Update an existing webhook endpoint.
#[openapi(tag = "Webhooks")]
#[put("/api/webhooks/<id>", data = "<body>")]
pub async fn update_webhook(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<WebhookEndpointRequest>,
) -> ApiResult<WebhookEndpointResponse> {
    let hook_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let hook = validate_request(&body)?;
    let updated = db::webhook::update(pool.inner(), hook_id, hook)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("webhook not found"))?;
    Ok(Json(to_response(updated)))
}

/// Delete a webhook endpoint.
#[openapi(tag = "Webhooks")]
#[delete("/api/webhooks/<id>")]
pub async fn delete_webhook(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<ApiError>)> {
    let hook_id = Uuid::parse_str(id).map_err(|_| bad_request("invalid UUID"))?;
    let deleted = db::webhook::delete(pool.inner(), hook_id)
        .await
        .map_err(internal)?;
    if !deleted {
        return Err(not_found("webhook not found"));
    }
    Ok(Status::NoContent)
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![list_webhooks, create_webhook, update_webhook, delete_webhook,]
}
