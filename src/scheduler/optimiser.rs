// This file handles:
// 1. Unpacking a CBZ File
// 2. Checking if the images inside are not WebP
// 3. Re-encoding them into lossess WebP
// 4. Repacking the CBZ File.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::{chapter as db_chapter, library as db_library, manga as db_manga};

/// Re-encode all image pages in a chapter's CBZ to WebP, reducing file size.
/// The chapter must already be Downloaded (CBZ file on disk).
/// The CBZ is replaced in-place: images are decoded and re-encoded to WebP.
pub async fn optimise_chapter(pool: &SqlitePool, chapter_id: Uuid) -> Result<(), String> {
    let chapter = db_chapter::get_by_id(pool, chapter_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("chapter {chapter_id} not found"))?;

    let manga = db_manga::get_by_id(pool, chapter.manga_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("manga {} not found", chapter.manga_id))?;

    let library = db_library::get_by_id(pool, manga.library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("library {} not found", manga.library_id))?;

    let cbz_path = library
        .root_path
        .join(&manga.relative_path)
        .join(format!("Chapter {}.cbz", chapter.number_raw));

    if !cbz_path.exists() {
        return Err(format!("CBZ not found: {}", cbz_path.display()));
    }

    let path = cbz_path.clone();
    tokio::task::spawn_blocking(move || repack_cbz_webp(&path))
        .await
        .map_err(|e| format!("optimise task panicked: {e}"))??;

    log::info!(
        "[optimizer] Optimised '{}' Ch.{} → WebP",
        manga.metadata.title,
        chapter.number_raw
    );

    Ok(())
}

/// Read a CBZ, re-encode all JPEG/PNG pages to WebP, write back.
fn repack_cbz_webp(path: &Path) -> Result<(), String> {
    let original = std::fs::read(path).map_err(|e| format!("read CBZ: {e}"))?;

    let cursor = Cursor::new(&original);
    let mut zip_in =
        zip::ZipArchive::new(cursor).map_err(|e| format!("open zip: {e}"))?;

    let mut out_buf = Vec::with_capacity(original.len());
    let out_cursor = Cursor::new(&mut out_buf);
    let mut zip_out = zip::ZipWriter::new(out_cursor);

    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    let mut any_converted = false;
    for i in 0..zip_in.len() {
        let mut entry = zip_in.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let name = entry.name().to_owned();

        let is_image = name.ends_with(".jpg")
            || name.ends_with(".jpeg")
            || name.ends_with(".png");

        if is_image {
            let mut raw = Vec::new();
            entry.read_to_end(&mut raw).map_err(|e| format!("read entry '{name}': {e}"))?;

            match convert_to_webp(&raw) {
                Ok(webp_bytes) => {
                    // Replace extension with .webp
                    let new_name = if let Some(stem) = name
                        .strip_suffix(".jpg")
                        .or_else(|| name.strip_suffix(".jpeg"))
                        .or_else(|| name.strip_suffix(".png"))
                    {
                        format!("{stem}.webp")
                    } else {
                        name.clone()
                    };
                    zip_out
                        .start_file(&new_name, opts)
                        .map_err(|e| format!("write entry '{new_name}': {e}"))?;
                    zip_out
                        .write_all(&webp_bytes)
                        .map_err(|e| format!("write bytes '{new_name}': {e}"))?;
                    any_converted = true;
                }
                Err(e) => {
                    // Fall back to original bytes if conversion fails
                    log::warn!("[optimizer] Skipping '{name}' (convert failed: {e})");
                    zip_out
                        .start_file(&name, opts)
                        .map_err(|e| format!("write entry '{name}': {e}"))?;
                    zip_out
                        .write_all(&raw)
                        .map_err(|e| format!("write bytes '{name}': {e}"))?;
                }
            }
        } else {
            // Non-image entry (e.g. ComicInfo.xml): copy verbatim
            let mut raw = Vec::new();
            entry.read_to_end(&mut raw).map_err(|e| format!("read entry '{name}': {e}"))?;
            zip_out
                .start_file(&name, opts)
                .map_err(|e| format!("write entry '{name}': {e}"))?;
            zip_out
                .write_all(&raw)
                .map_err(|e| format!("write bytes '{name}': {e}"))?;
        }
    }

    zip_out.finish().map_err(|e| format!("finalize zip: {e}"))?;

    if !any_converted {
        return Ok(()); // Nothing to rewrite
    }

    std::fs::write(path, &out_buf).map_err(|e| format!("write CBZ: {e}"))?;

    Ok(())
}

/// Decode an image from raw bytes and re-encode as WebP.
fn convert_to_webp(raw: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(raw).map_err(|e| format!("decode: {e}"))?;

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::WebP)
        .map_err(|e| format!("encode webp: {e}"))?;

    Ok(buf)
}
