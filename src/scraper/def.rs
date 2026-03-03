/// Deserialized representation of a provider YAML config file.
///
/// Users and developers add providers by placing a `.yaml` file in the
/// providers directory (default: `./providers/`, or `REBARR_PROVIDERS_DIR`).
/// No Rust code required.
use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDef {
    /// Display name for this provider (e.g. "MangaFire").
    pub name: String,

    /// Root URL of the site (e.g. "https://mangafire.to"). Used in URL
    /// templates as `{base_url}`.
    pub base_url: String,

    /// Provider preference score, 0–100. Higher = preferred when multiple
    /// providers have the same chapter. Defaults to 50.
    #[serde(default = "default_score")]
    pub score: u8,

    /// Set to true if this provider requires a JavaScript-capable browser.
    /// The headless Chromium pool is only started if at least one provider
    /// needs it. Defaults to false.
    #[serde(default)]
    pub needs_browser: bool,

    /// Per-provider rate limiting.
    #[serde(default)]
    pub rate_limit: RateLimitDef,

    /// How to search for a manga by title.
    pub search: Option<SearchDef>,

    /// How to fetch the chapter list for a manga.
    pub chapters: Option<ChaptersDef>,

    /// How to fetch page image URLs for a single chapter.
    pub pages: Option<PagesDef>,
}

fn default_score() -> u8 {
    50
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitDef {
    /// Maximum HTTP requests per minute to this provider's domain.
    #[serde(default = "default_rpm")]
    pub requests_per_minute: u32,
}

impl Default for RateLimitDef {
    fn default() -> Self {
        Self {
            requests_per_minute: default_rpm(),
        }
    }
}

fn default_rpm() -> u32 {
    30
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SearchDef {
    /// URL template for the search page. `{query}` is replaced with the
    /// URL-encoded search string. May be a path (appended to base_url) or
    /// an absolute URL.
    pub url: String,

    /// How to extract individual result entries from the search page HTML.
    pub results: ResultsDef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResultsDef {
    /// CSS selector matching one element per result card / row.
    pub selector: String,

    /// Named fields extracted from each matched element.
    pub fields: ResultFieldsDef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResultFieldsDef {
    pub title: FieldDef,
    pub url: FieldDef,
    pub cover: Option<FieldDef>,
}

// ---------------------------------------------------------------------------
// Chapters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ChaptersDef {
    /// Direct URL template. `{manga_url}` is the series URL as stored.
    /// Use this OR `url_transform`, not both.
    pub url: Option<String>,

    /// Build the chapter-list URL by stripping a path segment from the stored
    /// series URL and appending a new one. Useful when the chapter list is at
    /// a sibling path (e.g. `.../full-chapter-list`) rather than the series
    /// URL itself.
    pub url_transform: Option<ChapterUrlTransform>,

    pub list: ChapterListDef,
}

/// Derives a chapter-list URL from the stored series URL.
///
/// Example: series URL `/series/ID/Manga-Title` with
/// `strip_last_segments: 1` and `append: "full-chapter-list"`
/// → `/series/ID/full-chapter-list`
#[derive(Debug, Clone, Deserialize)]
pub struct ChapterUrlTransform {
    /// Number of path segments to remove from the end of the series URL.
    #[serde(default = "default_strip")]
    pub strip_last_segments: usize,
    /// Segment to append after stripping.
    pub append: String,
}

fn default_strip() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChapterListDef {
    /// CSS selector matching one element per chapter row.
    pub selector: String,

    pub fields: ChapterFieldsDef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChapterFieldsDef {
    pub number_raw: FieldDef,
    pub title: Option<FieldDef>,
    pub url: FieldDef,
    pub scanlator_group: Option<FieldDef>,
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PagesDef {
    /// URL to fetch for page image extraction. Defaults to `{chapter_url}`
    /// (the chapter URL as stored). Use this when a separate endpoint returns
    /// a cleaner HTML fragment (e.g. `{chapter_url}/images?reading_style=long_strip`).
    pub url: Option<String>,

    /// Declarative extraction rules. Used when the page structure is
    /// straightforward. Mutually exclusive with `script` (script wins).
    pub extract: Option<ExtractDef>,

    /// Inline Lua script for complex extraction. Called with globals:
    ///   `html`     (string) — rendered HTML of the chapter reader page
    ///   `base_url` (string) — provider base URL
    ///
    /// Available helper functions:
    ///   `select(html, selector)` → array of element userdata
    ///   `attr(element, name)`    → string
    ///   `text(element)`          → string
    ///   `json_decode(str)`       → table
    ///   `url_join(base, path)`   → string
    ///
    /// Must return an array of tables: `{ url: string, index: number }`.
    pub script: Option<String>,
}

// ---------------------------------------------------------------------------
// Extraction rule variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtractDef {
    /// Find elements by CSS selector and extract an attribute value from each.
    CssAttr {
        selector: String,
        attr: String,
        #[serde(default)]
        prefix: String,
    },

    /// Find elements by CSS selector and extract their text content.
    CssText {
        selector: String,
        #[serde(default)]
        prefix: String,
    },

    /// Find a `<script>` tag, extract its text, optionally slice/trim, then
    /// parse as JSON and navigate to an array of URL strings.
    ScriptJson {
        /// CSS selector for the script tag (e.g. `"script:contains(__DATA__)"`)
        selector: String,
        /// Take everything after the first occurrence of this string.
        after: Option<String>,
        #[serde(default)]
        trim: bool,
        /// Strip this suffix from the extracted string before JSON parsing.
        remove_suffix: Option<String>,
        /// Dot-separated path into the JSON object (e.g. `"data.images"`).
        json_path: Option<String>,
        /// Optional rule for constructing the final URL from each raw value.
        build_url: Option<BuildUrlDef>,
    },

    /// Fetch a JSON API endpoint and navigate to an array of URL strings.
    ApiJson {
        /// URL template (supports `{manga_url}`, `{base_url}`, etc.).
        url_template: String,
        /// Dot-separated path into the JSON response.
        json_path: Option<String>,
        build_url: Option<BuildUrlDef>,
    },
}

/// Conditional URL construction rule.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildUrlDef {
    pub if_starts_with: String,
    pub then: String,
    pub r#else: String,
}

// ---------------------------------------------------------------------------
// Generic field extractor (used in search results and chapter lists)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    /// CSS selector relative to the parent element.
    pub selector: String,

    /// What to extract from the matched element.
    pub content: ContentKind,

    /// For `content: attr`, the attribute name to read.
    pub attr_name: Option<String>,

    /// String prepended to the extracted value (e.g. `"{base_url}"`).
    #[serde(default)]
    pub prefix: String,

    /// Optional mapping from raw extracted value to a display label.
    /// If the raw value matches a key, the mapped value is returned instead.
    /// Useful for turning attribute values (e.g. an SVG stroke color) into
    /// human-readable strings (e.g. "Official").
    #[serde(default)]
    pub value_map: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Attr,
}
