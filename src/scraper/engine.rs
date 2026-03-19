use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};

static DUMP_COUNTER: AtomicU32 = AtomicU32::new(0);

use log::{debug, info, warn};
use scraper::{ElementRef, Html, Selector};

use crate::scraper::{
    def::{ActionDef, ContentKind, FieldDef, ForeachDef, InterceptDef, ProviderDef, StepDef},
    error::ScraperError,
    {PageUrl, Provider, ProviderChapterInfo, ProviderSearchResult, ScraperCtx},
};

/// Diagnostic data collected during a `foreach` step.
struct ForeachStats {
    element_count: usize,
    /// Per-field: (success_count, fail_count).
    field_counts: Vec<(String, usize, usize)>,
    first_record: Option<HashMap<String, String>>,
}

/// A scraping provider driven by a YAML `ProviderDef`.
pub struct YamlProvider {
    pub(crate) def: ProviderDef,
}

impl YamlProvider {
    pub fn new(def: ProviderDef) -> Self {
        Self { def }
    }

    // ------------------------------------------------------------------
    // Template expansion
    // ------------------------------------------------------------------

    /// Replace `{key}` placeholders. Relative paths get base_url prepended.
    fn expand(&self, template: &str, vars: &HashMap<String, String>) -> String {
        let mut s = template.replace("{base_url}", &self.def.base_url);
        for (k, v) in vars {
            s = s.replace(&format!("{{{k}}}"), v);
            // {var|strip_last_segment}: remove the last /segment from a URL path.
            let stripped = v
                .rfind('/')
                .filter(|&i| i > 0)
                .map_or(v.as_str(), |i| &v[..i]);
            s = s.replace(&format!("{{{k}|strip_last_segment}}"), stripped);
        }
        if s.starts_with('/') {
            format!("{}{}", self.def.base_url.trim_end_matches('/'), s)
        } else {
            s
        }
    }

    // ------------------------------------------------------------------
    // Field extraction (used inside foreach)
    // ------------------------------------------------------------------

    fn extract_field(
        &self,
        element: &ElementRef,
        field: &FieldDef,
    ) -> Result<String, ScraperError> {
        if let Some(ref v) = field.static_value {
            return Ok(v.clone());
        }

        let child = if field.selector.is_empty() {
            *element
        } else {
            let sel = Selector::parse(&field.selector).map_err(|e| {
                ScraperError::Parse(format!("bad selector '{}': {e:?}", field.selector))
            })?;
            element.select(&sel).next().ok_or_else(|| {
                ScraperError::Parse(format!("selector '{}' matched nothing", field.selector))
            })?
        };

        let content = field.content.as_ref().ok_or_else(|| {
            ScraperError::Parse(format!(
                "field with selector '{}' has no 'content'",
                field.selector
            ))
        })?;
        let raw = match content {
            ContentKind::Text => child.text().collect::<String>().trim().to_owned(),
            ContentKind::OwnText => {
                use scraper::node::Node;
                child
                    .children()
                    .filter_map(|n| match n.value() {
                        Node::Text(t) => Some(t.to_string()),
                        _ => None,
                    })
                    .collect::<String>()
                    .trim()
                    .to_owned()
            }
            ContentKind::Attr => {
                let attr_name = field.attr_name.as_deref().ok_or_else(|| {
                    ScraperError::Parse(format!(
                        "field with selector '{}' uses content: attr but has no attr_name",
                        field.selector
                    ))
                })?;
                child
                    .value()
                    .attr(attr_name)
                    .ok_or_else(|| ScraperError::Parse(format!("attr '{attr_name}' not found")))?
                    .to_owned()
            }
        };

        let raw = field.value_map.get(&raw).cloned().unwrap_or(raw);

        let raw = if let Some(ref pattern) = field.regex {
            let re = regex::Regex::new(pattern)
                .map_err(|e| ScraperError::Parse(format!("bad regex '{pattern}': {e}")))?;
            match re.captures(&raw) {
                Some(caps) => caps
                    .get(1)
                    .or_else(|| caps.get(0))
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_default(),
                None => raw,
            }
        } else {
            raw
        };

        // If this field has a date_format, parse the date and return a Unix timestamp string.
        if let Some(ref fmt) = field.date_format {
            return match parse_date(&raw, fmt) {
                Some(ts) => Ok(ts.to_string()),
                None => Err(ScraperError::Parse(format!(
                    "date '{raw}' did not match format '{fmt}'"
                ))),
            };
        }

        if raw.starts_with("http://") || raw.starts_with("https://") {
            return Ok(raw);
        }
        let prefix = field.prefix.replace("{base_url}", &self.def.base_url);
        Ok(format!("{prefix}{raw}"))
    }

