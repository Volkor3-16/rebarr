use std::path::PathBuf;

use chrono::Utc;
use rocket::{
    form::Form,
    get, post,
    response::{content::RawHtml, Redirect},
    routes, State,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db,
    manga::{Library, Manga, MangaMetadata, MangaSource, MangaType, PublishingStatus},
    metadata::anilist::ALClient,
};

// ---------------------------------------------------------------------------
// HTML helpers
// ---------------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: String) -> RawHtml<String> {
    RawHtml(format!(
        r#"<!DOCTYPE html>
<html>
<head><title>{title} - REBARR</title></head>
<body>
<pre>+================================================+
| REBARR -- Manga Library Manager                |
+================================================+</pre>
<a href="/">[Home]</a> |
<a href="/search">[Search Manga]</a> |
<a href="/library/new">[Add Library]</a>
<hr>
<h2>{title}</h2>
{body}
</body>
</html>"#
    ))
}

fn error_page(msg: &str) -> RawHtml<String> {
    page(
        "ERROR",
        format!(
            "<p><b>!! ERROR !!</b><br>{}</p><p><a href=\"/\">[Back to Home]</a></p>",
            html_escape(msg)
        ),
    )
}

// ---------------------------------------------------------------------------
// GET / -- home: list libraries
// ---------------------------------------------------------------------------

#[get("/")]
async fn index(pool: &State<SqlitePool>) -> RawHtml<String> {
    let libs = match db::library::get_all(pool.inner()).await {
        Ok(l) => l,
        Err(e) => return error_page(&e.to_string()),
    };

    let mut body = String::new();

    if libs.is_empty() {
        body.push_str("<p>No libraries configured yet.</p>\n");
    } else {
        body.push_str("<p>Libraries:</p>\n<ul>\n");
        for lib in &libs {
            let type_str = match lib.r#type {
                MangaType::Manga => "Manga",
                MangaType::Comics => "Comics",
            };
            body.push_str(&format!(
                "<li>[{type_str}] <a href=\"/library/{uuid}\">{path}</a></li>\n",
                uuid = lib.uuid,
                path = html_escape(&lib.root_path.to_string_lossy()),
            ));
        }
        body.push_str("</ul>\n");
    }

    body.push_str("<p><a href=\"/library/new\">[+ Add Library]</a></p>\n");
    page("Home", body)
}

// ---------------------------------------------------------------------------
// GET /library/new -- add library form
// ---------------------------------------------------------------------------

#[get("/library/new")]
fn library_new_form() -> RawHtml<String> {
    let body = r#"<form method="POST" action="/library/new">
<table>
<tr>
  <td>Library Type:</td>
  <td>
    <select name="library_type">
      <option value="Manga">Manga</option>
      <option value="Comics">Comics (Western)</option>
    </select>
  </td>
</tr>
<tr>
  <td>Root Path:</td>
  <td><input type="text" name="root_path" size="50" placeholder="/data/manga"></td>
</tr>
</table>
<br>
<input type="submit" value="[Add Library]">
<a href="/">[Cancel]</a>
</form>"#;

    page("Add Library", body.to_string())
}

// ---------------------------------------------------------------------------
// POST /library/new
// ---------------------------------------------------------------------------

#[derive(rocket::FromForm)]
struct LibraryForm {
    library_type: String,
    root_path: String,
}

#[post("/library/new", data = "<form>")]
async fn library_new_post(
    pool: &State<SqlitePool>,
    form: Form<LibraryForm>,
) -> Result<Redirect, RawHtml<String>> {
    let r#type = match form.library_type.as_str() {
        "Comics" => MangaType::Comics,
        _ => MangaType::Manga,
    };

    if form.root_path.trim().is_empty() {
        return Err(error_page("Root path cannot be empty."));
    }

    let lib = Library {
        uuid: Uuid::new_v4(),
        r#type,
        root_path: PathBuf::from(form.root_path.trim()),
    };

    db::library::insert(pool.inner(), &lib)
        .await
        .map_err(|e| error_page(&e.to_string()))?;

    Ok(Redirect::to("/"))
}

