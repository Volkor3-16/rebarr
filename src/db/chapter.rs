use chrono::{DateTime, TimeZone, Utc};
use ordered_float::OrderedFloat;
use sqlx::SqlitePool;
use tracing::debug;
use uuid::Uuid;

use crate::db::{manga as db_manga, provider_settings, quality_rules};
use crate::manga::core::{Chapter, DownloadStatus};
use crate::manga::metadata_rules::{self, MetadataRule};
use crate::manga::scoring::compute_score;
use crate::scraper::ProviderChapterInfo;

// Chapter normalization and canonical selection types
#[derive(Debug, Clone, PartialEq)]
enum BundleType {
    Full,
    Split,
    Extra,
}

#[derive(Debug, Clone)]
struct ProviderBundle {
    entries: Vec<Chapter>,
    bundle_type: BundleType,
    coverage: usize,
}

// Helper functions for slot assignment and split detection
async fn assign_slot_id(chapter_base: i32, chapter_variant: i32, all_chapters: &[Chapter]) -> f64 {
    if chapter_variant == 0 {
        return chapter_base as f64;
    }

    // Check if this is a standalone decimal chapter
    let has_split_structure = all_chapters.iter().any(|ch| {
        ch.chapter_base == chapter_base && ch.chapter_variant >= 1 && ch.chapter_variant <= 4
    });

    if !has_split_structure {
        // Standalone decimal - use full chapter number as slot
        chapter_base as f64 + chapter_variant as f64 * 0.1
    } else {
        // Part of a split structure - use base chapter as slot
        chapter_base as f64
    }
}

async fn detect_split_chapters(chapter_base: i32, all_chapters: &[Chapter]) -> bool {
    let variants: Vec<i32> = all_chapters
        .iter()
        .filter(|ch| ch.chapter_base == chapter_base)
        .map(|ch| ch.chapter_variant)
        .collect();

    // Explicit split presence: variants 1-4 exist
    let has_explicit_split = variants.iter().any(|v| *v >= 1 && *v <= 4);

    // Implicit split presence: more than 1 variant exists
    let has_implicit_split = variants.len() > 1;

    has_explicit_split || has_implicit_split
}

async fn classify_bundle(pool: &SqlitePool, bundle: &[Chapter]) -> BundleType {
    if bundle.len() == 1 {
        let chapter = &bundle[0];
        if chapter.chapter_variant == 0 {
            BundleType::Full
        } else {
            // Check if this is an extra chapter
            let all_chapters = get_all_for_manga(pool, chapter.manga_id).await.unwrap();
            let is_extra =
                assign_slot_id(chapter.chapter_base, chapter.chapter_variant, &all_chapters).await
                    != chapter.chapter_base as f64;
            if is_extra {
                BundleType::Extra
            } else {
                BundleType::Full
            }
        }
    } else {
        // Multiple entries with the SAME chapter_variant are competing releases from different
        // groups (same provider, same chapter number). select_best_bundle should pick only the
        // best-scored entry — treat as Full regardless of whether variant is 0 or a decimal.
        // Multiple entries with DIFFERENT variants are genuine split parts (e.g. 3.1, 3.2, 3.3)
        // where every part must be canonical.
        let all_same_variant = bundle
            .iter()
            .all(|ch| ch.chapter_variant == bundle[0].chapter_variant);
        if all_same_variant {
            BundleType::Full
        } else {
            BundleType::Split
        }
    }
}

async fn compute_bundle_coverage(pool: &SqlitePool, bundle: &[Chapter]) -> usize {
    if let BundleType::Split = classify_bundle(pool, bundle).await {
        bundle.len()
    } else {
        1
    }
}

/// Apply metadata rules to a chapter's mutable fields (title, scanlator_group) for scoring.
/// Returns a cloned Chapter with rules applied — the DB copy is never mutated.
fn apply_meta_rules(ch: &Chapter, meta_rules: &[MetadataRule]) -> Chapter {
    Chapter {
        title: metadata_rules::apply_rules(
            meta_rules,
            ch.provider_name.as_deref(),
            "title",
            ch.title.as_deref(),
        ),
        scanlator_group: metadata_rules::apply_rules(
            meta_rules,
            ch.provider_name.as_deref(),
            "scanlator_group",
            ch.scanlator_group.as_deref(),
        ),
        ..ch.clone()
    }
}

