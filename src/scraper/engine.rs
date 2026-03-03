use scraper::{ElementRef, Html, Selector};

use crate::scraper::{
    def::{BuildUrlDef, ChapterUrlTransform, ChaptersDef, ContentKind, ExtractDef, FieldDef, PagesDef, ProviderDef, SearchDef},
    error::ScraperError,
    {PageUrl, Provider, ProviderChapterInfo, ProviderSearchResult, ScraperCtx},
};

/// A scraping provider driven by a YAML `ProviderDef`. Implements the
/// `Provider` trait — no Rust code required to add a new site.
pub struct YamlProvider {
    pub(crate) def: ProviderDef,
}

impl YamlProvider {
    pub fn new(def: ProviderDef) -> Self {
        Self { def }
    }

    // ------------------------------------------------------------------
    // Shared helpers
    // ------------------------------------------------------------------

    /// Expand `{key}` placeholders in a URL template.
    fn expand(&self, template: &str, extra: &[(&str, &str)]) -> String {
        let mut s = template.replace("{base_url}", &self.def.base_url);
        for (k, v) in extra {
            s = s.replace(&format!("{{{}}}", k), v);
        }
        // If the result is a path (starts with /), prepend base_url.
        if s.starts_with('/') {
            format!("{}{}", self.def.base_url.trim_end_matches('/'), s)
        } else {
            s
        }
    }

    /// Fetch the HTML for a URL, using the headless browser if the provider
    /// needs JavaScript rendering, or plain HTTP otherwise.
    async fn fetch_html(&self, ctx: &ScraperCtx, url: &str) -> Result<String, ScraperError> {
        if self.def.needs_browser {
            let browser = ctx.browser.get().await?;
            let page = browser
                .new_page(url)
                .await
                .map_err(|e| ScraperError::Browser(e.to_string()))?;
            // find_element waits until the element appears in the DOM.
            page.find_element("body")
                .await
                .map_err(|e| ScraperError::Browser(e.to_string()))?;
            let html = page
                .content()
                .await
                .map_err(|e| ScraperError::Browser(e.to_string()))?;
            page.close().await.ok();
            Ok(html)
        } else {
            Ok(ctx
                .http
                .get(url)
                .header("User-Agent", "Mozilla/5.0 (compatible; Rebarr/0.1)")
                .send()
                .await?
                .text()
                .await?)
        }
    }

    // ------------------------------------------------------------------
    // Field extraction helpers (used by search + chapters)
    // ------------------------------------------------------------------

    fn extract_field(
        &self,
        element: &ElementRef,
        field: &FieldDef,
    ) -> Result<String, ScraperError> {
        // Empty selector means "use the element itself" (e.g. to read an
        // attribute from the row/card element rather than a descendant).
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

        let raw = match field.content {
            ContentKind::Text => child.text().collect::<String>().trim().to_owned(),
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
                    .ok_or_else(|| {
                        ScraperError::Parse(format!("attr '{attr_name}' not found on element"))
                    })?
                    .to_owned()
            }
        };

        // Apply value_map before prefix/URL logic.
        let raw = if let Some(mapped) = field.value_map.get(&raw) {
            mapped.clone()
        } else {
            raw
        };

        // Skip prefix when the value is already an absolute URL.
        if raw.starts_with("http://") || raw.starts_with("https://") {
            return Ok(raw);
        }
        let prefix = field.prefix.replace("{base_url}", &self.def.base_url);
        Ok(format!("{prefix}{raw}"))
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    async fn do_search(
        &self,
        ctx: &ScraperCtx,
        def: &SearchDef,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        let encoded = urlencoding::encode(title);
        let url = self.expand(&def.url, &[("query", &encoded)]);
        let html = self.fetch_html(ctx, &url).await?;
        let doc = Html::parse_document(&html);

        let card_sel = Selector::parse(&def.results.selector).map_err(|e| {
            ScraperError::Parse(format!("bad results selector: {e:?}"))
        })?;

        let mut results = Vec::new();
        for card in doc.select(&card_sel) {
            let Ok(title) = self.extract_field(&card, &def.results.fields.title) else {
                continue;
            };
            let Ok(url) = self.extract_field(&card, &def.results.fields.url) else {
                continue;
            };
            let cover_url = def
                .results
                .fields
                .cover
                .as_ref()
                .and_then(|f| self.extract_field(&card, f).ok());

            results.push(ProviderSearchResult {
                title,
                url,
                cover_url,
            });
        }

        Ok(results)
    }

    // ------------------------------------------------------------------
    // Chapters
    // ------------------------------------------------------------------

