use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::settings;

/// Record a provider failure with timestamp and error message.
pub async fn record(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
    error_message: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO ProviderFailure (provider_name, manga_id, failed_at, error_message)
         VALUES (?, ?, ?, ?)",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .bind(now)
    .bind(error_message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the number of consecutive failures for a provider + manga pair.
/// Counts failures since the last successful sync (from MangaProvider.last_synced_at).
/// If the provider was never synced, counts all failures.
pub async fn consecutive_failures(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
    backoff_minutes: i64,
) -> Result<i64, sqlx::Error> {
    let cutoff = Utc::now().timestamp() - (backoff_minutes * 60);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ProviderFailure
         WHERE provider_name = ? AND manga_id = ? AND failed_at >= ?",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .bind(cutoff)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Check if a provider should be auto-disabled for a specific manga.
/// Returns true if consecutive failures >= threshold.
pub async fn is_auto_disabled(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let threshold: i64 = settings::get(pool, "provider_disable_threshold", "5")
        .await
        .unwrap_or_else(|_| "5".to_string())
        .parse()
        .unwrap_or(5);
    let backoff: i64 = settings::get(pool, "provider_backoff_minutes", "60")
        .await
        .unwrap_or_else(|_| "60".to_string())
        .parse()
        .unwrap_or(60);

    let failures = consecutive_failures(pool, provider_name, manga_id, backoff).await?;
    Ok(failures >= threshold)
}

/// Get provider failure stats across all manga for a provider.
/// Returns (total_recent_failures, unique_manga_affected, oldest_failure_timestamp).
pub async fn provider_stats(
    pool: &SqlitePool,
    provider_name: &str,
    backoff_minutes: i64,
) -> Result<(i64, i64, Option<i64>), sqlx::Error> {
    let cutoff = Utc::now().timestamp() - (backoff_minutes * 60);
    let row: (i64, i64, Option<i64>) = sqlx::query_as(
        "SELECT
             COUNT(*) as total_failures,
             COUNT(DISTINCT manga_id) as manga_affected,
             MIN(failed_at) as oldest_failure
         FROM ProviderFailure
         WHERE provider_name = ? AND failed_at >= ?",
    )
    .bind(provider_name)
    .bind(cutoff)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Clear failure records older than the backoff period for a provider + manga.
/// Called after a successful operation to reset the failure counter.
pub async fn clear_for_manga(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM ProviderFailure WHERE provider_name = ? AND manga_id = ?",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Clear all failure records older than the backoff period.
/// Returns number of rows deleted.
pub async fn cleanup_old(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let backoff: i64 = settings::get(pool, "provider_backoff_minutes", "60")
        .await
        .unwrap_or_else(|_| "60".to_string())
        .parse()
        .unwrap_or(60);
    let cutoff = Utc::now().timestamp() - (backoff * 60);
    let result = sqlx::query("DELETE FROM ProviderFailure WHERE failed_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Get failure info for display in the dashboard.
/// Returns (consecutive_failures, last_error, last_failure_at).
pub async fn failure_info(
    pool: &SqlitePool,
    provider_name: &str,
    manga_id: Uuid,
    backoff_minutes: i64,
) -> Result<(i64, Option<String>, Option<i64>), sqlx::Error> {
    let cutoff = Utc::now().timestamp() - (backoff_minutes * 60);
    let row: (i64, Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT
             COUNT(*) as failure_count,
             (SELECT error_message FROM ProviderFailure
              WHERE provider_name = ? AND manga_id = ? AND failed_at >= ?
              ORDER BY failed_at DESC LIMIT 1) as last_error,
             MAX(failed_at) as last_failure_at
         FROM ProviderFailure
         WHERE provider_name = ? AND manga_id = ? AND failed_at >= ?",
    )
    .bind(provider_name)
    .bind(manga_id.to_string())
    .bind(cutoff)
    .bind(provider_name)
    .bind(manga_id.to_string())
    .bind(cutoff)
    .fetch_one(pool)
    .await?;
    Ok(row)
}