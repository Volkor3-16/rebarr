use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::manga::{Manga, MangaMetadata, MangaSource, PublishingStatus};

/// Flat DB row — matches Manga table columns exactly.
#[derive(sqlx::FromRow)]
struct MangaRow {
    uuid: String,
    library_id: String,
    anilist_id: Option<i64>,
    mal_id: Option<i64>,
    relative_path: String,
    title: String,
    title_og: String,
    title_roman: String,
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
            title_og: row.title_og,
            title_roman: row.title_roman,
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

    sqlx::query(
        r#"INSERT INTO Manga (
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, title_og, title_roman, synopsis, publishing_status,
            start_year, end_year, chapter_count, downloaded_count,
            metadata_source, thumbnail_url, monitored, created_at, metadata_updated_at
        ) VALUES (
            ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?,
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
    .bind(&manga.metadata.title_og)
    .bind(&manga.metadata.title_roman)
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
            title, title_og, title_roman, synopsis, publishing_status,
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
            title, title_og, title_roman, synopsis, publishing_status,
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

pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM Manga WHERE uuid = ?")
        .bind(id.to_string())
        .execute(pool)
        .await?;
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

    sqlx::query(
        r#"UPDATE Manga SET
            title = ?, title_og = ?, title_roman = ?, synopsis = ?,
            publishing_status = ?, start_year = ?, end_year = ?,
            metadata_source = ?, thumbnail_url = ?,
            anilist_id = ?, mal_id = ?,
            metadata_updated_at = ?
         WHERE uuid = ?"#,
    )
    .bind(&manga.metadata.title)
    .bind(&manga.metadata.title_og)
    .bind(&manga.metadata.title_roman)
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

/// Fetch all monitored manga across all libraries, each with their tags.
pub async fn get_all_monitored(pool: &SqlitePool) -> Result<Vec<Manga>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MangaRow>(
        r#"SELECT
            uuid, library_id, anilist_id, mal_id, relative_path,
            title, title_og, title_roman, synopsis, publishing_status,
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
