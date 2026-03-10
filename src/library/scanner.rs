use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::{chapter as db_chapter, library as db_library, manga as db_manga};
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
            // Directory may not exist yet — not an error
            log::info!("[scanner] Series directory does not exist for '{}': {e}", manga.metadata.title);
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
        // Strip prefix "Chapter " and suffix ".cbz"
        let Some(rest) = name.strip_prefix("Chapter ") else { continue };
        let Some(num_str) = rest.strip_suffix(".cbz") else { continue };
        let Ok(number_sort) = num_str.parse::<f32>() else { continue };

        // Get mtime for downloaded_at
        let downloaded_at = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| {
                let secs = t
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                chrono::DateTime::from_timestamp(secs as i64, 0)
            });

        // Check if chapter exists in DB
        match db_chapter::get_by_number(pool, manga_id, number_sort).await {
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
            }
            Ok(None) => {
                // Insert a minimal chapter record marked as Downloaded
                let chapter_base = number_sort.floor();
                let frac = (number_sort - chapter_base).abs();
                let chapter_variant = (frac * 10.0).round() as u8;
                let is_extra = frac >= 0.5 - f32::EPSILON;
                let chapter = Chapter {
                    id: Uuid::new_v4(),
                    manga_id,
                    number_raw: num_str.to_owned(),
                    number_sort,
                    chapter_base,
                    chapter_variant,
                    is_extra,
                    title: None,
                    volume: None,
                    scanlator_group: None,
                    preferred_provider: None,
                    download_status: DownloadStatus::Downloaded,
                    downloaded_at,
                    created_at: downloaded_at.unwrap_or_else(chrono::Utc::now),
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

    // Update chapter_count and downloaded_count
    db_chapter::update_manga_counts(pool, manga_id)
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "[scanner] Disk scan for '{}': {found} chapter(s) marked as downloaded.",
        manga.metadata.title
    );

    Ok(())
}
