use std::io::Read as _;
use std::path::Path;

use crate::manga::manga::{Chapter, Manga, Synonym, SynonymSource};
use chrono::Datelike;

// This file handles the creation of ComicInfo.xml using the various sources of info

// ---------------------------------------------------------------------------
// ComicInfo.xml parser
// ---------------------------------------------------------------------------

/// Metadata extracted from a ComicInfo.xml file (series-level and/or chapter-level).
#[derive(Debug, Default)]
pub struct ParsedComicInfo {
    pub title: Option<String>,
    /// Parsed as Synonym with Manual source (user-added from ComicInfo.xml)
    pub other_titles: Option<Vec<Synonym>>,
    pub synopsis: Option<String>,
    pub start_year: Option<i32>,
    pub tags: Vec<String>,
    /// AniList series ID, parsed from `<Web>` URL or JSON Notes.
    pub anilist_id: Option<u32>,
    /// Provider name, parsed from JSON Notes or legacy `rebarr:provider=` text.
    pub provider_name: Option<String>,
    // Chapter-level fields
    pub chapter_title: Option<String>,
    pub scanlator: Option<String>,
    pub language: Option<String>,
    pub release_year: Option<i32>,
    // Extended fields from JSON Notes (new format)
    pub chapter_uuid: Option<uuid::Uuid>,
    pub chapter_url: Option<String>,
    pub released_at: Option<i64>,
    pub downloaded_at: Option<i64>,
    pub scraped_at: Option<i64>,
    /// Chapter number from `<Number>` tag (used by importer for Tier 1/2 matching).
    pub chapter_number: Option<f32>,
    /// `is_extra` flag, parsed from JSON Notes (rebarr-generated CBZs only).
    pub is_extra: Option<bool>,
}

/// Extract the text content of the first occurrence of `<tag>...</tag>` in `xml`.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    let val = xml[start..start + end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_owned())
    }
}

/// Parse a ComicInfo.xml string into a `ParsedComicInfo`.
/// Returns default ParsedComicInfo if parsing fails.
pub fn parse_comicinfo(xml: &str) -> ParsedComicInfo {
    let mut info = ParsedComicInfo::default();

    // Safely extract all fields with error handling
    info.title = extract_tag(xml, "Series");
    info.other_titles = extract_tag(xml, "AlternateSeries").map(|s| {
        s.split(';')
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .map(|title| Synonym {
                title,
                source: SynonymSource::Manual,
                hidden: false,
                filter_reason: None,
            })
            .collect::<Vec<_>>()
    });
    if let Some(ref v) = info.other_titles {
        if v.is_empty() {
            info.other_titles = None;
        }
    }
    info.synopsis = extract_tag(xml, "Summary");
    info.start_year = extract_tag(xml, "Year").and_then(|s| s.parse().ok());

    // Parse tags from <Tags> field (preferred) or fall back to <Genre> field for backward compatibility
    info.tags = extract_tag(xml, "Tags")
        .or_else(|| extract_tag(xml, "Genre"))
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // AniList ID from <Web>https://anilist.co/manga/12345</Web>
    info.anilist_id = extract_tag(xml, "Web").and_then(|url| {
        url.trim_end_matches('/')
            .rsplit('/')
            .next()
            .and_then(|s| s.parse().ok())
    });

    // Notes field: either new JSON format {"series":{...},"chapter":{...}}
    // or legacy text format: rebarr:anilist_id=12345 rebarr:provider=ProviderName
    if let Some(notes) = extract_tag(xml, "Notes") {
        let notes = xml_unescape(&notes);
        if notes.trim_start().starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&notes) {
                if info.anilist_id.is_none() {
                    info.anilist_id = v["series"]["anilist_id"].as_u64().map(|n| n as u32);
                }
                let ch = &v["chapter"];
                info.chapter_uuid = ch["uuid"].as_str().and_then(|s| s.parse().ok());
                info.provider_name = ch["provider_name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_owned());
                if info.chapter_title.is_none() {
                    info.chapter_title = ch["title"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_owned());
                }
                if info.scanlator.is_none() {
                    info.scanlator = ch["scanlator_group"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_owned());
                }
                if info.language.is_none() {
                    info.language = ch["language"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_owned());
                }
                info.is_extra = ch["is_extra"].as_bool();
                info.released_at = ch["released_at"].as_i64();
                info.downloaded_at = ch["downloaded_at"].as_i64();
                info.scraped_at = ch["scraped_at"].as_i64();
                info.chapter_url = ch["chapter_url"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_owned());
            }
        } else {
            // Legacy text format: rebarr:anilist_id=12345 rebarr:provider=ProviderName
            if info.anilist_id.is_none() {
                info.anilist_id = notes
                    .split("rebarr:anilist_id=")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse().ok());
            }
            info.provider_name = notes
                .split("rebarr:provider=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_owned());
        }
    }

    info.chapter_title = extract_tag(xml, "Title");
    info.scanlator = extract_tag(xml, "ScanInformation");
    info.language = extract_tag(xml, "LanguageISO");
    info.release_year = extract_tag(xml, "Year").and_then(|s| s.parse().ok());
    info.chapter_number = extract_tag(xml, "Number").and_then(|s| s.trim().parse().ok());

    info
}

