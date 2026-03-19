use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::manga::{Manga, MangaMetadata, MangaSource, PublishingStatus};

// ---------------------------------------------------------------------------
// Deterministic UUID
// ---------------------------------------------------------------------------

/// Fixed namespace for manga UUID v5 derivation.
const MANGA_NAMESPACE: Uuid = Uuid::from_bytes([
    0xc2, 0x7a, 0x5f, 0x91, 0x03, 0xe8, 0x4b, 0x20,
    0xb1, 0x6d, 0x00, 0xd4, 0x8e, 0x2f, 0x73, 0xa1,
]);

/// Compute the deterministic UUID for a manga tracked via AniList.
///
/// Key: the AniList ID — globally unique, same UUID across all Rebarr installs.
pub fn manga_uuid(anilist_id: u32) -> Uuid {
    Uuid::new_v5(&MANGA_NAMESPACE, anilist_id.to_string().as_bytes())
}

/// Compute the deterministic UUID for a manually-added manga (no AniList ID).
///
/// Key: `relative_path` only — library-agnostic, so the UUID survives moving
/// a manga between libraries.
pub fn manual_manga_uuid(relative_path: &str) -> Uuid {
    Uuid::new_v5(&MANGA_NAMESPACE, relative_path.as_bytes())
}

/// Flat DB row — matches Manga table columns exactly.
#[derive(sqlx::FromRow)]
struct MangaRow {
    uuid: String,
    library_id: String,
    anilist_id: Option<i64>,
    mal_id: Option<i64>,
    relative_path: String,
    title: String,
    other_titles: Option<String>,
    synopsis: Option<String>,
    publishing_status: String,
    start_year: Option<i32>,
    end_year: Option<i32>,
    chapter_count: Option<i64>,
    downloaded_count: Option<i64>,
    metadata_source: String,
    thumbnail_url: Option<String>,
    created_at: i64,
    metadata_updated_at: i64,
    monitored: bool,
}

/// Fetch tags for a single manga.
async fn fetch_tags(pool: &SqlitePool, manga_id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT tag FROM MangaTags WHERE manga_id = ? ORDER BY tag ASC")
        .bind(manga_id)
        .fetch_all(pool)
        .await
}

/// Fetch tags for all manga in a library in one query, grouped by manga UUID.
async fn fetch_tags_for_library(
    pool: &SqlitePool,
    library_id: &str,
) -> Result<HashMap<String, Vec<String>>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct TagRow {
        manga_id: String,
        tag: String,
    }

    let rows = sqlx::query_as::<_, TagRow>(
        "SELECT manga_id, tag FROM MangaTags
         WHERE manga_id IN (SELECT uuid FROM Manga WHERE library_id = ?)
         ORDER BY manga_id, tag ASC",
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        map.entry(row.manga_id).or_default().push(row.tag);
    }
    Ok(map)
}

/// Parse other_titles JSON string from DB into Option<Vec<String>>
fn parse_other_titles(json: Option<String>) -> Option<Vec<String>> {
    json.and_then(|s| serde_json::from_str(&s).ok())
}

fn manga_from_parts(row: MangaRow, tags: Vec<String>) -> Result<Manga, sqlx::Error> {
    let id = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let library_id =
        Uuid::parse_str(&row.library_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    let publishing_status = match row.publishing_status.as_str() {
        "Completed" => PublishingStatus::Completed,
        "Ongoing" => PublishingStatus::Ongoing,
        "Hiatus" => PublishingStatus::Hiatus,
        "Cancelled" => PublishingStatus::Cancelled,
        "NotYetReleased" => PublishingStatus::NotYetReleased,
        _ => PublishingStatus::Unknown,
    };

    let metadata_source = match row.metadata_source.as_str() {
        "AniList" => MangaSource::AniList,
        _ => MangaSource::Local,
    };

    let other_titles = parse_other_titles(row.other_titles);

    Ok(Manga {
        id,
        library_id,
        anilist_id: row.anilist_id.map(|v| v as u32),
        mal_id: row.mal_id.map(|v| v as u32),
        relative_path: PathBuf::from(row.relative_path),
        downloaded_count: row.downloaded_count.map(|v| v as i32),
        chapter_count: row.chapter_count.map(|v| v as u32),
        metadata_source,
        thumbnail_url: row.thumbnail_url,
        monitored: row.monitored,
        created_at: row.created_at,
        metadata_updated_at: row.metadata_updated_at,
        metadata: MangaMetadata {
            title: row.title,
            other_titles,
            synopsis: row.synopsis,
            publishing_status,
            tags,
            start_year: row.start_year,
            end_year: row.end_year,
        },
    })
}

fn publishing_status_str(s: &PublishingStatus) -> &'static str {
    match s {
        PublishingStatus::Completed => "Completed",
        PublishingStatus::Ongoing => "Ongoing",
        PublishingStatus::Hiatus => "Hiatus",
        PublishingStatus::Cancelled => "Cancelled",
        PublishingStatus::NotYetReleased => "NotYetReleased",
        PublishingStatus::Unknown => "Unknown",
    }
}