// ---------------------------------------------------------------------------
// GET /library/<uuid> -- view library contents
// ---------------------------------------------------------------------------

#[get("/library/<uuid>")]
async fn library_view(pool: &State<SqlitePool>, uuid: &str) -> RawHtml<String> {
    let id = match Uuid::parse_str(uuid) {
        Ok(id) => id,
        Err(_) => return error_page("Invalid library ID."),
    };

    let lib = match db::library::get_by_id(pool.inner(), id).await {
        Ok(Some(l)) => l,
        Ok(None) => return error_page("Library not found."),
        Err(e) => return error_page(&e.to_string()),
    };

    let manga_list = match db::manga::get_all_for_library(pool.inner(), id).await {
        Ok(m) => m,
        Err(e) => return error_page(&e.to_string()),
    };

    let type_str = match lib.r#type {
        MangaType::Manga => "Manga",
        MangaType::Comics => "Comics",
    };

    let mut body = format!(
        "<pre>Path : {}\nType : {}</pre>\n",
        html_escape(&lib.root_path.to_string_lossy()),
        type_str
    );

    if manga_list.is_empty() {
        body.push_str("<p>No manga in this library yet.</p>\n");
    } else {
        body.push_str(&format!("<p>{} series:</p>\n<ul>\n", manga_list.len()));
        for m in &manga_list {
            let year = m
                .metadata
                .start_year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "?".to_string());
            let downloaded = m.downloaded_count.unwrap_or(0);
            let total = m
                .chapter_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string());

            body.push_str(&format!(
                "<li><a href=\"/manga/{id}\">{title}</a> ({year}) -- {downloaded}/{total} ch.</li>\n",
                id = m.id,
                title = html_escape(&m.metadata.title),
            ));
        }
        body.push_str("</ul>\n");
    }

    body.push_str("<p><a href=\"/search\">[+ Search and Add Manga]</a></p>\n");
    page(&lib.root_path.to_string_lossy(), body)
}

// ---------------------------------------------------------------------------
// GET /search?<q> -- AniList search
// ---------------------------------------------------------------------------

