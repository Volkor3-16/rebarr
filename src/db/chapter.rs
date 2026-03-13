use chrono::{DateTime, TimeZone, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::manga::{Chapter, DownloadStatus};
use crate::manga::scoring::compute_tier;
use crate::scraper::ProviderChapterInfo;

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ChapterRow {
    uuid: String,
    manga_id: String,
    chapter_base: i64,
    chapter_variant: i64,
    title: Option<String>,
    language: String,
    scanlator_group: Option<String>,
    provider_name: Option<String>,
    chapter_url: Option<String>,
    download_status: String,
    released_at: Option<i64>,
    downloaded_at: Option<i64>,
    scraped_at: Option<i64>,
}

/// Converts unix timestamp to datetime object
fn ts_to_dt(secs: Option<i64>) -> Option<DateTime<Utc>> {
    secs.and_then(|s| Utc.timestamp_opt(s, 0).single())
}

/// Converts datetime object to unix timestamp
fn dt_to_ts(dt: Option<DateTime<Utc>>) -> Option<i64> {
    dt.map(|d| d.timestamp())
}

fn chapter_from_row(row: ChapterRow) -> Result<Chapter, sqlx::Error> {
    let id = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let manga_id = Uuid::parse_str(&row.manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let download_status = match row.download_status.as_str() {
        "Downloading" => DownloadStatus::Downloading,
        "Downloaded" => DownloadStatus::Downloaded,
        "Failed" => DownloadStatus::Failed,
        _ => DownloadStatus::Missing,
    };
    Ok(Chapter {
        id,
        manga_id,
        chapter_base: row.chapter_base as i32,
        chapter_variant: row.chapter_variant as i32,
        title: row.title,
        language: row.language,
        scanlator_group: row.scanlator_group,
        provider_name: row.provider_name,
        chapter_url: row.chapter_url,
        download_status,
        released_at: ts_to_dt(row.released_at),
        downloaded_at: ts_to_dt(row.downloaded_at),
        scraped_at: ts_to_dt(row.scraped_at),
    })
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Upsert chapters from a provider scrape into the new Chapters table.
/// - New rows are inserted with status `Missing`.
/// - Existing rows are updated (scraped_at, chapter_url, title/scanlator_group back-filled if missing).
/// Returns UUIDs of newly inserted rows.
pub async fn upsert_from_scrape(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
    infos: &[ProviderChapterInfo],
) -> Result<Vec<Uuid>, sqlx::Error> {
    let manga_id_str = manga_id.to_string();
    let now = Utc::now().timestamp();
    let mut new_ids = Vec::new();

    for info in infos {
        let new_id = Uuid::new_v4();
        let language = info.language.as_deref().unwrap_or("EN").to_uppercase();
        let released_at = info.date_released;

        let result = sqlx::query(
            "INSERT INTO Chapters
                (uuid, manga_id, chapter_base, chapter_variant, title, language,
                 scanlator_group, provider_name, chapter_url, download_status, released_at, scraped_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Missing', ?, ?)
             ON CONFLICT(manga_id, chapter_base, chapter_variant, language, provider_name)
             DO UPDATE SET
                 scraped_at       = excluded.scraped_at,
                 chapter_url      = excluded.chapter_url,
                 title            = COALESCE(excluded.title, Chapters.title),
                 scanlator_group  = COALESCE(excluded.scanlator_group, Chapters.scanlator_group)",
        )
        .bind(new_id.to_string())
        .bind(&manga_id_str)
        .bind(info.chapter_base as i64)
        .bind(info.chapter_variant as i64)
        .bind(&info.title)
        .bind(&language)
        .bind(&info.scanlator_group)
        .bind(provider_name)
        .bind(&info.url)
        .bind(released_at)
        .bind(now)
        .execute(pool)
        .await?;

        // rows_affected > 1 means an update happened; == 1 means a new insert
        if result.rows_affected() == 1 {
            // Check if the row we just touched is the new_id we generated
            // (i.e., it was a fresh insert, not an update of existing row)
            let exists: Option<String> = sqlx::query_scalar(
                "SELECT uuid FROM Chapters WHERE uuid = ?",
            )
            .bind(new_id.to_string())
            .fetch_optional(pool)
            .await?;

            if exists.is_some() {
                new_ids.push(new_id);
            }
        }
    }

    Ok(new_ids)
}

/// Get all Chapters rows for a manga, ordered by chapter_base ASC, chapter_variant ASC.
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<Chapter>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, chapter_base, chapter_variant, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at
         FROM Chapters
         WHERE manga_id = ?
         ORDER BY chapter_base ASC, chapter_variant ASC",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(chapter_from_row).collect()
}

/// Get all Chapters rows for a specific chapter number (all providers).
pub async fn get_all_for_chapter(
    pool: &SqlitePool,
    manga_id: Uuid,
    chapter_base: i32,
    chapter_variant: i32,
) -> Result<Vec<Chapter>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, chapter_base, chapter_variant, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at
         FROM Chapters
         WHERE manga_id = ? AND chapter_base = ? AND chapter_variant = ?",
    )
    .bind(manga_id.to_string())
    .bind(chapter_base as i64)
    .bind(chapter_variant as i64)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(chapter_from_row).collect()
}

/// Get a chapter by UUID.
pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Chapter>, sqlx::Error> {
    let row = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, chapter_base, chapter_variant, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at
         FROM Chapters WHERE uuid = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(chapter_from_row).transpose()
}

/// Get the canonical list of chapter UUIDs for a manga (from CanonicalChapters).
/// Returns an empty Vec if no canonical entry exists yet.
async fn get_canonical_uuids(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT canonical_list FROM CanonicalChapters WHERE manga_id = ?",
    )
    .bind(manga_id.to_string())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(json) => Ok(serde_json::from_str::<Vec<String>>(&json).unwrap_or_default()),
        None => Ok(Vec::new()),
    }
}