// Select the best bundle according to the structured selection rules.
// Returns all chapters that should be canonical for this slot:
// - Split bundles → all entries (every part is canonical)
// - Full/Extra bundles → the single best-scored entry
async fn select_best_bundle(
    bundles: &[ProviderBundle],
    _all_chapters: &[Chapter],
    quality_rules: &[quality_rules::QualityRule],
    meta_rules: &[MetadataRule],
) -> Vec<Chapter> {
    if bundles.is_empty() {
        return vec![];
    }

    // Step 1: Apply quality score
    let mut best_bundles: Vec<&ProviderBundle> = Vec::new();
    let mut best_score = i32::MIN;

    for bundle in bundles {
        // Use the highest-scored entry in the bundle as the representative
        let bundle_score = bundle
            .entries
            .iter()
            .map(|e| compute_score(&apply_meta_rules(e, meta_rules), quality_rules))
            .max()
            .unwrap_or(i32::MIN);

        if bundle_score > best_score {
            best_score = bundle_score;
            best_bundles.clear();
            best_bundles.push(bundle);
        } else if bundle_score == best_score {
            best_bundles.push(bundle);
        }
    }

    // Step 2: Apply coverage (for bundles with multiple entries)
    if best_bundles.len() > 1 {
        let mut best_coverage = 0;
        let mut coverage_bundles: Vec<&ProviderBundle> = Vec::new();

        for bundle in &best_bundles {
            if bundle.entries.len() > 1 && bundle.coverage > best_coverage {
                best_coverage = bundle.coverage;
                coverage_bundles.clear();
                coverage_bundles.push(bundle);
            } else if bundle.entries.len() == 1 || bundle.coverage == best_coverage {
                coverage_bundles.push(bundle);
            }
        }

        best_bundles = coverage_bundles;
    }

    // Step 4: Stability rule — pick first remaining bundle.
    // For split bundles, all parts are canonical. For full/extra, pick the best-scored entry.
    if let Some(selected_bundle) = best_bundles.first() {
        if selected_bundle.bundle_type == BundleType::Split {
            // Split bundles include all parts, but a provider may have multiple competing
            // releases of the same part (e.g. two groups releasing 3.5 alongside 3.1–3.3).
            // Deduplicate by (chapter_base, chapter_variant), keeping the best-scored entry.
            let mut best_by_part: std::collections::HashMap<(i32, i32), (i32, Chapter)> =
                std::collections::HashMap::new();
            for entry in &selected_bundle.entries {
                let key = (entry.chapter_base, entry.chapter_variant);
                let score = compute_score(&apply_meta_rules(entry, meta_rules), quality_rules);
                best_by_part
                    .entry(key)
                    .and_modify(|(best_score, best_ch)| {
                        if score > *best_score {
                            *best_score = score;
                            *best_ch = entry.clone();
                        }
                    })
                    .or_insert((score, entry.clone()));
            }
            let mut parts: Vec<Chapter> = best_by_part.into_values().map(|(_, ch)| ch).collect();
            parts.sort_by_key(|ch| (ch.chapter_base, ch.chapter_variant));
            return parts;
        }

        // Full or Extra: return the single best-scored entry.
        let best_entry = selected_bundle
            .entries
            .iter()
            .max_by_key(|e| compute_score(&apply_meta_rules(e, meta_rules), quality_rules))
            .unwrap(); // entries is non-empty by construction
        return vec![best_entry.clone()];
    }

    vec![]
}

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

