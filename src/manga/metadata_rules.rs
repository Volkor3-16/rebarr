//! Metadata filtering rules: user-defined rules that clean up or override metadata
//! from specific providers before it is used in merging or display.
//!
//! Rules are stored as a JSON array in the settings table under key "metadata_rules".
//! They are evaluated on read in the API layer (not persisted back into the DB).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

// ---------------------------------------------------------------------------
// Rule types
// ---------------------------------------------------------------------------

/// A single metadata filtering rule.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataRule {
    pub id: String,
    pub sort_order: i32,
    pub name: String,
    /// Provider to match (exact, case-insensitive). None = applies to all providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    /// Field to transform: "title" or "scanlator_group"
    pub field: String,
    /// Action: "clear" | "set" | "replace"
    pub action: String,
    /// For "clear"/"replace": regex pattern to match against the field value.
    /// "clear" removes the value if pattern matches (or always if pattern is None).
    /// "replace" performs a regex substitution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// For "set": override value. For "replace": replacement string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

// ---------------------------------------------------------------------------
// Load / save from settings
// ---------------------------------------------------------------------------

const SETTINGS_KEY: &str = "metadata_rules";

pub async fn load(pool: &SqlitePool) -> Result<Vec<MetadataRule>, sqlx::Error> {
    let json = crate::db::settings::get(pool, SETTINGS_KEY, "[]").await?;
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

pub async fn save(pool: &SqlitePool, rules: &[MetadataRule]) -> Result<(), sqlx::Error> {
    let json = serde_json::to_string(rules).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    crate::db::settings::set(pool, SETTINGS_KEY, &json).await
}

// ---------------------------------------------------------------------------
// Apply rules to a field value
// ---------------------------------------------------------------------------

/// Apply all matching metadata rules to a (provider, field, value) combination.
/// Returns the transformed value (may be None if cleared).
pub fn apply_rules(
    rules: &[MetadataRule],
    provider_name: Option<&str>,
    field: &str,
    value: Option<&str>,
) -> Option<String> {
    let mut current: Option<String> = value.map(str::to_owned);

    for rule in rules {
        if rule.field != field {
            continue;
        }
        // Provider filter: skip if rule specifies a provider and it doesn't match.
        if let Some(ref rule_provider) = rule.provider_name {
            let matches = provider_name
                .map(|p| p.eq_ignore_ascii_case(rule_provider))
                .unwrap_or(false);
            if !matches {
                continue;
            }
        }

        current = apply_action(&rule.action, &rule.pattern, &rule.value, current);
    }

    current
}

fn apply_action(
    action: &str,
    pattern: &Option<String>,
    value: &Option<String>,
    current: Option<String>,
) -> Option<String> {
    match action {
        "clear" => {
            if let Some(pat) = pattern {
                // Only clear if pattern matches.
                if let Some(cur) = &current {
                    if regex_matches(pat, cur) {
                        return None;
                    }
                }
                current
            } else {
                // No pattern: always clear.
                None
            }
        }
        "set" => {
            // Override unconditionally with the given value.
            value.clone()
        }
        "replace" => {
            if let (Some(pat), Some(replacement), Some(cur)) =
                (pattern, value, &current)
            {
                if let Ok(re) = regex::Regex::new(pat) {
                    return Some(re.replace_all(cur, replacement.as_str()).into_owned());
                }
            }
            current
        }
        _ => current,
    }
}

fn regex_matches(pattern: &str, value: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|re| re.is_match(value))
        .unwrap_or(false)
}
