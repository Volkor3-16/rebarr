/// Deserialized representation of a provider YAML config file.
///
/// Users add providers by placing a `.yaml` file in the providers directory
/// (default: `./providers/`, or `REBARR_PROVIDERS_DIR`). No Rust code required.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDef {
    /// Display name (e.g. "WeebCentral").
    pub name: String,

    /// Root URL (e.g. "https://weebcentral.com"). Used as `{base_url}` in templates.
    pub base_url: String,

    /// Provider version string (e.g. "1", "2.1"). Purely informational — useful for
    /// tracking compatibility when the YAML format or site structure changes.
    #[serde(default)]
    pub version: Option<String>,

    /// Quality / characteristic tags. Displayed in the UI to warn users about
    /// Cloudflare protection, scan quality issues, adult content, etc.
    #[serde(default)]
    pub tags: Vec<ProviderTag>,

    /// Default score for chapter ranking tiebreaks within the same tier.
    /// Can be overridden globally or per-series via the API.
    #[serde(default)]
    pub default_score: i32,

    /// Per-provider rate limiting.
    #[serde(default)]
    pub rate_limit: RateLimitDef,

    /// Steps to search for a manga by title.
    pub search: Option<ActionDef>,

    /// Steps to fetch the chapter list for a manga.
    pub chapters: Option<ActionDef>,

    /// Steps to fetch page image URLs for a single chapter.
    pub pages: Option<ActionDef>,
}

// ---------------------------------------------------------------------------
// Provider tags
// ---------------------------------------------------------------------------

/// Well-known quality / characteristic tags for a provider.
///
/// Unknown values in YAML cause a load error (intentional — keeps tags validated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTag {
    /// Provider is behind Cloudflare; may require a FlareSolverr instance.
    Cloudflare,
    /// Scan quality is known to be poor (low resolution, watermarks, etc.).
    LowQualityScans,
    /// Scan quality is known to be excellent.
    HighQualityScans,
    /// Chapter numbers, dates, or titles are frequently wrong or missing.
    BadMetadata,
    /// Contains adult / NSFW content.
    Nsfw,
    /// Consistently slow to respond; expect longer scrape times.
    Slow,
    /// Hosts Official / Licensed content.
    Official,
    /// Aggregates chapters from multiple upstream sources.
    Aggregator,
    /// Hosts fan-translated chapters not yet licensed officially.
    Hub,
    /// This is a site for a single scanlator group
    ScanlatorSite,
    /// Offers multiple languages
    MultiLanguage,
    /// The site frequently goes down, or has other problems with access.
    Unstable,
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitDef {
    #[serde(default = "default_rpm")]
    pub requests_per_minute: u32,
    /// Milliseconds to sleep between individual page image downloads.
    /// Helps avoid rate-limiting / IP bans on providers that throttle aggressively.
    /// Defaults to 0 (no delay).
    #[serde(default)]
    pub page_delay_ms: u64,
}

impl Default for RateLimitDef {
    fn default() -> Self {
        Self {
            requests_per_minute: default_rpm(),
            page_delay_ms: 0,
        }
    }
}

fn default_rpm() -> u32 {
    30
}

