#![allow(dead_code)]

pub mod static_provider;

use std::sync::Arc;

use rebarr::{
    db,
    manga::core::{Library, Manga, MangaMetadata, MangaSource, PublishingStatus},
    scraper::{ProviderRegistry, ScraperCtx, browser::BrowserPool, executor::ProviderExecutor},
};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Spin up an in-memory SQLite database and run all migrations.
pub async fn test_db() -> SqlitePool {
    db::init("sqlite::memory:").await.expect("test DB init failed")
}

/// Build a ScraperCtx wired to the given ProviderRegistry (no real browser started).
pub fn test_ctx(registry: &ProviderRegistry) -> ScraperCtx {
    let executor = Arc::new(ProviderExecutor::new(registry, 4));
    ScraperCtx::new(reqwest::Client::new(), BrowserPool::new(), executor)
}

/// Insert a minimal Library row and return it.
pub async fn insert_library(pool: &SqlitePool) -> Library {
    use std::path::PathBuf;
    let lib = Library {
        uuid: Uuid::new_v4(),
        r#type: rebarr::manga::core::MangaType::Manga,
        root_path: PathBuf::from("/tmp/rebarr-test"),
    };
    db::library::insert(pool, &lib).await.expect("insert library");
    lib
}

/// Insert a minimal Manga row pointing at `library_id` and return it.
pub async fn insert_manga(pool: &SqlitePool, library_id: Uuid, title: &str) -> Manga {
    let manga = Manga {
        id: Uuid::new_v4(),
        library_id,
        anilist_id: None,
        mal_id: None,
        metadata: MangaMetadata {
            title: title.to_owned(),
            other_titles: None,
            synopsis: None,
            publishing_status: PublishingStatus::Ongoing,
            tags: vec![],
            start_year: None,
            start_month: None,
            start_day: None,
            end_year: None,
            writer: None,
            penciller: None,
            inker: None,
            colorist: None,
            letterer: None,
            editor: None,
            translator: None,
            genre: None,
            community_rating: None,
        },
        relative_path: std::path::PathBuf::from(title.to_lowercase().replace(' ', "-")),
        downloaded_count: Some(0),
        chapter_count: None,
        metadata_source: MangaSource::Local,
        thumbnail_url: None,
        monitored: false,
        created_at: chrono::Utc::now().timestamp(),
        metadata_updated_at: chrono::Utc::now().timestamp(),
        last_checked_at: None,
    };
    db::manga::insert(pool, &manga).await.expect("insert manga");
    manga
}
