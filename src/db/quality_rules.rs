use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Rule condition types
// ---------------------------------------------------------------------------

/// A single condition within a quality rule.
/// All conditions in a rule must match for the rule's score to be applied.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuleCondition {
    /// Field to test: "scanlator_group", "provider_name", "language", "title", "released_at"
    pub field: String,
    /// Operator: "eq", "contains", "regex", "present", "not_present"
    pub op: String,
    /// Value to compare against (required for eq/contains/regex, ignored for present/not_present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// If true, invert the match result.
    #[serde(default)]
    pub negate: bool,
}

// ---------------------------------------------------------------------------
// Rule type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QualityRule {
    pub id: String,
    pub sort_order: i32,
    pub name: String,
    pub score: i32,
    pub conditions: Vec<RuleCondition>,
}

// ---------------------------------------------------------------------------
// Row type (DB)
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct QualityRuleRow {
    id: String,
    sort_order: i64,
    name: String,
    score: i64,
    conditions: String,
}

fn rule_from_row(row: QualityRuleRow) -> QualityRule {
    let conditions: Vec<RuleCondition> = serde_json::from_str(&row.conditions).unwrap_or_default();
    QualityRule {
        id: row.id,
        sort_order: row.sort_order as i32,
        name: row.name,
        score: row.score as i32,
        conditions,
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// Fetch all quality rules, ordered by sort_order.
pub async fn get_all(pool: &SqlitePool) -> Result<Vec<QualityRule>, sqlx::Error> {
    let rows = sqlx::query_as::<_, QualityRuleRow>(
        "SELECT id, sort_order, name, score, conditions FROM QualityRules ORDER BY sort_order ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(rule_from_row).collect())
}

/// Insert a new rule. Returns the new rule with its generated UUID.
pub async fn insert(
    pool: &SqlitePool,
    name: &str,
    score: i32,
    sort_order: i32,
    conditions: &[RuleCondition],
) -> Result<QualityRule, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let conditions_json =
        serde_json::to_string(conditions).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    sqlx::query(
        "INSERT INTO QualityRules (id, sort_order, name, score, conditions) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(sort_order as i64)
    .bind(name)
    .bind(score as i64)
    .bind(&conditions_json)
    .execute(pool)
    .await?;

    Ok(QualityRule {
        id,
        sort_order,
        name: name.to_owned(),
        score,
        conditions: conditions.to_vec(),
    })
}

/// Update an existing rule by id.
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    score: i32,
    sort_order: i32,
    conditions: &[RuleCondition],
) -> Result<bool, sqlx::Error> {
    let conditions_json =
        serde_json::to_string(conditions).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let result = sqlx::query(
        "UPDATE QualityRules SET name = ?, score = ?, sort_order = ?, conditions = ? WHERE id = ?",
    )
    .bind(name)
    .bind(score as i64)
    .bind(sort_order as i64)
    .bind(&conditions_json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Delete a rule by id. Returns true if a row was deleted.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM QualityRules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Bulk-update sort_order values. Accepts a slice of (id, new_sort_order) pairs.
pub async fn reorder(pool: &SqlitePool, ordering: &[(String, i32)]) -> Result<(), sqlx::Error> {
    for (id, order) in ordering {
        sqlx::query("UPDATE QualityRules SET sort_order = ? WHERE id = ?")
            .bind(*order as i64)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Default provider rules
// ---------------------------------------------------------------------------

/// Ensure that default quality rules exist for each provider that has a default_score defined.
/// Will only add a rule if there are **no existing rules at all** for that provider.
/// Existing rules (even with different score) are never modified or removed.
pub async fn ensure_default_provider_rules(
    pool: &SqlitePool,
    providers: &[crate::scraper::def::ProviderDef],
) -> Result<(), sqlx::Error> {
    use tracing::info;

    let existing_rules = get_all(pool).await?;

    // Collect all provider names that already have any rule
    let mut providers_with_existing_rules = std::collections::HashSet::new();
    for rule in &existing_rules {
        for condition in &rule.conditions {
            if condition.field == "provider_name" && condition.op == "eq" {
                if let Some(ref provider_name) = condition.value {
                    providers_with_existing_rules.insert(provider_name);
                }
            }
        }
    }

    let mut added_count = 0;

    for provider in providers {
        // Skip if no default score defined
        let Some(score) = provider.default_score else {
            continue;
        };

        // Skip if this provider already has any rule
        if providers_with_existing_rules.contains(&provider.name) {
            continue;
        }

        // Create default rule for this provider
        let rule_name = format!("Default: {}", provider.name);
        let conditions = vec![RuleCondition {
            field: "provider_name".to_string(),
            op: "eq".to_string(),
            value: Some(provider.name.clone()),
            negate: false,
        }];

        // Insert with sort_order 0 (lowest priority, runs last)
        match insert(pool, &rule_name, score, 0, &conditions).await {
            Ok(_) => {
                info!(
                    "Added default quality rule for provider '{}' with score {}",
                    provider.name, score
                );
                added_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to add default quality rule for provider '{}': {}",
                    provider.name,
                    e
                );
            }
        }
    }

    if added_count > 0 {
        info!("Added {} default provider quality rules", added_count);
    }

    Ok(())
}