// ---------------------------------------------------------------------------
// Action (sequence of steps)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    pub steps: Vec<StepDef>,
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// A single step in a provider action. Steps are executed in order.
///
/// Each variant is a YAML map with exactly one key naming the step type.
/// serde_yaml 0.9 uses `!tag` syntax for externally-tagged enums; by using
/// `#[serde(untagged)]` with struct variants we get the readable `key: value`
/// map format instead.
///
/// Template placeholders available in string values:
///   `{base_url}`    — provider base URL
///   `{query}`       — URL-encoded search query (search action only)
///   `{manga_url}`   — series URL (chapters action only)
///   `{chapter_url}` — chapter URL (pages action only)
///   `{var_name}`    — any variable set by a previous `extract_js` step
///
/// Relative paths (starting with `/`) in `open` are automatically prefixed with `{base_url}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StepDef {
    /// `- open: "/path"` — Navigate the browser to a URL.
    Open { open: String },

    /// `- wait_for: "selector"` — Poll for a CSS selector to appear (up to 10 s).
    WaitFor { wait_for: String },

    /// `- click: "selector"` — Click the first element matching a CSS selector.
    Click { click: String },

    /// `- type: {selector: "...", value: "..."}` — Set an input value and fire events.
    Type {
        #[serde(rename = "type")]
        type_def: TypeDef,
    },

    /// `- sleep: 500` — Sleep for N milliseconds.
    Sleep { sleep: u64 },

    /// `- script: "js"` — Execute JavaScript for side effects (return value ignored).
    Script { script: String },

    /// `- extract_js: {var: name, script: expr}` — Eval JS, store result in a variable.
    ExtractJs { extract_js: ExtractJsDef },

    /// `- intercept: {url_contains: "...", var: name}` — Intercept fetch/XHR matching a
    /// URL pattern. Place BEFORE `open` to use `addScriptToEvaluateOnNewDocument` so
    /// the monkey-patch runs before any page scripts. After `open`, the engine polls
    /// `window.__rebarr_captures[url_contains]` for up to 10 s.
    Intercept { intercept: InterceptDef },

    /// `- foreach: {selector: "...", extract: {field: ...}}` — Iterate over DOM elements,
    /// extract named fields from each, and accumulate them as result records.
    ///
    /// For `search`: fields `title`, `url`, and optionally `cover`.
    /// For `chapters`: fields `number_raw`, `url`, and optionally `title`,
    ///   `scanlator_group`, `volume`.
    /// For `pages`: field `url` (indexed by DOM order).
    Foreach { foreach: ForeachDef },

    /// `- return: "{var}"` — Return a specific value instead of accumulated foreach results.
    /// Exits the step loop immediately. For `pages`, value must be a JSON array of URL strings.
    Return {
        #[serde(rename = "return")]
        value: String,
    },

    /// `- scroll: "bottom"` — Scroll to the bottom, or `- scroll: "selector"` to an element.
    Scroll { scroll: String },

    /// `- fetch: {url: "...", var: name}` — Execute an HTTP request via browser's fetch()
    /// and store the response body as a string variable. All traffic goes through Chromium.
    Fetch { fetch: FetchDef },

    /// `- graphql: {url: "...", query: "...", variables: {...}, var: name}` — 
    /// Sugar over fetch for GraphQL endpoints. Sends POST with JSON body.
    Graphql { graphql: GraphqlDef },

    /// `- from_json: {var: source_var, extract: {...}}` — Map a stored JSON array 
    /// directly to result rows, replacing the extract_js → foreach pattern.
    FromJson { from_json: FromJsonDef },
}

// ---------------------------------------------------------------------------
// Step argument types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct TypeDef {
    /// CSS selector for the input element.
    pub selector: String,
    /// Value to type. Supports template placeholders.
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractJsDef {
    /// Variable name to store the result in.
    pub var: String,
    /// JavaScript expression to evaluate. The result is coerced to a string.
    pub script: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterceptDef {
    /// Match intercepted request URLs containing this string.
    pub url_contains: String,
    /// Variable name to store the response body in.
    pub var: String,
    /// Optional dot-separated JSON path to navigate after parsing the body.
    pub json_path: Option<String>,
}

// Note: TypeDef, ExtractJsDef, InterceptDef are inner values of StepDef variants.

#[derive(Debug, Clone, Deserialize)]
pub struct ForeachDef {
    /// CSS selector for repeating elements (one per result record).
    pub selector: String,
    /// Map of output field name → extraction definition.
    /// Field names are arbitrary; the engine uses them by convention
    /// (e.g. `title`, `url`, `number_raw`).
    pub extract: HashMap<String, FieldDef>,
}

// ---------------------------------------------------------------------------
// Generic field extractor (used inside `foreach`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    /// CSS selector relative to the foreach element. Empty = use the element itself.
    #[serde(default)]
    pub selector: String,

    /// What to extract from the matched element. Not required when `static_value` is set.
    pub content: Option<ContentKind>,

    /// For `content: attr`, the attribute name to read.
    pub attr_name: Option<String>,

    /// String prepended to the extracted value (e.g. `"{base_url}"`).
    /// Skipped when the value is already an absolute URL.
    #[serde(default)]
    pub prefix: String,

    /// Optional regex applied to the extracted value. If the pattern has a
    /// capture group, the first group is returned; otherwise the full match.
    pub regex: Option<String>,

    /// Map raw extracted value → display label.
    /// Useful for turning attribute values (e.g. an SVG stroke color) into
    /// human-readable strings (e.g. "Official").
    #[serde(default)]
    pub value_map: HashMap<String, String>,

    /// Hard-coded value. When present, all selector/content extraction is skipped
    /// and this literal string is returned directly. Useful for provider-level
    /// constants like scanlator group names.
    pub static_value: Option<String>,

    /// strftime format string to parse this field as a date, converting the
    /// extracted string to a Unix timestamp (stored as a decimal string).
    /// Special value `"relative"` handles English relative dates
    /// ("3 days ago", "yesterday", "just now").
    /// When absent, the value is passed through as-is.
    pub date_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// All descendant text nodes concatenated.
    Text,
    /// Only direct child text nodes (excludes text inside descendant elements).
    OwnText,
    /// Read a named attribute from the element.
    Attr,
}

