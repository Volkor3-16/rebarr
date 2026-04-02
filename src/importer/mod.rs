use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::warn;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::task::TaskType;
use crate::db::{chapter as db_chapter, library as db_library, manga as db_manga, task as db_task};
use crate::http::metadata::AniListMetadata;
use crate::manga::{comicinfo, covers, files};
use crate::manga::core::{Chapter, DownloadStatus, Manga};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImportTier {
    /// Rebarr-generated CBZ: Notes field contains full JSON with chapter UUID.
    Rebarr,
    /// External CBZ with a readable ComicInfo.xml (<Series> and/or <Number> tags).
    ComicInfo,
    /// No usable ComicInfo; chapter number extracted from filename via regex.
    Filename,
}

#[derive(Debug, Serialize)]
pub struct SuggestedManga {
    pub manga_id: String,
    pub title: String,
    pub anilist_id: Option<u32>,
    /// 1.0 = exact AniList ID match; < 1.0 = Jaro-Winkler title similarity.
    pub confidence: f32,
}

#[derive(Debug, Serialize)]
pub struct ImportCandidate {
    pub cbz_path: String,
    pub file_name: String,
    pub import_tier: ImportTier,
    pub anilist_id: Option<u32>,
    /// Series title from ComicInfo `<Series>` tag.
    pub detected_title: Option<String>,
    /// Chapter number (f32): e.g. 1.5 → base 1, variant 5.
    pub chapter_number: Option<f32>,
    pub chapter_title: Option<String>,
    pub scanlator_group: Option<String>,
    pub language: Option<String>,
    /// Preserved from rebarr JSON Notes (Tier 1 only).
    pub provider_name: Option<String>,
    pub is_extra: Option<bool>,
    pub chapter_uuid: Option<String>,
    pub released_at: Option<i64>,
    pub downloaded_at: Option<i64>,
    pub scraped_at: Option<i64>,
    pub suggested_manga: Option<SuggestedManga>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmedImport {
    pub cbz_path: String,
    pub manga_id: String,
    /// Required — user must confirm or fill in before executing.
    pub chapter_number: f32,
    pub chapter_title: Option<String>,
    pub scanlator_group: Option<String>,
    /// BCP 47 language code (defaults to "EN").
    pub language: Option<String>,
    /// Provider name (defaults to "Local").
    pub provider_name: Option<String>,
    #[serde(default)]
    pub is_extra: bool,
    /// When true, leave the source file in place (copy instead of move).
    #[serde(default)]
    pub copy: bool,
    /// Chapter UUID to preserve (from Tier 1 rebarr CBZ).
    pub chapter_uuid: Option<String>,
    pub released_at: Option<i64>,
    pub downloaded_at: Option<i64>,
    pub scraped_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ImportSummary {
    pub moved: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Series import types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FolderEntry {
    pub folder_name: String,
    pub folder_path: String,
    pub cbz_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmedSeriesImport {
    /// Carried for error messages only — never written to DB.
    pub folder_path: String,
    pub anilist_id: i32,
    pub library_id: String,
    pub relative_path: String,
}

#[derive(Debug, Serialize)]
pub struct SeriesImportSummary {
    pub added: u32,
    pub skipped_duplicates: u32,
    pub errors: Vec<String>,
    /// UUIDs of newly-created manga entries.
    pub manga_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Series-level scan and import
// ---------------------------------------------------------------------------

/// List immediate subdirectories of `dir` as series candidates.
/// Skips hidden directories (names starting with `.`).
/// Returns entries sorted alphabetically by folder_name.
pub async fn scan_series_dir(dir: PathBuf) -> Result<Vec<FolderEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if folder_name.is_empty() || folder_name.starts_with('.') {
                continue;
            }
            let cbz_count = count_cbz_shallow(&path);
            entries.push(FolderEntry {
                folder_name,
                folder_path: path.to_string_lossy().into_owned(),
                cbz_count,
            });
        }
        entries.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn count_cbz_shallow(dir: &Path) -> u32 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.eq_ignore_ascii_case("cbz"))
                        .unwrap_or(false)
                })
                .count() as u32
        })
        .unwrap_or(0)
}

