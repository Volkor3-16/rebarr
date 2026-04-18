pub mod browser;
pub mod def;
pub mod downloader;
pub mod engine;
pub mod error;
pub mod executor;

use async_trait::async_trait;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing::{info, warn};

use browser::BrowserPool;
use def::{DownloadMethod, ProviderDef, ProviderTag};
use engine::YamlProvider;
use error::ScraperError;
use executor::ProviderExecutor;

// If you're reading this. I'm so sorry.

// ---------------------------------------------------------------------------
// Output types (runtime only — never persisted to DB)
// ---------------------------------------------------------------------------

pub type ProviderVariables = HashMap<String, String>;

/// A manga entry returned by a provider's search.
#[derive(Debug, Clone)]
pub struct ProviderSearchResult {
    pub title: String,
    pub url: String,
    pub cover_url: Option<String>,
    pub variables: ProviderVariables,
}

/// Info about a single chapter as returned by a provider's chapter list.
#[derive(Debug, Clone)]
pub struct ProviderChapterInfo {
    pub raw_number: String,  // Raw value as scraped (e.g. "12.5", "12a")
    pub number: f32,         // Parsed chapter number for ordering (e.g. 12.5, 12.1)
    pub chapter_base: f32,   // Integer part of the chapter number (e.g. 12.0)
    pub chapter_variant: u8, // Sub-part index: 0=full, 1-9=split part index
    pub is_extra: bool,      // True if this is a bonus/extra chapter (inferred from title keywords)
    pub title: Option<String>,
    pub url: Option<String>,
    pub volume: Option<u32>,
    pub scanlator_group: Option<String>,
    /// BCP 47 language code scraped from the provider (e.g. "en", "ja"). None = assume "en".
    pub language: Option<String>,
    /// Publication date scraped from the provider as a Unix timestamp. None if not provided
    /// or if the YAML field's `date_format` did not match the scraped value.
    pub date_released: Option<i64>,
}

/// A single page image inside a chapter.
#[derive(Debug, Clone)]
pub struct PageUrl {
    pub url: String,
    pub index: u32,
    /// Optional per-page referrer override (e.g. supplied by a provider's return step).
    /// Falls back to the chapter URL if absent.
    pub referrer: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared context passed to every provider call
// ---------------------------------------------------------------------------

/// Everything a provider needs to make requests.
///
/// Stored in Rocket's managed state so API handlers can reach all providers
/// through one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ScraperDebugLevel {
    #[default]
    Off,
    Summary,
    Verbose,
}

#[derive(Debug, Clone, Default)]
struct ScraperDebugContext {
    phase: Option<String>,
    last_step: Option<String>,
    last_request: Option<String>,
    source_var: Option<String>,
    parse_stage: Option<String>,
}

#[derive(Clone)]
pub struct ScraperCtx {
    /// Pre-configured HTTP client (respects timeouts, user-agent, etc.)
    pub http: reqwest::Client,
    /// Lazily-started headless browser pool. Only materialised if a
    /// provider calls `browser.get()`.
    pub browser: BrowserPool,
    /// Shared scheduler for provider calls.
    pub executor: Arc<ProviderExecutor>,
    /// When true, dump page HTML to `./scraper_dump_N.html` after every `open` step.
    /// Useful for debugging provider YAML issues.
    pub dump_html: bool,
    /// Controls user-facing provider trace output in the CLI.
    pub debug_level: ScraperDebugLevel,
    debug_context: Arc<Mutex<ScraperDebugContext>>,
}

impl ScraperCtx {
    pub fn new(
        http: reqwest::Client,
        browser: BrowserPool,
        executor: Arc<ProviderExecutor>,
    ) -> Self {
        Self {
            http,
            browser,
            executor,
            dump_html: false,
            debug_level: ScraperDebugLevel::Off,
            debug_context: Arc::new(Mutex::new(ScraperDebugContext::default())),
        }
    }

    pub fn set_debug_level(&mut self, level: ScraperDebugLevel) {
        self.debug_level = level;
    }

    pub fn is_debug_enabled(&self, level: ScraperDebugLevel) -> bool {
        self.debug_level >= level
    }

    pub fn begin_phase(&self, phase: impl Into<String>) {
        let phase = phase.into();
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        *ctx = ScraperDebugContext {
            phase: Some(phase),
            ..ScraperDebugContext::default()
        };
    }

