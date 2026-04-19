use rocket::{State, delete, get, post, put, serde::json::Json};
use rocket_okapi::openapi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::metadata_rules::{self, MetadataRule};

use super::errors::{ApiResult, bad_request, internal, not_found};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateMetadataRuleRequest {
    pub name: String,
    pub sort_order: i32,
    /// Provider to match (exact, case-insensitive). Omit to match all providers.
    pub provider_name: Option<String>,
    /// Field to transform: "title" or "scanlator_group"
    pub field: String,
    /// Action: "clear" | "set" | "replace"
    pub action: String,
    /// For "clear"/"replace": regex pattern to match against the field value.
    pub pattern: Option<String>,
    /// For "set": override value. For "replace": replacement string.
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateMetadataRuleRequest {
    pub name: Option<String>,
    pub sort_order: Option<i32>,
    pub provider_name: Option<String>,
    pub field: Option<String>,
    pub action: Option<String>,
    pub pattern: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MetadataRuleResponse {
    pub id: String,
    pub sort_order: i32,
    pub name: String,
    pub provider_name: Option<String>,
    pub field: String,
    pub action: String,
    pub pattern: Option<String>,
    pub value: Option<String>,
}

impl From<MetadataRule> for MetadataRuleResponse {
    fn from(r: MetadataRule) -> Self {
        Self {
            id: r.id,
            sort_order: r.sort_order,
            name: r.name,
            provider_name: r.provider_name,
            field: r.field,
            action: r.action,
            pattern: r.pattern,
            value: r.value,
        }
    }
}

// ---------------------------------------------------------------------------
// GET /api/metadata-rules
// ---------------------------------------------------------------------------

/// List all metadata filtering rules.
#[openapi(tag = "Metadata Rules")]
#[get("/api/metadata-rules")]
pub async fn list_rules(pool: &State<SqlitePool>) -> ApiResult<Vec<MetadataRuleResponse>> {
    let rules = metadata_rules::load(pool.inner()).await.map_err(internal)?;
    Ok(Json(rules.into_iter().map(Into::into).collect()))
}

// ---------------------------------------------------------------------------
// POST /api/metadata-rules
// ---------------------------------------------------------------------------

/// Create a new metadata filtering rule.
#[openapi(tag = "Metadata Rules")]
#[post("/api/metadata-rules", data = "<req>")]
pub async fn create_rule(
    pool: &State<SqlitePool>,
    req: Json<CreateMetadataRuleRequest>,
) -> ApiResult<MetadataRuleResponse> {
    validate_field(&req.field)?;
    validate_action(&req.action)?;

    let mut rules = metadata_rules::load(pool.inner()).await.map_err(internal)?;
    let rule = MetadataRule {
        id: Uuid::new_v4().to_string(),
        sort_order: req.sort_order,
        name: req.name.clone(),
        provider_name: req.provider_name.clone(),
        field: req.field.clone(),
        action: req.action.clone(),
        pattern: req.pattern.clone(),
        value: req.value.clone(),
    };
    rules.push(rule.clone());
    rules.sort_by_key(|r| r.sort_order);
    metadata_rules::save(pool.inner(), &rules)
        .await
        .map_err(internal)?;
    Ok(Json(rule.into()))
}

// ---------------------------------------------------------------------------
// PUT /api/metadata-rules/<id>
// ---------------------------------------------------------------------------

/// Update an existing metadata filtering rule.
#[openapi(tag = "Metadata Rules")]
#[put("/api/metadata-rules/<id>", data = "<req>")]
pub async fn update_rule(
    pool: &State<SqlitePool>,
    id: &str,
    req: Json<UpdateMetadataRuleRequest>,
) -> ApiResult<MetadataRuleResponse> {
    let mut rules = metadata_rules::load(pool.inner()).await.map_err(internal)?;
    let rule = rules
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| not_found("metadata rule not found"))?;

    if let Some(name) = &req.name {
        rule.name = name.clone();
    }
    if let Some(sort_order) = req.sort_order {
        rule.sort_order = sort_order;
    }
    if req.provider_name.is_some() {
        rule.provider_name = req.provider_name.clone();
    }
    if let Some(field) = &req.field {
        validate_field(field)?;
        rule.field = field.clone();
    }
    if let Some(action) = &req.action {
        validate_action(action)?;
        rule.action = action.clone();
    }
    rule.pattern = req.pattern.clone();
    rule.value = req.value.clone();

    let updated = rule.clone();
    rules.sort_by_key(|r| r.sort_order);
    metadata_rules::save(pool.inner(), &rules)
        .await
        .map_err(internal)?;
    Ok(Json(updated.into()))
}

// ---------------------------------------------------------------------------
// DELETE /api/metadata-rules/<id>
// ---------------------------------------------------------------------------

/// Delete a metadata filtering rule.
#[openapi(tag = "Metadata Rules")]
#[delete("/api/metadata-rules/<id>")]
pub async fn delete_rule(pool: &State<SqlitePool>, id: &str) -> ApiResult<()> {
    let mut rules = metadata_rules::load(pool.inner()).await.map_err(internal)?;
    let before = rules.len();
    rules.retain(|r| r.id != id);
    if rules.len() == before {
        return Err(not_found("metadata rule not found"));
    }
    metadata_rules::save(pool.inner(), &rules)
        .await
        .map_err(internal)?;
    Ok(Json(()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn validate_field(
    field: &str,
) -> Result<(), (rocket::http::Status, Json<super::errors::ApiError>)> {
    match field {
        "title" | "scanlator_group" => Ok(()),
        _ => Err(bad_request("field must be 'title' or 'scanlator_group'")),
    }
}

fn validate_action(
    action: &str,
) -> Result<(), (rocket::http::Status, Json<super::errors::ApiError>)> {
    match action {
        "clear" | "set" | "replace" => Ok(()),
        _ => Err(bad_request("action must be 'clear', 'set', or 'replace'")),
    }
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![list_rules, create_rule, update_rule, delete_rule]
}
