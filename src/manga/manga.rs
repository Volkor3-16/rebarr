use std::path::PathBuf;

use anilist_moe::objects::media::Media;
use chrono::{DateTime, Utc};
use log::trace;
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
    pub id: Uuid,                      // internal, canonical
    pub library_id: Uuid,              // The Library the manga belongs to
    pub anilist_id: Option<u32>,       // external identity
    pub mal_id: Option<u32>, // MAL cross-reference ID (sourced from AniList's id_mal field)
    pub metadata: MangaMetadata, // Stores all the metadata for the series
    pub relative_path: PathBuf, // Relative (to the library root) path of the manga files.
    pub downloaded_count: Option<i32>, // How many chapters are on disk already.
    pub chapter_count: Option<u32>, // anilist doesn't support chapter counts in any sane way, we need to build this from providers @ scrape time.
    pub metadata_source: MangaSource, // The source of the metadata, not where we download it from.
    pub thumbnail_url: Option<String>, // Cached cover image URL from metadata source.
    pub monitored: bool,            // If true, new chapters are automatically downloaded.
    pub created_at: i64,            // When the manga was first added to the library
    pub metadata_updated_at: i64,   // When the manga last metadata refresh
    /// Timestamp of when we last checked for new chapters (null = never)
    pub last_checked_at: Option<i64>,
}

/// Source of a synonym title
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SynonymSource {
    /// Synonym fetched from AniList - can be hidden by user
    AniList,
    /// User manually added - always used for search
    Manual,
}

/// A single alternative title with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Synonym {
    pub title: String,
    pub source: SynonymSource,
    /// If true, this synonym is hidden from provider searches
    /// Used to hide AniList synonyms without losing them on refresh
    pub hidden: bool,
    /// Reason why this synonym is hidden: "manual" (user hid it) or "language" (matched filter)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_reason: Option<String>,
}

impl Synonym {
    /// Create a new AniList synonym (visible by default)
    pub fn anilist(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            source: SynonymSource::AniList,
            hidden: false,
            filter_reason: None,
        }
    }

    /// Create a new manual synonym (always visible)
    pub fn manual(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            source: SynonymSource::Manual,
            hidden: false,
            filter_reason: None,
        }
    }
}

