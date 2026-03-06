pub mod browser;
pub mod def;
pub mod engine;
pub mod error;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;

use browser::BrowserPool;
use def::ProviderDef;
use engine::YamlProvider;
use error::ScraperError;

// ---------------------------------------------------------------------------
// Output types (runtime only — never persisted to DB)
// ---------------------------------------------------------------------------

/// A manga entry returned by a provider's search.
#[derive(Debug, Clone)]
pub struct ProviderSearchResult {
    pub title: String,
    pub url: String,
    pub cover_url: Option<String>,
}

/// Info about a single chapter as returned by a provider's chapter list.
#[derive(Debug, Clone)]
pub struct ProviderChapterInfo {
    pub raw_number: String, // Raw value as scraped (e.g. "12.5")
    pub number: f32,        // Parsed chapter number for ordering
    pub title: Option<String>,
    pub url: Option<String>,
    pub volume: Option<u32>,
    pub scanlator_group: Option<String>,
}

/// A single page image inside a chapter.
#[derive(Debug, Clone)]
pub struct PageUrl {
    pub url: String,
    pub index: u32,
}

// ---------------------------------------------------------------------------
// Shared context passed to every provider call
// ---------------------------------------------------------------------------

/// Everything a provider needs to make requests.
///
/// Stored in Rocket's managed state so API handlers can reach all providers
/// through one value.
#[derive(Clone)]
pub struct ScraperCtx {
    /// Pre-configured HTTP client (respects timeouts, user-agent, etc.)
    pub http: reqwest::Client,
    /// Lazily-started headless browser pool. Only materialised if a
    /// provider calls `browser.get()`.
    pub browser: BrowserPool,
    /// When true, dump page HTML to `./scraper_dump_N.html` after every `open` step.
    /// Useful for debugging provider YAML issues.
    pub dump_html: bool,
    /// Base URL of a running FlareSolverr instance (e.g. `http://localhost:8191`).
    /// When set, Cloudflare challenge pages are bypassed by calling FlareSolverr
    /// and injecting the resulting cookies before reloading.
    pub flaresolverr_url: Option<String>,
}

impl ScraperCtx {
    pub fn new(http: reqwest::Client, browser: BrowserPool) -> Self {
        Self {
            http,
            browser,
            dump_html: false,
            flaresolverr_url: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// The interface every scraping provider must implement.
///
/// YAML-defined providers implement this through `YamlProvider` + the
/// declarative engine. Complex providers can also be implemented directly
/// in Rust by implementing this trait.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Human-readable provider name (e.g. "MangaFire").
    fn name(&self) -> &str;

    /// Preference score 0–100. Higher = preferred when multiple providers
    /// have the same chapter. Used by the merge/download logic.
    fn score(&self) -> u8;

    /// Returns true if this provider requires JavaScript rendering.
    /// Always true for YAML-driven providers (all actions use the headless browser).
    fn needs_browser(&self) -> bool {
        true
    }

    /// Maximum requests per minute to enforce for this provider.
    /// Used by the worker rate limiter. Defaults to 30.
    fn rate_limit_rpm(&self) -> u32 {
        30
    }

    /// Search for a manga by title. Returns ranked candidates.
    async fn search(
        &self,
        ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError>;

    /// Fetch all chapters for a manga given its URL on this provider.
    /// The returned vec is sorted ascending by chapter number.
    async fn chapters(
        &self,
        ctx: &ScraperCtx,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError>;

    /// Fetch ordered page image URLs for a single chapter.
    async fn pages(
        &self,
        ctx: &ScraperCtx,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError>;
}

// ---------------------------------------------------------------------------
// ProviderRegistry
// ---------------------------------------------------------------------------

/// Holds all loaded providers. Stored as Rocket managed state.
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// Load every `*.yaml` file found in `REBARR_PROVIDERS_DIR` (or
    /// `./providers/` if the env var is unset).
    pub async fn load() -> Result<Self, ScraperError> {
        let dir: PathBuf = std::env::var("REBARR_PROVIDERS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./providers"));

        let mut providers: Vec<Arc<dyn Provider>> = Vec::new();

        if !dir.exists() {
            log::info!(
                "Provider directory '{}' does not exist — no providers loaded. \
                 Create the directory and add YAML files to enable scraping.",
                dir.display()
            );
            return Ok(Self { providers });
        }

        let mut read_dir = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }

            let content = tokio::fs::read_to_string(&path).await?;
            match serde_yaml::from_str::<ProviderDef>(&content) {
                Ok(def) => {
                    log::info!("Loaded provider '{}' from {}", def.name, path.display());
                    providers.push(Arc::new(YamlProvider::new(def)));
                }
                Err(e) => {
                    log::warn!("Skipping invalid provider config '{}': {e}", path.display());
                }
            }
        }

        log::info!("Loaded {} provider(s) total.", providers.len());
        Ok(Self { providers })
    }

    /// All providers sorted by descending score (highest quality first).
    pub fn by_score(&self) -> Vec<&Arc<dyn Provider>> {
        let mut v: Vec<_> = self.providers.iter().collect();
        v.sort_by(|a, b| b.score().cmp(&a.score()));
        v
    }

    /// Providers that require a headless browser, used to decide whether to
    /// pre-warm the `BrowserPool` at startup.
    pub fn browser_providers(&self) -> impl Iterator<Item = &Arc<dyn Provider>> {
        self.providers.iter().filter(|p| p.needs_browser())
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}
