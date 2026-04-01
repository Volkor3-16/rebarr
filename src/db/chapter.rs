use chrono::{DateTime, TimeZone, Utc};
use tracing::debug;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::provider_scores;
use crate::manga::core::{Chapter, DownloadStatus};
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
    is_extra: i64,
    title: Option<String>,
    language: String,
    scanlator_group: Option<String>,
    provider_name: Option<String>,
    chapter_url: Option<String>,
    download_status: String,
    released_at: Option<i64>,
    downloaded_at: Option<i64>,
    scraped_at: Option<i64>,
    file_size_bytes: Option<i64>,
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
        "Queued" => DownloadStatus::Queued,
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
        is_extra: row.is_extra != 0,
        title: row.title,
        language: row.language,
        scanlator_group: row.scanlator_group,
        provider_name: row.provider_name,
        chapter_url: row.chapter_url,
        download_status,
        released_at: ts_to_dt(row.released_at),
        downloaded_at: ts_to_dt(row.downloaded_at),
        scraped_at: ts_to_dt(row.scraped_at),
        file_size_bytes: row.file_size_bytes,
    })
}

// ---------------------------------------------------------------------------
// Deterministic UUID
// ---------------------------------------------------------------------------

/// Fixed namespace for chapter UUID v5 derivation. Must never change after
/// first deployment — changing it would invalidate all existing chapter IDs.
const CHAPTER_NAMESPACE: Uuid = Uuid::from_bytes([
    0x7a, 0x2f, 0x4e, 0x10, 0xc1, 0x3b, 0x5a, 0x80, 0xb4, 0xe2, 0x00, 0xc0, 0x9d, 0x1a, 0x77, 0xf3,
]);

