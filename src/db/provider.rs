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
    pub provider_url: String,
    pub last_synced_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct MangaProviderRow {
    manga_id: String,
    provider_name: String,
    provider_url: String,
    last_synced_at: Option<DateTime<Utc>>,
}

fn from_row(row: MangaProviderRow) -> Result<MangaProvider, sqlx::Error> {
    let manga_id = Uuid::parse_str(&row.manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(MangaProvider {
        manga_id,
        provider_name: row.provider_name,
        provider_url: row.provider_url,
        last_synced_at: row.last_synced_at,
    })
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Insert or replace a MangaProvider entry.
pub async fn upsert(pool: &SqlitePool, entry: &MangaProvider) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR REPLACE INTO MangaProvider (manga_id, provider_name, provider_url, last_synced_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(entry.manga_id.to_string())
    .bind(&entry.provider_name)
    .bind(&entry.provider_url)
    .bind(entry.last_synced_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch all provider entries for a manga.
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<MangaProvider>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MangaProviderRow>(
        "SELECT manga_id, provider_name, provider_url, last_synced_at
         FROM MangaProvider WHERE manga_id = ?",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(from_row).collect()
}

/// Check whether a provider entry already exists for this (manga, provider) pair.
pub async fn exists(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM MangaProvider WHERE manga_id = ? AND provider_name = ?",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}
