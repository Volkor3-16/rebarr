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
    /// Auto-computed quality score (higher = better).
    /// Updated by `score_providers()` after each scan.
    /// Ignored when `score_override` is set.
    pub provider_score: f64,
    /// User-set override score. When `Some`, replaces `provider_score` for
    /// download provider selection and `score_providers()` skips this entry.
    pub score_override: Option<f64>,
}

impl MangaProvider {
    /// Effective score used for provider selection: override if set, else auto score.
    pub fn effective_score(&self) -> f64 {
        self.score_override.unwrap_or(self.provider_score)
    }
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
    provider_score: f64,
    score_override: Option<f64>,
}

fn from_row(row: MangaProviderRow) -> Result<MangaProvider, sqlx::Error> {
    let manga_id = Uuid::parse_str(&row.manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(MangaProvider {
        manga_id,
        provider_name: row.provider_name,
        provider_url: row.provider_url,
        last_synced_at: row.last_synced_at,
        provider_score: row.provider_score,
        score_override: row.score_override,
    })
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Insert or update a MangaProvider entry.
/// On conflict (manga_id, provider_name) only `provider_url` and
/// `last_synced_at` are updated — computed scores and overrides are preserved.
pub async fn upsert(pool: &SqlitePool, entry: &MangaProvider) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO MangaProvider
             (manga_id, provider_name, provider_url, last_synced_at, provider_score, score_override)
         VALUES (?, ?, ?, ?, 0, NULL)
         ON CONFLICT(manga_id, provider_name) DO UPDATE SET
             provider_url    = excluded.provider_url,
             last_synced_at  = excluded.last_synced_at",
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
/// Entries with a score override sort first (by override value desc), then
/// auto-scored entries (by provider_score desc).
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<MangaProvider>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MangaProviderRow>(
        "SELECT manga_id, provider_name, provider_url, last_synced_at, provider_score, score_override
         FROM MangaProvider WHERE manga_id = ?
         ORDER BY COALESCE(score_override, provider_score) DESC",
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

/// Compute and persist per-manga provider scores based on chapter coverage.
/// Skips providers that have a `score_override` set by the user.
///
/// Score = MAX(chapter_number_sort) + COUNT(*) / 1000.0
pub async fn score_providers(pool: &SqlitePool, manga_id: Uuid) -> Result<(), sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct ScoreRow {
        provider_name: String,
        max_chapter: Option<f64>,
        chapter_count: i64,
    }

    let rows = sqlx::query_as::<_, ScoreRow>(
        "SELECT mp.provider_name,
                MAX(pcu.chapter_number_sort) AS max_chapter,
                COUNT(pcu.chapter_number_sort) AS chapter_count
         FROM MangaProvider mp
         LEFT JOIN ProviderChapterUrl pcu
             ON pcu.manga_id = mp.manga_id AND pcu.provider_name = mp.provider_name
         WHERE mp.manga_id = ? AND mp.score_override IS NULL
         GROUP BY mp.provider_name",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    for row in rows {
        let Some(max_ch) = row.max_chapter else {
            continue; // no cached URLs yet
        };
        let score = max_ch + (row.chapter_count as f64 / 1000.0);

        sqlx::query(
            "UPDATE MangaProvider SET provider_score = ? WHERE manga_id = ? AND provider_name = ?",
        )
        .bind(score)
        .bind(manga_id.to_string())
        .bind(&row.provider_name)
        .execute(pool)
        .await?;

        log::debug!(
            "[score] {} → score {:.3} (max={}, count={})",
            row.provider_name,
            score,
            max_ch,
            row.chapter_count
        );
    }

    Ok(())
}

/// Set or clear a user score override for a specific provider/manga pair.
/// Pass `None` to clear the override and return to auto-scoring.
pub async fn set_score_override(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
    score_override: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE MangaProvider SET score_override = ? WHERE manga_id = ? AND provider_name = ?",
    )
    .bind(score_override)
    .bind(manga_id.to_string())
    .bind(provider_name)
    .execute(pool)
    .await?;
    Ok(())
}
