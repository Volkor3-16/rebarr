use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manga {
    pub id: Uuid,               // internal, canonical
    pub mal_id: Option<u32>,    // external identity
    pub title: String,
    pub synopsis: Option<String>,
    pub status: MangaStatus,
    pub chapter_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MangaStatus {
    Ongoing,
    Completed,
    Hiatus,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: Uuid,
    pub manga_id: Uuid,
    pub number: Option<f32>,      // supports 12.5
    pub title: Option<String>,
    pub volume: Option<u32>,
    pub scanlator_group: Option<String>,
}

// We use this for storing the scraped-provided metadata, but since this can change on the site, we save it on scrape, and never again?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaAlias {
    pub manga_id: Uuid,
    pub source: AliasSource,
    pub title: String,
}

// Where the metadata came from
// It defaults to MAL api, and failing that, the scraped site, but also allowing for 'manual' entries, if you wanna do it yourself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AliasSource {
    MyAnimeList,
    Site(String),
    Manual,
}

