use rocket::{get, response::content::RawHtml, routes};

// ---------------------------------------------------------------------------
// GET / -- serve the single-page frontend
// All data is loaded via fetch() calls to the /api/... REST endpoints.
// ---------------------------------------------------------------------------

#[get("/")]
pub fn index() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

#[get("/library")]
pub fn library_page() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

#[get("/series/<_id>")]
pub fn series_page(_id: &str) -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

#[get("/search")]
pub fn search_page() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

#[get("/settings")]
pub fn settings_page() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

#[get("/logs")]
pub fn logs_page() -> RawHtml<&'static str> {
    RawHtml(FRONTEND_HTML)
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    routes![index, library_page, series_page, search_page, settings_page, logs_page]
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
  nav a { margin-right: 1rem; cursor: pointer; color: #06c; text-decoration: none; }
  nav a:hover { text-decoration: underline; }
  nav a.active { font-weight: bold; color: #000; text-decoration: underline; }
  h2 { margin-top: 0; }
  table { border-collapse: collapse; width: 100%; }
  td, th { padding: 0.3rem 0.6rem; text-align: left; border-bottom: 1px solid #eee; }
  th { font-weight: bold; }
  img.cover { width: 80px; height: auto; }
  img.cover-lg { width: 160px; height: auto; }
  .error { color: red; }
  .tag { background: #eee; padding: 0.1rem 0.4rem; border-radius: 3px; margin: 0.1rem; display: inline-block; font-size: 0.85em; }
  .st-missing { color: #888; }
  .st-downloading { color: #f80; font-weight: bold; }
  .st-downloaded { color: #393; font-weight: bold; }
  .st-failed { color: #c33; font-weight: bold; }
  .task-pending { color: #888; }
  .task-running { color: #f80; font-weight: bold; }
  .task-completed { color: #393; }
  .task-failed { color: #c33; font-weight: bold; }
  .task-cancelled { color: #aaa; }
  .task-banner { background: #fffbe6; border: 1px solid #f0d060; padding: 0.4rem 0.8rem; margin-bottom: 0.5rem; border-radius: 3px; }
  input[type=text], select { width: 100%; box-sizing: border-box; padding: 0.3rem; margin-bottom: 0.4rem; }
  button { padding: 0.4rem 0.8rem; cursor: pointer; }
  button.btn-sm { padding: 0.2rem 0.5rem; font-size: 0.85em; }
  button.btn-danger { color: #c33; }
  #app { min-height: 200px; }
  pre { white-space: pre-wrap; }
  .lib-row td { vertical-align: middle; }
  .edit-form { display: none; padding: 0.4rem 0; }
  .edit-form input { width: auto; display: inline; margin-bottom: 0; }
</style>
</head>
<body>
<pre>+================================================+
| REBARR -- Manga Library Manager                |
+================================================+</pre>
<nav id="nav">
  <a onclick="navigate('/')" data-path="/">Home</a>
  <a onclick="navigate('/library')" data-path="/library">Libraries</a>
  <a onclick="navigate('/search')" data-path="/search">Search</a>
  <a onclick="navigate('/settings')" data-path="/settings">Settings</a>
  <a onclick="navigate('/logs')" data-path="/logs">Logs</a>
</nav>
<div id="app"><p>Loading...</p></div>

<script>
// ---------------------------------------------------------------------------
// Core helpers
// ---------------------------------------------------------------------------
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
  if (r.status === 204 || r.status === 202) return null;
  return r.json();
}

function escape(s) {
  if (s == null) return '';
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

function statusBadge(s) {
  const cls = { Missing:'st-missing', Downloading:'st-downloading', Downloaded:'st-downloaded', Failed:'st-failed' }[s] || 'st-missing';
  return `<span class="${cls}">${escape(s)}</span>`;
}

function taskBadge(s) {
  const cls = { Pending:'task-pending', Running:'task-running', Completed:'task-completed', Failed:'task-failed', Cancelled:'task-cancelled' }[s] || 'task-pending';
  return `<span class="${cls}">${escape(s)}</span>`;
}

function toPathSafe(s) {
  return (s || '').replace(/[\/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim() || 'manga';
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
let _pollHandle = null;

function stopPolling() {
  if (_pollHandle) { clearInterval(_pollHandle); _pollHandle = null; }
}

function navigate(path) {
  stopPolling();
  history.pushState({}, '', path);
  dispatch(path);
}

window.onpopstate = () => { stopPolling(); dispatch(window.location.pathname); };

function dispatch(path) {
  document.querySelectorAll('#nav a').forEach(a => {
    a.classList.toggle('active', path === a.dataset.path || (a.dataset.path !== '/' && path.startsWith(a.dataset.path)));
  });
  const routes = [
    [/^\/$/, viewHome],
    [/^\/library$/, viewLibraries],
    [/^\/series\/([^/]+)$/, viewSeries],
    [/^\/search$/, viewSearch],
    [/^\/settings$/, viewSettings],
    [/^\/logs$/, viewLogs],
  ];
  for (const [pat, fn] of routes) {
    const m = path.match(pat);
    if (m) { fn(...m.slice(1)); return; }
  }
  render('<p class="error">404 — page not found</p>');
}

// ---------------------------------------------------------------------------
// Home — all manga across all libraries
// ---------------------------------------------------------------------------
async function viewHome() {
  render('<p>Loading...</p>');
  try {
    const libs = await api('GET', '/api/libraries');
    if (libs.length === 0) {
      render(`<h2>Welcome to REBARR</h2><p>No libraries configured yet. <a onclick="navigate('/library')" style="cursor:pointer;color:#06c">Add one!</a></p>`);
      return;
    }
    const mangaLists = await Promise.all(libs.map(lib => api('GET', `/api/libraries/${lib.uuid}/manga`)));
    let html = '';
    libs.forEach((lib, i) => {
      const mangas = mangaLists[i];
      const t = lib.type === 'Comics' ? 'Comics' : 'Manga';
      html += `<h3>${escape(lib.root_path)} <small>[${t}]</small></h3>`;
      if (mangas.length === 0) {
        html += `<p><small>No manga yet. <a onclick="navigate('/search?library_id=${lib.uuid}')" style="cursor:pointer;color:#06c">Add some!</a></small></p>`;
      } else {
        const rows = mangas.map(m => {
          const dl = m.downloaded_count ?? 0;
          const total = m.chapter_count != null ? m.chapter_count : '?';
          const year = m.metadata?.start_year ?? '?';
          const thumb = m.thumbnail_url ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">` : '';
          return `<tr>
            <td>${thumb}</td>
            <td><a onclick='navigate("/series/${m.id}")' style="cursor:pointer;color:#06c">${escape(m.metadata?.title)}</a></td>
            <td>${escape(year)}</td>
            <td>${dl} / ${total}</td>
          </tr>`;
        }).join('');
        html += `<table><tr><th></th><th>Title</th><th>Year</th><th>Chapters</th></tr>${rows}</table>`;
      }
    });
    render(html);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// ---------------------------------------------------------------------------
// Libraries — list, edit, delete, add
// ---------------------------------------------------------------------------
async function viewLibraries() {
  render('<p>Loading...</p>');
  try {
    const libs = await api('GET', '/api/libraries');
    let libRows = libs.map(lib => {
      const t = lib.type === 'Comics' ? 'Comics' : 'Manga';
      return `<tr class="lib-row" id="librow-${lib.uuid}">
        <td>${escape(t)}</td>
        <td>
          <span id="libpath-${lib.uuid}">${escape(lib.root_path)}</span>
          <div class="edit-form" id="libedit-${lib.uuid}">
            <input type="text" id="libinput-${lib.uuid}" value="${escape(lib.root_path)}" style="width:60%;display:inline">
            <button class="btn-sm" onclick='saveLibrary("${lib.uuid}")'>Save</button>
            <button class="btn-sm" onclick='cancelEditLibrary("${lib.uuid}")'>Cancel</button>
          </div>
        </td>
        <td>
          <button class="btn-sm" onclick='editLibrary("${lib.uuid}")'>Edit</button>
          &nbsp;<button class="btn-sm btn-danger" onclick='deleteLibrary("${lib.uuid}")'>Delete</button>
          &nbsp;<button class="btn-sm" onclick='navigate("/search?library_id=${lib.uuid}")'>Add Manga</button>
        </td>
      </tr>`;
    }).join('');

    render(`<h2>Libraries</h2>
      ${libs.length > 0
        ? `<table><tr><th>Type</th><th>Root Path</th><th></th></tr>${libRows}</table>`
        : '<p>No libraries yet.</p>'}
      <hr>
      <h3>Add Library</h3>
      <label>Type:<br><select id="al-type" style="width:auto">
        <option value="Manga">Manga</option>
        <option value="Comics">Comics (Western)</option>
      </select></label><br>
      <label>Root Path:<br><input type="text" id="al-path" placeholder="/data/manga" style="width:60%"></label><br><br>
      <button onclick="doAddLibrary()">+ Add Library</button>
      <div id="al-status"></div>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

function editLibrary(uuid) {
  document.getElementById(`libpath-${uuid}`).style.display = 'none';
  document.getElementById(`libedit-${uuid}`).style.display = 'block';
  document.getElementById(`libinput-${uuid}`).focus();
}

function cancelEditLibrary(uuid) {
  document.getElementById(`libpath-${uuid}`).style.display = '';
  document.getElementById(`libedit-${uuid}`).style.display = 'none';
}

async function saveLibrary(uuid) {
  const newPath = document.getElementById(`libinput-${uuid}`).value.trim();
  if (!newPath) { alert('Root path cannot be empty.'); return; }
  try {
    await api('PUT', `/api/libraries/${uuid}`, { root_path: newPath });
    viewLibraries();
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function deleteLibrary(uuid) {
  if (!confirm('Delete this library and ALL its manga records? (Files on disk are not deleted.)')) return;
  try {
    await api('DELETE', `/api/libraries/${uuid}`);
    viewLibraries();
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function doAddLibrary() {
  const t = document.getElementById('al-type').value;
  const p = document.getElementById('al-path').value.trim();
  if (!p) { document.getElementById('al-status').innerHTML = '<p class="error">Root path required.</p>'; return; }
  try {
    await api('POST', '/api/libraries', { library_type: t, root_path: p });
    viewLibraries();
  } catch(e) {
    document.getElementById('al-status').innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Series detail — manga info + chapters + live task status
// ---------------------------------------------------------------------------
async function viewSeries(id) {
  render('<p>Loading...</p>');
  try {
    const m = await api('GET', `/api/manga/${id}`);
    const meta = m.metadata ?? {};
    const year = meta.start_year ? (meta.end_year ? `${meta.start_year} - ${meta.end_year}` : `${meta.start_year} - ongoing`) : '?';
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    const thumb = m.thumbnail_url ? `<img class="cover-lg" src="${escape(m.thumbnail_url)}" alt="cover"><br><br>` : '';
    const tags = (meta.tags ?? []).map(t => `<span class="tag">${escape(t)}</span>`).join(' ');
    const aniLink = m.anilist_id ? `<a href="https://anilist.co/manga/${m.anilist_id}" target="_blank">[AniList]</a>` : '';
    document.title = `${meta.title ?? 'Manga'} — REBARR`;

    render(`${thumb}<h2>${escape(meta.title)} ${aniLink}</h2>
      <pre>Romaji   : ${escape(meta.title_roman)}
Original : ${escape(meta.title_og)}
Years    : ${escape(year)}
Status   : ${escape(meta.publishing_status)}
Chapters : ${dl} / ${total} downloaded
Folder   : ${escape(m.relative_path)}</pre>
      <p><b>Synopsis:</b><br>${escape(meta.synopsis ?? 'No synopsis available.')}</p>
      <p><b>Tags:</b><br>${tags || 'None'}</p>
      <h3>Chapters</h3>
      <button onclick='doScan("${m.id}")'>Scan for chapters</button>
      &nbsp;<button onclick='loadChapters("${m.id}")'>Refresh</button>
      <span id="scan-status"></span>
      <div id="tasks-banner"></div>
      <div id="chapters-list"><p>Loading...</p></div>
      <br><p><a onclick="navigate('/library')" style="cursor:pointer;color:#06c">[Back to Libraries]</a></p>`);

    loadChapters(m.id);

    // Poll for active tasks every 3s
    let prevHadActive = false;
    const pollTasks = async () => {
      try {
        const tasks = await api('GET', `/api/tasks?manga_id=${m.id}&limit=10`);
        const active = tasks.filter(t => t.status === 'Running' || t.status === 'Pending');
        const banner = document.getElementById('tasks-banner');
        if (!banner) return;
        if (active.length > 0) {
          const lines = active.map(t => `<b>${escape(t.task_type)}</b>: ${taskBadge(t.status)}`).join(' &nbsp;|&nbsp; ');
          banner.innerHTML = `<div class="task-banner">${lines}</div>`;
          prevHadActive = true;
        } else {
          banner.innerHTML = '';
          if (prevHadActive) { prevHadActive = false; loadChapters(m.id); }
        }
      } catch(_) {}
    };
    pollTasks();
    _pollHandle = setInterval(pollTasks, 3000);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function loadChapters(mangaId) {
  const el = document.getElementById('chapters-list');
  if (!el) return;
  el.innerHTML = '<p>Loading...</p>';
  try {
    const chapters = await api('GET', `/api/manga/${mangaId}/chapters`);
    if (chapters.length === 0) {
      el.innerHTML = '<p>No chapters found. Try scanning.</p>';
      return;
    }
    const rows = chapters.map(ch => {
      const numFloat = ch.number_sort;
      const title = ch.title ? escape(ch.title) : `Chapter ${escape(ch.number_raw)}`;
      const vol = ch.volume ? `Vol.${escape(ch.volume)} ` : '';
      const group = ch.scanlator_group ? `<small>[${escape(ch.scanlator_group)}]</small>` : '';
      const status = ch.download_status;
      const dlBtn = (status === 'Missing' || status === 'Failed')
        ? `<button class="btn-sm" onclick='doDownload("${mangaId}", ${numFloat})'>Download</button>`
        : '';
      return `<tr>
        <td>${vol}${title} ${group}</td>
        <td>${statusBadge(status)}</td>
        <td>${dlBtn}</td>
      </tr>`;
    }).join('');
    el.innerHTML = `<table><tr><th>Chapter</th><th>Status</th><th></th></tr>${rows}</table>`;
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

async function doScan(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing scan...';
  try {
    await api('POST', `/api/manga/${mangaId}/scan`);
    if (statusEl) statusEl.textContent = ' Scan queued!';
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
}

async function doDownload(mangaId, chapterNum) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${chapterNum}/download`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Download error: ' + e.message);
  }
}

// ---------------------------------------------------------------------------
// Search — AniList search + add manga
// ---------------------------------------------------------------------------
async function viewSearch() {
  const preselectedLibId = new URLSearchParams(window.location.search).get('library_id');
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
      const thumb = m.thumbnail_url ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">` : '';
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

async function showAddManga(anilistId, title) {
  render('<p>Loading libraries...</p>');
  try {
    const libs = await api('GET', '/api/libraries');
    if (libs.length === 0) {
      render('<p class="error">No libraries found. <a onclick="navigate(\'/library\')" style="cursor:pointer;color:#06c">Add one first.</a></p>');
      return;
    }
    const libOptions = libs.map(lib => {
      const sel = window._preselectedLibId === lib.uuid ? 'selected' : '';
      return `<option value="${lib.uuid}" ${sel}>${escape(lib.root_path)}</option>`;
    }).join('');
    render(`<h2>Add Manga (AniList #${anilistId})</h2>
      <p>Choose a destination library and folder name. Metadata will be fetched on add.</p>
      <label>Library:<br><select id="am-lib">${libOptions}</select></label>
      <label>Folder name:<br><input type="text" id="am-path" value="${escape(toPathSafe(title))}"></label><br>
      <button onclick='doAddManga(${anilistId})'>Add to Library</button>
      &nbsp;<a onclick="navigate('/search')" style="cursor:pointer;color:#06c">[Cancel]</a>
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
    const manga = await api('POST', '/api/manga', { anilist_id: anilistId, library_id: libId, relative_path: path });
    navigate(`/series/${manga.id}`);
  } catch(e) {
    document.getElementById('am-status').innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Settings — providers + links
// ---------------------------------------------------------------------------
async function viewSettings() {
  render('<p>Loading...</p>');
  try {
    const providers = await api('GET', '/api/providers');
    const pRows = providers.length === 0
      ? '<tr><td colspan="3">No providers loaded. Add YAML files to the providers/ directory.</td></tr>'
      : providers.map(p => `<tr>
          <td>${escape(p.name)}</td>
          <td>${p.score}</td>
          <td>${p.needs_browser ? 'Yes (browser)' : 'No'}</td>
        </tr>`).join('');
    render(`<h2>Settings</h2>
      <h3>Providers</h3>
      <p><small>Providers are loaded from YAML files. Restart to pick up changes.</small></p>
      <table><tr><th>Name</th><th>Score</th><th>Browser?</th></tr>${pRows}</table>
      <br>
      <h3>Libraries</h3>
      <p>Manage libraries (add, edit paths, delete) on the <a onclick="navigate('/library')" style="cursor:pointer;color:#06c">Libraries page</a>.</p>`);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// ---------------------------------------------------------------------------
// Logs — recent task history with live polling
// ---------------------------------------------------------------------------
async function viewLogs() {
  render('<h2>Task Log</h2><div id="logs-list"><p>Loading...</p></div>');
  await refreshLogs();
  _pollHandle = setInterval(refreshLogs, 3000);
}

async function refreshLogs() {
  const el = document.getElementById('logs-list');
  if (!el) return;
  try {
    const tasks = await api('GET', '/api/tasks?limit=100');
    if (tasks.length === 0) {
      el.innerHTML = '<p>No tasks yet.</p>';
      return;
    }
    const rows = tasks.map(t => {
      const ts = new Date(t.created_at).toLocaleString();
      const manga = t.manga_title ? escape(t.manga_title) : '<small>—</small>';
      const err = t.last_error ? `<br><small class="error">${escape(t.last_error)}</small>` : '';
      return `<tr>
        <td><small>${escape(ts)}</small></td>
        <td>${escape(t.task_type)}</td>
        <td>${manga}</td>
        <td>${taskBadge(t.status)}${err}</td>
      </tr>`;
    }).join('');
    el.innerHTML = `<table>
      <tr><th>Time</th><th>Type</th><th>Manga</th><th>Status</th></tr>
      ${rows}
    </table>`;
  } catch(e) {
    if (el) el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// Boot
dispatch(window.location.pathname);
</script>
</body>
</html>"#;