fn metadata_source_str(s: &MangaSource) -> &'static str {
    match s {
        MangaSource::AniList => "AniList",
        MangaSource::Local => "Local",
    }
}

// ---------------------------------------------------------------------------
// Public query functions
// ---------------------------------------------------------------------------

/// Serialize other_titles to JSON for storage in DB
fn serialize_other_titles(titles: &Option<Vec<String>>) -> Option<String> {
    titles.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default())
}

/// Insert a manga and all its tags in a single transaction.
pub async fn insert(pool: &SqlitePool, manga: &Manga) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let id = manga.id.to_string();
    let library_id = manga.library_id.to_string();
    let relative_path = manga.relative_path.to_string_lossy().to_string();
    let publishing_status = publishing_status_str(&manga.metadata.publishing_status);
    let metadata_source = metadata_source_str(&manga.metadata_source);
    let anilist_id = manga.anilist_id.map(|v| v as i64);
    let mal_id = manga.mal_id.map(|v| v as i64);
    let chapter_count = manga.chapter_count.map(|v| v as i64);
    let downloaded_count = manga.downloaded_count.map(|v| v as i64);
    let other_titles_json = serialize_other_titles(&manga.metadata.other_titles);

    sqlx::query(
        r#"INSERT INTO Manga (
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, other_titles, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        ) VALUES (
            ?, ?, ?, ?, ?,
            ?, ?, ?, ?,
            ?, ?, ?, ?,
            ?, ?, ?, ?, ?
        )"#,
    )
    .bind(&id)
    .bind(&library_id)
    .bind(anilist_id)
    .bind(mal_id)
    .bind(&relative_path)
    .bind(&manga.metadata.title)
    .bind(&other_titles_json)
    .bind(&manga.metadata.synopsis)
    .bind(publishing_status)
    .bind(manga.metadata.start_year)
    .bind(manga.metadata.end_year)
    .bind(chapter_count)
    .bind(downloaded_count)
    .bind(metadata_source)
    .bind(manga.thumbnail_url.as_deref())
    .bind(manga.monitored as i64)
    .bind(manga.created_at)
    .bind(manga.metadata_updated_at)
    .execute(&mut *tx)
    .await?;

    for tag in &manga.metadata.tags {
        sqlx::query("INSERT OR IGNORE INTO MangaTags (manga_id, tag) VALUES (?, ?)")
            .bind(&id)
            .bind(tag)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await
}

/// Fetch a single manga by UUID, including its tags.
pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Manga>, sqlx::Error> {
    let id_str = id.to_string();

    let row = sqlx::query_as::<_, MangaRow>(
        r#"SELECT
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, other_titles, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        FROM Manga WHERE uuid = ?"#,
    )
    .bind(&id_str)
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(row) => {
            let tags = fetch_tags(pool, &row.uuid).await?;
            manga_from_parts(row, tags).map(Some)
        }
    }
}

/// Fetch all manga in a library, each with their tags.
/// Uses two queries (manga + all tags) instead of N+1.
pub async fn get_all_for_library(
    pool: &SqlitePool,
    library_id: Uuid,
) -> Result<Vec<Manga>, sqlx::Error> {
    let lib_str = library_id.to_string();

    let rows = sqlx::query_as::<_, MangaRow>(
        r#"SELECT
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, other_titles, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        FROM Manga WHERE library_id = ? ORDER BY title ASC"#,
    )
    .bind(&lib_str)
    .fetch_all(pool)
    .await?;

    let mut tag_map = fetch_tags_for_library(pool, &lib_str).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = tag_map.remove(&row.uuid).unwrap_or_default();
        out.push(manga_from_parts(row, tags)?);
    }
    Ok(out)
}

/// Check if a manga with the given anilist_id or mal_id already exists in a library.
/// Returns the existing manga if found, None otherwise.
pub async fn exists_by_external_ids(
    pool: &SqlitePool,
    library_id: Uuid,
    anilist_id: Option<u32>,
    mal_id: Option<u32>,
) -> Result<Option<Manga>, sqlx::Error> {
    let lib_str = library_id.to_string();
    let al_id = anilist_id.map(|v| v as i64);
    let m_id = mal_id.map(|v| v as i64);

    // Only search if we have at least one external ID
    if al_id.is_none() && m_id.is_none() {
        return Ok(None);
    }

    let row = sqlx::query_as::<_, MangaRow>(
        r#"SELECT
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, other_titles, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        FROM Manga 
        WHERE library_id = ? 
          AND (anilist_id = ? OR mal_id = ?)"#,
    )
    .bind(&lib_str)
    .bind(al_id)
    .bind(m_id)
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(row) => {
            let tags = fetch_tags(pool, &row.uuid).await?;
            manga_from_parts(row, tags).map(Some)
        }
    }
}

pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    
    // Delete all chapters for this manga (also cleans up CanonicalChapters)
    sqlx::query("DELETE FROM Chapters WHERE manga_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    
    // Delete canonical chapters entry
    sqlx::query("DELETE FROM CanonicalChapters WHERE manga_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    
    // Delete all tags for this manga
    sqlx::query("DELETE FROM MangaTags WHERE manga_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    
    // Delete all provider records for this manga
    sqlx::query("DELETE FROM MangaProvider WHERE manga_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    
    // Delete the manga itself
    sqlx::query("DELETE FROM Manga WHERE uuid = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    
    tx.commit().await?;
    Ok(())
}

/// Update the monitored flag for a manga.
pub async fn set_monitored(pool: &SqlitePool, id: Uuid, monitored: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Manga SET monitored = ? WHERE uuid = ?")
        .bind(monitored as i64)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the mutable metadata fields for an existing manga record.
/// Tags are replaced atomically (delete old, insert new).
/// Does NOT touch library_id, relative_path, chapter_count, downloaded_count, or created_at.
pub async fn update_metadata(pool: &SqlitePool, manga: &Manga) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let id = manga.id.to_string();
    let publishing_status = publishing_status_str(&manga.metadata.publishing_status);
    let metadata_source = metadata_source_str(&manga.metadata_source);
    let other_titles_json = serialize_other_titles(&manga.metadata.other_titles);

    sqlx::query(
        r#"UPDATE Manga SET
            title = ?, other_titles = ?, synopsis = ?,
            publishing_status = ?, start_year = ?, end_year = ?,
            metadata_source = ?, thumbnail_url = ?,
            anilist_id = ?, mal_id = ?,
            metadata_updated_at = ?
         WHERE uuid = ?"#,
    )
    .bind(&manga.metadata.title)
    .bind(&other_titles_json)
    .bind(&manga.metadata.synopsis)
    .bind(publishing_status)
    .bind(manga.metadata.start_year)
    .bind(manga.metadata.end_year)
    .bind(metadata_source)
    .bind(manga.thumbnail_url.as_deref())
    .bind(manga.anilist_id.map(|v| v as i64))
    .bind(manga.mal_id.map(|v| v as i64))
    .bind(manga.metadata_updated_at)
    .bind(&id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM MangaTags WHERE manga_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;

    for tag in &manga.metadata.tags {
        sqlx::query("INSERT OR IGNORE INTO MangaTags (manga_id, tag) VALUES (?, ?)")
            .bind(&id)
            .bind(tag)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await
}

// ---------------------------------------------------------------------------
// Data migration
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct MangaKeyRow {
    uuid: String,
    anilist_id: Option<i64>,
    relative_path: String,
}

/// One-time startup migration: recompute all manga UUIDs deterministically.
///
/// Idempotent — skips if `DataMigrations` already records
/// `deterministic_manga_uuids_v1`. Updates all FK tables that reference
/// `Manga.uuid`: Chapters, CanonicalChapters, MangaTags, MangaProvider, Task.
///
/// Returns `true` if any UUIDs were changed (caller should re-run chapter
/// backfill in that case).
pub async fn backfill_deterministic_uuids(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let already_ran: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM DataMigrations WHERE name = 'deterministic_manga_uuids_v1'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    if already_ran > 0 {
        return Ok(false);
    }

    let rows: Vec<MangaKeyRow> = sqlx::query_as::<_, MangaKeyRow>(
        "SELECT uuid, anilist_id, relative_path FROM Manga",
    )
    .fetch_all(pool)
    .await?;

    let mut any_changed = false;
    let mut tx = pool.begin().await?;

    sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *tx).await?;

    for row in &rows {
        let new_id = if let Some(al_id) = row.anilist_id {
            manga_uuid(al_id as u32)
        } else {
            manual_manga_uuid(&row.relative_path)
        };

        let new_id_str = new_id.to_string();

        if new_id_str == row.uuid {
            continue;
        }

        let collision: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM Manga WHERE uuid = ?")
                .bind(&new_id_str)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or(0);

        if collision > 0 {
            log::warn!("[backfill] Manga UUID collision {} → {} — skipping.", row.uuid, new_id_str);
            continue;
        }

        sqlx::query("UPDATE Manga SET uuid = ? WHERE uuid = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE Chapters SET manga_id = ? WHERE manga_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE CanonicalChapters SET manga_id = ? WHERE manga_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE MangaTags SET manga_id = ? WHERE manga_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE MangaProvider SET manga_id = ? WHERE manga_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE Task SET manga_id = ? WHERE manga_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;

        any_changed = true;
    }

    sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *tx).await?;

    sqlx::query(
        "INSERT OR IGNORE INTO DataMigrations (name, ran_at) VALUES ('deterministic_manga_uuids_v1', unixepoch())",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    if any_changed {
        log::info!("[backfill] Backfilled deterministic UUIDs for {} manga.", rows.len());
    }

    Ok(any_changed)
}

/// Fetch all monitored manga across all libraries, each with their tags.
pub async fn get_all_monitored(pool: &SqlitePool) -> Result<Vec<Manga>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MangaRow>(
        r#"SELECT
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, other_titles, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        FROM Manga WHERE monitored = 1 ORDER BY title ASC"#,
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = fetch_tags(pool, &row.uuid).await?;
        out.push(manga_from_parts(row, tags)?);
    }
    Ok(out)
}
