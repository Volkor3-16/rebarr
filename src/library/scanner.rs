use sqlx::SqlitePool;
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
            log::info!(
                "[scanner] Series directory does not exist for '{}': {e}",
                manga.metadata.title
            );
            return Ok(());
        }
    };

    let mut found = 0u32;
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
                // Always refresh file size (may not have been recorded before)
                if let Some(size) = file_size {
                    let _ = db_chapter::set_file_size(pool, ch.id, size).await;
                }
            }
            Ok(None) => {
                // No canonical entry — insert a row marked as Downloaded (provider_name = NULL),
                // enriched with metadata from the embedded ComicInfo.xml when available.
                let released_at = cbz_info
                    .as_ref()
                    .and_then(|i| i.release_year)
                    .and_then(|y| chrono::NaiveDate::from_ymd_opt(y, 1, 1))
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc());
                let language = cbz_info
                    .as_ref()
                    .and_then(|i| i.language.clone())
                    .unwrap_or_else(|| "EN".to_owned());
                let scanlator_group = cbz_info.as_ref().and_then(|i| i.scanlator.clone());
                let provider_name = cbz_info.as_ref().and_then(|i| i.provider_name.clone());
                let chapter = Chapter {
                    id: db_chapter::chapter_uuid(
                        manga_id,
                        chapter_base,
                        chapter_variant,
                        &language,
                        scanlator_group.as_deref(),
                        provider_name.as_deref(),
                    ),
                    manga_id,
                    chapter_base,
                    chapter_variant,
                    is_extra: false,
                    title: cbz_info.as_ref().and_then(|i| i.chapter_title.clone()),
                    language,
                    scanlator_group,
                    // Recover provider from embedded ComicInfo Notes
                    provider_name,
                    chapter_url: None,
                    download_status: DownloadStatus::Downloaded,
                    released_at,
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
                log::warn!("[scanner] DB error for chapter {num_str}: {e}");
            }
        }
    }

    // Recompute canonical chapters (disk-scanned files win with no trusted groups needed)
    db_chapter::update_canonical(pool, manga_id, &[], "")
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "[scanner] Disk scan for '{}': {found} chapter(s) marked as downloaded.",
        manga.metadata.title
    );

    Ok(())
}
