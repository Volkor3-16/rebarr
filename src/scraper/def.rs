/// Deserialized representation of a provider YAML config file.
///
/// Users add providers by placing a `.yaml` file in the providers directory
/// (default: `./providers/`, or `REBARR_PROVIDERS_DIR`). No Rust code required.
use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDef {
    /// Display name (e.g. "WeebCentral").
    pub name: String,

    /// Root URL (e.g. "https://weebcentral.com"). Used as `{base_url}` in templates.
    pub base_url: String,

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