// ---------------------------------------------------------------------------
// HTTP fetch step
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FetchDef {
    /// URL to request. Supports template placeholders.
    pub url: String,
    /// HTTP method. Defaults to GET.
    #[serde(default = "default_method_get")]
    pub method: String,
    /// Request headers (key → value). Supports template placeholders in values.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body for POST/PUT. Supports template placeholders.
    #[serde(default)]
    pub body: Option<String>,
    /// Variable name to store the response body in.
    pub var: String,
    /// Optional JSON path to extract a specific value from the response.
    #[serde(default)]
    pub json_path: Option<String>,
    /// Optional pagination configuration for fetching multiple pages.
    #[serde(default)]
    pub pagination: Option<PaginationDef>,
}

fn default_method_get() -> String {
    "GET".to_string()
}

// ---------------------------------------------------------------------------
// GraphQL step (sugar over fetch)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct GraphqlDef {
    /// GraphQL endpoint URL.
    pub url: String,
    /// GraphQL query string.
    pub query: String,
    /// Variables for the query. Supports strings (with template placeholders),
    /// numbers, booleans, and nested objects/arrays.
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,
    /// Variable name to store the response in.
    pub var: String,
    /// Optional JSON path to extract from the response (e.g. "data.mangas").
    #[serde(default)]
    pub json_path: Option<String>,
    /// Request headers (key → value). Supports template placeholders in values.
    /// Content-Type: application/json is always included automatically.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Optional pagination configuration for fetching multiple pages.
    #[serde(default)]
    pub pagination: Option<PaginationDef>,
}

// ---------------------------------------------------------------------------
// From JSON step (direct JSON array to results)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FromJsonDef {
    /// Name of the variable containing the JSON array string.
    pub var: String,
    /// Map of output field name → JSON key to extract.
    /// Example: { title: "name", url: "id" }
    pub extract: HashMap<String, String>,
    /// Optional per-field prefixes (e.g. to prepend base_url to URLs).
    #[serde(default)]
    pub prefix: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Pagination configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PaginationDef {
    /// JSON path to pagination metadata in the response (e.g., "data.pagination", "meta").
    /// If not provided, falls back to counting items and stopping when empty.
    pub meta_path: Option<String>,
    /// Field in metadata containing current page number (default: "current_page").
    #[serde(default = "default_current_page_field")]
    pub current_page_field: String,
    /// Field in metadata containing last page number (default: "last_page").
    #[serde(default = "default_last_page_field")]
    pub last_page_field: String,
    /// Field in metadata containing total count (default: "total").
    #[serde(default = "default_total_field")]
    pub total_field: String,
    /// Field in metadata containing per-page count (default: "per_page").
    #[serde(default = "default_per_page_field")]
    pub per_page_field: String,
    /// Query parameter name for page number (default: "page").
    #[serde(default = "default_page_param")]
    pub page_param: String,
    /// Starting page number (default: 1).
    #[serde(default = "default_start_page")]
    pub start_page: u32,
    /// Maximum number of pages to fetch (default: 20, safety limit).
    #[serde(default = "default_max_pages")]
    pub max_pages: u32,
}

impl Default for PaginationDef {
    fn default() -> Self {
        Self {
            meta_path: None,
            current_page_field: default_current_page_field(),
            last_page_field: default_last_page_field(),
            total_field: default_total_field(),
            per_page_field: default_per_page_field(),
            page_param: default_page_param(),
            start_page: default_start_page(),
            max_pages: default_max_pages(),
        }
    }
}

fn default_current_page_field() -> String {
    "current_page".to_string()
}

fn default_last_page_field() -> String {
    "last_page".to_string()
}

fn default_total_field() -> String {
    "total".to_string()
}

fn default_per_page_field() -> String {
    "per_page".to_string()
}

fn default_page_param() -> String {
    "page".to_string()
}

fn default_start_page() -> u32 {
    1
}

fn default_max_pages() -> u32 {
    20
}
