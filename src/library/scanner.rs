use log::{info, warn};
use sqlx::SqlitePool;
use std::collections::HashSet;
use uuid::Uuid;

use crate::db::{chapter as db_chapter, library as db_library, manga as db_manga};
use crate::manga::comicinfo;
use crate::manga::manga::{Chapter, DownloadStatus};

/// Scan the manga's directory on disk for existing CBZ files and mark the
/// corresponding chapter records as Downloaded.
///
/// Filenames must match the convention written by the downloader:
///   `Chapter {number}.cbz`
///
/// Chapters that exist in the DB are updated to Downloaded. Chapters that
/// don't exist yet are inserted as Downloaded (useful for pre-existing files).
pub async fn scan_existing_chapters(pool: &SqlitePool, manga_id: Uuid) -> Result<(), String> {
    let manga = db_manga::get_by_id(pool, manga_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("manga {manga_id} not found"))?;

    let library = db_library::get_by_id(pool, manga.library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("library {} not found", manga.library_id))?;

    let series_dir = library.root_path.join(&manga.relative_path);

    let entries = match std::fs::read_dir(&series_dir) {
        Ok(e) => e,
        Err(e) => {
            info!(
                "[scanner] Series directory does not exist for '{}': {e}",
                manga.metadata.title
            );
            return Ok(());
        }
    };

    let mut found = 0u32;
    let mut found_ids: HashSet<Uuid> = HashSet::new();
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if !name.ends_with(".cbz") {
            continue;
        }
        let Some(rest) = name.strip_prefix("Chapter ") else {
            continue;
        };
        let Some(num_str) = rest.strip_suffix(".cbz") else {
            continue;
        };
        // num_str may be "1", "1.5", "1 - Title [Group]", "1.5 - Title [Group]"
        // Extract just the leading numeric token before any whitespace.
        let number_part = num_str.split_whitespace().next().unwrap_or("");
        let Ok(number_sort) = number_part.parse::<f32>() else {
            continue;
        };

        // Derive chapter_base and chapter_variant from the float
        let chapter_base = number_sort.floor() as i32;
        let frac = (number_sort - number_sort.floor()).abs();
        let chapter_variant = (frac * 10.0).round() as i32;

        let entry_meta = entry.metadata().ok();
        let downloaded_at = entry_meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| {
                let secs = t
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                chrono::DateTime::from_timestamp(secs as i64, 0)
            });
        let file_size = entry_meta.as_ref().map(|m| m.len() as i64);

        let cbz_info = comicinfo::read_cbz_comicinfo(&entry.path());

        if let Some(ref ci) = cbz_info {
            // ComicInfo present — use it as the authoritative source for chapter identity
            // and metadata, regardless of what canonical chapter exists in the DB.
            let language = ci.language.clone().unwrap_or_else(|| "EN".to_owned());
            let scanlator_group = ci.scanlator.clone();
            let provider_name = ci.provider_name.clone();
            let chapter_url = ci.chapter_url.clone();

            // For CBZs with JSON Notes (indicated by a stored chapter UUID), use the
            // released_at from JSON directly — null means the provider didn't give a date.
            // For old CBZs without JSON Notes, fall back to the XML <Year> tag.
            let released_at = ci
                .released_at
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .or_else(|| {
                    if ci.chapter_uuid.is_some() {
                        return None; // JSON Notes present — respect null released_at
                    }
                    ci.release_year
                        .and_then(|y| chrono::NaiveDate::from_ymd_opt(y, 1, 1))
                        .and_then(|d| d.and_hms_opt(0, 0, 0))
                        .map(|dt| dt.and_utc())
                });

            let ci_downloaded_at = ci
                .downloaded_at
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .or(downloaded_at);

            let scraped_at = ci
                .scraped_at
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

            // Prefer UUID stored in JSON Notes for round-trip fidelity
            let id = ci.chapter_uuid.unwrap_or_else(|| {
                db_chapter::chapter_uuid(
                    manga_id,
                    chapter_base,
                    chapter_variant,
                    &language,
                    scanlator_group.as_deref(),
                    provider_name.as_deref(),
                )
            });

            let chapter = Chapter {
                id,
                manga_id,
                chapter_base,
                chapter_variant,
                is_extra: false,
                title: ci.chapter_title.clone(),
                language,
                scanlator_group,
                provider_name,
                chapter_url,
                download_status: DownloadStatus::Downloaded,
                released_at,
                downloaded_at: ci_downloaded_at,
                scraped_at,
                file_size_bytes: file_size,
            };

            db_chapter::insert(pool, &chapter)
                .await
                .map_err(|e| e.to_string())?;

            // set_status handles both the newly inserted row and the case where
            // INSERT OR IGNORE skipped because the row already existed.
            db_chapter::set_status(pool, chapter.id, DownloadStatus::Downloaded, ci_downloaded_at)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(size) = file_size {
                let _ = db_chapter::set_file_size(pool, chapter.id, size).await;
            }
            found_ids.insert(chapter.id);
            found += 1;
        } else {
            // No ComicInfo — fall back to canonical lookup.
            match db_chapter::get_canonical_by_number(pool, manga_id, chapter_base, chapter_variant)
                .await
            {
                Ok(Some(ch)) => {
                    if ch.download_status != DownloadStatus::Downloaded {
                        db_chapter::set_status(
                            pool,
                            ch.id,
                            DownloadStatus::Downloaded,
                            downloaded_at,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        found += 1;
                    }
                    if let Some(size) = file_size {
                        let _ = db_chapter::set_file_size(pool, ch.id, size).await;
                    }
                }
                Ok(None) => {
                    // No canonical and no ComicInfo — insert a minimal placeholder.
                    let language = "EN".to_owned();
                    let id = db_chapter::chapter_uuid(
                        manga_id,
                        chapter_base,
                        chapter_variant,
                        &language,
                        None,
                        None,
                    );
                    let chapter = Chapter {
                        id,
                        manga_id,
                        chapter_base,
                        chapter_variant,
                        is_extra: false,
                        title: None,
                        language,
                        scanlator_group: None,
                        provider_name: None,
                        chapter_url: None,
                        download_status: DownloadStatus::Downloaded,
                        released_at: None,
                        downloaded_at,
                        scraped_at: None,
                        file_size_bytes: file_size,
                    };
                    db_chapter::insert(pool, &chapter)
                        .await
                        .map_err(|e| e.to_string())?;
                    found += 1;
                }
                Err(e) => {
                    warn!("[scanner] DB error for chapter {num_str}: {e}");
                }
            }
        }
    }

    // For previously-Downloaded chapters whose files no longer exist:
    // - Local/unknown-provider chapters are deleted (they won't be recreated by any provider).
    // - Provider-backed chapters are marked Missing (they'll resurface on the next scan).
    let downloaded = db_chapter::get_downloaded(pool, manga_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut marked_missing = 0u32;
    let mut deleted = 0u32;
    for (chapter_id, provider_name) in downloaded {
        if found_ids.contains(&chapter_id) {
            continue;
        }
        let is_local = provider_name
            .as_deref()
            .map_or(true, |p| p.eq_ignore_ascii_case("local"));
        if is_local {
            db_chapter::delete(pool, chapter_id)
                .await
                .map_err(|e| e.to_string())?;
            deleted += 1;
        } else {
            db_chapter::set_status(pool, chapter_id, DownloadStatus::Missing, None)
                .await
                .map_err(|e| e.to_string())?;
            marked_missing += 1;
        }
    }
    if marked_missing > 0 || deleted > 0 {
        info!(
            "[scanner] Disk scan for '{}': {marked_missing} chapter(s) marked Missing, {deleted} local chapter(s) deleted.",
            manga.metadata.title
        );
    }

    // Recompute canonical chapters (disk-scanned files win with no trusted groups needed)
    db_chapter::update_canonical(pool, manga_id, &[], "", &std::collections::HashMap::new())
        .await
        .map_err(|e| e.to_string())?;

    info!(
        "[scanner] Disk scan for '{}': {found} chapter(s) marked as downloaded.",
        manga.metadata.title
    );

    Ok(())
}