/// Bulk-create manga entries from confirmed series matches.
/// Always enqueues `ScanDisk` for each added manga.
/// Optionally enqueues `BuildFullChapterList` when `queue_chapter_scan` is true.
pub async fn execute_series_imports(
    imports: Vec<ConfirmedSeriesImport>,
    pool: &SqlitePool,
    al_client: &AniListMetadata,
    http_client: &reqwest::Client,
    queue_chapter_scan: bool,
) -> SeriesImportSummary {
    let mut added = 0u32;
    let mut skipped_duplicates = 0u32;
    let mut errors = Vec::new();
    let mut manga_ids = Vec::new();

    for imp in imports {
        let library_id = match Uuid::parse_str(&imp.library_id) {
            Ok(id) => id,
            Err(e) => {
                errors.push(format!("{}: invalid library_id: {e}", imp.folder_path));
                manga_ids.push(String::new());
                continue;
            }
        };

        let al_id_u32 = imp.anilist_id as u32;

        // Duplicate check by anilist_id in this library
        match db_manga::exists_by_external_ids(pool, library_id, Some(al_id_u32), None).await {
            Ok(Some(existing)) => {
                // Return the existing UUID so the frontend can still import CBZs for this series
                skipped_duplicates += 1;
                manga_ids.push(existing.id.to_string());
                continue;
            }
            Err(e) => {
                errors.push(format!(
                    "{}: DB error checking duplicates: {e}",
                    imp.folder_path
                ));
                manga_ids.push(String::new());
                continue;
            }
            Ok(None) => {}
        }

        // Fetch fresh metadata from AniList
        let mut manga = match al_client.grab_manga(imp.anilist_id).await {
            Ok(m) => m,
            Err(e) => {
                errors.push(format!("{}: AniList fetch failed: {e}", imp.folder_path));
                manga_ids.push(String::new());
                continue;
            }
        };

        // Apply library and path overrides
        manga.id = db_manga::manga_uuid(al_id_u32);
        manga.library_id = library_id;
        manga.relative_path = std::path::PathBuf::from(imp.relative_path.trim());
        manga.created_at = Utc::now().timestamp();
        manga.metadata_updated_at = Utc::now().timestamp();

        // Load library for cover download path
        let library = match db_library::get_by_id(pool, library_id).await {
            Ok(Some(l)) => l,
            Ok(None) => {
                errors.push(format!("{}: library not found", imp.folder_path));
                manga_ids.push(String::new());
                continue;
            }
            Err(e) => {
                errors.push(format!(
                    "{}: DB error fetching library: {e}",
                    imp.folder_path
                ));
                manga_ids.push(String::new());
                continue;
            }
        };

        // Download cover (non-fatal: fall back to original URL)
        if let Some(url) = manga.thumbnail_url.take() {
            let series_dir = files::series_dir(&library.root_path, &manga);
            manga.thumbnail_url = covers::download_cover(http_client, &url, manga.id, &series_dir)
                .await
                .or(Some(url));
        }

        // Insert manga into DB
        if let Err(e) = db_manga::insert(pool, &manga).await {
            errors.push(format!("{}: DB insert failed: {e}", imp.folder_path));
            manga_ids.push(String::new());
            continue;
        }

        // Write series-level ComicInfo.xml (non-fatal)
        let series_dir = files::series_dir(&library.root_path, &manga);
        if let Err(e) = comicinfo::write_series_comicinfo(&series_dir, &manga).await {
            warn!(
                "[series-import] Failed to write ComicInfo.xml for '{}': {e}",
                manga.metadata.title
            );
        }

        // Note: source folder is NOT moved here. The chapter import step (importApi.execute)
        // handles individual CBZ files with proper renaming and DB entries.

        // Always enqueue ScanDisk to pick up any CBZ files that land in the library dir
        let _ = db_task::enqueue(pool, TaskType::ScanDisk, Some(manga.id), None, 5).await;

        // Optionally enqueue full chapter list build
        if queue_chapter_scan {
            let _ =
                db_task::enqueue(pool, TaskType::BuildFullChapterList, Some(manga.id), None, 5)
                    .await;
        }

        manga_ids.push(manga.id.to_string());
        added += 1;
    }

    SeriesImportSummary {
        added,
        skipped_duplicates,
        errors,
        manga_ids,
    }
}


