use std::path::Path;

use crate::manga::{Chapter, Manga};

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Series-level ComicInfo.xml
// ---------------------------------------------------------------------------

/// Generate a series-level ComicInfo.xml string from manga metadata.
///
/// This file is placed at `{series_dir}/ComicInfo.xml` and lets comic
/// readers (Komga, Kavita, etc.) pick up series metadata without needing
/// to open any CBZ.
pub fn generate_series_xml(manga: &Manga) -> String {
    let m = &manga.metadata;

    // Alternate series: prefer romanised title, fall back to native
    let alt_series = if !m.title_roman.is_empty() && m.title_roman != m.title {
        Some(m.title_roman.as_str())
    } else if !m.title_og.is_empty() {
        Some(m.title_og.as_str())
    } else {
        None
    };

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
    xml.push_str(&opt_str_elem("AlternateSeries", alt_series));
    xml.push_str(&opt_str_elem("Summary", m.synopsis.as_deref()));
    xml.push_str(&opt_int_elem("Year", m.start_year));
    xml.push_str(&opt_str_elem("Genre", genre.as_deref()));
    xml.push_str("  <Manga>Yes</Manga>\n");
    xml.push_str(&opt_str_elem("Web", web.as_deref()));

    xml.push_str("</ComicInfo>\n");
    xml
}

// ---------------------------------------------------------------------------
// Chapter-level ComicInfo.xml (embedded in CBZ)
// ---------------------------------------------------------------------------

/// Generate a chapter-level ComicInfo.xml string for embedding inside a CBZ.
///
/// Includes all series fields plus chapter-specific fields (number, title,
/// volume, scanlator group, page count).
pub fn generate_chapter_xml(manga: &Manga, chapter: &Chapter, page_count: usize) -> String {
    let m = &manga.metadata;

    let alt_series = if !m.title_roman.is_empty() && m.title_roman != m.title {
        Some(m.title_roman.as_str())
    } else if !m.title_og.is_empty() {
        Some(m.title_og.as_str())
    } else {
        None
    };

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
    xml.push_str(&opt_str_elem("AlternateSeries", alt_series));
    xml.push_str(&format!("  <Number>{}</Number>\n", xml_escape(&chapter.number_raw)));
    xml.push_str(&opt_int_elem("Volume", chapter.volume));

    // Series metadata
    xml.push_str(&opt_int_elem("Year", m.start_year));
    xml.push_str(&opt_str_elem("Summary", m.synopsis.as_deref()));
    xml.push_str(&opt_str_elem("Genre", genre.as_deref()));
    xml.push_str("  <Manga>Yes</Manga>\n");
    xml.push_str(&opt_str_elem("Web", web.as_deref()));

    // Scanlator / page count
    xml.push_str(&opt_str_elem("ScanInformation", chapter.scanlator_group.as_deref()));
    if page_count > 0 {
        xml.push_str(&format!("  <PageCount>{page_count}</PageCount>\n"));
    }

    xml.push_str("</ComicInfo>\n");
    xml
}

// ---------------------------------------------------------------------------
// Write series ComicInfo.xml to disk
// ---------------------------------------------------------------------------

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
