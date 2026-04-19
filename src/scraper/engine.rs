use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

static DUMP_COUNTER: AtomicU32 = AtomicU32::new(0);

use scraper::{ElementRef, Html, Selector};
use tracing::{debug, info, warn};

use crate::scraper::{
    browser::close_page_tab,
    def::{ActionDef, ContentKind, FieldDef, ForeachDef, InterceptDef, ProviderDef, StepDef},
    error::ScraperError,
    {
        PageUrl, Provider, ProviderChapterInfo, ProviderSearchResult, ProviderVariables,
        ScraperCtx, ScraperDebugLevel,
    },
};

/// Diagnostic data collected during a `foreach` step.
struct ForeachStats {
    element_count: usize,
    /// Per-field: (success_count, fail_count).
    field_counts: Vec<(String, usize, usize)>,
    first_record: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct BrowserFetchEnvelope {
    ok: bool,
    status: u16,
    status_text: String,
    url: String,
    headers: HashMap<String, String>,
    body: String,
    error: Option<String>,
}

#[derive(Debug)]
struct JsonPathSuccess {
    value: String,
    kind: &'static str,
}

#[derive(Debug)]
enum JsonPathError {
    InvalidJson(String),
    MissingPath {
        path: String,
        missing_segment: String,
        container_kind: &'static str,
    },
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