#[get("/search?<q>")]
async fn search(al: &State<ALClient>, q: Option<String>) -> RawHtml<String> {
    let q_val = q.as_deref().unwrap_or("").trim().to_string();

    let mut body = format!(
        r#"<form method="GET" action="/search">
<input type="text" name="q" size="40" placeholder="Search for manga..." value="{}">
<input type="submit" value="[Search]">
</form>
<hr>
"#,
        html_escape(&q_val)
    );

    if !q_val.is_empty() {
        match al.search_manga(&q_val).await {
            Ok(results) => {
                if results.data.is_empty() {
                    body.push_str("<p>No results found.</p>\n");
                } else {
                    body.push_str(&format!(
                        "<p>Results for &quot;{}&quot;:</p>\n",
                        html_escape(&q_val)
                    ));
                    for media in &results.data {
                        let id = media.id.unwrap_or(0);
                        let title = media
                            .title
                            .as_ref()
                            .and_then(|t| t.english.as_deref().or(t.romaji.as_deref()))
                            .unwrap_or("Unknown Title");
                        let romaji = media
                            .title
                            .as_ref()
                            .and_then(|t| t.romaji.as_deref())
                            .unwrap_or("");
                        let year = media
                            .start_date
                            .as_ref()
                            .and_then(|d| d.year)
                            .map(|y| y.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        let status = media
                            .status
                            .as_ref()
                            .map(|s| format!("{s:?}"))
                            .unwrap_or_else(|| "Unknown".to_string());
                        let thumb = media
                            .cover_image
                            .as_ref()
                            .and_then(|c| c.medium.as_deref().or(c.large.as_deref()));

                        body.push_str("<table><tr>\n");
                        if let Some(url) = thumb {
                            body.push_str(&format!(
                                "<td><img src=\"{}\" width=\"60\" alt=\"cover\"></td>\n",
                                html_escape(url)
                            ));
                        }
                        body.push_str(&format!(
                            r#"<td>
<b><a href="https://anilist.co/manga/{id}" target="_blank">{title}</a></b>
({year}) [{status}]<br>
Romaji: {romaji}<br>
<a href="/manga/add?anilist_id={id}">[Add to Library]</a>
</td>"#,
                            title = html_escape(title),
                            romaji = html_escape(romaji),
                        ));
                        body.push_str("</tr></table>\n<hr>\n");
                    }
                }
            }
            Err(e) => {
                body.push_str(&format!(
                    "<p><b>!! Search error !!</b><br>{}</p>\n",
                    html_escape(&e.to_string())
                ));
            }
        }
    }

    page("Search Manga", body)
}

/// ---------------------------------------------------------------------------
/// GET /manga/add?anilist_id=<id> -- pick library + folder name
/// All manga data is embedded as hidden fields so the POST needs no API call.
/// ---------------------------------------------------------------------------
#[get("/manga/add?<anilist_id>")]
async fn manga_add_form(
    pool: &State<SqlitePool>,
    al: &State<ALClient>,
    anilist_id: i32,
) -> RawHtml<String> {
    let manga = match al.grab_manga(anilist_id).await {
        Ok(m) => m,
        Err(e) => return error_page(&format!("AniList lookup failed: {e}")),
    };

    let libs = match db::library::get_all(pool.inner()).await {
        Ok(l) => l,
        Err(e) => return error_page(&e.to_string()),
    };

    if libs.is_empty() {
        return error_page("No libraries found. Please add a library first.");
    }

    let default_path = manga
        .metadata
        .title
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");

    let mut lib_options = String::new();
    for lib in &libs {
        let type_str = match lib.r#type {
            MangaType::Manga => "Manga",
            MangaType::Comics => "Comics",
        };
        lib_options.push_str(&format!(
            "<option value=\"{uuid}\">{path} [{type_str}]</option>\n",
            uuid = lib.uuid,
            path = html_escape(&lib.root_path.to_string_lossy()),
        ));
    }

    let synopsis_preview = manga
        .metadata
        .synopsis
        .as_deref()
        .unwrap_or("No synopsis.")
        .chars()
        .take(300)
        .collect::<String>();

    let thumb_html = match &manga.thumbnail_url {
        Some(url) => format!("<img src=\"{}\" width=\"120\" alt=\"cover\"><br><br>\n", html_escape(url)),
        None => String::new(),
    };

    // Serialise optional fields as empty strings so hidden inputs are always present.
    let mal_id_s = manga.mal_id.map(|v| v.to_string()).unwrap_or_default();
    let start_year_s = manga.metadata.start_year.map(|v| v.to_string()).unwrap_or_default();
    let end_year_s = manga.metadata.end_year.map(|v| v.to_string()).unwrap_or_default();
    let chapter_count_s = manga.chapter_count.map(|v| v.to_string()).unwrap_or_default();
    let thumbnail_s = manga.thumbnail_url.as_deref().unwrap_or_default();
    let synopsis_s = manga.metadata.synopsis.as_deref().unwrap_or_default();
    let status_s = publishing_status_str(&manga.metadata.publishing_status);

    let body = format!(
        r#"{thumb_html}<pre>Title  : <a href="https://anilist.co/manga/{anilist_id}" target="_blank">{title}</a>
Romaji : {romaji}
Year   : {year}
Status : {status}

{synopsis}...</pre>
<form method="POST" action="/manga/add">
<!-- manga metadata, so POST doesn't need to call AniList again -->
<input type="hidden" name="anilist_id"        value="{anilist_id}">
<input type="hidden" name="mal_id"            value="{mal_id}">
<input type="hidden" name="title"             value="{title_h}">
<input type="hidden" name="title_og"          value="{title_og_h}">
<input type="hidden" name="title_roman"       value="{title_roman_h}">
<input type="hidden" name="synopsis"          value="{synopsis_h}">
<input type="hidden" name="publishing_status" value="{status_s}">
<input type="hidden" name="start_year"        value="{start_year}">
<input type="hidden" name="end_year"          value="{end_year}">
<input type="hidden" name="chapter_count"     value="{chapter_count}">
<input type="hidden" name="thumbnail_url"     value="{thumbnail_h}">
<table>
<tr>
  <td>Library:</td>
  <td>
    <select name="library_id">
{lib_options}    </select>
  </td>
</tr>
<tr>
  <td>Folder name:</td>
  <td><input type="text" name="relative_path" size="50" value="{default_path}"></td>
</tr>
</table>
<br>
<input type="submit" value="[Add to Library]">
<a href="/search">[Cancel]</a>
</form>"#,
        title = html_escape(&manga.metadata.title),
        romaji = html_escape(&manga.metadata.title_roman),
        year = manga
            .metadata
            .start_year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "?".to_string()),
        status = status_s,
        synopsis = html_escape(&synopsis_preview),
        // hidden field values (already html-escaped where needed)
        mal_id = html_escape(&mal_id_s),
        title_h = html_escape(&manga.metadata.title),
        title_og_h = html_escape(&manga.metadata.title_og),
        title_roman_h = html_escape(&manga.metadata.title_roman),
        synopsis_h = html_escape(synopsis_s),
        status_s = status_s,
        start_year = html_escape(&start_year_s),
        end_year = html_escape(&end_year_s),
        chapter_count = html_escape(&chapter_count_s),
        thumbnail_h = html_escape(thumbnail_s),
        default_path = html_escape(&default_path),
    );

    page(&format!("Add: {}", manga.metadata.title), body)
}