/// Fetch canonical Chapter rows for a manga (the scored winners).
pub async fn get_canonical_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<Chapter>, sqlx::Error> {
    let uuids = get_canonical_uuids(pool, manga_id).await?;
    if uuids.is_empty() {
        return Ok(Vec::new());
    }

    // Build a query with the right number of placeholders
    let placeholders: String = uuids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT uuid, manga_id, chapter_base, chapter_variant, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at
         FROM Chapters
         WHERE uuid IN ({placeholders})
         ORDER BY chapter_base ASC, chapter_variant ASC"
    );

    let mut q = sqlx::query_as::<_, ChapterRow>(&sql);
    for uuid in &uuids {
        q = q.bind(uuid);
    }
    let rows = q.fetch_all(pool).await?;
    rows.into_iter().map(chapter_from_row).collect()
}

/// Get the canonical chapter for a specific chapter number.
pub async fn get_canonical_by_number(
    pool: &SqlitePool,
    manga_id: Uuid,
    chapter_base: i32,
    chapter_variant: i32,
) -> Result<Option<Chapter>, sqlx::Error> {
    let uuids = get_canonical_uuids(pool, manga_id).await?;
    if uuids.is_empty() {
        return Ok(None);
    }

    let placeholders: String = uuids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT uuid, manga_id, chapter_base, chapter_variant, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at
         FROM Chapters
         WHERE uuid IN ({placeholders})
           AND chapter_base = ?
           AND chapter_variant = ?
         LIMIT 1"
    );

    let mut q = sqlx::query_as::<_, ChapterRow>(&sql);
    for uuid in &uuids {
        q = q.bind(uuid);
    }
    q = q.bind(chapter_base as i64).bind(chapter_variant as i64);
    let row = q.fetch_optional(pool).await?;
    row.map(chapter_from_row).transpose()
}