    // ------------------------------------------------------------------
    // Step execution engine
    // ------------------------------------------------------------------

    /// Run the action, transparently restarting Chromium once if the CDP
    /// transport has died (WebSocket gone, reader thread exited, etc.).
    async fn execute_action(
        &self,
        ctx: &ScraperCtx,
        action: &ActionDef,
        input_vars: HashMap<String, String>,
    ) -> Result<ActionResult, ScraperError> {
        match self.run_action(ctx, action, input_vars.clone()).await {
            Err(ref e) if is_transport_error(e) => {
                warn!(
                    "provider '{}': CDP transport error — resetting browser and retrying: {e}",
                    self.def.name
                );
                ctx.browser.reset().await;
                self.run_action(ctx, action, input_vars).await
            }
            other => other,
        }
    }

    async fn run_action(
        &self,
        ctx: &ScraperCtx,
        action: &ActionDef,
        input_vars: HashMap<String, String>,
    ) -> Result<ActionResult, ScraperError> {
        let browser = ctx.browser.get().await?;

        // Lazily create the browser page on the first `open` step.
        let mut page: Option<eoka::Page> = None;
        let mut vars = input_vars;
        vars.insert("base_url".to_owned(), self.def.base_url.clone());

        let mut results: Vec<HashMap<String, String>> = Vec::new();
        // Intercept configs registered before any `open` step.
        let mut pending_intercepts: Vec<InterceptDef> = Vec::new();
        let mut early_return: Option<String> = None;

        for step in &action.steps {
            match step {
                StepDef::Open { open: url_tmpl } => {
                    let url = self.expand(url_tmpl, &vars);
                    debug!("open: {url}");

                    if let Some(ref p) = page {
                        // Subsequent navigation on the same page.
                        p.goto(url.as_str())
                            .await
                            .map_err(|e| ScraperError::Browser(e.to_string()))?;
                        // Post-nav: inject any pending intercepts.
                        for intercept in &pending_intercepts {
                            inject_intercept(p, &intercept.url_contains).await;
                        }
                    } else {
                        // eoka injects 15 stealth evasion scripts automatically on page creation.
                        let new_page = browser
                            .new_blank_page()
                            .await
                            .map_err(|e| ScraperError::Browser(e.to_string()))?;
                        new_page
                            .goto(url.as_str())
                            .await
                            .map_err(|e| ScraperError::Browser(e.to_string()))?;
                        // Post-nav: inject any pending intercepts.
                        for intercept in &pending_intercepts {
                            inject_intercept(&new_page, &intercept.url_contains).await;
                        }
                        page = Some(new_page);
                    }

                    let p = page.as_ref().unwrap();

                    // Wait for page body to load.
                    p.wait_for("body", 10_000)
                        .await
                        .map_err(|e| ScraperError::Timeout(format!("body did not appear: {e}")))?;
                    // Wait for any XHR/fetch requests triggered by page JS to settle.
                    // Ignored on timeout — some pages never reach true network idle.
                    // ZAC NOTE: this sucks
                    p.wait_for_network_idle(1000, 30_000).await.ok();

                    // Create screenshot of website (for debugging)
                    if ctx.dump_html {
                        let png = p.screenshot().await.unwrap();
                        std::fs::write("screenshot.png", png)?;
                    }

                    // Dump HTML to file if requested (for debugging).
                    if ctx.dump_html {
                        if let Ok(html) = p.content().await {
                            let n = DUMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                            let fname = format!("scraper_dump_{n}.html");
                            if let Err(e) = std::fs::write(&fname, html.as_bytes()) {
                                warn!("dump_html: failed to write {fname}: {e}");
                            } else {
                                info!("dump_html: wrote {fname} ({} bytes)", html.len());
                            }
                        }
                    }

                    // Resolve all pending intercept captures (post-navigation).
                    let intercepts = std::mem::take(&mut pending_intercepts);
                    for intercept in intercepts {
                        match poll_capture(p, &intercept.url_contains).await {
                            Some(body) => {
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] intercept captured {} bytes for '{}'",
                                        body.len(),
                                        intercept.url_contains
                                    );
                                }
                                let val = parse_capture_body(&body, intercept.json_path.as_deref());
                                vars.insert(intercept.var.clone(), val);
                            }
                            None => {
                                warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name, intercept.url_contains
                                );
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] intercept TIMEOUT for '{}'",
                                        intercept.url_contains
                                    );
                                }
                            }
                        }
                    }
                }

                StepDef::WaitFor { wait_for: selector } => {
                    let sel = self.expand(selector, &vars);
                    if ctx.verbose {
                        eprintln!("[step] wait_for → {sel}");
                    }
                    let p = require_page(&page, "wait_for")?;
                    match p.wait_for(&sel, 10_000).await {
                        Ok(_) => {
                            if ctx.verbose {
                                eprintln!("[step] wait_for '{sel}' → found");
                            }
                        }
                        Err(e) => {
                            if ctx.verbose {
                                eprintln!("[step] wait_for '{sel}' → TIMEOUT");
                            }
                            return Err(ScraperError::Timeout(format!("wait_for '{sel}': {e}")));
                        }
                    }
                }

                StepDef::Click { click: selector } => {
                    let sel = self.expand(selector, &vars);
                    if ctx.verbose {
                        eprintln!("[step] click → {sel}");
                    }
                    let p = require_page(&page, "click")?;
                    p.human_click(&sel)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("click '{sel}': {e}")))?;
                }

                StepDef::Type { type_def } => {
                    let selector = self.expand(&type_def.selector, &vars);
                    let value = self.expand(&type_def.value, &vars);
                    if ctx.verbose {
                        eprintln!("[step] type → {selector}");
                    }
                    let p = require_page(&page, "type")?;
                    p.human_type(&selector, &value)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("type '{selector}': {e}")))?;
                }

                StepDef::Sleep { sleep: ms } => {
                    if ctx.verbose {
                        eprintln!("[step] sleep → {ms}ms");
                    }
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                }

                StepDef::Script { script: js_tmpl } => {
                    let js = self.expand(js_tmpl, &vars);
                    if ctx.verbose {
                        let preview = &js[..js.len().min(80)];
                        eprintln!("[step] script → {preview}…");
                    }
                    let p = require_page(&page, "script")?;
                    let _ = p.execute(&js).await;
                }

                StepDef::ExtractJs { extract_js: def } => {
                    let script = self.expand(&def.script, &vars);
                    if ctx.verbose {
                        eprintln!("[step] extract_js → var={}", def.var);
                    }
                    let p = require_page(&page, "extract_js")?;
                    match p.evaluate::<serde_json::Value>(&script).await {
                        Ok(v) => {
                            let s = match v {
                                serde_json::Value::String(s) => s,
                                other => other.to_string(),
                            };
                            if ctx.verbose {
                                let preview = &s[..s.len().min(120)];
                                eprintln!("[step] extract_js '{}' = {preview}", def.var);
                            }
                            vars.insert(def.var.clone(), s);
                        }
                        Err(e) => {
                            warn!(
                                "provider '{}': extract_js '{}' failed: {e}",
                                self.def.name, def.var
                            );
                            if ctx.verbose {
                                eprintln!("[step] extract_js '{}' → FAILED: {e}", def.var);
                            }
                        }
                    }
                }

                StepDef::Intercept {
                    intercept: intercept_def,
                } => {
                    if ctx.verbose {
                        eprintln!(
                            "[step] intercept → url_contains='{}', var='{}'",
                            intercept_def.url_contains, intercept_def.var
                        );
                    }
                    if let Some(ref p) = page {
                        // Page already open: inject immediately and poll.
                        inject_intercept(p, &intercept_def.url_contains).await;
                        match poll_capture(p, &intercept_def.url_contains).await {
                            Some(body) => {
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] intercept captured {} bytes for '{}'",
                                        body.len(),
                                        intercept_def.url_contains
                                    );
                                }
                                let val =
                                    parse_capture_body(&body, intercept_def.json_path.as_deref());
                                vars.insert(intercept_def.var.clone(), val);
                            }
                            None => {
                                warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name, intercept_def.url_contains
                                );
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] intercept TIMEOUT for '{}'",
                                        intercept_def.url_contains
                                    );
                                }
                            }
                        }
                    } else {
                        // No page yet: defer until the next `open`.
                        if ctx.verbose {
                            eprintln!(
                                "[step] intercept deferred (no page yet) for '{}'",
                                intercept_def.url_contains
                            );
                        }
                        pending_intercepts.push(intercept_def.clone());
                    }
                }

                StepDef::Foreach {
                    foreach: foreach_def,
                } => {
                    if ctx.verbose {
                        eprintln!("[step] foreach → selector='{}'", foreach_def.selector);
                    }
                    let p = require_page(&page, "foreach")?;
                    let html = p
                        .content()
                        .await
                        .map_err(|e| ScraperError::Browser(e.to_string()))?;
                    let stats = self.collect_foreach_results(&html, foreach_def, &mut results)?;
                    if ctx.verbose {
                        eprintln!(
                            "[step] foreach → {} elements matched '{}'",
                            stats.element_count, foreach_def.selector
                        );
                        for (field, ok, fail) in &stats.field_counts {
                            eprintln!("         field '{field}': {ok} extracted, {fail} failed");
                        }
                        if let Some(ref first) = stats.first_record {
                            eprintln!("         sample (first match):");
                            let mut keys: Vec<&String> = first.keys().collect();
                            keys.sort();
                            for k in keys {
                                let v = &first[k];
                                let preview = &v[..v.len().min(100)];
                                eprintln!("           {k} = {preview}");
                            }
                        }
                    }
                }

                StepDef::Return { value: tmpl } => {
                    let val = self.expand(tmpl, &vars);
                    if ctx.verbose {
                        let count_hint = if val.starts_with('[') {
                            serde_json::from_str::<serde_json::Value>(&val)
                                .ok()
                                .and_then(|v| v.as_array().map(|a| a.len()))
                                .map(|n| format!(" ({n} URLs in array)"))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let preview = &val[..val.len().min(120)];
                        eprintln!("[step] return → {preview}{count_hint}");
                    }
                    early_return = Some(val);
                    break;
                }

                StepDef::Scroll { scroll: target } => {
                    if ctx.verbose {
                        eprintln!("[step] scroll → {target}");
                    }
                    let p = require_page(&page, "scroll")?;
                    let js = if target == "bottom" {
                        "window.scrollTo(0, document.body.scrollHeight)".to_owned()
                    } else {
                        let safe = js_escape(target);
                        format!("document.querySelector('{safe}')?.scrollIntoView()")
                    };
                    let _ = p.execute(&js).await;
                }
            }
        }

        // Page closes when dropped.
        drop(page);

        if let Some(val) = early_return {
            Ok(ActionResult::Value(val))
        } else {
            Ok(ActionResult::Records(results))
        }
    }

    fn collect_foreach_results(
        &self,
        html: &str,
        foreach_def: &ForeachDef,
        results: &mut Vec<HashMap<String, String>>,
    ) -> Result<ForeachStats, ScraperError> {
        let doc = Html::parse_document(html);
        let sel = Selector::parse(&foreach_def.selector)
            .map_err(|e| ScraperError::Parse(format!("bad foreach selector: {e:?}")))?;

        // Pre-allocate per-field counters in a stable order.
        let mut field_counts: Vec<(String, usize, usize)> = foreach_def
            .extract
            .keys()
            .map(|k| (k.clone(), 0, 0))
            .collect();
        field_counts.sort_by(|a, b| a.0.cmp(&b.0));

        let mut element_count = 0usize;
        let mut first_record: Option<HashMap<String, String>> = None;

        for element in doc.select(&sel) {
            element_count += 1;
            let mut record: HashMap<String, String> = HashMap::new();
            for (name, field_def) in &foreach_def.extract {
                match self.extract_field(&element, field_def) {
                    Ok(val) => {
                        record.insert(name.clone(), val);
                        if let Some(c) = field_counts.iter_mut().find(|c| c.0 == *name) {
                            c.1 += 1;
                        }
                    }
                    Err(_) => {
                        if let Some(c) = field_counts.iter_mut().find(|c| c.0 == *name) {
                            c.2 += 1;
                        }
                    }
                }
            }
            if !record.is_empty() {
                if first_record.is_none() {
                    first_record = Some(record.clone());
                }
                results.push(record);
            }
        }

        Ok(ForeachStats {
            element_count,
            field_counts,
            first_record,
        })
    }
}

