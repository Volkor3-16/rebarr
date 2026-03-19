use std::path::PathBuf;

use anilist_moe::objects::media::Media;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub uuid: Uuid, // The Unique ID of the library (for supporting diff types of 'manga')
    pub r#type: MangaType, // The type of the manga (Western Comics, Manga, whatever)
    pub root_path: PathBuf, // Where it saves new manga series that are in the library.
}

/// Contains all the important data about a Manga
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manga {
    pub id: Uuid,                           // internal, canonical
    pub library_id: Uuid,                   // The Library the manga belongs to
    pub anilist_id: Option<u32>,            // external identity
    pub mal_id: Option<u32>, // MAL cross-reference ID (sourced from AniList's id_mal field)
    pub metadata: MangaMetadata, // Stores all the metadata for the series
    pub relative_path: PathBuf, // Relative (to the library root) path of the manga files.
    pub downloaded_count: Option<i32>, // How many chapters are on disk already.
    pub chapter_count: Option<u32>, // anilist doesn't support chapter counts in any sane way, we need to build this from providers @ scrape time.
    pub metadata_source: MangaSource, // The source of the metadata, not where we download it from.
    pub thumbnail_url: Option<String>, // Cached cover image URL from metadata source.
    pub monitored: bool, // If true, new chapters are automatically downloaded.
    pub created_at: i64,  // When the manga was first added to the library
    pub metadata_updated_at: i64, // When the manga last metadata refresh
}

/// Contains all the scraped metadata about a Manga
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaMetadata {
    pub title: String,       // Title in English (or default lang)
    pub other_titles: Option<Vec<String>>, // List of alternative names
    pub synopsis: Option<String>,
    pub publishing_status: PublishingStatus,
    pub tags: Vec<String>,       // Tags according to anilist
    pub start_year: Option<i32>, // When the manga started publishing
    pub end_year: Option<i32>,   // When the manga finished publishing (or none)
}

/// The 'type' of manga it is. Used for having western comics and manga in one server instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MangaType {
    Comics,
    Manga,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PublishingStatus {
    Completed,
    Ongoing,
    Hiatus,
    Cancelled,
    NotYetReleased,
    Unknown,
}

/// Contains all supported (and future supported?) Manga Providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MangaSource {
    AniList,
    Local,
}

/// Strip HTML tags from AniList synopsis text.
/// Converts `<br>` variants to newlines, removes all other tags,
/// decodes common HTML entities, and collapses excessive blank lines.
fn strip_html(s: &str) -> String {
    // Normalise <br> variants to newlines before stripping
    let mut work = s.to_owned();
    for br in &["<br />", "<br/>", "<br>", "<BR />", "<BR/>", "<BR>"] {
        work = work.replace(br, "\n");
    }

    // Remove remaining tags
    let mut out = String::with_capacity(work.len());
    let mut in_tag = false;
    for ch in work.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    // Collapse runs of 3+ newlines down to 2
    let mut result = String::with_capacity(out.len());
    let mut newline_run = 0u32;
    for ch in out.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                result.push(ch);
            }
        } else {
            newline_run = 0;
            result.push(ch);
        }
    }

    result.trim().to_owned()
}