/// Clear downloaded_at and any tracked on-disk file size for a chapter.
pub async fn clear_download_artifacts(
    pool: &SqlitePool,
    chapter_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Chapters SET downloaded_at = NULL, file_size_bytes = NULL WHERE uuid = ?")
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Return the expected CBZ filenames (lowercased) for all Downloaded chapters of a manga.
/// Used to identify orphaned CBZ files on disk.
pub async fn get_downloaded_cbz_names(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let manga_id_str = manga_id.to_string();
    let rows: Vec<(i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT chapter_base, chapter_variant, title FROM Chapters WHERE manga_id = ? AND download_status = 'Downloaded'",
    )
    .bind(&manga_id_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(chapter_base, chapter_variant, title)| {
            let number_sort = chapter_base as f32 + chapter_variant as f32 * 0.1;
            let mut name = format!("Chapter {number_sort}");
            if let Some(t) = title.as_deref().filter(|s| !s.is_empty()) {
                name.push_str(&format!(" - {t}"));
            }
            // Apply the same sanitization as manga::files::sanitize_chapter_filename
            let sanitized: String = name
                .chars()
                .map(|c| {
                    if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            format!("{sanitized}.cbz").to_lowercase()
        })
        .collect())
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
pub async fn get_canonical_overrides_map(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
    load_canonical_overrides(pool, manga_id).await
}

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
    _preferred_language: &str,
) -> Result<(), sqlx::Error> {
    let all_raw = get_all_for_manga(pool, manga_id).await?;

    // Capture all DB UUIDs before any filtering — used to prune truly-gone overrides only.
    // We must not prune overrides just because the chapter's provider is currently disabled
    // or doesn't match the preferred language; the user may re-enable the provider later.
    let all_raw_uuids: std::collections::HashSet<String> =
        all_raw.iter().map(|ch| ch.id.to_string()).collect();

    // Filter out chapters from disabled providers.
    let globally_disabled = provider_settings::get_globally_disabled(pool).await?;
    let series_overrides = provider_settings::get_all_series_overrides(pool, manga_id).await?;
    let all: Vec<Chapter> = all_raw
        .iter()
        .cloned()
        .filter(|ch| {
            let name = match &ch.provider_name {
                Some(n) => n,
                None => return true, // keep chapters without provider (e.g. disk-scanned)
            };
            // Per-series override takes priority over global setting.
            series_overrides
                .get(name)
                .copied()
                .unwrap_or_else(|| !globally_disabled.contains(name))
        })
        .collect();

    // Filter to preferred language so non-matching chapters can't win canonical selection
    // and then be silently hidden in the API. Falls back to all languages when no chapters
    // match (e.g. a manga that only exists in Japanese).
    let preferred_language = crate::db::settings::get(pool, "preferred_language", "").await?;
    let all: Vec<Chapter> = if preferred_language.is_empty() {
        all
    } else {
        let lang_filtered: Vec<Chapter> = all
            .iter()
            .filter(|ch| {
                ch.language.eq_ignore_ascii_case(&preferred_language) || ch.language.is_empty()
            })
            .cloned()
            .collect();
        if lang_filtered.is_empty() {
            all // fallback: no chapters at all match preferred language
        } else {
            lang_filtered
        }
    };

    // Auto-classify extras per (provider_name, chapter_base):
    // If a provider releases Ch.1.1–1.7 those are all split parts, even the .5+.
    // Only flag variant>=5 as extra when a provider has NO low-numbered split parts (1–4).
    // The SQL guard in set_is_extra ensures manual user overrides (is_extra_manual IS NOT NULL)
    // are never clobbered by this auto-classification.
    {
        // Group by (provider_name, chapter_base) — None provider grouped separately per base.
        let mut by_provider_base: std::collections::HashMap<(Option<String>, i32), Vec<&Chapter>> =
            std::collections::HashMap::new();
        for ch in &all {
            by_provider_base
                .entry((ch.provider_name.clone(), ch.chapter_base))
                .or_default()
                .push(ch);
        }
        for chs in by_provider_base.values() {
            let has_low = chs
                .iter()
                .any(|c| c.chapter_variant >= 1 && c.chapter_variant <= 4);

            // Get sorted list of all variants for this provider+base
            let mut variants: Vec<i32> = chs.iter().map(|c| c.chapter_variant).collect();
            variants.sort_unstable();

            for ch in chs.iter().filter(|c| c.chapter_variant >= 1) {
                // First check title for explicit extra indicators
                let title_lower = ch.title.as_deref().unwrap_or_default().to_lowercase();
                let has_extra_title = title_lower.contains("extra")
                    || title_lower.contains("bonus")
                    || title_lower.contains("special")
                    || title_lower.contains("omake")
                    || title_lower.contains("side")
                    || title_lower.contains("extras");

                if has_extra_title {
                    // Title explicitly says it's extra - always mark as extra
                    set_is_extra(pool, ch.id, true).await?;
                    continue;
                }

                // For variants >=5, check if they are actually sequential
                if ch.chapter_variant >= 5 {
                    // Check if all previous numbers exist sequentially
                    let expected = ch.chapter_variant - 1;
                    let has_previous = variants.binary_search(&expected).is_ok();

                    if !has_previous {
                        // Missing previous variant number - this is an extra, not part of split
                        set_is_extra(pool, ch.id, true).await?;
                    } else if has_low {
                        // Sequential and part of split sequence - not extra
                        set_is_extra(pool, ch.id, false).await?;
                    } else {
                        // Standalone .5+ with no split siblings - extra/bonus
                        set_is_extra(pool, ch.id, true).await?;
                    }
                }
            }
        }
    }

    // Build a set of all valid chapter UUIDs for this manga (for override validation).
    let valid_uuids: std::collections::HashSet<String> =
        all.iter().map(|ch| ch.id.to_string()).collect();

    // Load quality rules for scoring.
    let quality_rules = quality_rules::get_all(pool).await?;

    // Load metadata rules so scoring sees cleaned-up field values (cleared/replaced titles etc.).
    let meta_rules = metadata_rules::load(pool).await?;

    // Load user-set overrides before re-scoring.
    let overrides = load_canonical_overrides(pool, manga_id).await?;
    let _disable_chapter_upgrades =
        crate::db::settings::get(pool, "disable_chapter_upgrades", "false").await? == "true";

    // Group by (slot_id, provider_name, is_full) to create Provider Bundles.
    // A full chapter (variant=0) and split parts (variant>0) from the same provider at the
    // same slot are *different logical representations* and must be separate competing bundles.
    let mut bundles: std::collections::HashMap<
        (OrderedFloat<f64>, Option<String>, bool),
        Vec<Chapter>,
    > = std::collections::HashMap::new();
    for ch in &all {
        let slot_id = assign_slot_id(ch.chapter_base, ch.chapter_variant, &all).await;
        let is_full = ch.chapter_variant == 0;
        bundles
            .entry((
                ordered_float::OrderedFloat(slot_id),
                ch.provider_name.clone(),
                is_full,
            ))
            .or_default()
            .push(ch.clone());
    }

    // Classify bundles and collect all bundles by slot_id
    let mut bundles_by_slot: std::collections::HashMap<OrderedFloat<f64>, Vec<ProviderBundle>> =
        std::collections::HashMap::new();
    for ((slot_id, provider_name, _is_full), entries) in bundles {
        let bundle_type = classify_bundle(pool, &entries).await;
        let coverage = compute_bundle_coverage(pool, &entries).await;
        let _is_split_chapter = detect_split_chapters(entries[0].chapter_base, &all).await;

        debug!(
            "[canonical] slot={slot_id:.1} provider={:?} type={:?} coverage={coverage} entries=[{}]",
            provider_name,
            bundle_type,
            entries.iter().map(|e| format!("{}.{} score={}", e.chapter_base, e.chapter_variant, compute_score(&apply_meta_rules(e, &meta_rules), &quality_rules))).collect::<Vec<_>>().join(","),
        );

        let bundle = ProviderBundle {
            entries,
            bundle_type,
            coverage,
        };

        bundles_by_slot.entry(slot_id).or_default().push(bundle);
    }

    let mut canonical_uuids: Vec<String> = Vec::with_capacity(bundles_by_slot.len());

    for (slot_id, bundles) in &bundles_by_slot {
        // Apply deterministic selection priority; splits return all parts.
        let winners = select_best_bundle(bundles, &all, &quality_rules, &meta_rules).await;
        for winner in &winners {
            debug!(
                "[canonical] slot={slot_id:.1} → winner {}.{} score={} provider={:?} group={:?}",
                winner.chapter_base, winner.chapter_variant,
                compute_score(&apply_meta_rules(winner, &meta_rules), &quality_rules),
                winner.provider_name,
                winner.scanlator_group,
            );
            canonical_uuids.push(winner.id.to_string());
        }
    }

    // Apply user overrides: for each chapter_base that has an override, replace the
    // auto-selected winner with the user's choice.  Group by base first so we don't
    // process the same base twice when there are multiple override keys for it (e.g.
    // "2:1" and "2:2" both pointing into the same split bundle).
    {
        let mut applied_bases: std::collections::HashSet<i32> =
            std::collections::HashSet::new();
        for override_uuid in overrides.values() {
            // Search all_raw so overrides pointing to disabled-provider chapters still apply.
            if let Some(override_ch) = all_raw.iter().find(|ch| ch.id.to_string() == *override_uuid) {
                let base = override_ch.chapter_base;
                if !applied_bases.insert(base) {
                    continue; // already handled this base
                }

                // Remove auto-selected chapters for this base
                let base_uuid_set: std::collections::HashSet<String> = all
                    .iter()
                    .filter(|ch| ch.chapter_base == base)
                    .map(|ch| ch.id.to_string())
                    .collect();
                canonical_uuids.retain(|uuid| !base_uuid_set.contains(uuid));

                // Re-add the override choice (full chapter or entire split bundle)
                if override_ch.chapter_variant == 0 {
                    canonical_uuids.push(override_uuid.clone());
                } else {
                    // Search all_raw so split siblings from disabled providers are included.
                    let siblings: Vec<String> = all_raw
                        .iter()
                        .filter(|ch| {
                            ch.chapter_base == base
                                && ch.provider_name == override_ch.provider_name
                                && ch.chapter_variant >= 1
                                && ch.chapter_variant <= 4
                                && !ch.is_extra
                        })
                        .map(|ch| ch.id.to_string())
                        .collect();
                    if siblings.len() > 1 {
                        for s in siblings {
                            if !canonical_uuids.contains(&s) {
                                canonical_uuids.push(s);
                            }
                        }
                    } else if !canonical_uuids.contains(override_uuid) {
                        canonical_uuids.push(override_uuid.clone());
                    }
                }
            }
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

    // Prune stale overrides — only remove entries where the chapter is truly gone from the DB.
    // Do NOT prune based on the filtered `all` (which excludes disabled providers and
    // non-preferred-language chapters): that would silently delete the user's choice whenever
    // a provider is globally disabled or language settings change.
    let pruned_overrides: std::collections::HashMap<String, String> = overrides
        .into_iter()
        .filter(|(_, uuid)| all_raw_uuids.contains(uuid.as_str()))
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

    db_manga::update_last_chapter(pool, manga_id).await?;

    Ok(())
}

/// Set the is_extra flag for a chapter (auto-classifier only).
/// Does NOT touch rows where `is_extra_manual` is set — user overrides are preserved.
pub async fn set_is_extra(
    pool: &SqlitePool,
    chapter_id: Uuid,
    is_extra: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Chapters SET is_extra = ? WHERE uuid = ? AND is_extra_manual IS NULL")
        .bind(is_extra as i64)
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Set is_extra for a chapter AND record it as a manual user override.
/// This value survives future auto-classification scans.
pub async fn set_is_extra_manual(
    pool: &SqlitePool,
    chapter_id: Uuid,
    is_extra: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Chapters SET is_extra = ?, is_extra_manual = ? WHERE uuid = ?")
        .bind(is_extra as i64)
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
    let all_chapters = get_all_for_manga(pool, manga_id).await?;

    // Get the chapter that the user selected
    let selected_chapter = all_chapters
        .iter()
        .find(|ch| ch.id == new_uuid)
        .ok_or_else(|| sqlx::Error::RowNotFound)?;

    // Remove ALL chapters with the same chapter_base (clear entire slot, not just exact variant)
    // This fixes the bug where both split and full chapters remained canonical
    let mut new_uuids: Vec<String> = current
        .iter()
        .filter(|ch| ch.chapter_base != chapter_base)
        .map(|ch| ch.id.to_string())
        .collect();

    if selected_chapter.chapter_variant == 0 {
        // User selected a full chapter — add just this one.
        // Do NOT pull in split siblings: the user explicitly chose the full version over splits.
        new_uuids.push(new_uuid.to_string());
    } else {
        // User selected a split part — auto-include all sibling parts from the same provider
        // so the entire bundle becomes canonical (e.g. picking 15.1 also adds 15.2, 15.3).
        let mut bundle_chapters: Vec<&Chapter> = all_chapters
            .iter()
            .filter(|ch| {
                ch.chapter_base == chapter_base
                    && ch.provider_name == selected_chapter.provider_name
                    && ch.chapter_variant >= 1
                    && ch.chapter_variant <= 4
                    && !ch.is_extra
            })
            .collect();

        // Sort by variant number to maintain correct order
        bundle_chapters.sort_by_key(|ch| ch.chapter_variant);

        if bundle_chapters.len() > 1 {
            // This is a split bundle - add ALL parts
            debug!(
                "[db] set_canonical_override: detected split bundle with {} parts",
                bundle_chapters.len()
            );
            for part in &bundle_chapters {
                new_uuids.push(part.id.to_string());
            }
        } else {
            // Only one split part found — add just this one
            new_uuids.push(new_uuid.to_string());
        }
    }

    let json = serde_json::to_string(&new_uuids).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    // Persist the user's override so it survives future auto-scans.
    // Clear any stale override keys for the same chapter_base first — if the user
    // previously overrode "2:1" (split) and now overrides "2:0" (full), the old key
    // must not remain, or update_canonical would see two conflicting overrides for
    // base 2 and apply whichever HashMap iteration returns first.
    let mut overrides = load_canonical_overrides(pool, manga_id).await?;
    let prefix = format!("{chapter_base}:");
    overrides.retain(|key, _| !key.starts_with(&prefix));
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

/// Remove a user-set canonical override for a specific (chapter_base, chapter_variant) slot.
/// The caller is responsible for calling `update_canonical` afterward so scoring re-picks the winner.
pub async fn remove_canonical_override(
    pool: &SqlitePool,
    manga_id: Uuid,
    chapter_base: i32,
    chapter_variant: i32,
) -> Result<(), sqlx::Error> {
    debug!("[db] remove_canonical_override: manga={manga_id}, ch={chapter_base}.{chapter_variant}");
    let mut overrides = load_canonical_overrides(pool, manga_id).await?;
    overrides.remove(&format!("{chapter_base}:{chapter_variant}"));
    let overrides_json =
        serde_json::to_string(&overrides).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    sqlx::query("UPDATE CanonicalChapters SET canonical_overrides = ? WHERE manga_id = ?")
        .bind(&overrides_json)
        .bind(manga_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
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

/// Delete all MISSING chapters from a specific provider for a manga.
/// Leaves downloaded chapters intact.
pub async fn delete_missing_for_provider(
    pool: &SqlitePool,
    manga_id: Uuid,
    provider_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM Chapters WHERE manga_id = ? AND provider_name = ? AND download_status = 'Missing'")
        .bind(manga_id.to_string())
        .bind(provider_name)
        .execute(pool)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Upgrade candidate detection
// ---------------------------------------------------------------------------

/// A chapter slot where the current canonical has a better score than what is
/// already Downloaded.
pub struct UpgradeCandidate {
    pub chapter_base: i32,
    pub chapter_variant: i32,
    /// New canonical UUID (higher score, currently Missing/Failed).
    pub new_canonical_id: Uuid,
    /// Currently-downloaded UUID (lower score).
    pub old_downloaded_id: Uuid,
}

/// Find chapters where a better-scored source is now canonical but an
/// inferior source is already Downloaded.
///
/// Only fires when `score(canonical) > score(downloaded)` (strictly better).
pub async fn find_upgrade_candidates(
    pool: &SqlitePool,
    manga_id: Uuid,
) -> Result<Vec<UpgradeCandidate>, sqlx::Error> {
    let all = get_all_for_manga(pool, manga_id).await?;
    let canonical_set: std::collections::HashSet<String> = get_canonical_uuids(pool, manga_id)
        .await?
        .into_iter()
        .collect();

    let rules = quality_rules::get_all(pool).await?;
    let meta_rules = metadata_rules::load(pool).await?;

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

        let canon_score = compute_score(&apply_meta_rules(canonical, &meta_rules), &rules);

        // Find Downloaded entries that have a lower score than the canonical.
        for entry in &entries {
            if entry.id == canonical.id {
                continue;
            }
            if entry.download_status != DownloadStatus::Downloaded {
                continue;
            }
            let entry_score = compute_score(&apply_meta_rules(entry, &meta_rules), &rules);
            if canon_score > entry_score {
                candidates.push(UpgradeCandidate {
                    chapter_base: base,
                    chapter_variant: variant,
                    new_canonical_id: canonical.id,
                    old_downloaded_id: entry.id,
                });
                // One candidate per slot is enough (take the first lower-scored Downloaded).
                break;
            }
        }
    }

    Ok(candidates)
}