    pub fn clear_debug_context(&self) {
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        let phase = ctx.phase.take();
        *ctx = ScraperDebugContext {
            phase,
            ..ScraperDebugContext::default()
        };
    }

    pub fn note_step(&self, step: impl Into<String>) {
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        ctx.last_step = Some(step.into());
    }

    pub fn note_request(&self, request: impl Into<String>) {
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        ctx.last_request = Some(request.into());
    }

    pub fn note_source_var(&self, var: impl Into<String>) {
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        ctx.source_var = Some(var.into());
    }

    pub fn note_parse_stage(&self, stage: impl Into<String>) {
        let mut ctx = self.debug_context.lock().expect("debug context poisoned");
        ctx.parse_stage = Some(stage.into());
    }

    pub fn emit_debug(&self, level: ScraperDebugLevel, message: impl AsRef<str>) {
        if self.is_debug_enabled(level) {
            eprintln!("{}", message.as_ref());
        }
    }

    pub fn last_debug_summary(&self) -> Option<String> {
        let ctx = self.debug_context.lock().expect("debug context poisoned");
        let mut parts = Vec::new();
        if let Some(phase) = &ctx.phase {
            parts.push(format!("phase={phase}"));
        }
        if let Some(step) = &ctx.last_step {
            parts.push(format!("step={step}"));
        }
        if let Some(request) = &ctx.last_request {
            parts.push(format!("request={request}"));
        }
        if let Some(source_var) = &ctx.source_var {
            parts.push(format!("var={source_var}"));
        }
        if let Some(parse_stage) = &ctx.parse_stage {
            parts.push(format!("parse={parse_stage}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
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
use std::any::Any;

#[async_trait]
pub trait Provider: Send + Sync + Any {
    /// Returns self as Any for downcasting.
    fn as_any(&self) -> &dyn Any where Self: Sized {
        self
    }
    /// Human-readable provider name (e.g. "MangaFire").
    fn name(&self) -> &str;

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

    /// Maximum number of concurrent jobs to run for this provider within a process.
    fn max_concurrency(&self) -> u32 {
        1
    }

    /// Provider version string, if declared in the YAML (e.g. "1", "2.1").
    fn version(&self) -> Option<&str> {
        None
    }

    /// Quality / characteristic tags declared for this provider.
    fn tags(&self) -> &[ProviderTag] {
        &[]
    }

    /// How image bytes should be fetched when downloading chapter pages.
    /// Defaults to `Auto` (try reqwest first, fall back to CDP).
    fn pages_download_method(&self) -> DownloadMethod {
        DownloadMethod::Auto
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
        variables: &ProviderVariables,
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
    /// All loaded providers
    providers: Vec<Arc<dyn Provider>>,
    /// Raw ProviderDefs from YAML
    defs: Vec<ProviderDef>,
}

impl ProviderRegistry {
    /// Load every `*.yaml` file found in `REBARR_PROVIDERS_DIR` (or
    /// `./providers/` if the env var is unset).
    pub async fn load() -> Result<Self, ScraperError> {
        let dir: PathBuf = std::env::var("REBARR_PROVIDERS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./providers"));

        let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
        let mut defs: Vec<ProviderDef> = Vec::new();

        if !dir.exists() {
            info!(
                "Provider directory '{}' does not exist — no providers loaded. \
                 Create the directory and add YAML files to enable scraping.",
                dir.display()
            );
            return Ok(Self { providers, defs });
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
                    info!("Loaded provider '{}' from {}", def.name, path.display());
                    providers.push(Arc::new(YamlProvider::new(def.clone())));
                    defs.push(def);
                }
                Err(e) => {
                    warn!("Skipping invalid provider config '{}': {}", path.display(), e);
                }
            }
        }

        info!("Loaded {} provider(s) total.", providers.len());
        Ok(Self { providers, defs })
    }

    /// All loaded providers in load order.
    pub fn all(&self) -> Vec<&Arc<dyn Provider>> {
        self.providers.iter().collect()
    }

    /// Providers that require a headless browser, used to decide whether to
    /// pre-warm the `BrowserPool` at startup.
    pub fn browser_providers(&self) -> impl Iterator<Item = &Arc<dyn Provider>> {
        self.providers.iter().filter(|p| p.needs_browser())
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn from_providers_for_tests(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers, defs: Vec::new() }
    }

    /// Get all ProviderDef instances for loaded YAML providers.
    /// Used to populate default quality rules.
    pub fn all_defs(&self) -> &[ProviderDef] {
        &self.defs
    }
}
