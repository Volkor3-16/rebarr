use std::path::PathBuf;

use anilist_moe::objects::media::Media;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub uuid: Uuid,         // The Unique ID of the library (for supporting diff types of 'manga')
    pub r#type: MangaType,  // The type of the manga (Western Comics, Manga, whatever)
    pub root_path: PathBuf, // Where it saves new manga series that are in the library.
}

/// Contains all the important data about a Manga
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manga {
    pub id: Uuid,                           // internal, canonical
    pub library_id: Uuid,                   // The Library the manga belongs to
    pub anilist_id: Option<u32>,            // external identity
    pub mal_id: Option<u32>,                // external identity #2
    pub metadata: MangaMetadata,            // Stores all the metadata for the series
    pub relative_path: PathBuf,             // Relative (to the library root) path of the manga files.
    pub downloaded_count: Option<i32>,      // How many chapters are on disk already.
    pub chapter_count: Option<u32>,         // anilist doesn't support chapter counts in any sane way, we need to build this from providers @ scrape time.
    pub metadata_source: MangaSource,                // The source of the metadata, not where we download it from.
    pub created_at: DateTime<Utc>,          // When the manga was first added to the library
    pub metadata_updated_at: DateTime<Utc>, // When the manga last metadata refresh
}

/// Contains all the scraped metadata about a Manga
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaMetadata {
    pub title: String,                      // Title in English (or default lang)
    pub title_og: String,                   // Title in Original Language (non-romanised)
    pub title_roman: String,                // Title in Original Language (romanised)
    pub synopsis: Option<String>,
    pub publishing_status: PublishingStatus,
    pub tags: Vec<String>,                  // Tags according to anilist
    pub start_year: Option<i32>,            // When the manga started publishing
    pub end_year: Option<i32>,              // When the manga finished publishing (or none)
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

impl From<Media> for Manga {
    /// Conversion from AniList API to Internal Struct
    fn from(media: Media) -> Self {
        // Extract titles - prefer English, fallback to romaji, then native
        let title = media.title.as_ref().and_then(|t| t.english.clone())
            .unwrap_or_else(|| media.title.as_ref().and_then(|t| t.romaji.clone()).unwrap_or_default());

        let title_og = media.title.as_ref().and_then(|t| t.native.clone()).unwrap_or_default();
        let title_roman = media.title.as_ref().and_then(|t| t.romaji.clone()).unwrap_or(title.clone());

        // Map Anilist status to PublishingStatus
        let status = match media.status {
            Some(anilist_moe::enums::media::MediaStatus::Finished) => PublishingStatus::Completed,
            Some(anilist_moe::enums::media::MediaStatus::Releasing) => PublishingStatus::Ongoing,
            Some(anilist_moe::enums::media::MediaStatus::NotYetReleased) => PublishingStatus::NotYetReleased,
            Some(anilist_moe::enums::media::MediaStatus::Cancelled) => PublishingStatus::Cancelled,
            Some(anilist_moe::enums::media::MediaStatus::Hiatus) => PublishingStatus::Hiatus,
            _ => PublishingStatus::Unknown,
        };

        // Convert chapters from i32 to u32 if available
        let chapter_count = media.chapters.map(|c| c as u32);

        let metadata = MangaMetadata {
            title,
            title_og,
            title_roman,
            synopsis: media.description,
            publishing_status: status,
            tags: media.tags
                .unwrap_or_default()
                .into_iter()
                .filter_map(|t| t.name)
                .collect(),
            start_year: media.start_date.and_then(|d| d.year),
            end_year: media.end_date.and_then(|d| d.year),
        };

        Manga {
            id: Uuid::new_v4(),
            library_id: Uuid::nil(),          // caller must set before persisting
            anilist_id: media.id.map(|id| id as u32),
            mal_id: media.id_mal.map(|id| id as u32),
            metadata,
            relative_path: PathBuf::new(),    // caller must set before persisting
            downloaded_count: None,
            chapter_count,
            metadata_source: MangaSource::AniList,
            created_at: Utc::now(),
            metadata_updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: Uuid,                               // Randomly Generated for each downloaded chapter, Downloading a new one overrided, will replace it.
    pub manga_id: Uuid,                         // The managa uuid that the chapter belongs to?
    pub number_raw: String,                     // supports 12.5 chapter numbering. (what the provider said the chapter was)
    pub number_sort: f32,                       // Easily sortable chapter number (what we pass to everyone else)
    pub title: Option<String>,                  // The title of the chapter, if provded by the.. provider.
    pub volume: Option<u32>,                    // The volume number of the chapter. if provded by the.. provider.
    pub scanlator_group: Option<String>,        // The name of the scanlator group, if provded by the.. provider.
    pub downloaded_at: Option<DateTime<Utc>>,   // When the chapter was downloaded.
}

// We use this for storing the scraped-provided metadata, but since this can change on the site, we save it on scrape, and never again?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaAlias {
    pub manga_id: Uuid,
    pub source: AliasSource,
    pub title: String,
}

/// Where the metadata came from
/// It defaults to MAL api, and failing that, the scraped site, but also allowing for 'manual' entries, if you wanna do it yourself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AliasSource {
    MyAnimeList,
    Site(String),
    Manual,
}

/// Struct to help map internal chapters to Provider's Chapters
pub struct ProviderMangaInfo {
    pub provider: ProviderKind,
    pub chapters: Vec<ProviderChapterInfo>,
}

/// Info about individual chapters from a specific provider
pub struct ProviderChapterInfo {
    pub raw_number: String,                 // Raw value of the full chapter as scraped from the provider
    pub number: f32,                        // Extracted chapter number
    pub title: Option<String>,              // Extracted chapter title (can be nothing)
    pub volume: Option<u32>,                // Extracted chapter volume (can be nothing)
    pub scanlator_group: Option<Scanlator>, // Extracted scanlator group (can be nothing)
}

pub enum Scanlator {
    Scanlator(String),  // Yes, is a scanlator, and the name.
    Official(String),   // No, is official release, and the publisher name.
    Unknown,            // ¯\_(ツ)_/¯
}

impl Scanlator {
    pub fn name(&self) -> &str {
        match self {
            Self::Scanlator(name) => name,
            Self::Official(name) => name,
            Self::Unknown => "Unknown",
        }
    }
}

pub enum ProviderKind {
    Comix,
    Kagane,
    MangaFire,
    Mangaball,
    Atsumaru,
    WeebCentral,
    Mangago,
    VyManga,
    WeebDex,
}