/// Open a CBZ archive and parse the embedded `ComicInfo.xml`, if present.
/// Returns `None` on any error (missing file, bad zip, parse failure).
pub fn read_cbz_comicinfo(cbz_path: &Path) -> Option<ParsedComicInfo> {
    // Check if file exists and is readable
    if !cbz_path.exists() || !cbz_path.is_file() {
        return None;
    }

    let file = std::fs::File::open(cbz_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    // Find ComicInfo.xml (case-insensitive, may be at root or in a subdirectory)
    let mut comicinfo_idx = None;
    for i in 0..archive.len() {
        if let Ok(f) = archive.by_index(i) {
            let n = f.name().to_ascii_lowercase();
            if n == "comicinfo.xml" || n.ends_with("/comicinfo.xml") {
                comicinfo_idx = Some(i);
                break;
            }
        }
    }

    let idx = comicinfo_idx?;
    let mut entry = archive.by_index(idx).ok()?;

    // Read contents with size limit to prevent memory exhaustion
    let mut contents = String::with_capacity(64 * 1024); // 64KB limit
    match entry.read_to_string(&mut contents) {
        Ok(_) => {
            // Basic validation - ensure it's actually XML
            if contents.trim_start().starts_with("<?xml")
                || contents.trim_start().starts_with("<ComicInfo")
            {
                Some(parse_comicinfo(&contents))
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Escape characters that are special in XML text content.
///
/// Only `&`, `<`, and `>` require escaping in element text nodes.
/// Double-quote and single-quote escaping is only required inside
/// attribute values and must NOT be applied here — the JSON stored
/// in `<Notes>` contains double-quotes that must remain unescaped
/// so `serde_json` can parse them back correctly.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Reverse the XML text-content escaping applied by `xml_escape`.
///
/// Called before passing a `<Notes>` value to `serde_json::from_str`
/// so that CBZs written with the old (over-escaped) code still parse.
fn xml_unescape(s: &str) -> std::borrow::Cow<str> {
    if !s.contains('&') {
        return std::borrow::Cow::Borrowed(s);
    }
    std::borrow::Cow::Owned(
        s.replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// Emit an optional string element; omit the tag entirely if `value` is None or empty.
fn opt_str_elem(tag: &str, value: Option<&str>) -> String {
    match value {
        Some(v) if !v.is_empty() => format!("  <{tag}>{}</{tag}>\n", xml_escape(v)),
        _ => String::new(),
    }
}

/// Emit an optional integer element; omit the tag entirely if `value` is None.
fn opt_int_elem<T: std::fmt::Display>(tag: &str, value: Option<T>) -> String {
    match value {
        Some(v) => format!("  <{tag}>{v}</{tag}>\n"),
        None => String::new(),
    }
}

/// Emit an optional integer element (for ratings); omit the tag entirely if `value` is None.
fn opt_int_rating_elem(tag: &str, value: Option<i32>) -> String {
    match value {
        Some(v) => format!("  <{tag}>{v}</{tag}>\n"),
        None => String::new(),
    }
}

/// Emit a vector of strings as a semicolon-separated element.
fn opt_vec_elem(tag: &str, value: Option<&[String]>) -> String {
    match value {
        Some(v) if !v.is_empty() => format!("  <{}>{}</{}>\n", tag, v.join("; "), tag),
        _ => String::new(),
    }
}

/// Serialize series metadata to minified JSON for the notes field.
fn serialize_series_metadata(manga: &Manga) -> Option<String> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct SeriesMetadata {
        uuid: String,
        anilist_id: Option<u32>,
        mal_id: Option<u32>,
        title: String,
        other_titles: Option<Vec<String>>,
        synopsis: Option<String>,
        publishing_status: String,
        start_year: Option<i32>,
        end_year: Option<i32>,
        metadata_source: String,
        monitored: bool,
        created_at: i64,
        metadata_updated_at: i64,
    }

    let other_titles = manga.metadata.other_titles.as_ref().map(|synonyms| {
        synonyms
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.title.clone())
            .collect::<Vec<_>>()
    });

    let series = SeriesMetadata {
        uuid: manga.id.to_string(),
        anilist_id: manga.anilist_id,
        mal_id: manga.mal_id,
        title: manga.metadata.title.clone(),
        other_titles,
        synopsis: manga.metadata.synopsis.clone(),
        publishing_status: match manga.metadata.publishing_status {
            crate::manga::manga::PublishingStatus::Completed => "Completed".to_string(),
            crate::manga::manga::PublishingStatus::Ongoing => "Ongoing".to_string(),
            crate::manga::manga::PublishingStatus::Hiatus => "Hiatus".to_string(),
            crate::manga::manga::PublishingStatus::Cancelled => "Cancelled".to_string(),
            crate::manga::manga::PublishingStatus::NotYetReleased => "NotYetReleased".to_string(),
            crate::manga::manga::PublishingStatus::Unknown => "Unknown".to_string(),
        },
        start_year: manga.metadata.start_year,
        end_year: manga.metadata.end_year,
        metadata_source: match manga.metadata_source {
            crate::manga::manga::MangaSource::AniList => "AniList".to_string(),
            crate::manga::manga::MangaSource::Local => "Local".to_string(),
        },
        monitored: manga.monitored,
        created_at: manga.created_at,
        metadata_updated_at: manga.metadata_updated_at,
    };

    match serde_json::to_string(&series) {
        Ok(json) => Some(json),
        Err(e) => {
            log::warn!(
                "Failed to serialize series metadata for {}: {}",
                manga.metadata.title,
                e
            );
            None
        }
    }
}

/// Serialize chapter metadata to minified JSON for the notes field.
fn serialize_chapter_metadata(chapter: &Chapter) -> Option<String> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct ChapterMetadata {
        uuid: String,
        manga_id: String,
        chapter_base: i32,
        chapter_variant: i32,
        is_extra: bool,
        title: Option<String>,
        language: String,
        scanlator_group: Option<String>,
        provider_name: Option<String>,
        chapter_url: Option<String>,
        released_at: Option<i64>,
        downloaded_at: Option<i64>,
        scraped_at: Option<i64>,
    }

    let chapter_meta = ChapterMetadata {
        uuid: chapter.id.to_string(),
        manga_id: chapter.manga_id.to_string(),
        chapter_base: chapter.chapter_base,
        chapter_variant: chapter.chapter_variant,
        is_extra: chapter.is_extra,
        title: chapter.title.clone(),
        language: chapter.language.clone(),
        scanlator_group: chapter.scanlator_group.clone(),
        provider_name: chapter.provider_name.clone(),
        chapter_url: chapter.chapter_url.clone(),
        released_at: chapter.released_at.map(|dt| dt.timestamp()),
        downloaded_at: chapter.downloaded_at.map(|dt| dt.timestamp()),
        scraped_at: chapter.scraped_at.map(|dt| dt.timestamp()),
    };

    match serde_json::to_string(&chapter_meta) {
        Ok(json) => Some(json),
        Err(e) => {
            log::warn!(
                "Failed to serialize chapter metadata for chapter {}: {}",
                chapter.number_sort(),
                e
            );
            None
        }
    }
}

/// Generate common XML elements shared between series and chapter XML.
fn generate_common_metadata_xml(manga: &Manga, xml: &mut String) {
    let m = &manga.metadata;

    // Series field (always present)
    xml.push_str(&opt_str_elem("Series", Some(&m.title)));

    // Alternate series: extract titles from Synonyms (joined with semicolon for ComicInfo.xml)
    let alt_series = m
        .other_titles
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(|synonyms| {
            synonyms
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        });
    xml.push_str(&opt_str_elem("AlternateSeries", alt_series.as_deref()));

    // Summary
    xml.push_str(&opt_str_elem("Summary", m.synopsis.as_deref()));

    // Creator roles
    xml.push_str(&opt_vec_elem("Writer", m.writer.as_deref()));
    xml.push_str(&opt_vec_elem("Penciller", m.penciller.as_deref()));
    xml.push_str(&opt_vec_elem("Inker", m.inker.as_deref()));
    xml.push_str(&opt_vec_elem("Colorist", m.colorist.as_deref()));
    xml.push_str(&opt_vec_elem("Letterer", m.letterer.as_deref()));
    xml.push_str(&opt_vec_elem("Editor", m.editor.as_deref()));
    xml.push_str(&opt_vec_elem("Translator", m.translator.as_deref()));

    // Tags (AniList tags)
    let tags_str = if m.tags.is_empty() {
        None
    } else {
        Some(m.tags.join(", "))
    };
    xml.push_str(&opt_str_elem("Tags", tags_str.as_deref()));

    // Genre (from MangaMetadata)
    xml.push_str(&opt_str_elem("Genre", m.genre.as_deref()));

    // Web link
    let web = manga
        .anilist_id
        .map(|id| format!("https://anilist.co/manga/{id}"));
    xml.push_str(&opt_str_elem("Web", web.as_deref()));

    // Community Rating
    xml.push_str(&opt_int_rating_elem("CommunityRating", m.community_rating));
}

/// Generate a series-level ComicInfo.xml string from manga metadata.
///
/// This file is placed at `{series_dir}/ComicInfo.xml` and lets comic
/// readers (Komga, Kavita, etc.) pick up series metadata without needing
/// to open any CBZ.
pub fn generate_series_xml(manga: &Manga) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"\n\
                    xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
    );

    // Generate common metadata
    generate_common_metadata_xml(manga, &mut xml);

    // Series-specific date fields
    let m = &manga.metadata;
    xml.push_str(&opt_int_elem("Year", m.start_year));
    xml.push_str(&opt_int_elem("Month", m.start_month));
    xml.push_str(&opt_int_elem("Day", m.start_day));

    // Enhanced notes field with comprehensive JSON metadata
    let notes = serialize_series_metadata(manga);
    xml.push_str(&opt_str_elem("Notes", notes.as_deref()));

    xml.push_str("</ComicInfo>\n");
    xml
}

/// Generate a chapter-level ComicInfo.xml string for embedding inside a CBZ.
///
/// Includes all series fields plus chapter-specific fields (number, title,
/// volume, scanlator group, page count). `provider_name` is embedded in Notes
/// for round-trip re-import via the disk scanner.
pub fn generate_chapter_xml(
    manga: &Manga,
    chapter: &Chapter,
    page_count: usize,
    provider_name: Option<&str>,
) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"\n\
                    xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
    );

    // Chapter-specific fields
    xml.push_str(&opt_str_elem("Title", chapter.title.as_deref()));
    xml.push_str(&format!("  <Number>{}</Number>\n", chapter.number_sort()));
    xml.push_str("  <Manga>Yes</Manga>\n");

    // Generate common metadata
    generate_common_metadata_xml(manga, &mut xml);

    // Chapter release date (if available)
    if let Some(released_at) = chapter.released_at {
        let year = released_at.year();
        let month = released_at.month();
        let day = released_at.day();

        xml.push_str(&opt_int_elem("Year", Some(year)));
        xml.push_str(&opt_int_elem("Month", Some(month)));
        xml.push_str(&opt_int_elem("Day", Some(day)));
    } else {
        // Fallback to series start date if chapter release date is not available
        let m = &manga.metadata;
        xml.push_str(&opt_int_elem("Year", m.start_year));
        xml.push_str(&opt_int_elem("Month", m.start_month));
        xml.push_str(&opt_int_elem("Day", m.start_day));
    }

    // Chapter-specific fields
    xml.push_str(&opt_str_elem(
        "ScanInformation",
        chapter.scanlator_group.as_deref(),
    ));
    if page_count > 0 {
        xml.push_str(&format!("  <PageCount>{page_count}</PageCount>\n"));
    }
    xml.push_str(&opt_str_elem(
        "LanguageISO",
        Some(&chapter.language.to_uppercase()),
    ));

    // Enhanced notes field with comprehensive JSON metadata
    let series_notes = serialize_series_metadata(manga);
    let chapter_notes = serialize_chapter_metadata(chapter);

    let notes = match (series_notes, chapter_notes) {
        (Some(series_json), Some(chapter_json)) => {
            format!(
                "{{\"series\":{},\"chapter\":{}}}",
                series_json, chapter_json
            )
        }
        (Some(series_json), None) => {
            format!("{{\"series\":{}}}", series_json)
        }
        (None, Some(chapter_json)) => {
            format!("{{\"chapter\":{}}}", chapter_json)
        }
        (None, None) => {
            // Fallback to simple format if JSON serialization fails
            let mut parts = Vec::new();
            if let Some(id) = manga.anilist_id {
                parts.push(format!("rebarr:anilist_id={id}"));
            }
            if let Some(p) = provider_name {
                if !p.is_empty() {
                    parts.push(format!("rebarr:provider={p}"));
                }
            }
            parts.join(" ")
        }
    };

    xml.push_str(&opt_str_elem("Notes", Some(&notes)));

    xml.push_str("</ComicInfo>\n");
    xml
}

/// Write a series-level `ComicInfo.xml` into `series_dir`.
///
/// Creates the directory if it doesn't exist. Errors are soft — callers
/// should log but not fail the overall operation if this fails.
pub async fn write_series_comicinfo(series_dir: &Path, manga: &Manga) -> std::io::Result<()> {
    tokio::fs::create_dir_all(series_dir).await?;
    let path = series_dir.join("ComicInfo.xml");
    let xml = generate_series_xml(manga);
    tokio::fs::write(path, xml).await
}
