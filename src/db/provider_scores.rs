use std::collections::HashMap;

use sqlx::SqlitePool;
use uuid::Uuid;

/// Get the global score override for a provider (None = not set, use YAML default).
pub async fn get_global_score(
    pool: &SqlitePool,
    provider_name: &str,
) -> Result<Option<i32>, sqlx::Error> {
    let row: Option<(i32,)> =
        sqlx::query_as("SELECT score FROM Providers WHERE provider_name = ? AND manga_id IS NULL")
            .bind(provider_name)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(score,)| score))
}

/// Get the per-series score override for a provider (None = not set, fall back to global).
pub async fn get_series_score(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
) -> Result<Option<i32>, sqlx::Error> {
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT score FROM Providers WHERE provider_name = ? AND manga_id = ?",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(score,)| score))
}

/// Get the enabled flag for a provider/series combination. Defaults to true if no row.
pub async fn get_enabled(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT enabled FROM Providers WHERE provider_name = ? AND manga_id = ?",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(e,)| e != 0).unwrap_or(true))
}

/// Get the global enabled flag for a provider. Defaults to true if no row.
pub async fn get_global_enabled(
    pool: &SqlitePool,
    provider_name: &str,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT enabled FROM Providers WHERE provider_name = ? AND manga_id IS NULL",
    )
    .bind(provider_name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(e,)| e != 0).unwrap_or(true))
}

/// Get all globally-disabled provider names.
pub async fn get_globally_disabled(
    pool: &SqlitePool,
) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT provider_name FROM Providers WHERE manga_id IS NULL AND enabled = 0",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(name,)| name).collect())
}

/// Upsert the global score (and enabled state) for a provider.
pub async fn upsert_global_score(
    pool: &SqlitePool,
    provider_name: &str,
    score: i32,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO Providers (provider_name, manga_id, score, enabled)
         VALUES (?, NULL, ?, ?)
         ON CONFLICT(provider_name) WHERE manga_id IS NULL
         DO UPDATE SET score = excluded.score, enabled = excluded.enabled",
    )
    .bind(provider_name)
    .bind(score)
    .bind(enabled as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert per-series score and enabled state for a provider.
pub async fn upsert_series(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
    score: i32,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO Providers (provider_name, manga_id, score, enabled)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(provider_name, manga_id) WHERE manga_id IS NOT NULL
         DO UPDATE SET score = excluded.score, enabled = excluded.enabled",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .bind(score)
    .bind(enabled as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the effective score for a provider/series: series > global > yaml_default.
pub async fn get_effective_score(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
    yaml_default: i32,
) -> Result<i32, sqlx::Error> {
    if let Some(score) = get_series_score(pool, provider_name, manga_id).await? {
        return Ok(score);
    }
    if let Some(score) = get_global_score(pool, provider_name).await? {
        return Ok(score);
    }
    Ok(yaml_default)
}

/// Load effective scores for all providers for a given manga.
/// Returns a map of provider_name → effective score.
/// `yaml_defaults` maps provider_name → default score from YAML.
pub async fn load_effective_scores(
    pool: &SqlitePool,
    manga_id: Uuid,
    yaml_defaults: &HashMap<String, i32>,
) -> Result<HashMap<String, i32>, sqlx::Error> {
    // Fetch all global rows
    let global_rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT provider_name, score FROM Providers WHERE manga_id IS NULL",
    )
    .fetch_all(pool)
    .await?;
    let global_map: HashMap<String, i32> = global_rows.into_iter().collect();

    // Fetch all series-specific rows for this manga
    let series_rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT provider_name, score FROM Providers WHERE manga_id = ?",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;
    let series_map: HashMap<String, i32> = series_rows.into_iter().collect();

    // Merge: series > global > yaml_default
    let mut result = HashMap::new();
    let all_names: std::collections::HashSet<&str> = yaml_defaults
        .keys()
        .map(|s| s.as_str())
        .chain(global_map.keys().map(|s| s.as_str()))
        .chain(series_map.keys().map(|s| s.as_str()))
        .collect();

    for name in all_names {
        let score = series_map
            .get(name)
            .copied()
            .or_else(|| global_map.get(name).copied())
            .or_else(|| yaml_defaults.get(name).copied())
            .unwrap_or(0);
        result.insert(name.to_owned(), score);
    }
    Ok(result)
}

/// Get all per-series provider overrides for a manga (score + enabled).
pub async fn get_all_series_overrides(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<HashMap<String, (i32, bool)>, sqlx::Error> {
    let rows: Vec<(String, i32, i64)> = sqlx::query_as(
        "SELECT provider_name, score, enabled FROM Providers WHERE manga_id = ?",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, score, enabled)| (name, (score, enabled != 0)))
        .collect())
}
