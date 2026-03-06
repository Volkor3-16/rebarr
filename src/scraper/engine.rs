use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static DUMP_COUNTER: AtomicU32 = AtomicU32::new(0);

use scraper::{ElementRef, Html, Selector};

use crate::scraper::{
    def::{ActionDef, ContentKind, FieldDef, ForeachDef, InterceptDef, ProviderDef, StepDef},
    error::ScraperError,
    {PageUrl, Provider, ProviderChapterInfo, ProviderSearchResult, ScraperCtx},
};

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
            let stripped = v.rfind('/').filter(|&i| i > 0).map_or(v.as_str(), |i| &v[..i]);
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

    fn extract_field(&self, element: &ElementRef, field: &FieldDef) -> Result<String, ScraperError> {
        if let Some(ref v) = field.static_value {
            return Ok(v.clone());
        }

        let child = if field.selector.is_empty() {
            *element
        } else {
            let sel = Selector::parse(&field.selector)
                .map_err(|e| ScraperError::Parse(format!("bad selector '{}': {e:?}", field.selector)))?;
            element
                .select(&sel)
                .next()
                .ok_or_else(|| ScraperError::Parse(format!("selector '{}' matched nothing", field.selector)))?
        };

        let content = field.content.as_ref().ok_or_else(|| {
            ScraperError::Parse(format!("field with selector '{}' has no 'content'", field.selector))
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

        if raw.starts_with("http://") || raw.starts_with("https://") {
            return Ok(raw);
        }
        let prefix = field.prefix.replace("{base_url}", &self.def.base_url);
        Ok(format!("{prefix}{raw}"))
    }

    // ------------------------------------------------------------------
    // Step execution engine
    // ------------------------------------------------------------------

    async fn execute_action(
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
                    log::info!("open: {url}");

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
                    p.wait_for_network_idle(5000, 30_000).await.ok();

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
                                log::warn!("dump_html: failed to write {fname}: {e}");
                            } else {
                                log::info!("dump_html: wrote {fname} ({} bytes)", html.len());
                            }
                        }
                    }

                    // Resolve all pending intercept captures (post-navigation).
                    let intercepts = std::mem::take(&mut pending_intercepts);
                    for intercept in intercepts {
                        match poll_capture(p, &intercept.url_contains).await {
                            Some(body) => {
                                let val = parse_capture_body(&body, intercept.json_path.as_deref());
                                vars.insert(intercept.var.clone(), val);
                            }
                            None => {
                                log::warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name,
                                    intercept.url_contains
                                );
                            }
                        }
                    }
                }

                StepDef::WaitFor { wait_for: selector } => {
                    let sel = self.expand(selector, &vars);
                    let p = require_page(&page, "wait_for")?;
                    p.wait_for(&sel, 10_000)
                        .await
                        .map_err(|e| ScraperError::Timeout(format!("wait_for '{sel}': {e}")))?;
                }

                StepDef::Click { click: selector } => {
                    let sel = self.expand(selector, &vars);
                    let p = require_page(&page, "click")?;
                    p.human_click(&sel)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("click '{sel}': {e}")))?;
                }

                StepDef::Type { type_def } => {
                    let selector = self.expand(&type_def.selector, &vars);
                    let value = self.expand(&type_def.value, &vars);
                    let p = require_page(&page, "type")?;
                    p.human_type(&selector, &value)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("type '{selector}': {e}")))?;
                }

                StepDef::Sleep { sleep: ms } => {
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                }

                StepDef::Script { script: js_tmpl } => {
                    let js = self.expand(js_tmpl, &vars);
                    let p = require_page(&page, "script")?;
                    let _ = p.execute(&js).await;
                }

                StepDef::ExtractJs { extract_js: def } => {
                    let script = self.expand(&def.script, &vars);
                    let p = require_page(&page, "extract_js")?;
                    match p.evaluate::<serde_json::Value>(&script).await {
                        Ok(v) => {
                            let s = match v {
                                serde_json::Value::String(s) => s,
                                other => other.to_string(),
                            };
                            vars.insert(def.var.clone(), s);
                        }
                        Err(e) => {
                            log::warn!(
                                "provider '{}': extract_js '{}' failed: {e}",
                                self.def.name,
                                def.var
                            );
                        }
                    }
                }

                StepDef::Intercept { intercept: intercept_def } => {
                    if let Some(ref p) = page {
                        // Page already open: inject immediately and poll.
                        inject_intercept(p, &intercept_def.url_contains).await;
                        match poll_capture(p, &intercept_def.url_contains).await {
                            Some(body) => {
                                let val = parse_capture_body(&body, intercept_def.json_path.as_deref());
                                vars.insert(intercept_def.var.clone(), val);
                            }
                            None => {
                                log::warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name,
                                    intercept_def.url_contains
                                );
                            }
                        }
                    } else {
                        // No page yet: defer until the next `open`.
                        pending_intercepts.push(intercept_def.clone());
                    }
                }

                StepDef::Foreach { foreach: foreach_def } => {
                    let p = require_page(&page, "foreach")?;
                    let html = p
                        .content()
                        .await
                        .map_err(|e| ScraperError::Browser(e.to_string()))?;
                    self.collect_foreach_results(&html, foreach_def, &mut results)?;
                }

                StepDef::Return { value: tmpl } => {
                    early_return = Some(self.expand(tmpl, &vars));
                    break;
                }

                StepDef::Scroll { scroll: target } => {
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
    ) -> Result<(), ScraperError> {
        let doc = Html::parse_document(html);
        let sel = Selector::parse(&foreach_def.selector)
            .map_err(|e| ScraperError::Parse(format!("bad foreach selector: {e:?}")))?;

        for element in doc.select(&sel) {
            let mut record: HashMap<String, String> = HashMap::new();
            for (name, field_def) in &foreach_def.extract {
                if let Ok(val) = self.extract_field(&element, field_def) {
                    record.insert(name.clone(), val);
                }
            }
            if !record.is_empty() {
                results.push(record);
            }
        }
        Ok(())
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

    fn score(&self) -> u8 {
        self.def.score
    }

    fn needs_browser(&self) -> bool {
        true
    }

    fn rate_limit_rpm(&self) -> u32 {
        self.def.rate_limit.requests_per_minute
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
        let def = self.def.chapters.as_ref().ok_or(ScraperError::Unsupported)?;
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

fn records_to_chapters(records: Vec<HashMap<String, String>>) -> Vec<ProviderChapterInfo> {
    let mut chapters: Vec<ProviderChapterInfo> = records
        .into_iter()
        .filter_map(|mut r| {
            let raw_number = r.remove("number_raw")?;
            let number = raw_number
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            Some(ProviderChapterInfo {
                raw_number,
                number,
                title: r.remove("title").filter(|s| !s.is_empty()),
                url: r.remove("url").filter(|s| !s.is_empty()),
                volume: r.remove("volume").and_then(|s| s.parse().ok()),
                scanlator_group: r.remove("scanlator_group").filter(|s| !s.is_empty()),
            })
        })
        .collect();
    chapters.sort_by(|a, b| a.number.partial_cmp(&b.number).unwrap_or(std::cmp::Ordering::Equal));
    chapters
}

fn result_to_pages(result: ActionResult) -> Result<Vec<PageUrl>, ScraperError> {
    match result {
        ActionResult::Records(records) => Ok(records
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut r)| {
                r.remove("url").map(|url| PageUrl { url, index: (i + 1) as u32 })
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
                        v.as_str().map(|u| PageUrl { url: u.to_owned(), index: (i + 1) as u32 })
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
// Browser helpers
// ---------------------------------------------------------------------------

/// Return a reference to the page, or an error if it has not been opened yet.
fn require_page<'a>(page: &'a Option<eoka::Page>, step: &str) -> Result<&'a eoka::Page, ScraperError> {
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