fn publishing_status_str(s: &PublishingStatus) -> &'static str {
    match s {
        PublishingStatus::Completed => "Completed",
        PublishingStatus::Ongoing => "Ongoing",
        PublishingStatus::Hiatus => "Hiatus",
        PublishingStatus::Cancelled => "Cancelled",
        PublishingStatus::NotYetReleased => "NotYetReleased",
        PublishingStatus::Unknown => "Unknown",
    }
}

fn parse_publishing_status(s: &str) -> PublishingStatus {
    match s {
        "Completed" => PublishingStatus::Completed,
        "Ongoing" => PublishingStatus::Ongoing,
        "Hiatus" => PublishingStatus::Hiatus,
        "Cancelled" => PublishingStatus::Cancelled,
        "NotYetReleased" => PublishingStatus::NotYetReleased,
        _ => PublishingStatus::Unknown,
    }
}

// ---------------------------------------------------------------------------
// POST /manga/add -- construct Manga from form fields, no AniList call
// ---------------------------------------------------------------------------

#[derive(rocket::FromForm)]
struct MangaAddForm {
    // User choices
    library_id: String,
    relative_path: String,
    // Hidden manga data (serialised as strings, empty = None)
    anilist_id: i32,
    mal_id: String,
    title: String,
    title_og: String,
    title_roman: String,
    synopsis: String,
    publishing_status: String,
    start_year: String,
    end_year: String,
    chapter_count: String,
    thumbnail_url: String,
}