/// Contains all the scraped metadata about a Manga
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaMetadata {
    pub title: String,                      // Title in English (or default lang)
    pub other_titles: Option<Vec<Synonym>>, // List of alternative names with metadata
    pub synopsis: Option<String>,
    pub publishing_status: PublishingStatus,
    pub tags: Vec<String>,       // Tags according to anilist
    pub start_year: Option<i32>, // When the manga started publishing
    pub start_month: Option<i32>, // When the manga started publishing (month)
    pub start_day: Option<i32>,   // When the manga started publishing (day)
    pub end_year: Option<i32>,   // When the manga finished publishing (or none)

    // ComicInfo fields
    pub writer: Option<Vec<String>>,       // Mangaka/Author
    pub penciller: Option<Vec<String>>,    // Artist
    pub inker: Option<Vec<String>>,        // Inker
    pub colorist: Option<Vec<String>>,     // Colorist
    pub letterer: Option<Vec<String>>,     // Letterer
    pub editor: Option<Vec<String>>,       // Editor (already exists in Manga, move here)
    pub translator: Option<Vec<String>>,   // Translator (already exists in Manga, move here)
    pub genre: Option<String>,              // Primary genre
    pub community_rating: Option<i32>,      // Average score from AniList
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

/// Staff role mapping for categorizing AniList staff roles into ComicInfo fields
#[derive(Debug, Clone, Copy)]
enum StaffRole {
    Writer,
    Penciller,
    Inker,
    Colorist,
    Letterer,
    Editor,
    Translator,
}

impl StaffRole {
    /// Determine staff role from AniList role string
    fn from_role(role: &str) -> Option<Self> {
        let normalized = Self::normalize_role(role);
        
        // Check role mappings in order of specificity
        if Self::is_writer_role(&normalized) {
            Some(Self::Writer)
        } else if Self::is_penciller_role(&normalized) {
            Some(Self::Penciller)
        } else if Self::is_inker_role(&normalized) {
            Some(Self::Inker)
        } else if Self::is_colorist_role(&normalized) {
            Some(Self::Colorist)
        } else if Self::is_letterer_role(&normalized) {
            Some(Self::Letterer)
        } else if Self::is_editor_role(&normalized) {
            Some(Self::Editor)
        } else if Self::is_translator_role(&normalized) {
            Some(Self::Translator)
        } else {
            None
        }
    }

    /// Normalize role string by removing parentheses and extra information
    fn normalize_role(role: &str) -> String {
        let role_lower = role.to_lowercase();
        
        // Remove content in parentheses (e.g., "Lettering (English, chs 1-2)" -> "lettering")
        let without_parens = if let Some(open_paren) = role_lower.find('(') {
            role_lower[..open_paren].trim().to_string()
        } else {
            role_lower
        };

        // Remove common suffixes and normalize
        without_parens
            .replace(" & art", "")
            .replace(" & story", "")
            .replace(" (english)", "")
            .replace(" (japanese)", "")
            .replace(" (chinese)", "")
            .replace(" (korean)", "")
            .trim()
            .to_string()
    }

    /// Check if role indicates a writer/author
    fn is_writer_role(role: &str) -> bool {
        role.contains("story") || 
        role.contains("writer") || 
        role.contains("mangaka") || 
        role.contains("author") ||
        role.contains("script")
    }

    /// Check if role indicates a penciller/artist
    fn is_penciller_role(role: &str) -> bool {
        role.contains("art") || 
        role.contains("artist") || 
        role.contains("illustrat") || 
        role.contains("pencil") || 
        role.contains("draw") ||
        role.contains("sketch")
    }

    /// Check if role indicates an inker
    fn is_inker_role(role: &str) -> bool {
        role.contains("ink")
    }

    /// Check if role indicates a colorist
    fn is_colorist_role(role: &str) -> bool {
        role.contains("color") || role.contains("colour")
    }

    /// Check if role indicates a letterer
    fn is_letterer_role(role: &str) -> bool {
        role.contains("letter")
    }

    /// Check if role indicates an editor
    fn is_editor_role(role: &str) -> bool {
        role.contains("edit")
    }

    /// Check if role indicates a translator
    fn is_translator_role(role: &str) -> bool {
        role.contains("translat")
    }
}

/// Extract staff information from Media object and categorize by role
fn extract_staff_from_media(media: &Media) -> (
    Option<Vec<String>>, // writer
    Option<Vec<String>>, // penciller  
    Option<Vec<String>>, // inker
    Option<Vec<String>>, // colorist
    Option<Vec<String>>, // letterer
    Option<Vec<String>>, // editor
    Option<Vec<String>>, // translator
) {
    // Initialize vectors to collect staff by role
    let mut writers = Vec::new();
    let mut pencillers = Vec::new();
    let mut inkers = Vec::new();
    let mut colorists = Vec::new();
    let mut letterers = Vec::new();
    let mut editors = Vec::new();
    let mut translators = Vec::new();

    // Check if staff data exists
    if let Some(staff_edges) = &media.staff {
        if let Some(edges) = &staff_edges.edges {
            for staff_edge in edges {
                if let Some(staff_node) = &staff_edge.node {
                    if let Some(staff_name) = extract_staff_name(staff_node) {
                        if let Some(role) = &staff_edge.role {
                            if let Some(staff_role) = StaffRole::from_role(role) {
                                match staff_role {
                                    StaffRole::Writer => writers.push(staff_name),
                                    StaffRole::Penciller => pencillers.push(staff_name),
                                    StaffRole::Inker => inkers.push(staff_name),
                                    StaffRole::Colorist => colorists.push(staff_name),
                                    StaffRole::Letterer => letterers.push(staff_name),
                                    StaffRole::Editor => editors.push(staff_name),
                                    StaffRole::Translator => translators.push(staff_name),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Return None if no staff found for a category, otherwise Some(vec)
    (
        if writers.is_empty() { None } else { Some(writers) },
        if pencillers.is_empty() { None } else { Some(pencillers) },
        if inkers.is_empty() { None } else { Some(inkers) },
        if colorists.is_empty() { None } else { Some(colorists) },
        if letterers.is_empty() { None } else { Some(letterers) },
        if editors.is_empty() { None } else { Some(editors) },
        if translators.is_empty() { None } else { Some(translators) },
    )
}

/// Extract the full name from a Staff object
fn extract_staff_name(staff: &anilist_moe::objects::staff::Staff) -> Option<String> {
    staff.name.as_ref().and_then(|name| {
        name.user_preferred
            .clone()
            .or_else(|| name.full.clone())
            .or_else(|| {
                // Build name from parts if full name not available
                let first = name.first.as_ref()?;
                let last = name.last.as_ref()?;
                Some(format!("{} {}", first, last))
            })
    })
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

        // Collect existing titles to avoid duplicates
        let mut existing_titles: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        existing_titles.insert(title.clone());

        // Start with synonyms from AniList
        let mut other_titles: Vec<Synonym> = media
            .synonyms
            .as_ref()
            .map(|synonyms| synonyms.iter().filter(|s| existing_titles.insert(s.to_string())).map(|s| Synonym::anilist(s)).collect())
            .unwrap_or_default();

        // Add Romaji if it's different from main title
        if let Some(romaji) = media.title.as_ref().and_then(|t| t.romaji.clone()) {
            if existing_titles.insert(romaji.clone()) {
                other_titles.push(Synonym::anilist(&romaji));
            }
        }

        // Add native if it's different from main title and Romaji
        if let Some(native) = media.title.as_ref().and_then(|t| t.native.clone()) {
            if existing_titles.insert(native.clone()) {
                other_titles.push(Synonym::anilist(&native));
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

        // Staff extraction - do this first before moving any fields from media
        let (writer, penciller, inker, colorist, letterer, editor, translator) = 
            extract_staff_from_media(&media);
        trace!("Extracted Writers: {writer:?}");
        trace!("Extracted penciller: {penciller:?}");
        trace!("Extracted inker: {inker:?}");
        trace!("Extracted colorist: {colorist:?}");
        trace!("Extracted letterer: {letterer:?}");
        trace!("Extracted editor: {editor:?}");
        trace!("Extracted translator: {translator:?}");

        // Extract tags and genre
        let tags: Vec<String> = media
            .tags
            .as_ref()
            .map(|tags| tags.iter().filter_map(|t| t.name.clone()).collect())
            .unwrap_or_default();
        trace!("Extracted Tags: {tags:?}");
        let genre = Some(media.genres.unwrap_or_default().first().unwrap().clone());
        trace!("Extracted Genre: {genre:?}");

        // Community rating %
        let community_rating = media.average_score;

        // chapter_count stays None until scraped from providers; AniList data is unreliable
        let chapter_count = None;

        let metadata = MangaMetadata {
            title,
            other_titles: Some(other_titles),
            synopsis: media.description.as_deref().map(strip_html),
            publishing_status: status,
            tags,
            start_year: media.start_date.as_ref().and_then(|d| d.year),
            start_month: media.start_date.as_ref().and_then(|d| d.month),
            start_day: media.start_date.as_ref().and_then(|d| d.day),
            end_year: media.end_date.and_then(|d| d.year),

            // ComicInfo fields
            writer,
            penciller,
            inker,
            colorist,
            letterer,
            editor,
            translator,
            genre,
            community_rating,
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
            last_checked_at: None,
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