/// Compute the deterministic UUID v5 for a chapter row.
///
/// The key mirrors the UNIQUE INDEX on Chapters exactly:
/// `manga_id : chapter_base : chapter_variant : LANGUAGE : scanlator_group : provider_name`
///
/// `None` values use `""` to match the DB convention (NULLs are stored as
/// empty strings in the unique constraint columns).
pub fn chapter_uuid(
    manga_id: Uuid,
    chapter_base: i32,
    chapter_variant: i32,
    language: &str,
    scanlator_group: Option<&str>,
    provider_name: Option<&str>,
) -> Uuid {
    let key = format!(
        "{}:{}:{}:{}:{}:{}",
        manga_id,
        chapter_base,
        chapter_variant,
        language.to_uppercase(),
        scanlator_group.unwrap_or(""),
        provider_name.unwrap_or(""),
    );
    Uuid::new_v5(&CHAPTER_NAMESPACE, key.as_bytes())
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Upsert chapters from a provider scrape into the new Chapters table.
/// - New rows are inserted with status `Missing`.
/// - Existing rows are updated (scraped_at, chapter_url, title/scanlator_group back-filled if missing).
///   Returns UUIDs of newly inserted rows.
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
        let language = info.language.as_deref().unwrap_or("EN").to_uppercase();
        let released_at = info.date_released;

        // Normalize NULL to empty string for conflict detection.
        // In SQLite, NULL != NULL in unique constraints, causing duplicate inserts.
        // By using empty string, we ensure NULL + NULL = conflict detected.
        // The URL IS still updated on conflict (chapter_url = excluded.chapter_url).
        let scanlator_group = info.scanlator_group.as_deref().unwrap_or("");
        let title = info.title.as_deref().unwrap_or("");

        let det_id = chapter_uuid(
            manga_id,
            info.chapter_base as i32,
            info.chapter_variant as i32,
            &language,
            info.scanlator_group.as_deref(),
            Some(provider_name),
        );

        // Pre-insert existence check: deterministic IDs mean the same row would
        // produce the same UUID on conflict, so we can't use the old
        // post-insert "did our new_v4 survive?" heuristic.
        let pre_exists: bool =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM Chapters WHERE uuid = ?")
                .bind(det_id.to_string())
                .fetch_one(pool)
                .await?
                > 0;

        sqlx::query(
            "INSERT INTO Chapters
                (uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                 scanlator_group, provider_name, chapter_url, download_status, released_at, scraped_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'Missing', ?, ?)
             ON CONFLICT(manga_id, chapter_base, chapter_variant, language, scanlator_group, provider_name)
             DO UPDATE SET
                 scraped_at       = excluded.scraped_at,
                 chapter_url      = excluded.chapter_url,
                 title            = COALESCE(NULLIF(Chapters.title, ''), excluded.title),
                 scanlator_group  = COALESCE(Chapters.scanlator_group, excluded.scanlator_group),
                 is_extra         = CASE WHEN Chapters.is_extra = 0 THEN excluded.is_extra ELSE Chapters.is_extra END",
        )
        .bind(det_id.to_string())
        .bind(&manga_id_str)
        .bind(info.chapter_base as i64)
        .bind(info.chapter_variant as i64)
        .bind(info.is_extra as i64)
        .bind(title)
        .bind(&language)
        .bind(scanlator_group)
        .bind(provider_name)
        .bind(&info.url)
        .bind(released_at)
        .bind(now)
        .execute(pool)
        .await?;

        if !pre_exists {
            new_ids.push(det_id);
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
        "SELECT uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at, file_size_bytes
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
        "SELECT uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at, file_size_bytes
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
        "SELECT uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at, file_size_bytes
         FROM Chapters WHERE uuid = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(chapter_from_row).transpose()
}

/// Get the canonical list of chapter UUIDs for a manga (from CanonicalChapters).
/// Returns an empty Vec if no canonical entry exists yet.
pub async fn get_canonical_uuids(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT canonical_list FROM CanonicalChapters WHERE manga_id = ?")
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
        "SELECT uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at, file_size_bytes
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
        "SELECT uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
                scanlator_group, provider_name, chapter_url, download_status,
                released_at, downloaded_at, scraped_at, file_size_bytes
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
    sqlx::query("UPDATE Chapters SET download_status = ?, downloaded_at = ? WHERE uuid = ?")
        .bind(status.as_str())
        .bind(dt_to_ts(downloaded_at))
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the on-disk file size for a chapter (bytes).
pub async fn set_file_size(
    pool: &SqlitePool,
    chapter_id: Uuid,
    bytes: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Chapters SET file_size_bytes = ? WHERE uuid = ?")
        .bind(bytes)
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Return the UUIDs and provider names of all chapters for a manga that are currently Downloaded.
pub async fn get_downloaded(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<(Uuid, Option<String>)>, sqlx::Error> {
    let manga_id_str = manga_id.to_string();
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT uuid, provider_name FROM Chapters WHERE manga_id = ? AND download_status = 'Downloaded'",
    )
    .bind(&manga_id_str)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(s, p)| {
            Uuid::parse_str(&s)
                .map(|id| (id, p))
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))
        })
        .collect()
}