// ---------------------------------------------------------------------------
// Provider trait implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Provider for YamlProvider {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn needs_browser(&self) -> bool {
        true
    }

    fn rate_limit_rpm(&self) -> u32 {
        self.def.rate_limit.requests_per_minute
    }

    fn page_delay_ms(&self) -> u64 {
        self.def.rate_limit.page_delay_ms
    }

    async fn search(
        &self,
        ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        let def = self.def.search.as_ref().ok_or(ScraperError::Unsupported)?;
        let encoded = urlencoding::encode(title).into_owned();
        let mut input = HashMap::new();
        input.insert("query".to_owned(), encoded);

        let result = self.execute_action(ctx, def, input).await?;
        Ok(records_to_search_results(result.into_records()))
    }

    async fn chapters(
        &self,
        ctx: &ScraperCtx,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        let def = self
            .def
            .chapters
            .as_ref()
            .ok_or(ScraperError::Unsupported)?;
        let mut input = HashMap::new();
        input.insert("manga_url".to_owned(), manga_url.to_owned());

        let result = self.execute_action(ctx, def, input).await?;
        Ok(records_to_chapters(result.into_records()))
    }

    async fn pages(
        &self,
        ctx: &ScraperCtx,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        let def = self.def.pages.as_ref().ok_or(ScraperError::Unsupported)?;
        let mut input = HashMap::new();
        input.insert("chapter_url".to_owned(), chapter_url.to_owned());

        let result = self.execute_action(ctx, def, input).await?;
        result_to_pages(result)
    }
}