    /// Recursively expand `{key}` placeholders in all string values within a JSON value.
    fn expand_json_value(
        &self,
        value: &serde_json::Value,
        vars: &HashMap<String, String>,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => serde_json::Value::String(self.expand(s, vars)),
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.expand_json_value(v, vars)))
                    .collect(),
            ),
            serde_json::Value::Array(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|v| self.expand_json_value(v, vars))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Replace `{key}` placeholders. Relative paths get base_url prepended.
    fn expand(&self, template: &str, vars: &HashMap<String, String>) -> String {
        let mut s = template.replace("{base_url}", &self.def.base_url);

        // Process all {var} placeholders first (no modifiers)
        for (k, v) in vars {
            s = s.replace(&format!("{{{k}}}"), v);
        }

        // Process modifiers: {var|modifier1|modifier2|...}
        // Capture all patterns and replace them iteratively until none remain
        let re = regex::Regex::new(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\|([^}]+)\}").unwrap();
        let mut changed = true;
        while changed {
            changed = false;
            // Find patterns like {varname|modifier}
            while let Some(caps) = re.captures(&s) {
                let full_match = caps.get(0).unwrap().as_str();
                let var_name = caps.get(1).unwrap().as_str();
                let modifiers = caps.get(2).unwrap().as_str();

                // Get base value
                let base_val = vars.get(var_name).cloned().unwrap_or_default();

                // Apply modifiers in order
                let mut result = base_val;
                for mod_name in modifiers.split('|') {
                    result = match mod_name {
                        "strip_last_segment" => result
                            .rfind('/')
                            .filter(|&i| i > 0)
                            .map_or(result.clone(), |i| result[..i].to_string()),
                        "basename" => result
                            .rfind('/')
                            .map(|i| result[i + 1..].to_string())
                            .unwrap_or(result),
                        "js_escape" => js_escape(&result),
                        "strip_colons" => result.replace(':', ""),
                        "spaces_to_underscores" => result.replace(' ', "_"),
                        "lowercase" => result.to_lowercase(),
                        mod_name if mod_name.starts_with("slice:") => {
                            let parts: Vec<&str> = mod_name.split(':').collect();
                            if parts.len() >= 3 {
                                if let (Ok(start), Ok(end)) =
                                    (parts[1].parse::<usize>(), parts[2].parse::<usize>())
                                {
                                    if start < result.len() && end <= result.len() {
                                        result[start..end].to_string()
                                    } else if start < result.len() {
                                        result[start..].to_string()
                                    } else {
                                        result
                                    }
                                } else {
                                    result
                                }
                            } else {
                                result
                            }
                        }
                        _ => result,
                    };
                }

                s = s.replace(full_match, &result);
                changed = true;
            }
        }

        if s.starts_with('/') {
            format!("{}{}", self.def.base_url.trim_end_matches('/'), s)
        } else {
            s
        }
    }

    fn trace_step_start(
        &self,
        ctx: &ScraperCtx,
        step_index: usize,
        step_name: &str,
        detail: impl AsRef<str>,
    ) {
        let label = format!("#{step_index} {step_name}");
        ctx.note_step(label.clone());
        ctx.emit_debug(
            ScraperDebugLevel::Verbose,
            format!("    [{label}] {}", detail.as_ref()),
        );
    }

    fn trace_step_detail(&self, ctx: &ScraperCtx, detail: impl AsRef<str>) {
        ctx.emit_debug(
            ScraperDebugLevel::Verbose,
            format!("      {}", detail.as_ref()),
        );
    }

    fn trace_warning(&self, ctx: &ScraperCtx, detail: impl AsRef<str>) {
        ctx.emit_debug(
            ScraperDebugLevel::Verbose,
            format!("      warning: {}", detail.as_ref()),
        );
    }

    // ------------------------------------------------------------------
    // Field extraction (used inside foreach)
    // ------------------------------------------------------------------

    fn extract_field(
        &self,
        element: &ElementRef,
        field: &FieldDef,
        vars: &HashMap<String, String>,
    ) -> Result<String, ScraperError> {
        if let Some(ref v) = field.static_value {
            return Ok(self.expand(v, vars));
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

        let result = async {
            for (step_index, step) in action.steps.iter().enumerate() {
                let step_index = step_index + 1;
                tracing::trace!(step = ?std::mem::discriminant(step), "executing step");
                match step {
                StepDef::Open { open: url_tmpl } => {
                    let url = self.expand(url_tmpl, &vars);
                    self.trace_step_start(ctx, step_index, "open", format!("url={url}"));
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
                        ctx.browser.register_page(new_page.target_id());
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

                    // Poll for Cloudflare challenges while waiting for the page to load.
                    // The browser's stealth scripts can auto-bypass CF challenges,
                    // so we wait and re-check rather than failing immediately.
                    //
                    // We do a quick initial poll (500ms intervals) to detect CF early,
                    // but respect the full timeout so JS has time to execute API calls.
                    let timeout = Duration::from_secs(30);
                    let poll_interval = Duration::from_millis(500);
                    let min_settle_time = Duration::from_secs(3);
                    let start = std::time::Instant::now();
                    let mut cloudflare_detected = false;
                    let mut last_cf_click_attempt: Option<std::time::Instant> = None;

                    loop {
                        let elapsed = start.elapsed();
                        if elapsed >= timeout {
                            break;
                        }

                        // Check current HTML for Cloudflare challenge
                        match p.content().await {
                            Err(e) => warn!("[cf:poll] p.content() failed at {elapsed:.1?}: {e}"),
                            Ok(html) => {
                                let is_challenge = is_cf_challenge(&html);
                                debug!(
                                    "[cf:poll] t={elapsed:.1?} is_challenge={is_challenge} html_len={}",
                                    html.len()
                                );
                                if is_challenge {
                                    if !cloudflare_detected {
                                        info!(
                                            "Cloudflare challenge detected at {url}, waiting for auto-bypass..."
                                        );
                                        cloudflare_detected = true;
                                        // Prime the cooldown so the first click happens ~1s after
                                        // detection rather than immediately — the widget needs time
                                        // to finish rendering before a click will register.
                                        last_cf_click_attempt = Some(
                                            std::time::Instant::now()
                                                - Duration::from_millis(1_000),
                                        );
                                    }

                                    let should_try_click = last_cf_click_attempt
                                        .map(|t| t.elapsed() >= Duration::from_secs(2))
                                        .unwrap_or(true);
                                    if should_try_click {
                                        debug!(
                                            "[cf:click] attempting checkbox click at t={elapsed:.1?}"
                                        );
                                        try_cf_checkbox_click(p).await;
                                        last_cf_click_attempt = Some(std::time::Instant::now());
                                    }
                                } else if cloudflare_detected {
                                    // Challenge was present but page has loaded — bypassed!
                                    debug!("Cloudflare challenge auto-bypassed at {url}");
                                    break;
                                } else if elapsed >= min_settle_time {
                                    // No CF challenge and minimum settle time passed —
                                    // wait for network to become idle to let JS API calls complete
                                    let remaining_ms =
                                        timeout.saturating_sub(elapsed).as_millis() as u64;
                                    p.wait_for_network_idle(1000, remaining_ms).await.ok();
                                    break;
                                }
                            } // Ok(html) arm
                        } // match p.content()

                        tokio::time::sleep(poll_interval).await;
                    }

                    // Small delay to let page JS finish processing after API calls complete.
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    // Final check: if Cloudflare challenge still present, fail
                    if let Ok(html) = p.content().await {
                        if is_cf_challenge(&html) {
                            return Err(ScraperError::Browser(format!(
                                "Cloudflare challenge persisted at {url} — provider is blocked"
                            )));
                        }
                    }

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
                                self.trace_step_detail(
                                    ctx,
                                    format!("dumped html to {fname} ({} bytes)", html.len()),
                                );
                            }
                        }
                    }

                    // Close any popup/ad tabs the page may have spawned.
                    ctx.browser.close_popup_tabs(browser.as_ref(), p).await;

                    // Resolve all pending intercept captures (post-navigation).
                    let intercepts = std::mem::take(&mut pending_intercepts);
                    for intercept in intercepts {
                        match poll_capture(p, &intercept.url_contains).await {
                            Some(body) => {
                                debug!(
                                    "[step] intercept captured {} bytes for '{}'",
                                    body.len(),
                                    intercept.url_contains
                                );
                                let val = match intercept.json_path.as_deref() {
                                    Some(path) => match parse_json_path(&body, path) {
                                        Ok(parsed) => parsed.value,
                                        Err(err) => {
                                            let detail = format_json_path_error(&body, path, &err);
                                            warn!("[step] intercept json_path failed: {detail}");
                                            self.trace_warning(ctx, detail);
                                            body.clone()
                                        }
                                    },
                                    None => body.clone(),
                                };
                                vars.insert(intercept.var.clone(), val);
                            }
                            None => {
                                warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name, intercept.url_contains
                                );
                                debug!("[step] intercept TIMEOUT for '{}'", intercept.url_contains);
                            }
                        }
                    }
                }

                StepDef::WaitFor { wait_for: selector } => {
                    let sel = self.expand(selector, &vars);
                    self.trace_step_start(ctx, step_index, "wait_for", format!("selector={sel}"));
                    debug!("[step] wait_for → {sel}");
                    let p = require_page(&page, "wait_for")?;
                    match p.wait_for(&sel, 10_000).await {
                        Ok(_) => {
                            debug!("[step] wait_for '{sel}' → found");
                        }
                        Err(e) => {
                            debug!("[step] wait_for '{sel}' → TIMEOUT");
                            return Err(ScraperError::Timeout(format!("wait_for '{sel}': {e}")));
                        }
                    }
                }

                StepDef::Click { click: selector } => {
                    let sel = self.expand(selector, &vars);
                    self.trace_step_start(ctx, step_index, "click", format!("selector={sel}"));
                    debug!("[step] click → {sel}");
                    let p = require_page(&page, "click")?;
                    p.human_click(&sel)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("click '{sel}': {e}")))?;
                }

                StepDef::Type { type_def } => {
                    let selector = self.expand(&type_def.selector, &vars);
                    let value = self.expand(&type_def.value, &vars);
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "type",
                        format!("selector={selector} value={}", preview(&value, 80)),
                    );
                    debug!("[step] type → {selector}");
                    let p = require_page(&page, "type")?;
                    p.human_type(&selector, &value)
                        .await
                        .map_err(|e| ScraperError::Browser(format!("type '{selector}': {e}")))?;
                }

                StepDef::Sleep { sleep: ms } => {
                    self.trace_step_start(ctx, step_index, "sleep", format!("{ms}ms"));
                    debug!("[step] sleep → {ms}ms");
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                }

                StepDef::Script { script: js_tmpl } => {
                    let js = self.expand(js_tmpl, &vars);
                    let script_preview = &js[..js.len().min(80)];
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "script",
                        format!("script={}", preview(&js, 120)),
                    );
                    debug!("[step] script → {script_preview}…");
                    let p = require_page(&page, "script")?;
                    let _ = p.execute(&js).await;
                }

                StepDef::ExtractJs { extract_js: def } => {
                    let script = self.expand(&def.script, &vars);
                    ctx.note_source_var(def.var.clone());
                    ctx.note_parse_stage("extract_js");
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "extract_js",
                        format!("var={} script={}", def.var, preview(&script, 120)),
                    );
                    debug!("[step] extract_js → var={}", def.var);
                    let p = require_page(&page, "extract_js")?;
                    match p.evaluate::<serde_json::Value>(&script).await {
                        Ok(v) => {
                            let s = match v {
                                serde_json::Value::String(s) => s,
                                other => other.to_string(),
                            };
                            let rendered = preview(&s, 120);
                            let rendered_kind = if s.is_empty() { "empty" } else { "value" };
                            debug!("[step] extract_js '{}' = {rendered}", def.var);
                            self.trace_step_detail(
                                ctx,
                                format!("stored {} as '{}' = {}", rendered_kind, def.var, rendered),
                            );
                            vars.insert(def.var.clone(), s);
                        }
                        Err(e) => {
                            warn!(
                                "provider '{}': extract_js '{}' failed: {e}",
                                self.def.name, def.var
                            );
                            debug!("[step] extract_js '{}' → FAILED: {e}", def.var);
                            self.trace_warning(
                                ctx,
                                format!("extract_js '{}' failed: {e}", def.var),
                            );
                        }
                    }
                }

                StepDef::Intercept {
                    intercept: intercept_def,
                } => {
                    ctx.note_source_var(intercept_def.var.clone());
                    ctx.note_parse_stage(
                        intercept_def
                            .json_path
                            .as_deref()
                            .map(|path| format!("intercept json_path={path}"))
                            .unwrap_or_else(|| "intercept raw body".to_owned()),
                    );
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "intercept",
                        format!(
                            "url_contains={} -> var={}",
                            intercept_def.url_contains, intercept_def.var
                        ),
                    );
                    debug!(
                        "[step] intercept → url_contains='{}', var='{}'",
                        intercept_def.url_contains, intercept_def.var
                    );
                    if let Some(ref p) = page {
                        // Page already open: inject immediately and poll.
                        inject_intercept(p, &intercept_def.url_contains).await;
                        match poll_capture(p, &intercept_def.url_contains).await {
                            Some(body) => {
                                debug!(
                                    "[step] intercept captured {} bytes for '{}'",
                                    body.len(),
                                    intercept_def.url_contains
                                );
                                let val = match intercept_def.json_path.as_deref() {
                                    Some(path) => match parse_json_path(&body, path) {
                                        Ok(parsed) => {
                                            self.trace_step_detail(
                                                ctx,
                                                format!(
                                                    "json_path {path} -> {} ({})",
                                                    parsed.kind,
                                                    preview(&parsed.value, 120)
                                                ),
                                            );
                                            parsed.value
                                        }
                                        Err(err) => {
                                            let detail = format_json_path_error(&body, path, &err);
                                            warn!("[step] intercept json_path failed: {detail}");
                                            self.trace_warning(ctx, detail);
                                            body.clone()
                                        }
                                    },
                                    None => body.clone(),
                                };
                                vars.insert(intercept_def.var.clone(), val);
                            }
                            None => {
                                warn!(
                                    "provider '{}': intercept for '{}' timed out",
                                    self.def.name, intercept_def.url_contains
                                );
                                debug!(
                                    "[step] intercept TIMEOUT for '{}'",
                                    intercept_def.url_contains
                                );
                                self.trace_warning(
                                    ctx,
                                    format!(
                                        "intercept timed out for '{}'",
                                        intercept_def.url_contains
                                    ),
                                );
                            }
                        }
                    } else {
                        // No page yet: defer until the next `open`.
                        debug!(
                            "[step] intercept deferred (no page yet) for '{}'",
                            intercept_def.url_contains
                        );
                        self.trace_step_detail(ctx, "deferred until the next open step");
                        pending_intercepts.push(intercept_def.clone());
                    }
                }

                StepDef::Foreach {
                    foreach: foreach_def,
                } => {
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "foreach",
                        format!("selector={}", foreach_def.selector),
                    );
                    debug!("[step] foreach → selector='{}'", foreach_def.selector);
                    let p = require_page(&page, "foreach")?;
                    let html = p
                        .content()
                        .await
                        .map_err(|e| ScraperError::Browser(e.to_string()))?;
                    let stats = self.collect_foreach_results(&html, foreach_def, &mut results, &vars)?;
                    debug!(
                        "[step] foreach → {} elements matched '{}'",
                        stats.element_count, foreach_def.selector
                    );
                    self.trace_step_detail(
                        ctx,
                        format!(
                            "{} elements matched, {} total result rows",
                            stats.element_count,
                            results.len()
                        ),
                    );
                    for (field, ok, fail) in &stats.field_counts {
                        debug!("         field '{field}': {ok} extracted, {fail} failed");
                        if *fail > 0 {
                            self.trace_warning(
                                ctx,
                                format!("field '{field}' had {fail} extraction failures"),
                            );
                        }
                    }
                    if let Some(ref first) = stats.first_record {
                        debug!("         sample (first match):");
                        let mut keys: Vec<&String> = first.keys().collect();
                        keys.sort();
                        for k in keys {
                            let v = &first[k];
                            let preview = &v[..v.len().min(100)];
                            debug!("           {k} = {preview}");
                        }
                        self.trace_step_detail(
                            ctx,
                            format!("sample row: {}", preview_map(first, 120)),
                        );
                    }
                }

                StepDef::Return { value: tmpl } => {
                    let val = self.expand(tmpl, &vars);
                    ctx.note_parse_stage("return");
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "return",
                        format!("value={}", preview(&val, 120)),
                    );
                    let count_hint = if val.starts_with('[') {
                        serde_json::from_str::<serde_json::Value>(&val)
                            .ok()
                            .and_then(|v| v.as_array().map(|a| a.len()))
                            .map(|n| format!(" ({n} URLs in array)"))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let return_preview = preview(&val, 120);
                    debug!("[step] return → {return_preview}{count_hint}");
                    self.trace_step_detail(ctx, format!("returning {return_preview}{count_hint}"));
                    early_return = Some(val);
                    break;
                }

                StepDef::Scroll { scroll: target } => {
                    self.trace_step_start(ctx, step_index, "scroll", format!("target={target}"));
                    debug!("[step] scroll → {target}");
                    let p = require_page(&page, "scroll")?;
                    let js = if target == "bottom" {
                        "window.scrollTo(0, document.body.scrollHeight)".to_owned()
                    } else {
                        let safe = js_escape(target);
                        format!("document.querySelector('{safe}')?.scrollIntoView()")
                    };
                    let _ = p.execute(&js).await;
                }

                StepDef::Fetch { fetch: fetch_def } => {
                    let p = require_page(&page, "fetch")?;
                    ctx.note_source_var(fetch_def.var.clone());
                    ctx.note_parse_stage(
                        fetch_def
                            .json_path
                            .as_deref()
                            .map(|path| format!("fetch json_path={path}"))
                            .unwrap_or_else(|| "fetch raw body".to_owned()),
                    );
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "fetch",
                        format!(
                            "{} {} -> {}",
                            fetch_def.method.to_uppercase(),
                            self.expand(&fetch_def.url, &vars),
                            fetch_def.var
                        ),
                    );

                    if let Some(ref pagination) = fetch_def.pagination {
                        // Handle paginated fetch
                        let mut all_items: Vec<serde_json::Value> = Vec::new();
                        let mut current_page = pagination.start_page; // actual param value sent in URL
                        let mut pages_fetched = 0u32; // how many pages retrieved so far
                        let mut total_pages = pagination.max_pages; // total pages available (from response or max cap)
                        let page_step = pagination.page_step;

                        for _ in 0..pagination.max_pages {
                            let mut url = self.expand(&fetch_def.url, &vars);

                            // Add page parameter to URL
                            if url.contains('?') {
                                url.push_str(&format!(
                                    "&{}={}",
                                    pagination.page_param, current_page
                                ));
                            } else {
                                url.push_str(&format!(
                                    "?{}={}",
                                    pagination.page_param, current_page
                                ));
                            }

                            debug!("[step] fetch (page {current_page}) → url={url}");
                            let method = fetch_def.method.to_uppercase();
                            let expanded_headers: HashMap<String, String> = fetch_def
                                .headers
                                .iter()
                                .map(|(key, val)| (key.clone(), self.expand(val, &vars)))
                                .collect();
                            let request_body =
                                fetch_def.body.as_ref().map(|body| self.expand(body, &vars));
                            ctx.note_request(format!("{method} {url}"));
                            self.trace_step_detail(
                                ctx,
                                format!("page {current_page} request: {method} {url}"),
                            );
                            self.trace_step_detail(
                                ctx,
                                format!("headers: {}", format_headers(&expanded_headers)),
                            );
                            if let Some(body) = request_body.as_deref() {
                                self.trace_step_detail(
                                    ctx,
                                    format!("body: {}", preview(body, 160)),
                                );
                            }

                            let js = build_browser_fetch_js(
                                &method,
                                &url,
                                &expanded_headers,
                                request_body.as_deref(),
                                false,
                            );

                            let raw_response = match p.evaluate::<String>(&js).await {
                                Ok(response) => response,
                                Err(e) => {
                                    warn!("[step] fetch execute failed: {e}");
                                    self.trace_warning(ctx, format!("fetch execute failed: {e}"));
                                    break;
                                }
                            };
                            let envelope = match parse_browser_fetch_envelope(&raw_response) {
                                Ok(envelope) => envelope,
                                Err(e) => {
                                    warn!("[step] fetch parse failed: {e}");
                                    self.trace_warning(ctx, format!("fetch parse failed: {e}"));
                                    break;
                                }
                            };
                            if let Some(error) = &envelope.error {
                                warn!("[step] fetch failed: {error}");
                                self.trace_warning(ctx, format!("transport failed: {error}"));
                                break;
                            }

                            self.trace_step_detail(
                                ctx,
                                format!(
                                    "response: {} {} ok={} final_url={} ({} bytes)",
                                    envelope.status,
                                    envelope.status_text,
                                    envelope.ok,
                                    envelope.url,
                                    envelope.body.len()
                                ),
                            );
                            self.trace_step_detail(
                                ctx,
                                format!("response headers: {}", format_headers(&envelope.headers)),
                            );
                            self.trace_step_detail(
                                ctx,
                                format!("response body: {}", preview(&envelope.body, 200)),
                            );

                            let json =
                                match serde_json::from_str::<serde_json::Value>(&envelope.body) {
                                    Ok(json) => json,
                                    Err(e) => {
                                        let detail = format!(
                                            "fetch response is not valid JSON: {e}; body={}",
                                            preview(&envelope.body, 160)
                                        );
                                        warn!("[step] {detail}");
                                        self.trace_warning(ctx, detail);
                                        break;
                                    }
                                };

                            if let Some(ref meta_path) = pagination.meta_path {
                                let meta = parse_json_value(&json, meta_path);
                                if pagination.calculate_last_page {
                                    let total = meta
                                        .get(&pagination.total_field)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0);
                                    let limit = meta
                                        .get(&pagination.per_page_field)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(100);
                                    if limit > 0 {
                                        total_pages = total.div_ceil(limit) as u32;
                                    }
                                } else if let Some(last_page_val) =
                                    meta.get(&pagination.last_page_field)
                                {
                                    if let Some(lp) = last_page_val.as_u64() {
                                        total_pages = lp as u32;
                                    }
                                }
                            }

                            let items = if let Some(path) = fetch_def.json_path.as_deref() {
                                match parse_json_path(&envelope.body, path) {
                                    Ok(parsed) => {
                                        self.trace_step_detail(
                                            ctx,
                                            format!(
                                                "json_path {path} -> {} {}",
                                                parsed.kind,
                                                preview(&parsed.value, 160)
                                            ),
                                        );
                                        match serde_json::from_str::<serde_json::Value>(
                                            &parsed.value,
                                        ) {
                                            Ok(value) => {
                                                value.as_array().cloned().unwrap_or_default()
                                            }
                                            Err(e) => {
                                                let detail = format!(
                                                    "json_path '{path}' did not produce valid JSON array: {e}; value={}",
                                                    preview(&parsed.value, 160)
                                                );
                                                warn!("[step] {detail}");
                                                self.trace_warning(ctx, detail);
                                                Vec::new()
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        let detail =
                                            format_json_path_error(&envelope.body, path, &err);
                                        warn!("[step] fetch json_path failed: {detail}");
                                        self.trace_warning(ctx, detail);
                                        Vec::new()
                                    }
                                }
                            } else {
                                json.as_array().cloned().unwrap_or_default()
                            };

                            if items.is_empty() {
                                debug!(
                                    "[step] fetch (page {current_page}) → empty response, stopping"
                                );
                                self.trace_warning(
                                    ctx,
                                    format!("page {current_page} produced 0 items; stopping"),
                                );
                                break;
                            }

                            let items_count = items.len();
                            all_items.extend(items);
                            pages_fetched += 1;
                            debug!(
                                "[step] fetch (page {current_page}, {pages_fetched}/{total_pages}) → {items_count} items (total: {})",
                                all_items.len()
                            );
                            self.trace_step_detail(
                                ctx,
                                format!("page {current_page} ({pages_fetched}/{total_pages}) produced {items_count} items"),
                            );

                            // Check if we've fetched all available pages
                            if pages_fetched >= total_pages {
                                debug!("[step] fetch → fetched all {total_pages} pages, stopping");
                                break;
                            }

                            current_page += page_step;
                        }

                        // Store accumulated results
                        let value =
                            serde_json::to_string(&all_items).unwrap_or_else(|_| "[]".to_string());
                        debug!(
                            "[step] fetch pagination complete → {} total items stored in '{}'",
                            all_items.len(),
                            fetch_def.var
                        );
                        self.trace_step_detail(
                            ctx,
                            format!(
                                "stored {} paginated items in '{}'",
                                all_items.len(),
                                fetch_def.var
                            ),
                        );
                        vars.insert(fetch_def.var.clone(), value);
                    } else {
                        // Non-paginated fetch (original behavior)
                        let url = self.expand(&fetch_def.url, &vars);
                        debug!("[step] fetch → url={url}");
                        let method = fetch_def.method.to_uppercase();
                        let expanded_headers: HashMap<String, String> = fetch_def
                            .headers
                            .iter()
                            .map(|(key, val)| (key.clone(), self.expand(val, &vars)))
                            .collect();
                        let request_body =
                            fetch_def.body.as_ref().map(|body| self.expand(body, &vars));
                        ctx.note_request(format!("{method} {url}"));
                        self.trace_step_detail(ctx, format!("request: {method} {url}"));
                        self.trace_step_detail(
                            ctx,
                            format!("headers: {}", format_headers(&expanded_headers)),
                        );
                        if let Some(body) = request_body.as_deref() {
                            self.trace_step_detail(ctx, format!("body: {}", preview(body, 160)));
                        }

                        let js = build_browser_fetch_js(
                            &method,
                            &url,
                            &expanded_headers,
                            request_body.as_deref(),
                            false,
                        );

                        match p.evaluate::<String>(&js).await {
                            Ok(raw_response) => match parse_browser_fetch_envelope(&raw_response) {
                                Ok(envelope) => {
                                    if let Some(error) = &envelope.error {
                                        warn!("[step] fetch failed: {error}");
                                        self.trace_warning(
                                            ctx,
                                            format!("transport failed: {error}"),
                                        );
                                        continue;
                                    }

                                    self.trace_step_detail(
                                        ctx,
                                        format!(
                                            "response: {} {} ok={} final_url={} ({} bytes)",
                                            envelope.status,
                                            envelope.status_text,
                                            envelope.ok,
                                            envelope.url,
                                            envelope.body.len()
                                        ),
                                    );
                                    self.trace_step_detail(
                                        ctx,
                                        format!(
                                            "response headers: {}",
                                            format_headers(&envelope.headers)
                                        ),
                                    );
                                    self.trace_step_detail(
                                        ctx,
                                        format!("response body: {}", preview(&envelope.body, 200)),
                                    );

                                    let value = if let Some(ref path) = fetch_def.json_path {
                                        if looks_like_html(&envelope.body) {
                                            let detail = format!(
                                                "expected JSON for json_path '{path}' but response looks like HTML"
                                            );
                                            warn!("[step] {detail}");
                                            self.trace_warning(ctx, detail);
                                        }
                                        match parse_json_path(&envelope.body, path) {
                                            Ok(parsed) => {
                                                self.trace_step_detail(
                                                    ctx,
                                                    format!(
                                                        "json_path {path} -> {} {}",
                                                        parsed.kind,
                                                        preview(&parsed.value, 160)
                                                    ),
                                                );
                                                if parsed.value.is_empty()
                                                    || parsed.value == "[]"
                                                    || parsed.value == "null"
                                                {
                                                    self.trace_warning(
                                                        ctx,
                                                        format!(
                                                            "json_path '{path}' produced an empty value"
                                                        ),
                                                    );
                                                }
                                                parsed.value
                                            }
                                            Err(err) => {
                                                let detail = format_json_path_error(
                                                    &envelope.body,
                                                    path,
                                                    &err,
                                                );
                                                warn!("[step] fetch json_path failed: {detail}");
                                                self.trace_warning(ctx, detail);
                                                envelope.body
                                            }
                                        }
                                    } else {
                                        envelope.body
                                    };
                                    debug!(
                                        "[step] fetch stored in '{}': {}",
                                        fetch_def.var,
                                        preview(&value, 120)
                                    );
                                    self.trace_step_detail(
                                        ctx,
                                        format!(
                                            "stored '{}' = {}",
                                            fetch_def.var,
                                            preview(&value, 160)
                                        ),
                                    );
                                    vars.insert(fetch_def.var.clone(), value);
                                }
                                Err(e) => {
                                    warn!("[step] fetch envelope parse failed: {e}");
                                    self.trace_warning(
                                        ctx,
                                        format!("fetch envelope parse failed: {e}"),
                                    );
                                }
                            },
                            Err(e) => {
                                warn!("[step] fetch execute failed: {e}");
                                self.trace_warning(ctx, format!("fetch execute failed: {e}"));
                            }
                        }
                    }
                }

                StepDef::Graphql {
                    graphql: graphql_def,
                } => {
                    let p = require_page(&page, "graphql")?;

                    let url = self.expand(&graphql_def.url, &vars);
                    ctx.note_source_var(graphql_def.var.clone());
                    ctx.note_parse_stage(
                        graphql_def
                            .json_path
                            .as_deref()
                            .map(|path| format!("graphql json_path={path}"))
                            .unwrap_or_else(|| "graphql raw body".to_owned()),
                    );
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "graphql",
                        format!("POST {url} -> {}", graphql_def.var),
                    );
                    debug!("[step] graphql → url={url}");

                    // Expand templates in variables, then serialize as JSON.
                    // JSON is valid JS syntax so we can inline it directly as a literal.
                    let expanded_variables: serde_json::Map<String, serde_json::Value> =
                        graphql_def
                            .variables
                            .iter()
                            .map(|(k, v)| (k.clone(), self.expand_json_value(v, &vars)))
                            .collect();
                    let vars_json = serde_json::to_string(&expanded_variables)
                        .unwrap_or_else(|_| "{}".to_string());

                    // Escape the query for embedding in a JS single-quoted string.
                    let query_escaped = graphql_def
                        .query
                        .replace('\\', "\\\\")
                        .replace('\'', "\\'")
                        .replace('\n', "\\n")
                        .replace('\r', "");

                    // Build headers object - start with Content-Type, then add custom headers
                    let mut expanded_headers: HashMap<String, String> = graphql_def
                        .headers
                        .iter()
                        .map(|(key, val)| (key.clone(), self.expand(val, &vars)))
                        .collect();
                    expanded_headers
                        .insert("Content-Type".to_owned(), "application/json".to_owned());

                    let request_body =
                        format!(r#"{{"query":"{query_escaped}","variables":{vars_json}}}"#);
                    ctx.note_request(format!("POST {url}"));
                    self.trace_step_detail(ctx, format!("request: POST {url}"));
                    self.trace_step_detail(
                        ctx,
                        format!("headers: {}", format_headers(&expanded_headers)),
                    );
                    self.trace_step_detail(ctx, format!("body: {}", preview(&request_body, 160)));

                    let js = build_browser_fetch_js(
                        "POST",
                        &url,
                        &expanded_headers,
                        Some(&request_body),
                        true,
                    );

                    match p.evaluate::<String>(&js).await {
                        Ok(raw_response) => match parse_browser_fetch_envelope(&raw_response) {
                            Ok(envelope) => {
                                if let Some(error) = &envelope.error {
                                    warn!("[step] graphql failed: {error}");
                                    self.trace_warning(ctx, format!("transport failed: {error}"));
                                    continue;
                                }
                                self.trace_step_detail(
                                    ctx,
                                    format!(
                                        "response: {} {} ok={} final_url={} ({} bytes)",
                                        envelope.status,
                                        envelope.status_text,
                                        envelope.ok,
                                        envelope.url,
                                        envelope.body.len()
                                    ),
                                );
                                self.trace_step_detail(
                                    ctx,
                                    format!(
                                        "response headers: {}",
                                        format_headers(&envelope.headers)
                                    ),
                                );
                                self.trace_step_detail(
                                    ctx,
                                    format!("response body: {}", preview(&envelope.body, 200)),
                                );
                                let value = if let Some(ref path) = graphql_def.json_path {
                                    match parse_json_path(&envelope.body, path) {
                                        Ok(parsed) => {
                                            self.trace_step_detail(
                                                ctx,
                                                format!(
                                                    "json_path {path} -> {} {}",
                                                    parsed.kind,
                                                    preview(&parsed.value, 160)
                                                ),
                                            );
                                            parsed.value
                                        }
                                        Err(err) => {
                                            let detail =
                                                format_json_path_error(&envelope.body, path, &err);
                                            warn!("[step] graphql json_path failed: {detail}");
                                            self.trace_warning(ctx, detail);
                                            envelope.body
                                        }
                                    }
                                } else {
                                    envelope.body
                                };
                                self.trace_step_detail(
                                    ctx,
                                    format!(
                                        "stored '{}' = {}",
                                        graphql_def.var,
                                        preview(&value, 160)
                                    ),
                                );
                                vars.insert(graphql_def.var.clone(), value);
                            }
                            Err(e) => {
                                warn!("[step] graphql envelope parse failed: {e}");
                                self.trace_warning(
                                    ctx,
                                    format!("graphql envelope parse failed: {e}"),
                                );
                            }
                        },
                        Err(e) => {
                            warn!("[step] graphql execute failed: {e}");
                            self.trace_warning(ctx, format!("graphql execute failed: {e}"));
                        }
                    }
                }

                StepDef::FromJson {
                    from_json: from_json_def,
                } => {
                    ctx.note_source_var(from_json_def.var.clone());
                    ctx.note_parse_stage("from_json");
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "from_json",
                        format!("source_var={}", from_json_def.var),
                    );
                    debug!("[step] from_json → var={}", from_json_def.var);

                    let json_str = vars.get(&from_json_def.var).ok_or_else(|| {
                        ScraperError::Parse(format!(
                            "from_json: variable '{}' not found",
                            from_json_def.var
                        ))
                    })?;

                    let json_array: Vec<serde_json::Value> = serde_json::from_str(json_str)
                        .map_err(|e| {
                            ScraperError::Parse(format_from_json_parse_error(
                                &from_json_def.var,
                                json_str,
                                &e,
                            ))
                        })?;
                    self.trace_step_detail(
                        ctx,
                        format!(
                            "parsed {} JSON items from '{}'",
                            json_array.len(),
                            from_json_def.var
                        ),
                    );

                    for item in json_array {
                        // Apply filter if configured
                        if let Some(ref filter) = from_json_def.filter {
                            let field_value = extract_json_value(&item, &filter.field);
                            let has_field = field_value.is_some()
                                && field_value.as_deref() != Some("null")
                                && !field_value.as_deref().unwrap_or("").is_empty();
                            // Skip record if filter condition matches
                            if filter.exists && has_field {
                                debug!(
                                    "[step] from_json → filtered out record with field '{}'",
                                    filter.field
                                );
                                continue;
                            }
                            if !filter.exists && !has_field {
                                debug!(
                                    "[step] from_json → filtered out record missing field '{}'",
                                    filter.field
                                );
                                continue;
                            }
                        }

                        let mut record: HashMap<String, String> = HashMap::new();
                        for (output_key, json_key) in &from_json_def.extract {
                            // Handle both object-based and plain string arrays
                            let value = if let serde_json::Value::String(s) = &item {
                                // If the item is a plain string, use it directly
                                Some(s.clone())
                            } else {
                                // Otherwise extract from object using the key path
                                extract_json_value(&item, json_key)
                            };

                            if let Some(val) = value {
                                // Apply date format if configured for this field
                                let final_val = if let Some(date_fmt) =
                                    from_json_def.date_format.get(output_key)
                                {
                                    match parse_date(&val, date_fmt) {
                                        Some(ts) => ts.to_string(),
                                        None => val,
                                    }
                                } else if let Some(prefix) = from_json_def.prefix.get(output_key) {
                                    // Apply prefix if not absolute URL
                                    let expanded_prefix = self.expand(prefix, &vars);
                                    if val.starts_with("http://") || val.starts_with("https://") {
                                        val
                                    } else {
                                        format!("{expanded_prefix}{val}")
                                    }
                                } else {
                                    val
                                };
                                record.insert(output_key.clone(), final_val);
                            } else {
                                let detail = format!(
                                    "from_json missing field '{}' (json key '{}') in item {}",
                                    output_key,
                                    json_key,
                                    preview(&item.to_string(), 160)
                                );
                                warn!("[step] {detail}");
                                self.trace_warning(ctx, detail);
                            }
                        }
                        if !record.is_empty() {
                            if results.is_empty() {
                                self.trace_step_detail(
                                    ctx,
                                    format!(
                                        "sample transformed row: {}",
                                        preview_map(&record, 120)
                                    ),
                                );
                            }
                            results.push(record);
                        }
                    }

                    debug!("[step] from_json → {} records extracted", results.len());
                    if results.is_empty() {
                        self.trace_warning(ctx, "from_json produced 0 rows");
                    } else {
                        self.trace_step_detail(
                            ctx,
                            format!("from_json produced {} rows", results.len()),
                        );
                    }
                }

                StepDef::FilterJson {
                    filter_json: filter_def,
                } => {
                    ctx.note_source_var(filter_def.var.clone());
                    ctx.note_parse_stage("filter_json");
                    self.trace_step_start(
                        ctx,
                        step_index,
                        "filter_json",
                        format!("source_var={}", filter_def.var),
                    );
                    debug!("[step] filter_json → var={}", filter_def.var);

                    let json_str = vars.get(&filter_def.var).ok_or_else(|| {
                        ScraperError::Parse(format!(
                            "filter_json: variable '{}' not found",
                            filter_def.var
                        ))
                    })?;

                    let mut json_array: Vec<serde_json::Value> = serde_json::from_str(json_str)
                        .map_err(|e| {
                            ScraperError::Parse(format!("filter_json: failed to parse JSON: {e}"))
                        })?;

                    let original_count = json_array.len();
                    let condition = &filter_def.condition;

                    json_array.retain(|item| {
                        let field_value = extract_json_value(item, &condition.field);
                        let has_field = field_value.is_some()
                            && field_value.as_deref() != Some("null")
                            && !field_value.as_deref().unwrap_or("").is_empty();

                        // Keep record if filter condition does NOT match
                        // Remove when field existence matches condition (both exist or both don't exist)
                        condition.exists != has_field
                    });

                    let filtered_count = original_count - json_array.len();
                    debug!(
                        "[step] filter_json → removed {} records ({} remaining)",
                        filtered_count,
                        json_array.len()
                    );
                    self.trace_step_detail(
                        ctx,
                        format!(
                            "removed {filtered_count} records ({} remaining)",
                            json_array.len()
                        ),
                    );

                    // Store filtered array back
                    let value =
                        serde_json::to_string(&json_array).unwrap_or_else(|_| "[]".to_string());
                    vars.insert(filter_def.var.clone(), value);
                }
                }
            }

            if let Some(val) = early_return {
                Ok(ActionResult::Value(val))
            } else {
                Ok(ActionResult::Records(results))
            }
        }
        .await;

        // Always close the Chrome tab before dropping the Rust Page handle.
        if let Some(ref p) = page {
            ctx.browser.unregister_page(p.target_id());
            close_page_tab(browser.as_ref(), p).await;
        }
        drop(page);

        result
    }

    fn collect_foreach_results(
        &self,
        html: &str,
        foreach_def: &ForeachDef,
        results: &mut Vec<HashMap<String, String>>,
        vars: &HashMap<String, String>,
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
                match self.extract_field(&element, field_def, vars) {
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

    fn max_concurrency(&self) -> u32 {
        self.def.concurrency.workers.max(1)
    }

    fn version(&self) -> Option<&str> {
        self.def.version.as_deref()
    }

    fn tags(&self) -> &[crate::scraper::def::ProviderTag] {
        &self.def.tags
    }

    fn pages_download_method(&self) -> crate::scraper::def::DownloadMethod {
        self.def
            .pages
            .as_ref()
            .map(|p| p.download_method.clone())
            .unwrap_or_default()
    }

    #[tracing::instrument(skip(self, ctx), fields(provider = %self.def.name))]
    async fn search(
        &self,
        ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        let def = self.def.search.as_ref().ok_or(ScraperError::Unsupported)?;
        let encoded = urlencoding::encode(title).into_owned();
        let mut input = HashMap::new();
        input.insert("query".to_owned(), encoded);
        input.insert("query_raw".to_owned(), title.to_owned());

        let result = self.execute_action(ctx, def, input).await?;
        Ok(records_to_search_results(result.into_records()))
    }

    #[tracing::instrument(skip(self, ctx), fields(provider = %self.def.name))]
    async fn chapters(
        &self,
        ctx: &ScraperCtx,
        manga_url: &str,
        variables: &ProviderVariables,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        let def = self
            .def
            .chapters
            .as_ref()
            .ok_or(ScraperError::Unsupported)?;
        let mut input = HashMap::new();
        input.insert("manga_url".to_owned(), manga_url.to_owned());
        input.extend(variables.clone());

        let result = self.execute_action(ctx, def, input).await?;
        Ok(records_to_chapters(result.into_records()))
    }

    #[tracing::instrument(skip(self, ctx), fields(provider = %self.def.name))]
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
                variables: r,
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
    let rest = lower
        .strip_prefix("chapter ")
        .or_else(|| lower.strip_prefix("ch. "))
        .or_else(|| lower.strip_prefix("ch "));
    match rest {
        Some(s) => s.trim().parse::<f64>().is_ok(),
        None => false,
    }
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
                    referrer: r.remove("referrer").filter(|s| !s.is_empty()),
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
                        // Accept either a plain URL string or an object with a "url" key
                        // (and an optional "referrer" key).
                        if let Some(u) = v.as_str() {
                            Some(PageUrl {
                                url: u.to_owned(),
                                index: (i + 1) as u32,
                                referrer: None,
                            })
                        } else if let Some(obj) = v.as_object() {
                            obj.get("url")?.as_str().map(|u| PageUrl {
                                url: u.to_owned(),
                                index: (i + 1) as u32,
                                referrer: obj
                                    .get("referrer")
                                    .and_then(|r| r.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_owned),
                            })
                        } else {
                            None
                        }
                    })
                    .collect()),
                _ => Err(ScraperError::Parse(
                    "return value for pages must be a JSON array".to_owned(),
                )),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cloudflare challenge detection
