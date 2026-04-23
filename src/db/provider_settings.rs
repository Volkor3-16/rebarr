use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Global (provider-wide) enable/disable
// ---------------------------------------------------------------------------

/// Returns the names of all providers that are globally disabled.
pub async fn get_globally_disabled(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT provider_name FROM ProviderSettings WHERE manga_id IS NULL AND enabled = 0",
    )
    .fetch_all(pool)
    .await
}

/// Returns whether the provider is globally enabled (default: true when no row exists).
pub async fn get_global_enabled(pool: &SqlitePool, name: &str) -> Result<bool, sqlx::Error> {
    let val: Option<bool> = sqlx::query_scalar(
        "SELECT enabled FROM ProviderSettings WHERE provider_name = ? AND manga_id IS NULL",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(val.unwrap_or(true))
}

/// Upsert the global enabled flag for a provider.
pub async fn set_global_enabled(
    pool: &SqlitePool,
    name: &str,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ProviderSettings (provider_name, manga_id, enabled)
         VALUES (?, NULL, ?)
         ON CONFLICT (provider_name, manga_id) DO UPDATE SET enabled = excluded.enabled",
    )
    .bind(name)
    .bind(enabled)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-series (per-manga) enable/disable overrides
// ---------------------------------------------------------------------------

/// Returns the per-series enabled override for a provider+manga pair.
/// `None` means no override is set — the global setting applies.
pub async fn get_series_enabled(
    pool: &SqlitePool,
    name: &str,
    manga_id: Uuid,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT enabled FROM ProviderSettings WHERE provider_name = ? AND manga_id = ?",
    )
    .bind(name)
    .bind(manga_id.to_string())
    .fetch_optional(pool)
    .await
}

/// Returns the effective enabled status for a provider+manga pair:
/// series override > global setting > true (default).
pub async fn get_effective_enabled(
    pool: &SqlitePool,
    name: &str,
    manga_id: Uuid,
) -> Result<bool, sqlx::Error> {
    if let Some(series_val) = get_series_enabled(pool, name, manga_id).await? {
        return Ok(series_val);
    }
    get_global_enabled(pool, name).await
}

/// Upsert the per-series enabled override.
pub async fn set_series_enabled(
    pool: &SqlitePool,
    name: &str,
    manga_id: Uuid,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ProviderSettings (provider_name, manga_id, enabled)
         VALUES (?, ?, ?)
         ON CONFLICT (provider_name, manga_id) DO UPDATE SET enabled = excluded.enabled",
    )
    .bind(name)
    .bind(manga_id.to_string())
    .bind(enabled)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete the per-series override, reverting to the global setting.
pub async fn delete_series_setting(
    pool: &SqlitePool,
    name: &str,
    manga_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM ProviderSettings WHERE provider_name = ? AND manga_id = ?")
        .bind(name)
        .bind(manga_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Returns all per-series enabled overrides for a manga as a map of provider_name → enabled.
pub async fn get_all_series_overrides(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<HashMap<String, bool>, sqlx::Error> {
    let rows: Vec<(String, bool)> =
        sqlx::query_as("SELECT provider_name, enabled FROM ProviderSettings WHERE manga_id = ?")
            .bind(manga_id.to_string())
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Provider version tracking (for automatic re-sync on version bump)
// ---------------------------------------------------------------------------

/// Get the stored version for a provider. Returns `None` if this provider has
/// never been recorded (i.e. first time it's been seen since this feature landed).
pub async fn get_version(pool: &SqlitePool, name: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT version FROM ProviderVersion WHERE provider_name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// Persist (or update) the current version for a provider.
pub async fn set_version(
    pool: &SqlitePool,
    name: &str,
    version: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ProviderVersion (provider_name, version, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT (provider_name) DO UPDATE SET version = excluded.version, updated_at = excluded.updated_at",
    )
    .bind(name)
    .bind(version)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}
