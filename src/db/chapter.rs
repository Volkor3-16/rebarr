use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::{Chapter, DownloadStatus};
use crate::scraper::ProviderChapterInfo;

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ChapterRow {
    uuid: String,
    manga_id: String,
    number_raw: String,
    number_sort: f64,
    title: Option<String>,
    volume: Option<i64>,
    scanlator_group: Option<String>,
    download_status: String,
    downloaded_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn download_status_str(s: &DownloadStatus) -> &'static str {
    match s {
        DownloadStatus::Missing => "Missing",
        DownloadStatus::Downloading => "Downloading",
        DownloadStatus::Downloaded => "Downloaded",
        DownloadStatus::Failed => "Failed",
    }
}

fn chapter_from_row(row: ChapterRow) -> Result<Chapter, sqlx::Error> {
    let id = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let manga_id =
        Uuid::parse_str(&row.manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    let download_status = match row.download_status.as_str() {
        "Downloading" => DownloadStatus::Downloading,
        "Downloaded" => DownloadStatus::Downloaded,
        "Failed" => DownloadStatus::Failed,
        _ => DownloadStatus::Missing,
    };

    Ok(Chapter {
        id,
        manga_id,
        number_raw: row.number_raw,
        number_sort: row.number_sort as f32,
        title: row.title,
        volume: row.volume.map(|v| v as u32),
        scanlator_group: row.scanlator_group,
        download_status,
        downloaded_at: row.downloaded_at,
    })
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Insert a new chapter record. Does not overwrite an existing row.
pub async fn insert(pool: &SqlitePool, chapter: &Chapter) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO Chapter (uuid, manga_id, number_raw, number_sort, title, volume,
                              scanlator_group, download_status, downloaded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chapter.id.to_string())
    .bind(chapter.manga_id.to_string())
    .bind(&chapter.number_raw)
    .bind(chapter.number_sort as f64)
    .bind(&chapter.title)
    .bind(chapter.volume.map(|v| v as i64))
    .bind(&chapter.scanlator_group)
    .bind(download_status_str(&chapter.download_status))
    .bind(chapter.downloaded_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert chapters from a provider scrape:
/// - New chapters are inserted with status `Missing`.
/// - Existing chapters are NOT updated — preserving `Downloaded` status.
pub async fn upsert_from_scrape(
    pool: &SqlitePool,
    manga_id: Uuid,
    infos: &[ProviderChapterInfo],
) -> Result<usize, sqlx::Error> {
    let manga_id_str = manga_id.to_string();
    let mut inserted = 0usize;

    for info in infos {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO Chapter
                (uuid, manga_id, number_raw, number_sort, title, volume, scanlator_group,
                 download_status, downloaded_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'Missing', NULL)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&manga_id_str)
        .bind(&info.raw_number)
        .bind(info.number as f64)
        .bind(&info.title)
        .bind(info.volume.map(|v| v as i64))
        .bind(&info.scanlator_group)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            inserted += 1;
        }
    }

    Ok(inserted)
}

/// Fetch all chapters for a manga, sorted by chapter number ascending.
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<Chapter>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, number_raw, number_sort, title, volume,
                scanlator_group, download_status, downloaded_at
         FROM Chapter
         WHERE manga_id = ?
         ORDER BY number_sort ASC",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(chapter_from_row).collect()
}

/// Find a single chapter by its manga_id and approximate chapter number.
pub async fn get_by_number(
    pool: &SqlitePool,
    manga_id: Uuid,
    number_sort: f32,
) -> Result<Option<Chapter>, sqlx::Error> {
    // Use ABS(number_sort - ?) < 0.01 to handle float imprecision
    let row = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, number_raw, number_sort, title, volume,
                scanlator_group, download_status, downloaded_at
         FROM Chapter
         WHERE manga_id = ? AND ABS(number_sort - ?) < 0.01
         LIMIT 1",
    )
    .bind(manga_id.to_string())
    .bind(number_sort as f64)
    .fetch_optional(pool)
    .await?;

    row.map(chapter_from_row).transpose()
}

/// Get a chapter by its UUID.
pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Chapter>, sqlx::Error> {
    let row = sqlx::query_as::<_, ChapterRow>(
        "SELECT uuid, manga_id, number_raw, number_sort, title, volume,
                scanlator_group, download_status, downloaded_at
         FROM Chapter WHERE uuid = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(chapter_from_row).transpose()
}

/// Update the download status (and optional downloaded_at timestamp) for a chapter.
pub async fn set_status(
    pool: &SqlitePool,
    chapter_id: Uuid,
    status: DownloadStatus,
    downloaded_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE Chapter SET download_status = ?, downloaded_at = ? WHERE uuid = ?",
    )
    .bind(download_status_str(&status))
    .bind(downloaded_at)
    .bind(chapter_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Recompute chapter_count (max number_sort) and downloaded_count for a manga
/// and write the results back to the Manga table.
pub async fn update_manga_counts(pool: &SqlitePool, manga_id: Uuid) -> Result<(), sqlx::Error> {
    let manga_id_str = manga_id.to_string();

    sqlx::query(
        "UPDATE Manga SET
            chapter_count    = (SELECT CAST(MAX(number_sort) AS INTEGER) FROM Chapter WHERE manga_id = ?),
            downloaded_count = (SELECT COUNT(*) FROM Chapter WHERE manga_id = ? AND download_status = 'Downloaded')
         WHERE uuid = ?",
    )
    .bind(&manga_id_str)
    .bind(&manga_id_str)
    .bind(&manga_id_str)
    .execute(pool)
    .await?;
    Ok(())
}