// ---------------------------------------------------------------------------
// ActionResult
// ---------------------------------------------------------------------------

enum ActionResult {
    Records(Vec<HashMap<String, String>>),
    Value(String),
}

impl ActionResult {
    fn into_records(self) -> Vec<HashMap<String, String>> {
        match self {
            ActionResult::Records(r) => r,
            ActionResult::Value(_) => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Record → output type conversions
// ---------------------------------------------------------------------------

fn records_to_search_results(records: Vec<HashMap<String, String>>) -> Vec<ProviderSearchResult> {
    records
        .into_iter()
        .filter_map(|mut r| {
            Some(ProviderSearchResult {
                title: r.remove("title")?,
                url: r.remove("url")?,
                cover_url: r.remove("cover"),
            })
        })
        .collect()
}

/// Parse a raw chapter number string into (number_sort, chapter_base, chapter_variant).
///
/// Handles:
/// - Plain numbers: "12" → (12.0, 12.0, 0)
/// - Decimal splits: "12.1" → (12.1, 12.0, 1)
/// - Higher decimals: "12.5" → (12.5, 12.0, 5)  [whether extra is determined by title, not number]
/// - Letter suffixes: "12a" → (12.1, 12.0, 1), "12b" → (12.2, 12.0, 2)
/// - Prefixed: "Ch. 12.5" → takes last whitespace token before applying rules
/// - Fallback: returns (0.0, 0.0, 0) instead of silently losing chapters
fn parse_chapter_number(raw: &str) -> (f32, f32, u8) {
    // Take the last whitespace-separated token (strips "Ch.", "Chapter", "Vol.X Ch.Y" prefixes)
    let token = raw.split_whitespace().last().unwrap_or(raw).trim();

    // Try direct f32 parse first ("12", "12.5", "12.1")
    if let Ok(n) = token.parse::<f32>() {
        let base = n.floor();
        let frac = (n - base).abs();
        let variant = (frac * 10.0).round() as u8;
        return (n, base, variant);
    }

    // Try letter suffix pattern: digits followed by a single lowercase letter ("12a", "12b")
    if let Some(letter_pos) = token.rfind(|c: char| c.is_ascii_alphabetic()) {
        let (num_part, letter_part) = token.split_at(letter_pos);
        if let Ok(base) = num_part.parse::<f32>() {
            if let Some(letter) = letter_part.chars().next() {
                if letter.is_ascii_alphabetic() {
                    // a=1, b=2, c=3, ...
                    let variant = (letter.to_ascii_lowercase() as u8) - b'a' + 1;
                    let number = base + (variant as f32) / 10.0;
                    return (number, base, variant);
                }
            }
        }
    }

    // Fallback: could not parse — return 0 so the chapter still appears (not silently dropped)
    (0.0, 0.0, 0)
}

/// Returns true if the title is just a chapter number restatement (e.g. "Chapter 5", "Ch. 14.5").
/// "Chapter of the Dragon" → false. "Chapter 14" → true.
fn is_fake_chapter_title(title: &str) -> bool {
    let lower = title.trim().to_ascii_lowercase();
    let rest = if lower.starts_with("chapter ") {
        &lower["chapter ".len()..]
    } else if lower.starts_with("ch. ") {
        &lower["ch. ".len()..]
    } else if lower.starts_with("ch ") {
        &lower["ch ".len()..]
    } else {
        return false;
    };
    rest.trim().parse::<f64>().is_ok()
}

/// Infer whether a chapter is an extra/bonus from its title using keyword matching.
fn infer_is_extra(title: Option<&str>) -> bool {
    let Some(t) = title else { return false };
    let lower = t.to_lowercase();
    const KEYWORDS: &[&str] = &[
        "extra",
        "omake",
        "special",
        "bonus",
        "side story",
        "side chapter",
        "interlude",
        "gaiden",
    ];
    KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Parse a date string using an explicit strftime format, or `"relative"` for English
/// relative dates ("3 days ago", "yesterday", "just now").
///
/// Ordinal suffixes are stripped automatically before parsing so formats like
/// `%B %d %Y` work on inputs like "December 25th 2023".
///
/// Returns a Unix timestamp (seconds since epoch) or `None` if parsing fails.
fn parse_date(raw: &str, format: &str) -> Option<i64> {
    // Strip ordinal suffixes: "25th" → "25", "1st" → "1", etc.
    let stripped = regex::Regex::new(r"(\d+)(st|nd|rd|th)\b")
        .ok()?
        .replace_all(raw.trim(), "$1")
        .into_owned();
    let s = stripped.trim();

    if format == "relative" {
        let lower = s.to_lowercase();
        let now = Utc::now();

        if lower == "just now" || lower == "today" {
            return Some(now.timestamp());
        }
        if lower == "yesterday" {
            return Some((now - chrono::Duration::days(1)).timestamp());
        }

        let re = regex::Regex::new(r"(\d+)\s*(minute|hour|day|week|month|year)s?").ok()?;
        if let Some(caps) = re.captures(&lower) {
            let n: i64 = caps[1].parse().ok()?;
            let dt = match &caps[2] {
                "minute" => now - chrono::Duration::minutes(n),
                "hour" => now - chrono::Duration::hours(n),
                "day" => now - chrono::Duration::days(n),
                "week" => now - chrono::Duration::weeks(n),
                "month" => now - chrono::Duration::days(n * 30),
                "year" => now - chrono::Duration::days(n * 365),
                _ => return None,
            };
            return Some(dt.timestamp());
        }
        return None;
    }

    // Try NaiveDateTime first (has time component), then NaiveDate (midnight UTC).
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, format) {
        return Some(Utc.from_utc_datetime(&dt).timestamp());
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, format) {
        return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?).timestamp());
    }

    // Try with the current year appended for year-less formats like "%B %d".
    let with_year = format!("{s} {}", Utc::now().year());
    let format_with_year = format!("{format} %Y");
    if let Ok(d) = NaiveDate::parse_from_str(&with_year, &format_with_year) {
        return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?).timestamp());
    }

    None
}

