use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// Cache of "this manga lives at this URL on this provider".
/// Stored in the MangaProvider table to avoid re-searching on every sync.
#[derive(Debug, Clone)]
pub struct MangaProvider {
    pub manga_id: Uuid,
    pub provider_name: String,
    /// URL for this manga on the provider. `None` means we searched but found nothing.
    pub provider_url: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub search_attempted_at: Option<DateTime<Utc>>,
    // NOTE: provider_score and score_override remain in the DB schema but are no longer used.
    // Scoring is now done per-chapter using scanlator_group tiers.
}

impl MangaProvider {
    /// Whether this provider successfully found the manga.
    pub fn found(&self) -> bool {
        self.provider_url.is_some()
    }
}

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct MangaProviderRow {
    manga_id: String,
    provider_name: String,
    provider_url: Option<String>,
    last_synced_at: Option<DateTime<Utc>>,
    search_attempted_at: Option<DateTime<Utc>>,
}

fn from_row(row: MangaProviderRow) -> Result<MangaProvider, sqlx::Error> {
    let manga_id = Uuid::parse_str(&row.manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(MangaProvider {
        manga_id,
        provider_name: row.provider_name,
        provider_url: row.provider_url,
        last_synced_at: row.last_synced_at,
        search_attempted_at: row.search_attempted_at,
    })
}

// ---------------------------------------------------------------------------
// Public functions — MangaProvider
// ---------------------------------------------------------------------------

/// Insert or update a MangaProvider entry when the manga was found on a provider.
/// On conflict (manga_id, provider_name) only `provider_url`, `last_synced_at`,
/// and `search_attempted_at` are updated — scores are preserved (legacy columns).
pub async fn upsert(pool: &SqlitePool, entry: &MangaProvider) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO MangaProvider
             (manga_id, provider_name, provider_url, last_synced_at, provider_score, score_override, search_attempted_at)
         VALUES (?, ?, ?, ?, 0, NULL, ?)
         ON CONFLICT(manga_id, provider_name) DO UPDATE SET
             provider_url         = excluded.provider_url,
             last_synced_at       = excluded.last_synced_at,
             search_attempted_at  = excluded.search_attempted_at",
    )
    .bind(entry.manga_id.to_string())
    .bind(&entry.provider_name)
    .bind(&entry.provider_url)
    .bind(entry.last_synced_at)
    .bind(entry.search_attempted_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record that we searched this provider and found nothing.
/// Does not overwrite a record that already has a found URL.
pub async fn upsert_not_found(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO MangaProvider
             (manga_id, provider_name, provider_url, last_synced_at, provider_score, score_override, search_attempted_at)
         VALUES (?, ?, NULL, NULL, 0, NULL, ?)
         ON CONFLICT(manga_id, provider_name) DO UPDATE SET
             search_attempted_at = excluded.search_attempted_at
         WHERE provider_url IS NULL",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch all provider entries for a manga (including not-found ones).
/// Found entries sort first, then alphabetically.
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<MangaProvider>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MangaProviderRow>(
        "SELECT manga_id, provider_name, provider_url, last_synced_at, search_attempted_at
         FROM MangaProvider WHERE manga_id = ?
         ORDER BY
             CASE WHEN provider_url IS NOT NULL THEN 0 ELSE 1 END,
             provider_name",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(from_row).collect()
}

/// Check whether a valid (found) provider URL is already cached for this pair.
/// Returns `false` if no record exists OR if the existing record has url=NULL.
pub async fn has_url(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM MangaProvider
         WHERE manga_id = ? AND provider_name = ? AND provider_url IS NOT NULL",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Public functions — TrustedGroup
// ---------------------------------------------------------------------------

/// Fetch all trusted scanlation group names.
pub async fn get_trusted_groups(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM TrustedGroup ORDER BY name COLLATE NOCASE")
        .fetch_all(pool)
        .await
}

/// Add a name to the trusted group list. No-op if already present.
pub async fn add_trusted_group(pool: &SqlitePool, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO TrustedGroup (name) VALUES (?)")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a name from the trusted group list. No-op if not present.
pub async fn remove_trusted_group(pool: &SqlitePool, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM TrustedGroup WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}