impl From<Media> for Manga {
    /// Conversion from AniList API to Internal Struct
    fn from(media: Media) -> Self {
        // Extract titles - prefer English, fallback to romaji, then native
        let title = media
            .title
            .as_ref()
            .and_then(|t| t.english.clone())
            .unwrap_or_else(|| {
                media
                    .title
                    .as_ref()
                    .and_then(|t| t.romaji.clone())
                    .unwrap_or_default()
            });
        
        // Start with synonyms
        let mut other_titles: Vec<String> = media.synonyms.unwrap_or_default();

        // Add Romaji if it’s different from main title
        if let Some(romaji) = media.title.as_ref().and_then(|t| t.romaji.clone()) {
            if romaji != title && !other_titles.contains(&romaji) {
                other_titles.push(romaji);
            }
        }

        // Add native if it’s different from main title and Romaji
        if let Some(native) = media.title.as_ref().and_then(|t| t.native.clone()) {
            if native != title && !other_titles.contains(&native) {
                other_titles.push(native);
            }
        }

        // Map Anilist status to PublishingStatus
        let status = match media.status {
            Some(anilist_moe::enums::media::MediaStatus::Finished) => PublishingStatus::Completed,
            Some(anilist_moe::enums::media::MediaStatus::Releasing) => PublishingStatus::Ongoing,
            Some(anilist_moe::enums::media::MediaStatus::NotYetReleased) => {
                PublishingStatus::NotYetReleased
            }
            Some(anilist_moe::enums::media::MediaStatus::Cancelled) => PublishingStatus::Cancelled,
            Some(anilist_moe::enums::media::MediaStatus::Hiatus) => PublishingStatus::Hiatus,
            _ => PublishingStatus::Unknown,
        };

        // chapter_count stays None until scraped from providers; AniList data is unreliable
        let chapter_count = None;

        let metadata = MangaMetadata {
            title,
            other_titles: Some(other_titles),
            synopsis: media.description.as_deref().map(strip_html),
            publishing_status: status,
            tags: media
                .tags
                .unwrap_or_default()
                .into_iter()
                .filter_map(|t| t.name)
                .collect(),
            start_year: media.start_date.and_then(|d| d.year),
            end_year: media.end_date.and_then(|d| d.year),
        };

        let thumbnail_url = media.cover_image.as_ref().and_then(|c| {
            c.extra_large
                .clone()
                .or_else(|| c.large.clone())
                .or_else(|| c.medium.clone())
        });

        Manga {
            id: Uuid::new_v4(),
            library_id: Uuid::nil(), // caller must set before persisting
            anilist_id: media.id.map(|id| id as u32),
            mal_id: media.id_mal.map(|id| id as u32),
            metadata,
            relative_path: PathBuf::new(), // caller must set before persisting
            downloaded_count: None,
            chapter_count,
            metadata_source: MangaSource::AniList,
            thumbnail_url,
            monitored: true,
            created_at: Utc::now().timestamp(),
            metadata_updated_at: Utc::now().timestamp(),
        }
    }
}

/// All the data about a specific chapter (one row per provider + language per chapter number).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: Uuid,
    pub manga_id: Uuid,
    /// Integer chapter number (e.g. 1, 2, 100).
    pub chapter_base: i32,
    /// Sub-chapter index: 0 = full chapter, 1–9 = split part index.
    pub chapter_variant: i32,
    /// True if this chapter is an extra/bonus (inferred from title keywords or user-toggled).
    pub is_extra: bool,
    pub title: Option<String>,
    /// BCP 47 language code (e.g. "EN", "PT-BR"). Defaults to "EN".
    pub language: String,
    pub scanlator_group: Option<String>,
    /// Which provider this row came from. None = manually added from disk.
    pub provider_name: Option<String>,
    /// URL of this chapter on the provider site.
    pub chapter_url: Option<String>,
    pub download_status: DownloadStatus,
    /// When the provider published this chapter.
    pub released_at: Option<DateTime<Utc>>,
    /// When we downloaded it.
    pub downloaded_at: Option<DateTime<Utc>>,
    /// Last time we scraped this row from the provider.
    pub scraped_at: Option<DateTime<Utc>>,
    /// Size of the CBZ file on disk in bytes, populated after download or disk scan.
    pub file_size_bytes: Option<i64>,
}

impl Chapter {
    /// Sortable float representation: chapter_base + chapter_variant * 0.1
    pub fn number_sort(&self) -> f32 {
        self.chapter_base as f32 + self.chapter_variant as f32 * 0.1
    }
}

/// Tracks whether a chapter has been downloaded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DownloadStatus {
    Missing,
    Queued,
    Downloading,
    Downloaded,
    Failed,
}

impl DownloadStatus {
    /// Returns human readable string for the status
    pub fn as_str(&self) -> &'static str {
        match self {
            DownloadStatus::Missing => "Missing",
            DownloadStatus::Queued => "Queued",
            DownloadStatus::Downloading => "Downloading",
            DownloadStatus::Downloaded => "Downloaded",
            DownloadStatus::Failed => "Failed",
        }
    }
}

/// Alias titles are scraped from providers and saved once; they are not refreshed automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaAlias {
    pub manga_id: Uuid,
    pub source: AliasSource,
    pub title: String,
}

/// Where an alias title came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AliasSource {
    AniList,
    Site(String),
    Manual,
}