fn records_to_chapters(records: Vec<HashMap<String, String>>) -> Vec<ProviderChapterInfo> {
    let mut chapters: Vec<ProviderChapterInfo> = records
        .into_iter()
        .filter_map(|mut r| {
            let raw_number = r.remove("number_raw")?;
            let (number, chapter_base, chapter_variant) = parse_chapter_number(&raw_number);
            let title = r
                .remove("title")
                .filter(|s| !s.is_empty())
                .filter(|s| !is_fake_chapter_title(s));
            let is_extra = infer_is_extra(title.as_deref());
            Some(ProviderChapterInfo {
                raw_number,
                number,
                chapter_base,
                chapter_variant,
                is_extra,
                title,
                url: r.remove("url").filter(|s| !s.is_empty()),
                volume: r.remove("volume").and_then(|s| s.parse().ok()),
                scanlator_group: r.remove("scanlator_group").filter(|s| !s.is_empty()),
                language: r.remove("language").filter(|s| !s.is_empty()),
                date_released: r
                    .remove("date")
                    .filter(|s| !s.is_empty())
                    .and_then(|s| s.parse::<i64>().ok()),
            })
        })
        .collect();
    chapters.sort_by(|a, b| {
        a.number
            .partial_cmp(&b.number)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    chapters
}

fn result_to_pages(result: ActionResult) -> Result<Vec<PageUrl>, ScraperError> {
    match result {
        ActionResult::Records(records) => Ok(records
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut r)| {
                r.remove("url").map(|url| PageUrl {
                    url,
                    index: (i + 1) as u32,
                })
            })
            .collect()),
        ActionResult::Value(s) => {
            let arr: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| ScraperError::Parse(format!("return value is not valid JSON: {e}")))?;
            match arr {
                serde_json::Value::Array(items) => Ok(items
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        v.as_str().map(|u| PageUrl {
                            url: u.to_owned(),
                            index: (i + 1) as u32,
                        })
                    })
                    .collect()),
                _ => Err(ScraperError::Parse(
                    "return value for pages must be a JSON array of URL strings".to_owned(),
                )),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transport-error detection
// ---------------------------------------------------------------------------

/// Return true when `e` looks like a dead CDP WebSocket rather than a
/// recoverable page error. These errors require restarting Chromium.
fn is_transport_error(e: &ScraperError) -> bool {
    let ScraperError::Browser(msg) = e else {
        return false;
    };
    msg.contains("Transport error")
        || msg.contains("reader thread has exited")
        || msg.contains("WebSocket")
        || msg.contains("connection reset")
        || msg.contains("broken pipe")
}

// ---------------------------------------------------------------------------
// Browser helpers
// ---------------------------------------------------------------------------

/// Return a reference to the page, or an error if it has not been opened yet.
fn require_page<'a>(
    page: &'a Option<eoka::Page>,
    step: &str,
) -> Result<&'a eoka::Page, ScraperError> {
    page.as_ref().ok_or_else(|| {
        ScraperError::Parse(format!(
            "step '{step}' used before any 'open' step — no page available"
        ))
    })
}