    async fn do_chapters(
        &self,
        ctx: &ScraperCtx,
        def: &ChaptersDef,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        let url = if let Some(transform) = &def.url_transform {
            chapter_list_url(manga_url, transform)?
        } else if let Some(tmpl) = &def.url {
            self.expand(tmpl, &[("manga_url", manga_url)])
        } else {
            return Err(ScraperError::Parse(
                "chapters config must have either 'url' or 'url_transform'".to_owned(),
            ));
        };
        let html = self.fetch_html(ctx, &url).await?;
        let doc = Html::parse_document(&html);

        let row_sel = Selector::parse(&def.list.selector)
            .map_err(|e| ScraperError::Parse(format!("bad chapter list selector: {e:?}")))?;

        let mut chapters = Vec::new();
        for row in doc.select(&row_sel) {
            let Ok(raw_number) = self.extract_field(&row, &def.list.fields.number_raw) else {
                continue;
            };
            let number: f32 = raw_number
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);

            let title = def
                .list
                .fields
                .title
                .as_ref()
                .and_then(|f| self.extract_field(&row, f).ok())
                .filter(|s| !s.is_empty());

            let url = self
                .extract_field(&row, &def.list.fields.url)
                .ok()
                .filter(|s| !s.is_empty());

            let scanlator_group = def
                .list
                .fields
                .scanlator_group
                .as_ref()
                .and_then(|f| self.extract_field(&row, f).ok())
                .filter(|s| !s.is_empty());


            chapters.push(ProviderChapterInfo {
                raw_number,
                number,
                title,
                url,
                volume: None,
                scanlator_group,
            });
        }

