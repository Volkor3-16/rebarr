use rocket::{State, delete, get, http::Status, post, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::info;

use crate::db::quality_rules::{self, QualityRule, RuleCondition};
use crate::manga::scoring::KNOWN_FIELDS;

use super::errors::{ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CreateRuleRequest {
    pub name: String,
    pub score: i32,
    pub sort_order: i32,
    pub conditions: Vec<RuleCondition>,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateRuleRequest {
    pub name: String,
    pub score: i32,
    pub sort_order: i32,
    pub conditions: Vec<RuleCondition>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReorderRequest {
    /// Array of [id, new_sort_order] pairs.
    pub ordering: Vec<(String, i32)>,
}

// ---------------------------------------------------------------------------
// GET /api/quality-rules
// ---------------------------------------------------------------------------

/// List all quality rules ordered by sort_order.
#[openapi(tag = "Quality Rules")]
#[get("/api/quality-rules")]
pub async fn list_rules(pool: &State<SqlitePool>) -> ApiResult<Vec<QualityRule>> {
    let rules = quality_rules::get_all(pool.inner())
        .await
        .map_err(internal)?;
    Ok(Json(rules))
}

// ---------------------------------------------------------------------------
// GET /api/quality-rules/fields
// ---------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct FieldInfo {
    pub field: String,
    pub label: String,
    pub ops: Vec<String>,
}

/// Returns the known condition fields and their supported operators.
/// Used by the frontend rule builder to populate dropdowns.
#[openapi(tag = "Quality Rules")]
#[get("/api/quality-rules/fields")]
pub async fn list_fields() -> Json<Vec<FieldInfo>> {
    let fields = KNOWN_FIELDS
        .iter()
        .map(|f| FieldInfo {
            field: f.field.to_owned(),
            label: f.label.to_owned(),
            ops: f.ops.iter().map(|s| s.to_string()).collect(),
        })
        .collect();
    Json(fields)
}

// ---------------------------------------------------------------------------
// POST /api/quality-rules
// ---------------------------------------------------------------------------

/// Create a new quality rule.
#[openapi(tag = "Quality Rules")]
#[post("/api/quality-rules", data = "<body>")]
pub async fn create_rule(
    pool: &State<SqlitePool>,
    body: Json<CreateRuleRequest>,
) -> ApiResult<QualityRule> {
    if body.name.trim().is_empty() {
        return Err(bad_request("name must not be empty"));
    }
    let rule = quality_rules::insert(
        pool.inner(),
        &body.name,
        body.score,
        body.sort_order,
        &body.conditions,
    )
    .await
    .map_err(internal)?;
    info!(
        "[api] Quality rule created: {} (score={})",
        rule.name, rule.score
    );
    Ok(Json(rule))
}

// ---------------------------------------------------------------------------
// PUT /api/quality-rules/<id>
// ---------------------------------------------------------------------------

/// Update an existing quality rule.
#[openapi(tag = "Quality Rules")]
#[put("/api/quality-rules/<id>", data = "<body>")]
pub async fn update_rule(
    pool: &State<SqlitePool>,
    id: &str,
    body: Json<UpdateRuleRequest>,
) -> Result<Status, (Status, Json<super::errors::ApiError>)> {
    if body.name.trim().is_empty() {
        return Err(bad_request("name must not be empty"));
    }
    let updated = quality_rules::update(
        pool.inner(),
        id,
        &body.name,
        body.score,
        body.sort_order,
        &body.conditions,
    )
    .await
    .map_err(internal)?;
    if !updated {
        return Err(not_found("rule not found"));
    }
    info!("[api] Quality rule updated: {id}");
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// DELETE /api/quality-rules/<id>
// ---------------------------------------------------------------------------

/// Delete a quality rule.
#[openapi(tag = "Quality Rules")]
#[delete("/api/quality-rules/<id>")]
pub async fn delete_rule(
    pool: &State<SqlitePool>,
    id: &str,
) -> Result<Status, (Status, Json<super::errors::ApiError>)> {
    let deleted = quality_rules::delete(pool.inner(), id)
        .await
        .map_err(internal)?;
    if !deleted {
        return Err(not_found("rule not found"));
    }
    info!("[api] Quality rule deleted: {id}");
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// POST /api/quality-rules/reorder
// ---------------------------------------------------------------------------

/// Bulk-update sort_order for all rules (pass array of {id, sort_order} pairs).
#[openapi(tag = "Quality Rules")]
#[post("/api/quality-rules/reorder", data = "<body>")]
pub async fn reorder_rules(
    pool: &State<SqlitePool>,
    body: Json<ReorderRequest>,
) -> Result<Status, (Status, Json<super::errors::ApiError>)> {
    quality_rules::reorder(pool.inner(), &body.ordering)
        .await
        .map_err(internal)?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_rules,
        list_fields,
        create_rule,
        update_rule,
        delete_rule,
        reorder_rules,
    ]
}