/// Build the JS monkey-patch that intercepts fetch + XHR matching `url_fragment`.
fn build_intercept_js(url_fragment: &str) -> String {
    let safe = url_fragment.replace('\'', "\\'");
    format!(
        r#"(function(){{
            window.__rebarr_captures = window.__rebarr_captures || {{}};
            var _key = '{safe}';
            var _match = '{safe}';
            // Patch fetch
            var _fetch = window.fetch;
            window.fetch = function() {{
                var args = arguments;
                var url = typeof args[0] === 'string' ? args[0]
                          : (args[0] && args[0].url ? args[0].url : '');
                return _fetch.apply(this, args).then(function(resp) {{
                    if (url.indexOf(_match) !== -1 && !window.__rebarr_captures[_key]) {{
                        resp.clone().text().then(function(t) {{
                            window.__rebarr_captures[_key] = t;
                        }});
                    }}
                    return resp;
                }});
            }};
            // Patch XMLHttpRequest
            var _open = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {{
                if (typeof url === 'string' && url.indexOf(_match) !== -1) {{
                    this.addEventListener('load', function() {{
                        if (!window.__rebarr_captures[_key]) {{
                            window.__rebarr_captures[_key] = this.responseText;
                        }}
                    }});
                }}
                return _open.apply(this, arguments);
            }};
        }})();"#
    )
}

/// Inject the monkey-patch via execute (post-navigation injection).
async fn inject_intercept(page: &eoka::Page, url_fragment: &str) {
    let js = build_intercept_js(url_fragment);
    let _ = page.execute(&js).await;
}

/// Poll for a captured response in `window.__rebarr_captures[url_fragment]`.
/// Returns `Some(body)` or `None` on timeout (10 s).
async fn poll_capture(page: &eoka::Page, url_fragment: &str) -> Option<String> {
    let safe = url_fragment.replace('\'', "\\'");
    let js = format!("(window.__rebarr_captures && window.__rebarr_captures['{safe}']) || null");
    for _ in 0..20u32 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(Some(s)) = page.evaluate::<Option<String>>(&js).await {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Parse an intercepted response body, optionally navigating a JSON path.
fn parse_capture_body(body: &str, json_path: Option<&str>) -> String {
    if let Some(path) = json_path {
        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
            for key in path.split('.') {
                json = json[key].take();
            }
            return match json {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
        }
    }
    body.to_owned()
}

/// Escape a string for safe embedding in a JS single-quoted string literal.
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