        chapters.sort_by(|a, b| a.number.partial_cmp(&b.number).unwrap_or(std::cmp::Ordering::Equal));
        Ok(chapters)
    }

    // ------------------------------------------------------------------
    // Pages
    // ------------------------------------------------------------------

    async fn do_pages(
        &self,
        ctx: &ScraperCtx,
        def: &PagesDef,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        let fetch_url = match &def.url {
            Some(tmpl) => self.expand(tmpl, &[("chapter_url", chapter_url)]),
            None => chapter_url.to_owned(),
        };

        // Lua script takes priority over declarative rules.
        if let Some(script) = &def.script {
            let html = self.fetch_html(ctx, &fetch_url).await?;
            let base_url = self.def.base_url.clone();
            return run_lua_script(script, html, base_url).await;
        }

        let extract = def
            .extract
            .as_ref()
            .ok_or(ScraperError::Unsupported)?;

        let html = self.fetch_html(ctx, &fetch_url).await?;

        match extract {
            ExtractDef::CssAttr { selector, attr, prefix } => {
                let doc = Html::parse_document(&html);
                let sel = Selector::parse(selector)
                    .map_err(|e| ScraperError::Parse(format!("{e:?}")))?;
                let pages = doc
                    .select(&sel)
                    .enumerate()
                    .filter_map(|(i, el)| {
                        el.value().attr(attr.as_str()).map(|v| PageUrl {
                            url: format!(
                                "{}{}",
                                prefix.replace("{base_url}", &self.def.base_url),
                                v
                            ),
                            index: (i + 1) as u32,
                        })
                    })
                    .collect();
                Ok(pages)
            }

            ExtractDef::CssText { selector, prefix } => {
                let doc = Html::parse_document(&html);
                let sel = Selector::parse(selector)
                    .map_err(|e| ScraperError::Parse(format!("{e:?}")))?;
                let pages = doc
                    .select(&sel)
                    .enumerate()
                    .map(|(i, el)| PageUrl {
                        url: format!(
                            "{}{}",
                            prefix.replace("{base_url}", &self.def.base_url),
                            el.text().collect::<String>().trim()
                        ),
                        index: (i + 1) as u32,
                    })
                    .collect();
                Ok(pages)
            }

            ExtractDef::ScriptJson {
                selector,
                after,
                trim,
                remove_suffix,
                json_path,
                build_url,
            } => {
                let doc = Html::parse_document(&html);
                let sel = Selector::parse(selector)
                    .map_err(|e| ScraperError::Parse(format!("{e:?}")))?;
                let script_el = doc.select(&sel).next().ok_or(ScraperError::NotFound)?;
                let mut text = script_el.text().collect::<String>();

                if let Some(after_str) = after {
                    if let Some(pos) = text.find(after_str.as_str()) {
                        text = text[pos + after_str.len()..].to_owned();
                    }
                }
                if *trim {
                    text = text.trim().to_owned();
                }
                if let Some(suffix) = remove_suffix {
                    if text.ends_with(suffix.as_str()) {
                        text.truncate(text.len() - suffix.len());
                    }
                }

                let mut json: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| ScraperError::Parse(format!("JSON parse error: {e}")))?;

                if let Some(path) = json_path {
                    json = apply_json_path(json, path);
                }

                let urls = json_array_to_urls(&json)?;
                Ok(urls
                    .into_iter()
                    .enumerate()
                    .map(|(i, url)| PageUrl {
                        url: apply_build_url(build_url.as_ref(), &url, &self.def.base_url),
                        index: (i + 1) as u32,
                    })
                    .collect())
            }

            ExtractDef::ApiJson { url_template, json_path, build_url } => {
                let url = self.expand(url_template, &[("chapter_url", chapter_url)]);
                let resp = ctx.http.get(&url).send().await?.text().await?;
                let mut json: serde_json::Value = serde_json::from_str(&resp)
                    .map_err(|e| ScraperError::Parse(format!("API JSON parse error: {e}")))?;

                if let Some(path) = json_path {
                    json = apply_json_path(json, path);
                }

                let urls = json_array_to_urls(&json)?;
                Ok(urls
                    .into_iter()
                    .enumerate()
                    .map(|(i, url)| PageUrl {
                        url: apply_build_url(build_url.as_ref(), &url, &self.def.base_url),
                        index: (i + 1) as u32,
                    })
                    .collect())
            }
        }
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
        self.def.needs_browser
    }

    fn rate_limit_rpm(&self) -> u32 {
        self.def.rate_limit.requests_per_minute
    }

    async fn search(
        &self,
        ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        match &self.def.search {
            Some(def) => self.do_search(ctx, def, title).await,
            None => Err(ScraperError::Unsupported),
        }
    }

    async fn chapters(
        &self,
        ctx: &ScraperCtx,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        match &self.def.chapters {
            Some(def) => self.do_chapters(ctx, def, manga_url).await,
            None => Err(ScraperError::Unsupported),
        }
    }

    async fn pages(
        &self,
        ctx: &ScraperCtx,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        match &self.def.pages {
            Some(def) => self.do_pages(ctx, def, chapter_url).await,
            None => Err(ScraperError::Unsupported),
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Build a chapter-list URL by stripping N segments from the end of the series
/// URL path and appending a new segment.
///
/// `/series/ID/Manga-Title` + strip 1 + append "full-chapter-list"
/// → `/series/ID/full-chapter-list`
fn chapter_list_url(series_url: &str, t: &ChapterUrlTransform) -> Result<String, ScraperError> {
    // Split at the start of the path (after scheme://host).
    let path_start = series_url
        .find("://")
        .and_then(|i| series_url[i + 3..].find('/').map(|j| i + 3 + j))
        .ok_or_else(|| ScraperError::Parse(format!("url_transform: no path in '{series_url}'")))?;

    let origin = &series_url[..path_start];
    let path = series_url[path_start..].trim_end_matches('/');
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let n = t.strip_last_segments;
    if n > segments.len() {
        return Err(ScraperError::Parse(format!(
            "url_transform: strip_last_segments={n} but path only has {} segments",
            segments.len()
        )));
    }
    segments.truncate(segments.len() - n);
    segments.push(&t.append);

    Ok(format!("{}/{}", origin, segments.join("/")))
}

/// Navigate a JSON value along a dotted key path, consuming each level.
fn apply_json_path(mut json: serde_json::Value, path: &str) -> serde_json::Value {
    for key in path.split('.') {
        json = json[key].take();
    }
    json
}

/// Extract a flat `Vec<String>` from a JSON value that must be an array of strings.
fn json_array_to_urls(json: &serde_json::Value) -> Result<Vec<String>, ScraperError> {
    match json {
        serde_json::Value::Array(arr) => Ok(arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()),
        _ => Err(ScraperError::Parse(
            "expected JSON array of URL strings".to_owned(),
        )),
    }
}

fn apply_build_url(rule: Option<&BuildUrlDef>, value: &str, base_url: &str) -> String {
    match rule {
        None => value.to_owned(),
        Some(r) => {
            let template = if value.starts_with(&r.if_starts_with) {
                &r.then
            } else {
                &r.r#else
            };
            template
                .replace("{value}", value)
                .replace("{base_url}", base_url)
        }
    }
}

// ---------------------------------------------------------------------------
// Lua script execution
// ---------------------------------------------------------------------------

/// Run an inline Lua script that returns an array of `{url, index}` tables.
///
/// Executes in `spawn_blocking` so the async runtime is not blocked.
async fn run_lua_script(
    script: &str,
    html: String,
    base_url: String,
) -> Result<Vec<PageUrl>, ScraperError> {
    let script = script.to_owned();

    tokio::task::spawn_blocking(move || run_lua_blocking(&script, &html, &base_url))
        .await
        .map_err(|e| ScraperError::Script(format!("spawn_blocking panic: {e}")))?
}

fn run_lua_blocking(script: &str, html: &str, base_url: &str) -> Result<Vec<PageUrl>, ScraperError> {
    let lua = mlua::Lua::new();

    // -- select(html_string, css_selector) -> array of element tables
    // Each element table: { text = "...", attrs = { key = value, ... } }
    let select_fn = lua
        .create_function(|lua_ctx, (html_str, selector): (String, String)| {
            let doc = Html::parse_document(&html_str);
            let sel = Selector::parse(&selector)
                .map_err(|e| mlua::Error::RuntimeError(format!("bad selector '{selector}': {e:?}")))?;

            let result = lua_ctx.create_table()?;
            for (i, el) in doc.select(&sel).enumerate() {
                let el_table = lua_ctx.create_table()?;
                el_table.set("text", el.text().collect::<String>().trim().to_owned())?;
                el_table.set("inner_html", el.inner_html())?;

                let attrs_table = lua_ctx.create_table()?;
                for (name, value) in el.value().attrs() {
                    attrs_table.set(name, value)?;
                }
                el_table.set("attrs", attrs_table)?;
                result.set(i + 1, el_table)?;
            }
            Ok(result)
        })
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("select", select_fn)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // -- attr(element, name) -> string  (convenience wrapper)
    let attr_fn = lua
        .create_function(|_, (element, name): (mlua::Table, String)| {
            let attrs: mlua::Table = element.get("attrs")?;
            let val: Option<String> = attrs.get(name)?;
            Ok(val.unwrap_or_default())
        })
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("attr", attr_fn)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // -- text(element) -> string  (convenience wrapper)
    let text_fn = lua
        .create_function(|_, element: mlua::Table| {
            let t: String = element.get("text")?;
            Ok(t)
        })
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("text", text_fn)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // -- json_decode(str) -> table
    let json_decode_fn = lua
        .create_function(|lua_ctx, json_str: String| {
            let value: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| mlua::Error::RuntimeError(format!("json_decode: {e}")))?;
            json_to_lua(lua_ctx, &value)
        })
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("json_decode", json_decode_fn)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // -- url_join(base, path) -> string
    let url_join_fn = lua
        .create_function(|_, (base, path): (String, String)| {
            if path.starts_with("http://") || path.starts_with("https://") {
                Ok(path)
            } else {
                Ok(format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    path.trim_start_matches('/')
                ))
            }
        })
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("url_join", url_join_fn)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // Set the input globals.
    lua.globals()
        .set("html", html)
        .map_err(|e| ScraperError::Script(e.to_string()))?;
    lua.globals()
        .set("base_url", base_url)
        .map_err(|e| ScraperError::Script(e.to_string()))?;

    // Execute the script; it must return an array of {url, index} tables.
    let result: mlua::Value = lua
        .load(script)
        .eval()
        .map_err(|e| ScraperError::Script(format!("Lua error: {e}")))?;

    let table = match result {
        mlua::Value::Table(t) => t,
        _ => return Err(ScraperError::Script("script must return a table".to_owned())),
    };

    let mut pages = Vec::new();
    let mut i = 1i64;
    loop {
        match table
            .get::<mlua::Value>(i)
            .map_err(|e| ScraperError::Script(e.to_string()))?
        {
            mlua::Value::Table(entry) => {
                let url: String = entry
                    .get("url")
                    .map_err(|e| ScraperError::Script(format!("entry missing 'url': {e}")))?;
                let index: u32 = entry
                    .get("index")
                    .map_err(|e| ScraperError::Script(format!("entry missing 'index': {e}")))?;
                pages.push(PageUrl { url, index });
                i += 1;
            }
            mlua::Value::Nil => break,
            _ => {
                return Err(ScraperError::Script(
                    "each entry in the return table must be a table".to_owned(),
                ))
            }
        }
    }

    Ok(pages)
}

/// Recursively convert a `serde_json::Value` into a Lua value.
fn json_to_lua(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    match value {
        serde_json::Value::Null => Ok(mlua::Value::Nil),
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else {
                Ok(mlua::Value::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
        serde_json::Value::Object(obj) => {
            let t = lua.create_table()?;
            for (k, v) in obj {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
    }
}