/// Load the user's manual canonical overrides map from the DB.
/// Returns a HashMap of "base:variant" -> uuid strings.
async fn load_canonical_overrides(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT canonical_overrides FROM CanonicalChapters WHERE manga_id = ?")
            .bind(manga_id.to_string())
            .fetch_optional(pool)
            .await?;

    Ok(row
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
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
    provider_scores: &std::collections::HashMap<String, i32>,
) -> Result<(), sqlx::Error> {
    let all = get_all_for_manga(pool, manga_id).await?;

    // Filter out chapters from disabled providers.
    let globally_disabled = provider_scores::get_globally_disabled(pool).await?;
    let series_overrides = provider_scores::get_all_series_overrides(pool, manga_id).await?;
    let all: Vec<Chapter> = all
        .into_iter()
        .filter(|ch| {
            let name = match &ch.provider_name {
                Some(n) => n,
                None => return true, // keep chapters without provider (e.g. disk-scanned)
            };
            // Per-series override takes priority over global setting.
            let enabled = series_overrides
                .get(name)
                .map(|(_, enabled)| *enabled)
                .unwrap_or_else(|| !globally_disabled.contains(name));
            enabled
        })
        .collect();

    // Auto-classify: if a base has variant>=5 chapters but no variant 1–4, mark them as extras.
    // e.g. a lone Ch.1.5 with no Ch.1.1–1.4 siblings → extra/bonus, not a split part.
    {
        let mut by_base: std::collections::HashMap<i32, Vec<&Chapter>> =
            std::collections::HashMap::new();
        for ch in &all {
            by_base.entry(ch.chapter_base).or_default().push(ch);
        }
        for chs in by_base.values() {
            let has_low = chs
                .iter()
                .any(|c| c.chapter_variant >= 1 && c.chapter_variant <= 4);
            if !has_low {
                for ch in chs.iter().filter(|c| c.chapter_variant >= 5 && !c.is_extra) {
                    set_is_extra(pool, ch.id, true).await?;
                }
            }
        }
    }

    // Build a set of all valid chapter UUIDs for this manga (for override validation).
    let valid_uuids: std::collections::HashSet<String> =
        all.iter().map(|ch| ch.id.to_string()).collect();

    // Load user-set overrides before re-scoring.
    let overrides = load_canonical_overrides(pool, manga_id).await?;

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

    for ((base, variant), mut entries) in groups {
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

        // Primary: tier ascending (1=Official best, 4=No group worst).
        // Secondary within same tier: provider score descending (higher = better).
        // Score NEVER promotes a lower tier above a higher one.
        entries.sort_by(|a, b| {
            let tier_a = compute_tier(a.scanlator_group.as_deref(), trusted_groups, a.provider_name.as_deref());
            let tier_b = compute_tier(b.scanlator_group.as_deref(), trusted_groups, b.provider_name.as_deref());
            if tier_a != tier_b {
                return tier_a.cmp(&tier_b);
            }
            let score_a = a
                .provider_name
                .as_deref()
                .and_then(|n| provider_scores.get(n))
                .copied()
                .unwrap_or(0);
            let score_b = b
                .provider_name
                .as_deref()
                .and_then(|n| provider_scores.get(n))
                .copied()
                .unwrap_or(0);
            score_b.cmp(&score_a)
        });

        if let Some(winner) = entries.into_iter().next() {
            // Apply user override if present and the overridden chapter still exists.
            let key = format!("{base}:{variant}");
            let uuid = if let Some(ov_uuid) = overrides.get(&key) {
                if valid_uuids.contains(ov_uuid.as_str()) {
                    ov_uuid.clone()
                } else {
                    winner.id.to_string()
                }
            } else {
                winner.id.to_string()
            };
            canonical_uuids.push(uuid);
        }
    }

    debug!(
        "[db] update_canonical: manga={manga_id}, {} canonical chapters, {} active overrides",
        canonical_uuids.len(),
        overrides
            .values()
            .filter(|uuid| valid_uuids.contains(uuid.as_str()))
            .count(),
    );

    let json =
        serde_json::to_string(&canonical_uuids).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    // Prune stale overrides (chapters that no longer exist) before saving.
    let pruned_overrides: std::collections::HashMap<String, String> = overrides
        .into_iter()
        .filter(|(_, uuid)| valid_uuids.contains(uuid.as_str()))
        .collect();
    let overrides_json =
        serde_json::to_string(&pruned_overrides).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    sqlx::query(
        "INSERT OR REPLACE INTO CanonicalChapters (manga_id, canonical_list, canonical_overrides, last_updated)
         VALUES (?, ?, ?, unixepoch())",
    )
    .bind(manga_id.to_string())
    .bind(&json)
    .bind(&overrides_json)
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

    sqlx::query("UPDATE Manga SET chapter_count = ?, downloaded_count = ? WHERE uuid = ?")
        .bind(chapter_count)
        .bind(downloaded_count)
        .bind(manga_id.to_string())
        .execute(pool)
        .await?;

    Ok(())
}

/// Toggle the is_extra flag for a specific chapter row.
pub async fn set_is_extra(
    pool: &SqlitePool,
    chapter_id: Uuid,
    is_extra: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Chapters SET is_extra = ? WHERE uuid = ?")
        .bind(is_extra as i64)
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Manually override the canonical chapter for a specific (chapter_base, chapter_variant) slot.
/// Replaces whichever UUID was previously canonical for that slot with `new_uuid`.
/// The override is persisted in `canonical_overrides` so that it survives future scans.
pub async fn set_canonical_override(
    pool: &SqlitePool,
    manga_id: Uuid,
    chapter_base: i32,
    chapter_variant: i32,
    new_uuid: Uuid,
) -> Result<(), sqlx::Error> {
    debug!(
        "[db] set_canonical_override: manga={manga_id}, ch={chapter_base}.{chapter_variant} → {new_uuid}"
    );
    let current = get_canonical_for_manga(pool, manga_id).await?;

    let mut new_uuids: Vec<String> = current
        .iter()
        .filter(|ch| !(ch.chapter_base == chapter_base && ch.chapter_variant == chapter_variant))
        .map(|ch| ch.id.to_string())
        .collect();
    new_uuids.push(new_uuid.to_string());

    let json = serde_json::to_string(&new_uuids).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    // Persist the user's override so it survives future auto-scans.
    let mut overrides = load_canonical_overrides(pool, manga_id).await?;
    overrides.insert(
        format!("{chapter_base}:{chapter_variant}"),
        new_uuid.to_string(),
    );
    let overrides_json =
        serde_json::to_string(&overrides).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    sqlx::query(
        "INSERT OR REPLACE INTO CanonicalChapters (manga_id, canonical_list, canonical_overrides, last_updated)
         VALUES (?, ?, ?, unixepoch())",
    )
    .bind(manga_id.to_string())
    .bind(&json)
    .bind(&overrides_json)
    .execute(pool)
    .await?;

    update_manga_counts(pool, manga_id).await
}

/// Insert a new chapter row directly (used by disk scanner for manually-found CBZ files).
pub async fn insert(pool: &SqlitePool, chapter: &Chapter) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO Chapters
            (uuid, manga_id, chapter_base, chapter_variant, is_extra, title, language,
             scanlator_group, provider_name, chapter_url, download_status,
             released_at, downloaded_at, scraped_at, file_size_bytes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chapter.id.to_string())
    .bind(chapter.manga_id.to_string())
    .bind(chapter.chapter_base as i64)
    .bind(chapter.chapter_variant as i64)
    .bind(chapter.is_extra as i64)
    .bind(&chapter.title)
    .bind(&chapter.language)
    .bind(&chapter.scanlator_group)
    .bind(&chapter.provider_name)
    .bind(&chapter.chapter_url)
    .bind(chapter.download_status.as_str())
    .bind(dt_to_ts(chapter.released_at))
    .bind(dt_to_ts(chapter.downloaded_at))
    .bind(dt_to_ts(chapter.scraped_at))
    .bind(chapter.file_size_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a chapter by UUID and update canonical chapters list.
pub async fn delete(pool: &SqlitePool, chapter_id: Uuid) -> Result<(), sqlx::Error> {
    // First get the manga_id so we can update canonical chapters
    let chapter = get_by_id(pool, chapter_id).await?;

    if let Some(ch) = chapter {
        let manga_id = ch.manga_id;

        // Delete the chapter row
        sqlx::query("DELETE FROM Chapters WHERE uuid = ?")
            .bind(chapter_id.to_string())
            .execute(pool)
            .await?;

        // Remove from canonical chapters list
        let uuids = get_canonical_uuids(pool, manga_id).await?;
        let chapter_id_str = chapter_id.to_string();
        let new_uuids: Vec<String> = uuids
            .into_iter()
            .filter(|uuid| uuid != &chapter_id_str)
            .collect();

        let json =
            serde_json::to_string(&new_uuids).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO CanonicalChapters (manga_id, canonical_list, last_updated)
             VALUES (?, ?, unixepoch())",
        )
        .bind(manga_id.to_string())
        .bind(&json)
        .execute(pool)
        .await?;

        // Update manga chapter counts
        update_manga_counts(pool, manga_id).await?;
    }

    Ok(())
}

/// Delete all chapters for a manga (used when deleting a series).
pub async fn delete_all_for_manga(pool: &SqlitePool, manga_id: Uuid) -> Result<(), sqlx::Error> {
    // Delete all chapter rows
    sqlx::query("DELETE FROM Chapters WHERE manga_id = ?")
        .bind(manga_id.to_string())
        .execute(pool)
        .await?;

    // Delete canonical chapters entry
    sqlx::query("DELETE FROM CanonicalChapters WHERE manga_id = ?")
        .bind(manga_id.to_string())
        .execute(pool)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Upgrade candidate detection
// ---------------------------------------------------------------------------

/// A chapter slot where the current canonical has a better tier than what is
/// already Downloaded.
pub struct UpgradeCandidate {
    pub chapter_base: i32,
    pub chapter_variant: i32,
    /// New canonical UUID (better tier, currently Missing/Failed).
    pub new_canonical_id: Uuid,
    /// Currently-downloaded UUID (worse tier).
    pub old_downloaded_id: Uuid,
}

/// Find chapters where a better-ranked source is now canonical but an
/// inferior source is already Downloaded.
///
/// Only fires when `tier(canonical) < tier(downloaded)` (strictly better).
/// Same-tier differences do not trigger upgrades to avoid churn.
pub async fn find_upgrade_candidates(
    pool: &SqlitePool,
    manga_id: Uuid,
    trusted_groups: &[String],
) -> Result<Vec<UpgradeCandidate>, sqlx::Error> {
    let all = get_all_for_manga(pool, manga_id).await?;
    let canonical_set: std::collections::HashSet<String> = get_canonical_uuids(pool, manga_id)
        .await?
        .into_iter()
        .collect();

    // Group by (chapter_base, chapter_variant)
    let mut groups: std::collections::HashMap<(i32, i32), Vec<Chapter>> =
        std::collections::HashMap::new();
    for ch in all {
        groups
            .entry((ch.chapter_base, ch.chapter_variant))
            .or_default()
            .push(ch);
    }

    let mut candidates = Vec::new();

    for ((base, variant), entries) in groups {
        // Find the canonical entry for this slot.
        let canonical = match entries
            .iter()
            .find(|e| canonical_set.contains(&e.id.to_string()))
        {
            Some(c) => c,
            None => continue,
        };

        let canon_tier = compute_tier(canonical.scanlator_group.as_deref(), trusted_groups, canonical.provider_name.as_deref());

        // Find Downloaded entries that are worse tier than the canonical.
        for entry in &entries {
            if entry.id == canonical.id {
                continue;
            }
            if entry.download_status != DownloadStatus::Downloaded {
                continue;
            }
            let entry_tier = compute_tier(entry.scanlator_group.as_deref(), trusted_groups, entry.provider_name.as_deref());
            if canon_tier < entry_tier {
                candidates.push(UpgradeCandidate {
                    chapter_base: base,
                    chapter_variant: variant,
                    new_canonical_id: canonical.id,
                    old_downloaded_id: entry.id,
                });
                // One candidate per slot is enough (take the first worse-tier Downloaded).
                break;
            }
        }
    }

    Ok(candidates)
}