// ---------------------------------------------------------------------------

/// Return true when the page HTML looks like a Cloudflare challenge/IUAM page.
pub fn is_cf_challenge(html: &str) -> bool {
    html.contains("cf-browser-verification")
        || html.contains("__cf_chl")
        || (html.contains("Just a moment") && html.contains("cloudflare"))
}

/// Attempt to click the Cloudflare Turnstile checkbox.
///
/// The widget renders inside a closed shadow root so CSS selectors can't reach
/// it. We click at the screen coordinates where the checkbox visually appears,
/// using a human-like mouse sequence (move → press → release with varied delays)
/// so CF's behavioural scoring doesn't flag the interaction as synthetic.
pub async fn try_cf_checkbox_click(page: &eoka::Page) -> bool {
    // Layout on 1366×768: widget is horizontally centred (~300px wide),
    // left edge ≈ 533px, checkbox icon ≈ 25px from left edge → target x ≈ 558.
    // Widget appears roughly vertically centred, typically 300–430px from top.
    // Candidates spread across that area; varied delays between each attempt.
    let session = page.session();

    let targets: &[(f64, f64)] = &[
        (548.0, 334.0),
        (563.0, 350.0),
        (553.0, 384.0),
        (568.0, 310.0),
        (543.0, 415.0),
    ];

    for (i, &(x, y)) in targets.iter().enumerate() {
        // Approach from a slightly offset start so the move path looks natural.
        let ax = x - 14.0 - (i as f64 * 2.5);
        let ay = y + 9.0 - (i as f64 * 1.5);

        session
            .dispatch_mouse_event(eoka::cdp::MouseEventType::MouseMoved, ax, ay, None, None)
            .await
            .ok();
        tokio::time::sleep(Duration::from_millis(18 + i as u64 * 6)).await;

        session
            .dispatch_mouse_event(eoka::cdp::MouseEventType::MouseMoved, x, y, None, None)
            .await
            .ok();
        tokio::time::sleep(Duration::from_millis(32 + i as u64 * 9)).await;

        if let Err(e) = session
            .dispatch_mouse_event(
                eoka::cdp::MouseEventType::MousePressed,
                x,
                y,
                Some(eoka::cdp::MouseButton::Left),
                Some(1),
            )
            .await
        {
            warn!("[cf:click] mousedown at ({x:.0},{y:.0}) failed: {e}");
            continue;
        }
        tokio::time::sleep(Duration::from_millis(68 + i as u64 * 11)).await;

        session
            .dispatch_mouse_event(
                eoka::cdp::MouseEventType::MouseReleased,
                x,
                y,
                Some(eoka::cdp::MouseButton::Left),
                Some(1),
            )
            .await
            .ok();

        debug!("[cf:click] human click at ({x:.0},{y:.0})");
        tokio::time::sleep(Duration::from_millis(90)).await;
    }

    false
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
        || msg.contains("Failed to open a new tab") // Chrome -32000: target creation failed
        || msg.contains("code -32000") // catch-all for other -32000 CDP errors
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

fn preview(s: &str, limit: usize) -> String {
    let end = (0..=limit.min(s.len()))
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    let mut out = s[..end].replace('\n', "\\n");
    if s.len() > end {
        out.push_str("...");
    }
    out
}

fn preview_map(values: &HashMap<String, String>, limit: usize) -> String {
    let mut items: Vec<_> = values.iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    let joined = items
        .into_iter()
        .map(|(k, v)| format!("{k}={}", preview(v, 40)))
        .collect::<Vec<_>>()
        .join(", ");
    preview(&joined, limit)
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<body")
}

fn build_browser_fetch_js(
    method: &str,
    url: &str,
    headers: &HashMap<String, String>,
    body: Option<&str>,
    include_credentials: bool,
) -> String {
    let mut headers_js = String::new();
    for (key, val) in headers {
        headers_js.push_str(&format!("'{}': '{}',", key, val.replace('\'', "\\\'")));
    }

    let body_js = body
        .map(|b| format!(", body: `{}`", b.replace('`', "\\`")))
        .unwrap_or_default();
    let credentials_js = if include_credentials {
        ", credentials: 'include'"
    } else {
        ""
    };

    format!(
        r#"
        (async () => {{
            const headers = {{{}}};
            const opts = {{
                method: '{}',
                headers: headers{}{}
            }};
            try {{
                const resp = await fetch('{}', opts);
                const body = await resp.text();
                const headersObj = Object.fromEntries(Array.from(resp.headers.entries()));
                return JSON.stringify({{
                    ok: resp.ok,
                    status: resp.status,
                    status_text: resp.statusText,
                    url: resp.url,
                    headers: headersObj,
                    body,
                    error: null
                }});
            }} catch(e) {{
                return JSON.stringify({{
                    ok: false,
                    status: 0,
                    status_text: '',
                    url: '{}',
                    headers: {{}},
                    body: '',
                    error: e.message || String(e)
                }});
            }}
        }})()
    "#,
        headers_js,
        method,
        credentials_js,
        body_js,
        url.replace('\'', "\\\'"),
        url.replace('\'', "\\\'")
    )
}

fn parse_browser_fetch_envelope(raw: &str) -> Result<BrowserFetchEnvelope, ScraperError> {
    serde_json::from_str(raw).map_err(|e| {
        ScraperError::Parse(format!(
            "fetch bridge returned malformed response envelope: {e}; raw={}",
            preview(raw, 160)
        ))
    })
}

fn format_headers(headers: &HashMap<String, String>) -> String {
    if headers.is_empty() {
        return "(none)".to_owned();
    }
    let mut items: Vec<_> = headers.iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    items
        .into_iter()
        .map(|(k, v)| format!("{k}: {}", preview(v, 60)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_json_path_error(body: &str, json_path: &str, err: &JsonPathError) -> String {
    match err {
        JsonPathError::InvalidJson(reason) => format!(
            "json_path '{json_path}' could not parse response as JSON: {reason}; body={}",
            preview(body, 160)
        ),
        JsonPathError::MissingPath {
            path,
            missing_segment,
            container_kind,
        } => format!(
            "json_path '{json_path}' could not resolve segment '{missing_segment}' (resolved='{path}', container={container_kind}); body={}",
            preview(body, 160)
        ),
    }
}

fn format_from_json_parse_error(
    source_var: &str,
    json_str: &str,
    err: &serde_json::Error,
) -> String {
    format!(
        "from_json: failed to parse JSON from '{source_var}': {err}; source={}",
        preview(json_str, 160)
    )
}

/// Escape a string for safe embedding in a JS single-quoted string literal.
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Parse a JSON string and navigate to a specific path.
fn parse_json_path(body: &str, json_path: &str) -> Result<JsonPathSuccess, JsonPathError> {
    let mut json = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| JsonPathError::InvalidJson(e.to_string()))?;
    let mut resolved_path = Vec::new();

    for key in json_path.split('.') {
        let Some(next) = json.get(key).cloned() else {
            return Err(JsonPathError::MissingPath {
                path: resolved_path.join("."),
                missing_segment: key.to_owned(),
                container_kind: json_kind(&json),
            });
        };
        resolved_path.push(key.to_owned());
        json = next;
    }

    let kind = json_kind(&json);
    let value = if let serde_json::Value::String(s) = json {
        s
    } else {
        json.to_string()
    };
    Ok(JsonPathSuccess { value, kind })
}

/// Extract a value from a JSON object using a key path (e.g., "name" or "thumbnail.url").
fn extract_json_value(json: &serde_json::Value, key_path: &str) -> Option<String> {
    let mut current = json.clone();
    for key in key_path.split('.') {
        let next = current.get(key)?.clone();
        current = next;
    }
    match current {
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

/// Navigate to a specific path in a JSON value and return the result.
fn parse_json_value(json: &serde_json::Value, path: &str) -> serde_json::Value {
    let mut current = json.clone();
    for key in path.split('.') {
        current = current.get(key).cloned().unwrap_or(serde_json::Value::Null);
    }
    current
}

#[cfg(test)]
mod tests {
    use super::{
        JsonPathError, build_browser_fetch_js, format_from_json_parse_error,
        format_json_path_error, infer_is_extra, parse_json_path, preview,
    };
    use std::collections::HashMap;

    #[test]
    fn parse_json_path_returns_kind_and_value() {
        let body = r#"{"data":{"search":{"rows":[{"title":"Berserk"}]}}}"#;
        let parsed = parse_json_path(body, "data.search.rows").expect("json path should resolve");
        assert_eq!(parsed.kind, "array");
        assert_eq!(parsed.value, r#"[{"title":"Berserk"}]"#);
    }

    #[test]
    fn parse_json_path_reports_missing_segment() {
        let body = r#"{"data":{"search":{}}}"#;
        let err = parse_json_path(body, "data.search.rows").expect_err("path should fail");
        let detail = format_json_path_error(body, "data.search.rows", &err);
        match err {
            JsonPathError::MissingPath {
                missing_segment,
                container_kind,
                ..
            } => {
                assert_eq!(missing_segment, "rows");
                assert_eq!(container_kind, "object");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(detail.contains("could not resolve segment 'rows'"));
        assert!(detail.contains("container=object"));
    }

    #[test]
    fn from_json_parse_error_includes_source_preview() {
        let source = r#"{"oops":true}"#;
        let err = serde_json::from_str::<Vec<serde_json::Value>>(source).unwrap_err();
        let detail = format_from_json_parse_error("chapters_response", source, &err);
        assert!(detail.contains("chapters_response"));
        assert!(detail.contains(&preview(source, 160)));
    }

    #[test]
    fn browser_fetch_js_embeds_response_envelope_fields() {
        let mut headers = HashMap::new();
        headers.insert("X-Test".to_owned(), "abc".to_owned());
        let js = build_browser_fetch_js(
            "POST",
            "https://example.com/graphql",
            &headers,
            Some("{\"query\":\"{viewer{id}}\"}"),
            true,
        );
        assert!(js.contains("status_text"));
        assert!(js.contains("headersObj"));
        assert!(js.contains("credentials: 'include'"));
        assert!(js.contains("https://example.com/graphql"));
    }

    #[test]
    fn infer_is_extra_does_not_treat_side_story_as_extra() {
        assert!(!infer_is_extra(Some("Side Story 20")));
        assert!(!infer_is_extra(Some("Chapter 200 - Side Story 20")));
    }

    #[test]
    fn infer_is_extra_still_detects_real_extra_keywords() {
        for title in [
            "Chapter 12.5 Extra",
            "Bonus chapter",
            "Interlude",
            "Gaiden 3",
            "Omake",
            "Special edition",
        ] {
            assert!(
                infer_is_extra(Some(title)),
                "expected '{title}' to be extra"
            );
        }
    }
}
