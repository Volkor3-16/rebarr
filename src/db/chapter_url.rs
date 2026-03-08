use sqlx::SqlitePool;
use uuid::Uuid;

/// Cache the chapter page URL for a given manga + provider + chapter number.
pub async fn upsert(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
    number_sort: f32,
    url: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ProviderChapterUrl (manga_id, provider_name, chapter_number_sort, chapter_url)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(manga_id, provider_name, chapter_number_sort) DO UPDATE SET chapter_url = excluded.chapter_url",
    )
    .bind(manga_id.to_string())
    .bind(provider_name)
    .bind(number_sort as f64)
    .bind(url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Look up a cached chapter URL. Returns None if not yet cached.
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