// ---------------------------------------------------------------------------
// Chapter-level scan
// ---------------------------------------------------------------------------

pub async fn scan_directory(
    dir: PathBuf,
    pool: &SqlitePool,
) -> Result<Vec<ImportCandidate>, String> {
    // Load manga titles for matching (async DB query)
    let all_titles = db_manga::get_all_titles(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Filesystem walk and CBZ parsing in a blocking thread
    tokio::task::spawn_blocking(move || {
        let mut cbz_files = Vec::new();
        collect_cbz_files(&dir, &mut cbz_files).map_err(|e| e.to_string())?;

        let mut candidates: Vec<ImportCandidate> = cbz_files
            .iter()
            .map(|path| classify_cbz(path, &all_titles))
            .collect();

        // Tier 1 (rebarr) first, then tier 2 (comicinfo), then tier 3 (filename)
        // Within each tier sort by filename.
        candidates.sort_by(|a, b| {
            let ta = tier_order(&a.import_tier);
            let tb = tier_order(&b.import_tier);
            ta.cmp(&tb).then_with(|| a.file_name.cmp(&b.file_name))
        });

        Ok(candidates)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn tier_order(tier: &ImportTier) -> u8 {
    match tier {
        ImportTier::Rebarr => 0,
        ImportTier::ComicInfo => 1,
        ImportTier::Filename => 2,
    }
}

fn collect_cbz_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_cbz_files(&path, out)?;
        } else {
            let is_cbz = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("cbz"))
                .unwrap_or(false);
            if is_cbz {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn is_generic_chapter_title(title: &str) -> bool {
    let trimmed = title.trim();
    // Match patterns like "Chapter 43", "Ch. 43", "Ch 43", "chapter172", "Ch.27.4"
    let re = Regex::new(r"(?i)^ch(?:apter)?[._\s]*\d").ok();
    re.map(|r| r.is_match(trimmed)).unwrap_or(false)
}

fn extract_title_from_filename(filename: &str) -> Option<String> {
    let stem = filename
        .strip_suffix(".cbz")
        .or_else(|| filename.strip_suffix(".CBZ"))
        .unwrap_or(filename);

    // Try to extract title after chapter number patterns
    // Examples: "Chapter 43 - The Great Battle", "Ch.73 The Battle Begins", "Vol.4 Chapter 28 Title Here"
    let re = Regex::new(r"(?i)(?:vol\.?\s*\d+\s+)?ch(?:apter)?[._\s]*\d+(?:[._]\d+)?\s*[-–—:]?\s*(.+)").ok()?;
    
    if let Some(caps) = re.captures(stem) {
        if let Some(title_match) = caps.get(1) {
            let title = title_match.as_str().trim();
            // Remove trailing scanlator group info in brackets if present
            let title = Regex::new(r"\s*\[.*?\]\s*$")
                .ok().map(|r| r.replace(title, "").to_string())
                .unwrap_or_else(|| title.to_string());
            
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn classify_cbz(path: &Path, all_titles: &[db_manga::MangaSummary]) -> ImportCandidate {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let parsed = comicinfo::read_cbz_comicinfo(path);

    let tier;
    let anilist_id;
    let detected_title;
    let chapter_number;
    let chapter_title;
    let scanlator_group;
    let language;
    let provider_name;
    let is_extra;
    let chapter_uuid;
    let released_at;
    let downloaded_at;
    let scraped_at;

    if let Some(ref info) = parsed {
        if info.chapter_uuid.is_some() {
            // Tier 1: rebarr-generated CBZ (UUID only comes from rebarr JSON Notes)
            tier = ImportTier::Rebarr;
            anilist_id = info.anilist_id;
            detected_title = info.title.clone();
            chapter_number = info.chapter_number;
            // Filter out generic chapter titles (e.g. "Chapter 43")
            chapter_title = info.chapter_title.clone().filter(|t| !is_generic_chapter_title(t));
            scanlator_group = info.scanlator.clone();
            language = info.language.clone();
            provider_name = info.provider_name.clone();
            is_extra = info.is_extra;
            chapter_uuid = info.chapter_uuid.map(|u| u.to_string());
            released_at = info.released_at;
            downloaded_at = info.downloaded_at;
            scraped_at = info.scraped_at;
        } else if info.title.is_some() || info.chapter_number.is_some() || info.anilist_id.is_some()
        {
            // Tier 2: plain ComicInfo with useful fields
            tier = ImportTier::ComicInfo;
            anilist_id = info.anilist_id;
            detected_title = info.title.clone();
            chapter_number = info.chapter_number;
            // Filter out generic chapter titles (e.g. "Chapter 43")
            chapter_title = info.chapter_title.clone().filter(|t| !is_generic_chapter_title(t));
            scanlator_group = info.scanlator.clone();
            language = info.language.clone();
            provider_name = info.provider_name.clone();
            is_extra = None;
            chapter_uuid = None;
            released_at = info.released_at;
            downloaded_at = info.downloaded_at;
            scraped_at = info.scraped_at;
        } else {
            // ComicInfo present but empty — fall through to Tier 3
            tier = ImportTier::Filename;
            anilist_id = None;
            detected_title = None;
            chapter_number = extract_chapter_number_from_filename(&file_name);
            // Try to extract title from filename
            chapter_title = extract_title_from_filename(&file_name);
            scanlator_group = None;
            language = None;
            provider_name = None;
            is_extra = None;
            chapter_uuid = None;
            released_at = None;
            downloaded_at = None;
            scraped_at = None;
        }
    } else {
        // Tier 3: no ComicInfo at all
        tier = ImportTier::Filename;
        anilist_id = None;
        detected_title = None;
        chapter_number = extract_chapter_number_from_filename(&file_name);
        // Try to extract title from filename
        chapter_title = extract_title_from_filename(&file_name);
        scanlator_group = None;
        language = None;
        provider_name = None;
        is_extra = None;
        chapter_uuid = None;
        released_at = None;
        downloaded_at = None;
        scraped_at = None;
    }

    let suggested_manga = find_match(anilist_id, detected_title.as_deref(), all_titles);

    ImportCandidate {
        cbz_path: path.to_string_lossy().into_owned(),
        file_name,
        import_tier: tier,
        anilist_id,
        detected_title,
        chapter_number,
        chapter_title,
        scanlator_group,
        language,
        provider_name,
        is_extra,
        chapter_uuid,
        released_at,
        downloaded_at,
        scraped_at,
        suggested_manga,
    }
}

/// Best-effort chapter number extraction from filename via regex.
/// Handles formats like: "Ch.73", "Chapter 22.1", "Vol.4 Chapter 28", etc.
fn extract_chapter_number_from_filename(filename: &str) -> Option<f32> {
    let stem = filename
        .strip_suffix(".cbz")
        .or_else(|| filename.strip_suffix(".CBZ"))
        .unwrap_or(filename);

    // Matches: "Ch.73", "Ch. 73", "Chapter 22.1", "Chapter_014", "chapter172"
    // The number may use '.' or '_' as decimal separator (e.g. "Ch.27.4" or "27_4").
    let re = Regex::new(r"(?i)ch(?:apter)?[._\s]*(\d+(?:[._]\d+)?)").ok()?;

    if let Some(caps) = re.captures(stem) {
        let num_str = caps.get(1)?.as_str().replace('_', ".");
        // If there are multiple dots (e.g. "27.4.0"), take the first two parts.
        let clean: String = {
            let mut parts = num_str.splitn(3, '.');
            let int_part = parts.next().unwrap_or("0");
            match parts.next() {
                Some(dec) => format!("{int_part}.{dec}"),
                None => int_part.to_string(),
            }
        };
        return clean.parse().ok();
    }

    None
}

fn find_match(
    anilist_id: Option<u32>,
    detected_title: Option<&str>,
    all_titles: &[db_manga::MangaSummary],
) -> Option<SuggestedManga> {
    // Exact AniList ID match: confidence 1.0
    if let Some(al_id) = anilist_id {
        if let Some(m) = all_titles.iter().find(|m| m.anilist_id == Some(al_id)) {
            return Some(SuggestedManga {
                manga_id: m.id.to_string(),
                title: m.title.clone(),
                anilist_id: m.anilist_id,
                confidence: 1.0,
            });
        }
    }

    // Fuzzy title match (Jaro-Winkler, threshold 0.85)
    if let Some(title) = detected_title {
        let title_lower = title.to_lowercase();
        let mut best_score = 0.0f64;
        let mut best_idx: Option<usize> = None;

        for (i, m) in all_titles.iter().enumerate() {
            let score = strsim::jaro_winkler(&m.title.to_lowercase(), &title_lower);
            if score > best_score {
                best_score = score;
                best_idx = Some(i);
            }
        }

        if let Some(i) = best_idx {
            let m = &all_titles[i];
            return Some(SuggestedManga {
                manga_id: m.id.to_string(),
                title: m.title.clone(),
                anilist_id: m.anilist_id,
                confidence: best_score as f32,
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

pub async fn execute_imports(imports: Vec<ConfirmedImport>, pool: &SqlitePool) -> ImportSummary {
    let mut moved = 0u32;
    let mut skipped = 0u32;
    let mut errors = Vec::new();
    let mut manga_ids_to_scan = std::collections::HashSet::new();

    for imp in imports {
        let path_display = imp.cbz_path.clone();
        match process_single_import(imp, pool).await {
            Ok(manga_id) => {
                moved += 1;
                manga_ids_to_scan.insert(manga_id);
            }
            Err(e) => {
                errors.push(format!("{path_display}: {e}"));
                skipped += 1;
            }
        }
    }

    for manga_id in manga_ids_to_scan {
        let _ = db_task::enqueue(pool, TaskType::ScanDisk, Some(manga_id), None, 5).await;
    }

    ImportSummary {
        moved,
        skipped,
        errors,
    }
}

async fn process_single_import(imp: ConfirmedImport, pool: &SqlitePool) -> Result<Uuid, String> {
    let manga_id = Uuid::parse_str(&imp.manga_id).map_err(|e| e.to_string())?;

    let manga = db_manga::get_by_id(pool, manga_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("manga {manga_id} not found"))?;

    let library = db_library::get_by_id(pool, manga.library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("library {} not found", manga.library_id))?;

    let chapter = build_chapter(&imp, manga_id)?;

    let target_dir = library.root_path.join(&manga.relative_path);
    let target_path = target_dir.join(format!("{}.cbz", cbz_stem(&chapter)));
    let src_path = PathBuf::from(&imp.cbz_path);
    let tmp_path = target_path.with_extension("cbz.rebarr_tmp");

    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    // Rewrite CBZ in a blocking thread (copy images + fresh ComicInfo.xml)
    let src_clone = src_path.clone();
    let tmp_clone = tmp_path.clone();
    let manga_clone = manga.clone();
    let chapter_clone = chapter.clone();
    tokio::task::block_in_place(|| {
        rewrite_cbz(&src_clone, &tmp_clone, &manga_clone, &chapter_clone)
    })?;

    // Atomic rename of tmp → target
    std::fs::rename(&tmp_path, &target_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        e.to_string()
    })?;

    // Delete source if it differs from target and we're moving (not copying)
    if !imp.copy && src_path != target_path {
        if let Err(e) = std::fs::remove_file(&src_path) {
            warn!(
                "[import] Could not delete source {}: {e}",
                src_path.display()
            );
        }
    }

    // Record actual file size from the written CBZ
    let file_size_bytes = std::fs::metadata(&target_path).map(|m| m.len() as i64).ok();
    let mut chapter_record = chapter.clone();
    chapter_record.file_size_bytes = file_size_bytes;

    // Insert chapter DB record immediately (non-fatal: ScanDisk is a safety net)
    if let Err(e) = db_chapter::insert(pool, &chapter_record).await {
        warn!("[import] Failed to insert chapter record for {}: {e}", target_path.display());
    }

    Ok(manga_id)
}

fn build_chapter(imp: &ConfirmedImport, manga_id: Uuid) -> Result<Chapter, String> {
    let n = imp.chapter_number;
    let chapter_base = n.floor() as i32;
    let chapter_variant = ((n - n.floor()).abs() * 10.0).round() as i32;

    let chapter_id = imp
        .chapter_uuid
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4);

    let now = Utc::now();

    let released_at = imp
        .released_at
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

    let downloaded_at = Some(
        imp.downloaded_at
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .unwrap_or(now),
    );

    let scraped_at = imp
        .scraped_at
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

    Ok(Chapter {
        id: chapter_id,
        manga_id,
        chapter_base,
        chapter_variant,
        is_extra: imp.is_extra,
        title: imp.chapter_title.clone(),
        language: imp.language.clone().unwrap_or_else(|| "EN".to_string()),
        scanlator_group: imp.scanlator_group.clone(),
        provider_name: Some(
            imp.provider_name
                .clone()
                .unwrap_or_else(|| "Local".to_string()),
        ),
        chapter_url: None,
        download_status: DownloadStatus::Downloaded,
        released_at,
        downloaded_at,
        scraped_at,
        file_size_bytes: None,
    })
}

/// Compute the CBZ stem (without extension), matching the downloader's naming convention.
fn cbz_stem(chapter: &Chapter) -> String {
    let mut name = format!("Chapter {}", chapter.number_sort());
    if let Some(ref t) = chapter.title {
        if !t.is_empty() {
            name.push_str(&format!(" - {t}"));
        }
    }
    if let Some(ref g) = chapter.scanlator_group {
        if !g.is_empty() {
            name.push_str(&format!(" [{g}]"));
        }
    }
    name.chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Copy all image entries from `src` CBZ, replace ComicInfo.xml with a freshly generated one,
/// and write the result to `dst`.
fn rewrite_cbz(src: &Path, dst: &Path, manga: &Manga, chapter: &Chapter) -> Result<(), String> {
    let src_file = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(src_file).map_err(|e| e.to_string())?;

    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let name_lower = name.to_ascii_lowercase();

        // Skip existing ComicInfo.xml — we'll regenerate it
        if name_lower == "comicinfo.xml" || name_lower.ends_with("/comicinfo.xml") {
            continue;
        }

        // Only carry image files
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif"
        ) {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
            images.push((name, data));
        }
    }

    // Preserve original ordering by filename
    images.sort_by(|(a, _), (b, _)| a.cmp(b));

    let page_count = images.len();
    let comic_info = comicinfo::generate_chapter_xml(
        manga,
        chapter,
        page_count,
        chapter.provider_name.as_deref(),
    );

    let out_file = std::fs::File::create(dst).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(out_file);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("ComicInfo.xml", opts)
        .map_err(|e| e.to_string())?;
    zip.write_all(comic_info.as_bytes())
        .map_err(|e| e.to_string())?;

    for (name, data) in images {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}
