use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};

static DUMP_COUNTER: AtomicU32 = AtomicU32::new(0);

use tracing::{debug, info, warn};
use scraper::{ElementRef, Html, Selector};

use crate::scraper::{
    def::{
        ActionDef, ContentKind, FetchDef, FieldDef, FilterJsonDef, ForeachDef, FromJsonDef,
        GraphqlDef, InterceptDef, ProviderDef, StepDef,
    },
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
        let mut changed = true;
        while changed {
            changed = false;
            // Find patterns like {varname|modifier}
            let re = regex::Regex::new(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\|([^}]+)\}").unwrap();
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
            tracing::trace!(step = ?std::mem::discriminant(step), "executing step");
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
                        if let Ok(html) = p.content().await {
                            let is_challenge = is_cf_challenge(&html);
                            if is_challenge {
                                if !cloudflare_detected {
                                    debug!(
                                        "Cloudflare challenge detected at {url}, waiting for auto-bypass..."
                                    );
                                    cloudflare_detected = true;
                                }

                                // Some CF flows require a user-triggered checkbox click.
                                // Nudge likely controls periodically while challenge HTML remains.
                                let should_try_click = last_cf_click_attempt
                                    .map(|t| t.elapsed() >= Duration::from_secs(2))
                                    .unwrap_or(true);
                                if should_try_click {
                                    if try_cf_checkbox_click(p, ctx.verbose).await {
                                        debug!(
                                            "Cloudflare checkbox click attempt triggered at {url}"
                                        );
                                    }
                                    last_cf_click_attempt = Some(std::time::Instant::now());
                                }
                            } else if cloudflare_detected {
                                // Challenge was present but page has loaded — bypassed!
                                info!("Cloudflare challenge auto-bypassed at {url}");
                                break;
                            } else if elapsed >= min_settle_time {
                                // No CF challenge and minimum settle time passed —
                                // wait for network to become idle to let JS API calls complete
                                let remaining_ms =
                                    timeout.saturating_sub(elapsed).as_millis() as u64;
                                p.wait_for_network_idle(1000, remaining_ms).await.ok();
                                break;
                            }
                        }

                        tokio::time::sleep(poll_interval).await;
                    }

                    // Small delay to let page JS finish processing after API calls complete.
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    // Final check: if Cloudflare challenge still present, fail
                    if let Ok(html) = p.content().await {
                        if is_cf_challenge(&html) {
                            if let Some(ref p) = page {
                                let _ = browser.close_tab(p.target_id()).await;
                            }
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
                            // Close the tab before returning — otherwise it stays open in Chrome.
                            if let Some(ref p) = page {
                                let _ = browser.close_tab(p.target_id()).await;
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

                StepDef::Fetch { fetch: fetch_def } => {
                    let p = require_page(&page, "fetch")?;

                    if let Some(ref pagination) = fetch_def.pagination {
                        // Handle paginated fetch
                        let mut all_items: Vec<serde_json::Value> = Vec::new();
                        let mut current_page = pagination.start_page;
                        let mut last_page = pagination.max_pages;

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

                            if ctx.verbose {
                                eprintln!("[step] fetch (page {current_page}) → url={url}");
                            }

                            let method = fetch_def.method.to_uppercase();

                            // Build headers object
                            let mut headers_js = String::new();
                            for (key, val) in &fetch_def.headers {
                                let expanded_val = self.expand(val, &vars);
                                headers_js.push_str(&format!(
                                    "'{}': '{}',",
                                    key,
                                    expanded_val.replace('\'', "\\\'")
                                ));
                            }

                            // Build body if present - pass as raw string
                            let body_js = fetch_def
                                .body
                                .as_ref()
                                .map(|b| self.expand(b, &vars))
                                .map(|b| format!(", body: `{}`", b.replace('`', "\\`")))
                                .unwrap_or_default();

                            let js = format!(
                                r#"
                                (async () => {{
                                    const headers = {{{}}};
                                    const opts = {{
                                        method: '{}',
                                        headers: headers{}
                                    }};
                                    try {{
                                        const resp = await fetch('{}', opts);
                                        return await resp.text();
                                    }} catch(e) {{
                                        return 'ERROR:' + e.message;
                                    }}
                                }})()
                            "#,
                                headers_js,
                                method,
                                body_js,
                                url.replace('\'', "\\\'")
                            );

                            match p.evaluate::<String>(&js).await {
                                Ok(response) => {
                                    if response.starts_with("ERROR:") {
                                        warn!("[step] fetch failed: {}", response);
                                        break;
                                    }

                                    // Parse response
                                    if let Ok(json) =
                                        serde_json::from_str::<serde_json::Value>(&response)
                                    {
                                    // Extract pagination metadata if configured
                                    if let Some(ref meta_path) = pagination.meta_path {
                                        let meta = parse_json_value(&json, meta_path);
                                        if pagination.calculate_last_page {
                                            // Calculate last_page from total and limit
                                            let total = meta
                                                .get(&pagination.total_field)
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);
                                            let limit = meta
                                                .get(&pagination.per_page_field)
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(100);
                                            if limit > 0 {
                                                last_page = ((total + limit - 1) / limit) as u32;
                                            }
                                        } else if let Some(last_page_val) =
                                            meta.get(&pagination.last_page_field)
                                        {
                                            if let Some(lp) = last_page_val.as_u64() {
                                                last_page = lp as u32;
                                            }
                                        }
                                    }

                                        // Extract data items
                                        let data_path =
                                            fetch_def.json_path.as_deref().unwrap_or("");
                                        let items = if data_path.is_empty() {
                                            json.as_array().cloned().unwrap_or_default()
                                        } else {
                                            parse_json_value(&json, data_path)
                                                .as_array()
                                                .cloned()
                                                .unwrap_or_default()
                                        };

                                        if items.is_empty() {
                                            if ctx.verbose {
                                                eprintln!(
                                                    "[step] fetch (page {current_page}) → empty response, stopping"
                                                );
                                            }
                                            break;
                                        }

                                        let items_count = items.len();
                                        all_items.extend(items);

                                        if ctx.verbose {
                                            eprintln!(
                                                "[step] fetch (page {current_page}) → {items_count} items (total: {})",
                                                all_items.len()
                                            );
                                        }
                                    } else {
                                        warn!("[step] fetch response is not valid JSON");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("[step] fetch execute failed: {e}");
                                    break;
                                }
                            }

                            // Check if we've reached the last page
                            if current_page >= last_page {
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] fetch → reached last page ({last_page}), stopping"
                                    );
                                }
                                break;
                            }

                            current_page += 1;
                        }

                        // Store accumulated results
                        let value =
                            serde_json::to_string(&all_items).unwrap_or_else(|_| "[]".to_string());
                        if ctx.verbose {
                            eprintln!(
                                "[step] fetch pagination complete → {} total items stored in '{}'",
                                all_items.len(),
                                fetch_def.var
                            );
                        }
                        vars.insert(fetch_def.var.clone(), value);
                    } else {
                        // Non-paginated fetch (original behavior)
                        let url = self.expand(&fetch_def.url, &vars);
                        if ctx.verbose {
                            eprintln!("[step] fetch → url={url}");
                        }
                        let method = fetch_def.method.to_uppercase();

                        // Build headers object
                        let mut headers_js = String::new();
                        for (key, val) in &fetch_def.headers {
                            let expanded_val = self.expand(val, &vars);
                            headers_js.push_str(&format!(
                                "'{}': '{}',",
                                key,
                                expanded_val.replace('\'', "\\\'")
                            ));
                        }

                        // Build body if present - pass as raw string
                        let body_js = fetch_def
                            .body
                            .as_ref()
                            .map(|b| self.expand(b, &vars))
                            .map(|b| format!(", body: `{}`", b.replace('`', "\\`")))
                            .unwrap_or_default();

                        let js = format!(
                            r#"
                            (async () => {{
                                const headers = {{{}}};
                                const opts = {{
                                    method: '{}',
                                    headers: headers{}
                                }};
                                try {{
                                    const resp = await fetch('{}', opts);
                                    return await resp.text();
                                }} catch(e) {{
                                    return 'ERROR:' + e.message;
                                }}
                            }})()
                        "#,
                            headers_js,
                            method,
                            body_js,
                            url.replace('\'', "\\\'")
                        );

                        match p.evaluate::<String>(&js).await {
                            Ok(response) => {
                                if response.starts_with("ERROR:") {
                                    warn!("[step] fetch failed: {}", response);
                                } else {
                                    let value = if let Some(ref path) = fetch_def.json_path {
                                        parse_json_path(&response, path)
                                    } else {
                                        response
                                    };
                                    if ctx.verbose {
                                        let preview = &value[..value.len().min(120)];
                                        eprintln!(
                                            "[step] fetch stored in '{}': {}",
                                            fetch_def.var, preview
                                        );
                                    }
                                    vars.insert(fetch_def.var.clone(), value);
                                }
                            }
                            Err(e) => {
                                warn!("[step] fetch execute failed: {e}");
                            }
                        }
                    }
                }

                StepDef::Graphql {
                    graphql: graphql_def,
                } => {
                    let p = require_page(&page, "graphql")?;

                    let url = self.expand(&graphql_def.url, &vars);
                    if ctx.verbose {
                        eprintln!("[step] graphql → url={url}");
                    }

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
                    let mut headers_js = String::from("'Content-Type': 'application/json',");
                    for (key, val) in &graphql_def.headers {
                        let expanded_val = self.expand(val, &vars);
                        headers_js.push_str(&format!(
                            "'{}': '{}',",
                            key,
                            expanded_val.replace('\'', "\\\'")
                        ));
                    }

                    let js = format!(
                        r#"
                        (async () => {{
                            const opts = {{
                                method: 'POST',
                                headers: {{ {} }},
                                credentials: 'include',
                                body: JSON.stringify({{
                                    query: '{}',
                                    variables: {}
                                }})
                            }};
                            try {{
                                const resp = await fetch('{}', opts);
                                return await resp.text();
                            }} catch(e) {{
                                return 'ERROR:' + e.message;
                            }}
                        }})()
                    "#,
                        headers_js,
                        query_escaped,
                        vars_json,
                        url.replace('\'', "\\'")
                    );

                    match p.evaluate::<String>(&js).await {
                        Ok(response) => {
                            if response.starts_with("ERROR:") {
                                warn!("[step] graphql failed: {}", response);
                            } else {
                                if ctx.verbose {
                                    debug!(
                                        "[step] graphql raw response ({} bytes): {}",
                                        response.len(),
                                        &response[..response.len().min(500)]
                                    );
                                }
                                let value = if let Some(ref path) = graphql_def.json_path {
                                    parse_json_path(&response, path)
                                } else {
                                    response
                                };
                                if ctx.verbose {
                                    let preview = &value[..value.len().min(120)];
                                    debug!(
                                        "[step] graphql stored in '{}': {}",
                                        graphql_def.var, preview
                                    );
                                }
                                vars.insert(graphql_def.var.clone(), value);
                            }
                        }
                        Err(e) => {
                            warn!("[step] graphql execute failed: {e}");
                        }
                    }
                }

                StepDef::FromJson {
                    from_json: from_json_def,
                } => {
                    if ctx.verbose {
                        eprintln!("[step] from_json → var={}", from_json_def.var);
                    }

                    let json_str = vars.get(&from_json_def.var).ok_or_else(|| {
                        ScraperError::Parse(format!(
                            "from_json: variable '{}' not found",
                            from_json_def.var
                        ))
                    })?;

                    let json_array: Vec<serde_json::Value> = serde_json::from_str(json_str)
                        .map_err(|e| {
                            ScraperError::Parse(format!("from_json: failed to parse JSON: {e}"))
                        })?;

                    for item in json_array {
                        // Apply filter if configured
                        if let Some(ref filter) = from_json_def.filter {
                            let field_value = extract_json_value(&item, &filter.field);
                            let has_field = field_value.is_some()
                                && field_value.as_deref() != Some("null")
                                && !field_value.as_deref().unwrap_or("").is_empty();
                            // Skip record if filter condition matches
                            if filter.exists && has_field {
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] from_json → filtered out record with field '{}'",
                                        filter.field
                                    );
                                }
                                continue;
                            }
                            if !filter.exists && !has_field {
                                if ctx.verbose {
                                    eprintln!(
                                        "[step] from_json → filtered out record missing field '{}'",
                                        filter.field
                                    );
                                }
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
                                } else if let Some(prefix) =
                                    from_json_def.prefix.get(output_key)
                                {
                                    // Apply prefix if not absolute URL
                                    let expanded_prefix = self.expand(prefix, &vars);
                                    if val.starts_with("http://") || val.starts_with("https://") {
                                        val
                                    } else {
                                        format!("{}{}", expanded_prefix, val)
                                    }
                                } else {
                                    val
                                };
                                record.insert(output_key.clone(), final_val);
                            }
                        }
                        if !record.is_empty() {
                            results.push(record);
                        }
                    }

                    if ctx.verbose {
                        eprintln!("[step] from_json → {} records extracted", results.len());
                    }
                }

                StepDef::FilterJson {
                    filter_json: filter_def,
                } => {
                    if ctx.verbose {
                        eprintln!("[step] filter_json → var={}", filter_def.var);
                    }

                    let json_str = vars.get(&filter_def.var).ok_or_else(|| {
                        ScraperError::Parse(format!(
                            "filter_json: variable '{}' not found",
                            filter_def.var
                        ))
                    })?;

                    let mut json_array: Vec<serde_json::Value> = serde_json::from_str(json_str)
                        .map_err(|e| {
                            ScraperError::Parse(format!(
                                "filter_json: failed to parse JSON: {e}"
                            ))
                        })?;

                    let original_count = json_array.len();
                    let condition = &filter_def.condition;

                    json_array.retain(|item| {
                        let field_value = extract_json_value(item, &condition.field);
                        let has_field = field_value.is_some()
                            && field_value.as_deref() != Some("null")
                            && !field_value.as_deref().unwrap_or("").is_empty();

                        // Keep record if filter condition does NOT match
                        if condition.exists && has_field {
                            false // Remove records where field exists
                        } else if !condition.exists && !has_field {
                            false // Remove records where field does not exist
                        } else {
                            true // Keep the record
                        }
                    });

                    let filtered_count = original_count - json_array.len();
                    if ctx.verbose {
                        eprintln!(
                            "[step] filter_json → removed {} records ({} remaining)",
                            filtered_count,
                            json_array.len()
                        );
                    }

                    // Store filtered array back
                    let value = serde_json::to_string(&json_array)
                        .unwrap_or_else(|_| "[]".to_string());
                    vars.insert(filter_def.var.clone(), value);
                }
            }
        }

        // Close the Chrome tab explicitly before dropping the Rust Page handle.
        // Dropping Page only decrements Arc<Transport>; without this, the tab
        // stays open in Chrome and accumulates over thousands of scrape calls.
        if let Some(ref p) = page {
            let _ = browser.close_tab(p.target_id()).await;
        }
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

    fn default_score(&self) -> i32 {
        self.def.default_score
    }

    fn rate_limit_rpm(&self) -> u32 {
        self.def.rate_limit.requests_per_minute
    }

    fn page_delay_ms(&self) -> u64 {
        self.def.rate_limit.page_delay_ms
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
fn is_cf_challenge(html: &str) -> bool {
    html.contains("cf-browser-verification")
        || html.contains("__cf_chl")
        || (html.contains("Just a moment") && html.contains("cloudflare"))
}

/// Attempt to click likely Cloudflare challenge controls (checkbox/turnstile widgets).
///
/// Returns true if a click attempt was dispatched.
async fn try_cf_checkbox_click(page: &eoka::Page, verbose: bool) -> bool {
    // First try direct DOM selectors that might exist in same-origin challenge pages.
    let click_selectors = [
        "input[type='checkbox']",
        "button[type='submit']",
        "label.ctp-checkbox-label",
        "iframe[title*='challenge']",
        "iframe[title*='Cloudflare']",
        "iframe[title*='Turnstile']",
        "iframe[src*='challenges.cloudflare.com']",
    ];

    for sel in click_selectors {
        if page.human_click(sel).await.is_ok() {
            if verbose {
                eprintln!("[cf] clicked selector '{sel}'");
            }
            return true;
        }
    }

    // Fallback: click the center point of likely challenge frames/elements via JS.
    let js = r#"(function() {
        const candidates = [
          "iframe[title*='challenge']",
          "iframe[title*='Cloudflare']",
          "iframe[title*='Turnstile']",
          "iframe[src*='challenges.cloudflare.com']",
          "input[type='checkbox']",
          "[role='checkbox']"
        ];
        for (const sel of candidates) {
          const el = document.querySelector(sel);
          if (!el) continue;
          const rect = el.getBoundingClientRect();
          if (!rect || rect.width <= 0 || rect.height <= 0) continue;
          const x = Math.floor(rect.left + rect.width / 2);
          const y = Math.floor(rect.top + rect.height / 2);
          const target = document.elementFromPoint(x, y) || el;
          const events = ["mousemove", "mousedown", "mouseup", "click"];
          for (const type of events) {
            target.dispatchEvent(new MouseEvent(type, {
              bubbles: true,
              cancelable: true,
              view: window,
              clientX: x,
              clientY: y,
              button: 0
            }));
          }
          if (typeof target.click === "function") target.click();
          return true;
        }
        return false;
    })()"#;

    match page.evaluate::<bool>(js).await {
        Ok(true) => {
            if verbose {
                eprintln!("[cf] clicked via JS fallback");
            }
            true
        }
        _ => false,
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

/// Parse a JSON string and navigate to a specific path, returning the result as a JSON string.
fn parse_json_path(body: &str, json_path: &str) -> String {
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
        for key in json_path.split('.') {
            json = json[key].take();
        }
        // If the result is a JSON string (not an object/array), return the original string value
        // to preserve escaped content like nested JSON strings
        if let serde_json::Value::String(s) = &json {
            return s.clone();
        }
        return json.to_string();
    }
    body.to_owned()
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
