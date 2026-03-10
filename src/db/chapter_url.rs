use sqlx::SqlitePool;
use uuid::Uuid;

/// A provider's cached entry for a specific chapter.
#[derive(Debug, Clone)]
pub struct ChapterProviderEntry {
    pub chapter_number_sort: f32,
    pub provider_name: String,
    pub chapter_url: String,
    pub scanlator_group: Option<String>,
    /// The manga-level URL on this provider (from MangaProvider). Used for fallback re-scraping.
    pub manga_provider_url: Option<String>,
}

/// Cache the chapter page URL (and scanlator group) for a given manga + provider + chapter number.
pub async fn upsert(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
    number_sort: f32,
    url: &str,
    scanlator_group: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ProviderChapterUrl
             (manga_id, provider_name, chapter_number_sort, chapter_url, scanlator_group)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(manga_id, provider_name, chapter_number_sort) DO UPDATE SET
             chapter_url     = excluded.chapter_url,
             scanlator_group = excluded.scanlator_group",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .bind(number_sort as f64)
    .bind(url)
    .bind(scanlator_group)
    .execute(pool)
    .await?;
    Ok(())
}

/// Look up a cached chapter URL for a single provider. Returns None if not yet cached.
pub async fn get(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
    number_sort: f32,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT chapter_url FROM ProviderChapterUrl
         WHERE manga_id = ? AND provider_name = ? AND ABS(chapter_number_sort - ?) < 0.01
         LIMIT 1",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .bind(number_sort as f64)
    .fetch_optional(pool)
    .await
}

/// Row type shared by both per-chapter and per-manga queries.
#[derive(sqlx::FromRow)]
struct EntryRow {
    chapter_number_sort: f64,
    provider_name: String,
    chapter_url: String,
    scanlator_group: Option<String>,
    manga_provider_url: Option<String>,
}

fn from_entry_row(r: EntryRow) -> ChapterProviderEntry {
    ChapterProviderEntry {
        chapter_number_sort: r.chapter_number_sort as f32,
        provider_name: r.provider_name,
        chapter_url: r.chapter_url,
        scanlator_group: r.scanlator_group,
        manga_provider_url: r.manga_provider_url,
    }
}

/// Get all provider entries for a specific chapter, joined with MangaProvider for the manga URL.
/// Used by the downloader and by the chapter list API for per-chapter provider display.
pub async fn get_for_chapter(
    pool: &SqlitePool,
    manga_id: Uuid,
    number_sort: f32,
) -> Result<Vec<ChapterProviderEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EntryRow>(
        "SELECT pcu.chapter_number_sort, pcu.provider_name, pcu.chapter_url, pcu.scanlator_group,
                mp.provider_url AS manga_provider_url
         FROM ProviderChapterUrl pcu
         LEFT JOIN MangaProvider mp
             ON mp.manga_id = pcu.manga_id AND mp.provider_name = pcu.provider_name
         WHERE pcu.manga_id = ? AND ABS(pcu.chapter_number_sort - ?) < 0.01",
    )
    .bind(manga_id.to_string())
    .bind(number_sort as f64)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(from_entry_row).collect())
}

/// Get all ProviderChapterUrl entries for a manga, joined with MangaProvider for the manga URL.
/// Used by the chapter list API to build per-chapter provider info in a single query.
pub async fn get_all_for_manga(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<ChapterProviderEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EntryRow>(
        "SELECT pcu.chapter_number_sort, pcu.provider_name, pcu.chapter_url, pcu.scanlator_group,
                mp.provider_url AS manga_provider_url
         FROM ProviderChapterUrl pcu
         LEFT JOIN MangaProvider mp
             ON mp.manga_id = pcu.manga_id AND mp.provider_name = pcu.provider_name
         WHERE pcu.manga_id = ?",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(from_entry_row).collect())
}