#[post("/manga/add", data = "<form>")]
async fn manga_add_post(
    pool: &State<SqlitePool>,
    form: Form<MangaAddForm>,
) -> Result<Redirect, RawHtml<String>> {
    let library_id = Uuid::parse_str(&form.library_id)
        .map_err(|_| error_page("Invalid library ID."))?;

    if form.relative_path.trim().is_empty() {
        return Err(error_page("Folder name cannot be empty."));
    }

    if form.title.trim().is_empty() {
        return Err(error_page("Manga title is missing from form data."));
    }

    let manga = Manga {
        id: Uuid::new_v4(),
        library_id,
        anilist_id: Some(form.anilist_id as u32),
        mal_id: form.mal_id.parse::<u32>().ok(),
        relative_path: PathBuf::from(form.relative_path.trim()),
        downloaded_count: None,
        chapter_count: form.chapter_count.parse::<u32>().ok(),
        metadata_source: MangaSource::AniList,
        thumbnail_url: if form.thumbnail_url.is_empty() {
            None
        } else {
            Some(form.thumbnail_url.clone())
        },
        created_at: Utc::now(),
        metadata_updated_at: Utc::now(),
        metadata: MangaMetadata {
            title: form.title.clone(),
            title_og: form.title_og.clone(),
            title_roman: form.title_roman.clone(),
            synopsis: if form.synopsis.is_empty() {
                None
            } else {
                Some(form.synopsis.clone())
            },
            publishing_status: parse_publishing_status(&form.publishing_status),
            tags: vec![], // tags aren't passed through the form; acceptable for now
            start_year: form.start_year.parse::<i32>().ok(),
            end_year: form.end_year.parse::<i32>().ok(),
        },
    };

    db::manga::insert(pool.inner(), &manga)
        .await
        .map_err(|e| error_page(&e.to_string()))?;

    Ok(Redirect::to(format!("/library/{library_id}")))
}

// ---------------------------------------------------------------------------
// GET /manga/<uuid> -- manga detail (chapters stub)
// ---------------------------------------------------------------------------

#[get("/manga/<uuid>")]
async fn manga_view(pool: &State<SqlitePool>, uuid: &str) -> RawHtml<String> {
    let id = match Uuid::parse_str(uuid) {
        Ok(id) => id,
        Err(_) => return error_page("Invalid manga ID."),
    };

    let manga = match db::manga::get_by_id(pool.inner(), id).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_page("Manga not found."),
        Err(e) => return error_page(&e.to_string()),
    };

    let year_range = match (manga.metadata.start_year, manga.metadata.end_year) {
        (Some(s), Some(e)) => format!("{s} - {e}"),
        (Some(s), None) => format!("{s} - ongoing"),
        _ => "?".to_string(),
    };

    let tags = if manga.metadata.tags.is_empty() {
        "None".to_string()
    } else {
        manga.metadata.tags.join(", ")
    };

    let anilist_link = manga
        .anilist_id
        .map(|id| {
            format!(
                "<a href=\"https://anilist.co/manga/{id}\" target=\"_blank\">[View on AniList]</a>"
            )
        })
        .unwrap_or_default();

    let thumb_html = match &manga.thumbnail_url {
        Some(url) => format!("<img src=\"{}\" width=\"150\" alt=\"cover\"><br><br>\n", html_escape(url)),
        None => String::new(),
    };

    let body = format!(
        r#"{thumb_html}<pre>Title    : {title}  {anilist_link}
Romaji   : {romaji}
Original : {og}
Years    : {years}
Status   : {status}
Chapters : {dl} / {total} downloaded
Folder   : {path}

Synopsis:
{synopsis}

Tags: {tags}</pre>
<p>[ Chapter listing and download functionality coming soon ]</p>
<p><a href="/library/{lib_id}">[Back to Library]</a></p>"#,
        title = html_escape(&manga.metadata.title),
        romaji = html_escape(&manga.metadata.title_roman),
        og = html_escape(&manga.metadata.title_og),
        years = year_range,
        status = publishing_status_str(&manga.metadata.publishing_status),
        dl = manga.downloaded_count.unwrap_or(0),
        total = manga
            .chapter_count
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string()),
        path = html_escape(&manga.relative_path.to_string_lossy()),
        synopsis = html_escape(
            manga
                .metadata
                .synopsis
                .as_deref()
                .unwrap_or("No synopsis available.")
        ),
        tags = html_escape(&tags),
        lib_id = manga.library_id,
    );

    page(&manga.metadata.title, body)
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    routes![
        index,
        library_new_form,
        library_new_post,
        library_view,
        search,
        manga_add_form,
        manga_add_post,
        manga_view,
    ]
}