/// Update download_status (and optionally downloaded_at) for a chapter.
pub async fn set_status(
    pool: &SqlitePool,
    chapter_id: Uuid,
    status: DownloadStatus,
    downloaded_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE Chapters SET download_status = ?, downloaded_at = ? WHERE uuid = ?",
    )
    .bind(status.as_str())
    .bind(dt_to_ts(downloaded_at))
    .bind(chapter_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Score all Chapters rows for a manga, pick one winner per (chapter_base, chapter_variant),
/// write the result to CanonicalChapters, and update chapter_count/downloaded_count on Manga.
///
/// `preferred_language`: pass the user's preferred language (e.g. "en") or empty string for none.
pub async fn update_canonical(
    pool: &SqlitePool,
    manga_id: Uuid,
    trusted_groups: &[String],
    preferred_language: &str,
) -> Result<(), sqlx::Error> {
    let all = get_all_for_manga(pool, manga_id).await?;

    // Group by (chapter_base, chapter_variant)
    let mut groups: std::collections::BTreeMap<(i32, i32), Vec<Chapter>> =
        std::collections::BTreeMap::new();
    for ch in all {
        groups
            .entry((ch.chapter_base, ch.chapter_variant))
            .or_default()
            .push(ch);
    }

    let mut canonical_uuids: Vec<String> = Vec::with_capacity(groups.len());

    for (_, mut entries) in groups {
        // Language filter: prefer matching language but fall back to all
        if !preferred_language.is_empty() {
            let lang_filtered: Vec<_> = entries
                .iter()
                .filter(|e| e.language.eq_ignore_ascii_case(preferred_language))
                .cloned()
                .collect();
            if !lang_filtered.is_empty() {
                entries = lang_filtered;
            }
        }

        // Tier sort: lower tier number = better (1=Official, 4=No group)
        entries.sort_by_key(|e| compute_tier(e.scanlator_group.as_deref(), trusted_groups));

        if let Some(winner) = entries.into_iter().next() {
            canonical_uuids.push(winner.id.to_string());
        }
    }

    let json = serde_json::to_string(&canonical_uuids)
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    sqlx::query(
        "INSERT OR REPLACE INTO CanonicalChapters (manga_id, canonical_list, last_updated)
         VALUES (?, ?, unixepoch())",
    )
    .bind(manga_id.to_string())
    .bind(&json)
    .execute(pool)
    .await?;

    update_manga_counts(pool, manga_id).await
}

/// Recompute chapter_count and downloaded_count from canonical chapters and write to Manga.
pub async fn update_manga_counts(pool: &SqlitePool, manga_id: Uuid) -> Result<(), sqlx::Error> {
    let uuids = get_canonical_uuids(pool, manga_id).await?;

    let (chapter_count, downloaded_count) = if uuids.is_empty() {
        (0i64, 0i64)
    } else {
        let placeholders: String = uuids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let count_sql = format!(
            "SELECT COUNT(*), SUM(CASE WHEN download_status = 'Downloaded' THEN 1 ELSE 0 END)
             FROM Chapters WHERE uuid IN ({placeholders})"
        );
        let mut q = sqlx::query_as::<_, (i64, i64)>(&count_sql);
        for uuid in &uuids {
            q = q.bind(uuid);
        }
        q.fetch_one(pool).await?
    };

    sqlx::query(
        "UPDATE Manga SET chapter_count = ?, downloaded_count = ? WHERE uuid = ?",
    )
    .bind(chapter_count)
    .bind(downloaded_count)
    .bind(manga_id.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

/// Insert a new chapter row directly (used by disk scanner for manually-found CBZ files).
pub async fn insert(pool: &SqlitePool, chapter: &Chapter) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO Chapters
            (uuid, manga_id, chapter_base, chapter_variant, title, language,
             scanlator_group, provider_name, chapter_url, download_status,
             released_at, downloaded_at, scraped_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chapter.id.to_string())
    .bind(chapter.manga_id.to_string())
    .bind(chapter.chapter_base as i64)
    .bind(chapter.chapter_variant as i64)
    .bind(&chapter.title)
    .bind(&chapter.language)
    .bind(&chapter.scanlator_group)
    .bind(&chapter.provider_name)
    .bind(&chapter.chapter_url)
    .bind(chapter.download_status.as_str())
    .bind(dt_to_ts(chapter.released_at))
    .bind(dt_to_ts(chapter.downloaded_at))
    .bind(dt_to_ts(chapter.scraped_at))
    .execute(pool)
    .await?;
    Ok(())
}
