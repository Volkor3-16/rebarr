/// A test-only provider with fully controlled, in-memory data.
///
/// Build it with the builder API, inject it into a ProviderRegistry via
/// `ProviderRegistry::from_providers_for_tests`, and use it to drive
/// merge/download flows without hitting the network or a headless browser.
///
/// # Example
/// ```rust,no_run
/// let provider = StaticProvider::new("test")
///     .series("Berserk", "static://berserk")
///         .chapter("1").chapter("12").chapter("12.5").end_series()
///     .build();
/// ```
use async_trait::async_trait;
use rebarr::scraper::{
    PageUrl, Provider, ProviderChapterInfo, ProviderSearchResult, ScraperCtx,
    error::ScraperError,
};

// ---------------------------------------------------------------------------
// Chapter number parser (mirrors engine.rs private function)
// ---------------------------------------------------------------------------

/// Parse "1", "12.5", "100a", "EX1" → (number_sort, chapter_base, chapter_variant)
fn parse_chapter_number(raw: &str) -> (f32, f32, u8) {
    let token = raw.split_whitespace().last().unwrap_or(raw).trim();

    if let Ok(n) = token.parse::<f32>() {
        let base = n.floor();
        let frac = (n - base).abs();
        let variant = (frac * 10.0).round() as u8;
        return (n, base, variant);
    }

    if let Some(letter_pos) = token.rfind(|c: char| c.is_ascii_alphabetic()) {
        let (num_part, letter_part) = token.split_at(letter_pos);
        if let Ok(base) = num_part.parse::<f32>() {
            if let Some(letter) = letter_part.chars().next() {
                if letter.is_ascii_alphabetic() {
                    let variant = (letter.to_ascii_lowercase() as u8) - b'a' + 1;
                    let number = base + (variant as f32) / 10.0;
                    return (number, base, variant);
                }
            }
        }
    }

    (0.0, 0.0, 0)
}

fn infer_is_extra(raw: &str) -> bool {
    let lower = raw.to_lowercase();
    ["extra", "omake", "special", "bonus", "ex"].iter().any(|kw| lower.contains(kw))
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct StaticChapter {
    pub raw_number: String,
    pub title: Option<String>,
    pub language: String,
    pub scanlator_group: Option<String>,
    /// Number of synthetic page URLs to generate.
    pub page_count: u32,
}

impl StaticChapter {
    pub fn new(raw_number: impl Into<String>) -> Self {
        Self {
            raw_number: raw_number.into(),
            title: None,
            language: "EN".to_owned(),
            scanlator_group: None,
            page_count: 3,
        }
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.scanlator_group = Some(group.into());
        self
    }

    pub fn with_language(mut self, lang: impl Into<String>) -> Self {
        self.language = lang.into();
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_pages(mut self, n: u32) -> Self {
        self.page_count = n;
        self
    }
}

#[derive(Clone)]
pub struct StaticSeries {
    pub title: String,
    pub url: String,
    pub chapters: Vec<StaticChapter>,
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct StaticProvider {
    name: String,
    series: Vec<StaticSeries>,
}

impl StaticProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), series: Vec::new() }
    }

    /// Add a series with its chapters.
    pub fn with_series(
        mut self,
        title: impl Into<String>,
        url: impl Into<String>,
        chapters: Vec<StaticChapter>,
    ) -> Self {
        self.series.push(StaticSeries {
            title: title.into(),
            url: url.into(),
            chapters,
        });
        self
    }

    fn find_series_by_url(&self, url: &str) -> Option<&StaticSeries> {
        self.series.iter().find(|s| s.url == url)
    }
}

#[async_trait]
impl Provider for StaticProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn needs_browser(&self) -> bool {
        false
    }

    fn rate_limit_rpm(&self) -> u32 {
        u32::MAX
    }

    fn max_concurrency(&self) -> u32 {
        16
    }

    async fn search(
        &self,
        _ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        let lower = title.to_lowercase();
        let results = self
            .series
            .iter()
            .filter(|s| s.title.to_lowercase().contains(&lower) || lower.contains(&s.title.to_lowercase()))
            .map(|s| ProviderSearchResult {
                title: s.title.clone(),
                url: s.url.clone(),
                cover_url: None,
            })
            .collect();
        Ok(results)
    }

    async fn chapters(
        &self,
        _ctx: &ScraperCtx,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        let Some(series) = self.find_series_by_url(manga_url) else {
            return Ok(vec![]);
        };

        let chapters = series
            .chapters
            .iter()
            .map(|ch| {
                let (number, chapter_base, chapter_variant) =
                    parse_chapter_number(&ch.raw_number);
                let is_extra = infer_is_extra(&ch.raw_number)
                    || ch.title.as_deref().map(infer_is_extra).unwrap_or(false);
                ProviderChapterInfo {
                    raw_number: ch.raw_number.clone(),
                    number,
                    chapter_base,
                    chapter_variant,
                    is_extra,
                    title: ch.title.clone(),
                    url: Some(format!("{}/{}", manga_url, ch.raw_number)),
                    volume: None,
                    scanlator_group: ch.scanlator_group.clone(),
                    language: Some(ch.language.clone()),
                    date_released: None,
                }
            })
            .collect();

        Ok(chapters)
    }

    async fn pages(
        &self,
        _ctx: &ScraperCtx,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        // Derive page_count from the chapter URL by finding the series + chapter
        let page_count = self
            .series
            .iter()
            .flat_map(|s| {
                s.chapters.iter().filter_map(|ch| {
                    let url = format!("{}/{}", s.url, ch.raw_number);
                    if url == chapter_url {
                        Some(ch.page_count)
                    } else {
                        None
                    }
                })
            })
            .next()
            .unwrap_or(3);

        let pages = (0..page_count)
            .map(|i| PageUrl {
                url: format!("{}/page-{i}.png", chapter_url),
                index: i,
                referrer: None,
            })
            .collect();

        Ok(pages)
    }
}

// ---------------------------------------------------------------------------
// Convenience chapter constructors
// ---------------------------------------------------------------------------

/// Quick shorthand: `ch("12.5")`
pub fn ch(raw: &str) -> StaticChapter {
    StaticChapter::new(raw)
}

/// Chapter with a specific scanlator group.
pub fn ch_group(raw: &str, group: &str) -> StaticChapter {
    StaticChapter::new(raw).with_group(group)
}
