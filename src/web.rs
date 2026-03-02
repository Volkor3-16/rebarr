use rocket::{get, response::content::RawHtml, routes};

// ---------------------------------------------------------------------------
// GET / -- serve the single-page frontend
// All data is loaded via fetch() calls to the /api/... REST endpoints.
// ---------------------------------------------------------------------------

#[get("/")]
pub fn index() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    routes![index]
}

// ---------------------------------------------------------------------------
// Frontend HTML + JS
// Replace this with a proper frontend framework later; the REST API stays the same.
// ---------------------------------------------------------------------------

const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>REBARR</title>
<style>
  body { font-family: monospace; max-width: 900px; margin: 0 auto; padding: 1rem; }
  nav { border-bottom: 1px solid #ccc; padding-bottom: 0.5rem; margin-bottom: 1rem; }
  nav a { margin-right: 1rem; cursor: pointer; color: #06c; }
  h2 { margin-top: 0; }
  table { border-collapse: collapse; width: 100%; }
  td, th { padding: 0.3rem 0.6rem; text-align: left; border-bottom: 1px solid #eee; }
  th { font-weight: bold; }
  img.cover { width: 80px; height: auto; }
  img.cover-lg { width: 160px; height: auto; }
  .error { color: red; }
  .tag { background: #eee; padding: 0.1rem 0.4rem; border-radius: 3px; margin: 0.1rem; display: inline-block; font-size: 0.85em; }
  input[type=text], select { width: 100%; box-sizing: border-box; padding: 0.3rem; margin-bottom: 0.4rem; }
  button { padding: 0.4rem 0.8rem; cursor: pointer; }
  #app { min-height: 200px; }
  pre { white-space: pre-wrap; }
</style>
</head>
<body>
<pre>+================================================+
| REBARR -- Manga Library Manager                |
+================================================+</pre>
<nav>
  <a onclick="showHome()">Home</a>
  <a onclick="showSearch()">Search Manga</a>
  <a onclick="showAddLibrary()">Add Library</a>
</nav>
<div id="app"><p>Loading...</p></div>

<script>
// ---------------------------------------------------------------------------
// Tiny router / view switcher
// ---------------------------------------------------------------------------
let currentView = null;

function render(html) {
  document.getElementById('app').innerHTML = html;
}

async function api(method, path, body) {
  const opts = { method, headers: { 'Content-Type': 'application/json' } };
  if (body !== undefined) opts.body = JSON.stringify(body);
  const r = await fetch(path, opts);
  if (!r.ok) {
    const e = await r.json().catch(() => ({ error: r.statusText }));
    throw new Error(e.error || r.statusText);
  }
  if (r.status === 204) return null;
  return r.json();
}

function escape(s) {
  if (s == null) return '';
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// ---------------------------------------------------------------------------
// Home — list libraries
// ---------------------------------------------------------------------------
async function showHome() {
  render('<p>Loading libraries...</p>');
  try {
    const libs = await api('GET', '/api/libraries');
    if (libs.length === 0) {
      render('<p>No libraries configured yet. <a onclick="showAddLibrary()">Add one!</a></p>');
      return;
    }
    let rows = libs.map(lib => {
      const t = lib.type === 'Comics' ? 'Comics' : 'Manga';
      return `<tr>
        <td>${escape(t)}</td>
        <td><a onclick='showLibrary("${lib.uuid}")'>${escape(lib.root_path)}</a></td>
      </tr>`;
    }).join('');
    render(`<h2>Libraries</h2>
      <table><tr><th>Type</th><th>Path</th></tr>${rows}</table>
      <br><button onclick="showAddLibrary()">+ Add Library</button>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// ---------------------------------------------------------------------------
// Library view — list manga
// ---------------------------------------------------------------------------
async function showLibrary(libId) {
  render('<p>Loading...</p>');
  try {
    const [lib, mangas] = await Promise.all([
      api('GET', `/api/libraries/${libId}`),
      api('GET', `/api/libraries/${libId}/manga`),
    ]);
    const t = lib.type === 'Comics' ? 'Comics' : 'Manga';
    let rows = mangas.length === 0
      ? '<tr><td colspan="4">No manga yet.</td></tr>'
      : mangas.map(m => {
          const dl = m.downloaded_count ?? 0;
          const total = m.chapter_count != null ? m.chapter_count : '?';
          const year = m.metadata?.start_year ?? '?';
          const thumb = m.thumbnail_url
            ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">`
            : '';
          return `<tr>
            <td>${thumb}</td>
            <td><a onclick='showManga("${m.id}")'>${escape(m.metadata?.title)}</a></td>
            <td>${escape(year)}</td>
            <td>${dl} / ${total}</td>
          </tr>`;
        }).join('');
    render(`<h2>${escape(lib.root_path)} <small>[${t}]</small></h2>
      <table>
        <tr><th></th><th>Title</th><th>Year</th><th>Chapters</th></tr>
        ${rows}
      </table>
      <br><button onclick='showSearch("${libId}")'>+ Search &amp; Add Manga</button>
      &nbsp;<a onclick="showHome()">[Back]</a>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// ---------------------------------------------------------------------------
// Search — AniList search results
// ---------------------------------------------------------------------------
async function showSearch(preselectedLibId) {
  render(`<h2>Search Manga</h2>
    <input type="text" id="sq" placeholder="Search for manga..." onkeydown="if(event.key==='Enter')doSearch()">
    <button onclick="doSearch()">Search</button>
    <div id="results"></div>`);
  window._preselectedLibId = preselectedLibId || null;
}

async function doSearch() {
  const q = document.getElementById('sq').value.trim();
  if (!q) return;
  document.getElementById('results').innerHTML = '<p>Searching...</p>';
  try {
    const results = await api('GET', `/api/manga/search?q=${encodeURIComponent(q)}`);
    if (results.length === 0) {
      document.getElementById('results').innerHTML = '<p>No results.</p>';
      return;
    }
    const rows = results.map(m => {
      const id = m.anilist_id ?? 0;
      const title = m.metadata?.title ?? 'Unknown';
      const year = m.metadata?.start_year ?? '?';
      const status = m.metadata?.publishing_status ?? 'Unknown';
      const thumb = m.thumbnail_url
        ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">`
        : '';
      return `<tr>
        <td>${thumb}</td>
        <td>
          <b><a href="https://anilist.co/manga/${id}" target="_blank">${escape(title)}</a></b><br>
          ${escape(year)} [${escape(status)}]<br>
          Romaji: ${escape(m.metadata?.title_roman)}
        </td>
        <td><button onclick='showAddManga(${id}, ${JSON.stringify(title)})'>Add to Library</button></td>
      </tr>`;
    }).join('');
    document.getElementById('results').innerHTML =
      `<table><tr><th></th><th>Title</th><th></th></tr>${rows}</table>`;
  } catch(e) {
    document.getElementById('results').innerHTML =
      `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Add manga — pick library + folder name, then POST /api/manga
// ---------------------------------------------------------------------------
function toPathSafe(s) {
  // Remove characters not allowed in directory names, collapse spaces
  return (s || '').replace(/[\/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim() || 'manga';
}

async function showAddManga(anilistId, title) {
  render('<p>Loading...</p>');
  try {
    const [preview, libs] = await Promise.all([
      api('GET', `/api/manga/search?q=${anilistId}`).then(r => {
        // search by ID gives fuzzy results; fetch exact via /api/manga/search isn't ideal
        // We'll use the add form and let the POST do the real AniList lookup
        return null;
      }).catch(() => null),
      api('GET', '/api/libraries'),
    ]);

    if (libs.length === 0) {
      render('<p class="error">No libraries found. Please add a library first.</p>');
      return;
    }

    const libOptions = libs.map(lib => {
      const sel = window._preselectedLibId === lib.uuid ? 'selected' : '';
      return `<option value="${lib.uuid}" ${sel}>${escape(lib.root_path)}</option>`;
    }).join('');

    render(`<h2>Add Manga (AniList #${anilistId})</h2>
      <p>Choose a destination library and folder name. Metadata will be fetched on add.</p>
      <label>Library:<br>
        <select id="am-lib">${libOptions}</select>
      </label>
      <label>Folder name:<br>
        <input type="text" id="am-path" value="${escape(toPathSafe(title))}">
      </label>
      <br>
      <button onclick='doAddManga(${anilistId})'>Add to Library</button>
      &nbsp;<a onclick="showSearch()">[Cancel]</a>
      <div id="am-status"></div>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function doAddManga(anilistId) {
  const libId = document.getElementById('am-lib').value;
  const path = document.getElementById('am-path').value.trim();
  if (!path) { document.getElementById('am-status').innerHTML = '<p class="error">Folder name required.</p>'; return; }

  document.getElementById('am-status').innerHTML = '<p>Adding... (downloading cover, fetching metadata)</p>';
  try {
    const manga = await api('POST', '/api/manga', {
      anilist_id: anilistId,
      library_id: libId,
      relative_path: path,
    });
    showManga(manga.id);
  } catch(e) {
    document.getElementById('am-status').innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Manga detail
// ---------------------------------------------------------------------------
async function showManga(id) {
  render('<p>Loading...</p>');
  try {
    const m = await api('GET', `/api/manga/${id}`);
    const meta = m.metadata ?? {};
    const year = meta.start_year ? (meta.end_year ? `${meta.start_year} - ${meta.end_year}` : `${meta.start_year} - ongoing`) : '?';
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    const thumb = m.thumbnail_url
      ? `<img class="cover-lg" src="${escape(m.thumbnail_url)}" alt="cover"><br><br>`
      : '';
    const tags = (meta.tags ?? []).map(t => `<span class="tag">${escape(t)}</span>`).join(' ');
    const aniLink = m.anilist_id
      ? `<a href="https://anilist.co/manga/${m.anilist_id}" target="_blank">[AniList]</a>`
      : '';

    render(`${thumb}<h2>${escape(meta.title)} ${aniLink}</h2>
      <pre>Romaji   : ${escape(meta.title_roman)}
Original : ${escape(meta.title_og)}
Years    : ${escape(year)}
Status   : ${escape(meta.publishing_status)}
Chapters : ${dl} / ${total} downloaded
Folder   : ${escape(m.relative_path)}</pre>
      <p><b>Synopsis:</b><br>${escape(meta.synopsis ?? 'No synopsis available.')}</p>
      <p><b>Tags:</b><br>${tags || 'None'}</p>
      <p>[ Chapter listing and download functionality coming soon ]</p>
      <p><a onclick='showLibrary("${m.library_id}")'>[Back to Library]</a></p>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// ---------------------------------------------------------------------------
// Add Library form
// ---------------------------------------------------------------------------
function showAddLibrary() {
  render(`<h2>Add Library</h2>
    <label>Library Type:<br>
      <select id="al-type">
        <option value="Manga">Manga</option>
        <option value="Comics">Comics (Western)</option>
      </select>
    </label>
    <label>Root Path:<br>
      <input type="text" id="al-path" placeholder="/data/manga">
    </label>
    <br>
    <button onclick="doAddLibrary()">Add Library</button>
    &nbsp;<a onclick="showHome()">[Cancel]</a>
    <div id="al-status"></div>`);
}

async function doAddLibrary() {
  const t = document.getElementById('al-type').value;
  const p = document.getElementById('al-path').value.trim();
  if (!p) { document.getElementById('al-status').innerHTML = '<p class="error">Root path required.</p>'; return; }
  try {
    await api('POST', '/api/libraries', { library_type: t, root_path: p });
    showHome();
  } catch(e) {
    document.getElementById('al-status').innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// Boot
showHome();
</script>
</body>
</html>"#;
