use std::io::Read as _;
use std::path::Path;

use crate::manga::manga::{Chapter, Manga, Synonym, SynonymSource};

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
    /// AniList series ID, parsed from `<Web>` URL or `<Notes>rebarr:anilist_id=...</Notes>`.
    pub anilist_id: Option<u32>,
    /// Provider name, parsed from `<Notes>rebarr:provider=...</Notes>`.
    pub provider_name: Option<String>,
    // Chapter-level fields
    pub chapter_title: Option<String>,
    pub scanlator: Option<String>,
    pub language: Option<String>,
    pub release_year: Option<i32>,
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
pub fn parse_comicinfo(xml: &str) -> ParsedComicInfo {
    let mut info = ParsedComicInfo::default();

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
    info.tags = extract_tag(xml, "Genre")
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
    // Fallback / extended: <Notes>rebarr:anilist_id=12345 rebarr:provider=ProviderName</Notes>
    if let Some(notes) = extract_tag(xml, "Notes") {
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

    info.chapter_title = extract_tag(xml, "Title");
    info.scanlator = extract_tag(xml, "ScanInformation");
    info.language = extract_tag(xml, "LanguageISO");
    info.release_year = extract_tag(xml, "Year").and_then(|s| s.parse().ok());

    info
}

/// Open a CBZ archive and parse the embedded `ComicInfo.xml`, if present.
/// Returns `None` on any error (missing file, bad zip, parse failure).
pub fn read_cbz_comicinfo(cbz_path: &Path) -> Option<ParsedComicInfo> {
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
    let mut contents = String::new();
    entry.read_to_string(&mut contents).ok()?;
    Some(parse_comicinfo(&contents))
}

/// Escape characters that are special in XML attribute/element values.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

/// Generate a series-level ComicInfo.xml string from manga metadata.
///
/// This file is placed at `{series_dir}/ComicInfo.xml` and lets comic
/// readers (Komga, Kavita, etc.) pick up series metadata without needing
/// to open any CBZ.
pub fn generate_series_xml(manga: &Manga) -> String {
    let m = &manga.metadata;

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

    let genre = if m.tags.is_empty() {
        None
    } else {
        Some(m.tags.join(", "))
    };

    let web = manga
        .anilist_id
        .map(|id| format!("https://anilist.co/manga/{id}"));

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"\n\
                    xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
    );

    xml.push_str(&opt_str_elem("Series", Some(&m.title)));
    xml.push_str(&opt_str_elem("AlternateSeries", alt_series.as_deref()));
    xml.push_str(&opt_str_elem("Summary", m.synopsis.as_deref()));
    xml.push_str(&opt_int_elem("Year", m.start_year));
    xml.push_str(&opt_str_elem("Genre", genre.as_deref()));
    xml.push_str("  <Manga>Yes</Manga>\n");
    xml.push_str(&opt_str_elem("Web", web.as_deref()));
    // Embed AniList ID for clean round-trip re-import
    let notes = manga.anilist_id.map(|id| format!("rebarr:anilist_id={id}"));
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
    let m = &manga.metadata;

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

    let genre = if m.tags.is_empty() {
        None
    } else {
        Some(m.tags.join(", "))
    };

    let web = manga
        .anilist_id
        .map(|id| format!("https://anilist.co/manga/{id}"));

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"\n\
                    xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
    );

    // Chapter-specific
    xml.push_str(&opt_str_elem("Title", chapter.title.as_deref()));
    xml.push_str(&opt_str_elem("Series", Some(&m.title)));
    xml.push_str(&opt_str_elem("AlternateSeries", alt_series.as_deref()));
    xml.push_str(&format!("  <Number>{}</Number>\n", chapter.number_sort()));

    // Series metadata
    xml.push_str(&opt_int_elem("Year", m.start_year));
    xml.push_str(&opt_str_elem("Summary", m.synopsis.as_deref()));
    xml.push_str(&opt_str_elem("Genre", genre.as_deref()));
    xml.push_str("  <Manga>Yes</Manga>\n");
    xml.push_str(&opt_str_elem("Web", web.as_deref()));

    // Scanlator / page count / language
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
    // Embed AniList ID and provider for clean round-trip re-import
    let notes = {
        let mut parts = Vec::new();
        if let Some(id) = manga.anilist_id {
            parts.push(format!("rebarr:anilist_id={id}"));
        }
        if let Some(p) = provider_name {
            if !p.is_empty() {
                parts.push(format!("rebarr:provider={p}"));
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    };
    xml.push_str(&opt_str_elem("Notes", notes.as_deref()));

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
