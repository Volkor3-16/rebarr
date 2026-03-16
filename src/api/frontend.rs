use rocket::{get, response::content::RawHtml, routes};

// This file handles serving the frontend, and contains all the html frontend.

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

#[get("/queue")]
pub fn queue_page() -> RawHtml<&'static str> {
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
    routes![
        index,
        library_page,
        series_page,
        search_page,
        settings_page,
        queue_page,
        logs_page
    ]
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
  /* Chapter list styles */
  .ch-main { cursor: pointer; }
  .ch-main:hover { background: #f5f5f5; }
  .ch-variant { background: #fafafa; }
  .ch-variant:hover { background: #f0f0f0; }
  .ch-variant-toggle { cursor: pointer; color: #06c; margin-right: 0.3rem; }
  .ch-variant-toggle::before { content: '▶'; font-size: 0.7em; }
  .ch-variant-toggle.open::before { content: '▼'; }
  .ch-variant-row { display: none; }
  .ch-variant-row.open { display: table-row; }
  .ch-tier { display: inline-block; font-size: 0.7em; padding: 1px 4px; border-radius: 3px; color: #fff; margin-right: 0.3rem; }
  .ch-tier-1 { background: #393; }
  .ch-tier-2 { background: #06c; }
  .ch-tier-3 { background: #c70; }
  .ch-tier-4 { background: #888; }
  .ch-source { font-size: 0.85em; color: #666; }
  .ch-dl-icon { cursor: pointer; color: #06c; font-size: 1.1em; padding: 0.2rem; }
  .ch-dl-icon:hover { color: #084; }
  .ch-dl-icon.disabled { color: #ccc; cursor: default; }
  .ch-extra-btn { font-size: 0.75em; padding: 1px 4px; }
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
  <a onclick="navigate('/queue')" data-path="/queue">Queue</a>
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
  return String(s).replace(/&/g,'&').replace(/</g,'<').replace(/>/g,'>').replace(/"/g,'"').replace(/'/g,'&#039;');
}

function statusBadge(s) {
  const cls = { Missing:'st-missing', Downloading:'st-downloading', Downloaded:'st-downloaded', Failed:'st-failed' }[s] || 'st-missing';
  return `<span class="${cls}">${escape(s)}</span>`;
}

function taskBadge(s) {
  const cls = { Pending:'task-pending', Running:'task-running', Completed:'task-completed', Failed:'task-failed', Cancelled:'task-cancelled' }[s] || 'task-pending';
  return `<span class="${cls}">${escape(s)}</span>`;
}

// Live-updating relative time from unix timestamp (seconds)
// Updates automatically every 30 seconds without API calls
function relTime(ts) {
  if (!ts) return '—';
  const now = Math.floor(Date.now() / 1000);
  const diff = now - ts;
  const title = new Date(ts * 1000).toLocaleString();
  
  if (diff < 60) return `<span class="rel-time" data-ts="${ts}" title="${title}">just now</span>`;
  if (diff < 3600) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 60)}m ago</span>`;
  if (diff < 86400) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 3600)}h ago</span>`;
  if (diff < 2592000) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 86400)}d ago</span>`;
  if (diff < 31536000) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 2592000)}mo ago</span>`;
  return `<span class="rel-time" data-ts="${ts}" title="${title}">${new Date(ts * 1000).toLocaleDateString(undefined,{year:'numeric',month:'short'})}</span>`;
}

// Update all .rel-time spans to show fresh relative times
function updateRelTimes() {
  document.querySelectorAll('.rel-time').forEach(el => {
    const ts = parseInt(el.dataset.ts, 10);
    if (ts) {
      const now = Math.floor(Date.now() / 1000);
      const diff = now - ts;
      let text;
      if (diff < 60) text = 'just now';
      else if (diff < 3600) text = Math.floor(diff / 60) + 'm ago';
      else if (diff < 86400) text = Math.floor(diff / 3600) + 'h ago';
      else if (diff < 2592000) text = Math.floor(diff / 86400) + 'd ago';
      else if (diff < 31536000) text = Math.floor(diff / 2592000) + 'mo ago';
      else text = new Date(ts * 1000).toLocaleDateString(undefined,{year:'numeric',month:'short'});
      if (el.textContent !== text) el.textContent = text;
    }
  });
}

// Legacy support - converts ISO date string to relative time (for older data)
function relDate(str) {
  if (!str) return '—';
  const d = new Date(str);
  if (isNaN(d)) return '—';
  const now = new Date();
  const diffDays = Math.floor((now - d) / 86400000);
  const title = escape(d.toLocaleString());
  if (diffDays === 0) return `<span title="${title}">today</span>`;
  if (diffDays < 30) return `<span title="${title}">${diffDays}d ago</span>`;
  if (diffDays < 365) return `<span title="${title}">${Math.floor(diffDays/30)}mo ago</span>`;
  return `<span title="${title}">${d.toLocaleDateString(undefined,{year:'numeric',month:'short'})}</span>`;
}

function toPathSafe(s) {
  return (s || '').replace(/[\/\\:*?"<>|']/g, '').replace(/\s+/g, ' ').trim() || 'manga';
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
    [/^\/queue$/, viewQueue],
    [/^\/logs$/, viewQueue],
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
            <td><button class="btn-sm btn-danger" onclick='doDeleteManga("${m.id}")'>Delete</button></td>
          </tr>`;
        }).join('');
        html += `<table><tr><th></th><th>Title</th><th>Year</th><th>Chapters</th><th></th></tr>${rows}</table>`;
      }
    });
    render(html);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function doDeleteManga(mangaId) {
  if (!confirm('Delete this series and ALL its chapters from the database? (Files on disk are not deleted.)')) return;
  try {
    await api('DELETE', `/api/manga/${mangaId}`);
    viewHome();
  } catch(e) {
    alert('Error: ' + e.message);
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

    const monitoredChecked = m.monitored !== false ? 'checked' : '';
    render(`${thumb}<h2>${escape(meta.title)} ${aniLink}</h2>
      <label style="font-size:0.9em"><input type="checkbox" id="monitored-cb" ${monitoredChecked} onchange="toggleMonitored('${m.id}', this.checked)"> Monitored <small>(auto-download new chapters)</small></label>
      <pre>Other Titles: ${(meta.other_titles || []).map(t => `<span class="tag">${escape(t)}</span>`).join(' ')}
Years    : ${escape(year)}
Status   : ${escape(meta.publishing_status)}
Chapters : ${dl} / ${total} downloaded
Folder   : ${escape(m.relative_path)}</pre>
      <p><b>Synopsis:</b><br>${escape(meta.synopsis ?? 'No synopsis available.')}</p>
      <p><b>Tags:</b><br>${tags || 'None'}</p>
      <h3>Chapters</h3>
      <button onclick='doScan("${m.id}")'>Scan for chapters</button>
      &nbsp;<button onclick='doScanDisk("${m.id}")'>Scan Disk</button>
      &nbsp;<button onclick='loadChapters("${m.id}")'>Refresh</button>
      &nbsp;<button onclick='doRefreshMetadata("${m.id}")'>Refresh Metadata</button>
      &nbsp;<button onclick='doDownloadAllMissing("${m.id}")'>Download All Missing</button>
      &nbsp;<button onclick='doDownloadSelected("${m.id}")'>Download Selected</button>
      <span id="scan-status"></span>
      <div id="tasks-banner"></div>
      <div id="chapters-list"><p>Loading...</p></div>
      <h3>Providers</h3>
      <div id="providers-list"><p>Loading...</p></div>
      <br><p><a onclick="navigate('/library')" style="cursor:pointer;color:#06c">[Back to Libraries]</a></p>`);

    loadChapters(m.id);
    loadProviders(m.id);

    // Poll for active tasks every 3s
    let prevHadActive = false;
    const pollTasks = async () => {
      try {
        const tasks = await api('GET', `/api/tasks?manga_id=${m.id}&limit=20`);
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

// Helper to get tier info for a chapter
// Renders tier badge for a chapter row
function tierBadgeHtml(tier) {
  const tierLabel = {1:'Official',2:'Known Scanner',3:'Unknown Scanner',4:'No Group'}[tier] || 'T?';
  return `<span class="ch-tier ch-tier-${tier}">${tierLabel}</span>`;
}

// Renders a single chapter row.
// isVariant=true for sub-rows shown in the expandable section.
// extraActions is optional HTML appended to the last column (used for the toggle button on main rows).
function chapterRow(mangaId, ch, isVariant = false, extraActions = '') {
  const base = ch.chapter_base;
  const variant = ch.chapter_variant;
  const chNum = variant === 0 ? `Chapter ${base}` : `Chapter ${base}.${variant}`;
  const title = ch.title ? ` — ${escape(ch.title)}` : '';
  const chapterLabel = `<b>${chNum}</b>${title}`;

  const tierHtml = tierBadgeHtml(ch.tier || 4);

  const sourceUrl = ch.chapter_url;
  const sourceName = ch.provider_name ? escape(ch.provider_name) : (ch.scanlator_group ? escape(ch.scanlator_group) : '—');
  const sourceHtml = sourceUrl
    ? `<a href="${escape(sourceUrl)}" target="_blank" rel="noopener" class="ch-source">${sourceName}</a>`
    : `<span class="ch-source">${sourceName}</span>`;

  let langHtml = '';
  if (ch.language && ch.language.toLowerCase() !== 'en') {
    langHtml = ` <span style="font-size:0.7em;padding:1px 3px;border-radius:3px;background:#555;color:#fff">${escape(ch.language.toUpperCase())}</span>`;
  }

  const status = ch.download_status;
  const canDl = status === 'Missing' || status === 'Failed';

  // Checkboxes only on canonical main rows (non-variant), for bulk download
  const cb = (!isVariant && canDl)
    ? `<input type="checkbox" class="ch-checkbox" data-base="${base}" data-variant="${variant}">`
    : '';

  const dlBtn = canDl
    ? `<button class="btn-sm" onclick='doDownload("${mangaId}", ${base}, ${variant})'>DL</button>`
    : '';

  // Delete button: shown for canonical rows only
  const deleteBtn = ch.is_canonical
    ? `<button class="btn-sm btn-danger" onclick='doDeleteChapter("${mangaId}", ${base}, ${variant})'>Del</button>`
    : '';

  // "Use" button: shown for non-canonical variant rows — lets user promote this source
  const useBtn = (isVariant && !ch.is_canonical)
    ? `<button class="btn-sm" onclick='doSetCanonical("${mangaId}", ${base}, ${variant}, "${ch.id}")'>Use</button>`
    : '';

  // "Extra" toggle: shown for canonical rows only (is_extra is a per-slot property)
  const extraBtn = ch.is_canonical
    ? `<button class="btn-sm" onclick='doToggleExtra("${mangaId}", ${base}, ${variant})'>${ch.is_extra ? 'Un-extra' : 'Extra'}</button>`
    : '';

  const rowClass = isVariant ? 'ch-variant' : 'ch-main';

  const scanlatorHtml = ch.scanlator_group ? escape(ch.scanlator_group) : '—';

  return `<tr class="${rowClass}">
    <td>${cb}</td>
    <td>${chapterLabel}${langHtml}</td>
    <td>${scanlatorHtml}</td>
    <td>${tierHtml}</td>
    <td>${sourceHtml}</td>
    <td>${statusBadge(status)}</td>
    <td><small>${relTime(ch.released_at)}</small></td>
    <td><small>${relTime(ch.scraped_at)}</small></td>
    <td>${dlBtn}${deleteBtn}${useBtn}${extraBtn}${extraActions}</td>
  </tr>`;
}

// Renders the full chapter group: main row + collapsible sub-rows.
// mainCh      — canonical row for (base, 0), shown as the top-level row
// v0alts      — non-canonical rows for (base, 0) i.e. alternative provider sources
// splitParts  — array of { canonical, alts } for each variant > 0
function chapterGroupHtml(mangaId, base, mainCh, v0alts, splitParts) {
  if (!mainCh) return '';

  // Collect all sub-rows
  let subRows = '';
  for (const alt of v0alts) {
    subRows += chapterRow(mangaId, alt, true);
  }
  for (const sp of splitParts) {
    if (sp.canonical) subRows += chapterRow(mangaId, sp.canonical, true);
    for (const alt of sp.alts) {
      subRows += chapterRow(mangaId, alt, true);
    }
  }

  const totalSub = v0alts.length + splitParts.reduce((n, sp) => n + 1 + sp.alts.length, 0);
  const groupId = `ch-${mainCh.chapter_base}-${mainCh.chapter_variant}`;
  const toggleHtml = totalSub > 0
    ? `<br><span class="ch-variant-toggle" onclick="toggleVariants('${groupId}', this)">${totalSub} variant${totalSub !== 1 ? 's' : ''}</span>`
    : '';

  const mainRow = chapterRow(mangaId, mainCh, false, toggleHtml);

  if (!subRows) return mainRow;

  return mainRow +
    `<tr class="ch-variant-row" id="${groupId}"><td colspan="9" style="padding:0;border:0;background:#fafafa"><table style="width:100%">${subRows}</table></td></tr>`;
}

function toggleVariants(groupId, toggleEl) {
  const row = document.getElementById(groupId);
  if (!row) return;
  const isOpen = row.classList.toggle('open');
  toggleEl.classList.toggle('open', isOpen);
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

    // Group by chapter_base → chapter_variant → [rows]
    const baseMap = new Map();
    for (const ch of chapters) {
      if (!baseMap.has(ch.chapter_base)) baseMap.set(ch.chapter_base, new Map());
      const varMap = baseMap.get(ch.chapter_base);
      if (!varMap.has(ch.chapter_variant)) varMap.set(ch.chapter_variant, []);
      varMap.get(ch.chapter_variant).push(ch);
    }

    const sortedBases = [...baseMap.keys()].sort((a, b) => b - a);
    let rows = '';

    for (const base of sortedBases) {
      const varMap = baseMap.get(base);

      // Collect ALL extras for this base from ALL variants (not just variant 0)
      const extras = [];
      for (const [variant, chs] of varMap) {
        for (const ch of chs) {
          if (ch.is_extra) extras.push(ch);
        }
      }
      // Sort extras by variant descending (like regular chapters)
      extras.sort((a, b) => (b.chapter_variant || 0) - (a.chapter_variant || 0));

      // Get all variant 0 rows (excluding extras)
      const v0rows = (varMap.get(0) || []).filter(ch => !ch.is_extra);
      const v0canonical = v0rows.find(ch => ch.is_canonical) || null;
      const v0alts = v0rows.filter(ch => !ch.is_canonical).sort((a, b) => (a.tier || 4) - (b.tier || 4));

      // Variants > 0 (split parts), sorted by variant number (excluding extras)
      const splitParts = [...varMap.keys()]
        .filter(v => v > 0)
        .sort((a, b) => b - a)
        .map(v => {
          const vrows = varMap.get(v).filter(ch => !ch.is_extra);
          return {
            canonical: vrows.find(ch => ch.is_canonical) || null,
            alts: vrows.filter(ch => !ch.is_canonical).sort((a, b) => (a.tier || 4) - (b.tier || 4)),
          };
        });

      // Determine main row: prefer canonical for variant 0, else fall back
      let mainCh = v0canonical;
      let effectiveV0alts = v0alts;
      if (!mainCh) {
        if (v0alts.length > 0) {
          mainCh = v0alts[0];
          effectiveV0alts = v0alts.slice(1);
        }
      }

      if (mainCh) {
        rows += chapterGroupHtml(mangaId, base, mainCh, effectiveV0alts, splitParts);
      } else {
        // No variant-0 at all — each split part is its own standalone chapter
        for (const sp of splitParts) {
          const spMain = sp.canonical || sp.alts[0];
          if (spMain) {
            const spAlts = sp.canonical ? sp.alts : sp.alts.slice(1);
            rows += chapterGroupHtml(mangaId, base, spMain, spAlts, []);
          }
        }
      }

      // Render extras as standalone rows (not collapsible variants)
      for (const extra of extras) {
        rows += chapterRow(mangaId, extra, false, '');
      }
    }

    el.innerHTML = `<table>
      <tr><th style="width:30px"><input type="checkbox" title="Select all" onchange="toggleSelectAll(this.checked)"></th><th>Chapter</th><th>Scanlator</th><th style="width:40px">Tier</th><th>Source</th><th>Status</th><th>Released</th><th>Scraped</th><th style="width:50px"></th></tr>
      ${rows}
    </table>`;
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

async function doScanDisk(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing disk scan...';
  try {
    await api('POST', `/api/manga/${mangaId}/scan-disk`);
    if (statusEl) statusEl.textContent = ' Disk scan queued!';
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
}

async function doRefreshMetadata(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing metadata refresh...';
  try {
    await api('POST', `/api/manga/${mangaId}/refresh`);
    if (statusEl) statusEl.textContent = ' Metadata refresh queued!';
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
}

async function doMarkDownloaded(mangaId, base, variant) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${base}/${variant}/mark-downloaded`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function doOptimise(mangaId, base, variant) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${base}/${variant}/optimise`);
    alert('Optimise task queued.');
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function doDownload(mangaId, base, variant) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${base}/${variant}/download`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Download error: ' + e.message);
  }
}

async function doDeleteChapter(mangaId, base, variant) {
  if (!confirm('Delete this chapter? This will also remove downloaded files from disk.')) return;
  try {
    await api('DELETE', `/api/manga/${mangaId}/chapters/${base}/${variant}`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Delete error: ' + e.message);
  }
}

async function doToggleExtra(mangaId, base, variant) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${base}/${variant}/toggle-extra`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function doSetCanonical(mangaId, base, variant, chapterId) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${base}/${variant}/set-canonical`, { chapter_id: chapterId });
    loadChapters(mangaId);
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function toggleMonitored(mangaId, checked) {
  try {
    await api('PATCH', `/api/manga/${mangaId}`, { monitored: checked });
  } catch(e) {
    alert('Error updating monitored: ' + e.message);
  }
}

function toggleSelectAll(checked) {
  document.querySelectorAll('.ch-checkbox, .ch-checkbox-group').forEach(cb => cb.checked = checked);
}

async function doDownloadSelected(mangaId) {
  const checked = Array.from(document.querySelectorAll('.ch-checkbox:checked'));
  if (checked.length === 0) { alert('Select at least one chapter.'); return; }
  let count = 0;
  for (const cb of checked) {
    try { await api('POST', `/api/manga/${mangaId}/chapters/${cb.dataset.base}/${cb.dataset.variant}/download`); count++; } catch(_) {}
  }
  if (count > 0) loadChapters(mangaId);
}

async function doDownloadAllMissing(mangaId) {
  const cbs = Array.from(document.querySelectorAll('.ch-checkbox'));
  if (cbs.length === 0) { alert('No missing chapters to download.'); return; }
  for (const cb of cbs) {
    try { await api('POST', `/api/manga/${mangaId}/chapters/${cb.dataset.base}/${cb.dataset.variant}/download`); } catch(_) {}
  }
  loadChapters(mangaId);
}

// ---------------------------------------------------------------------------
// Providers — per-manga provider panel (search status + links)
// ---------------------------------------------------------------------------
async function loadProviders(mangaId) {
  const el = document.getElementById('providers-list');
  if (!el) return;
  try {
    const providers = await api('GET', `/api/manga/${mangaId}/providers`);
    if (providers.length === 0) {
      el.innerHTML = '<p><small>No providers found yet. Scan this manga to discover providers.</small></p>';
      return;
    }
    const rows = providers.map(p => {
      if (!p.found) {
        // search_attempted_at is unix timestamp in seconds, multiply by 1000 for JS Date
        const searched = p.search_attempted_at ? relTime(p.search_attempted_at) : 'never';
        return `<tr style="opacity:0.5">
          <td><b>${escape(p.provider_name)}</b></td>
          <td><span style="color:#a55">Not found</span></td>
          <td></td>
          <td><small>searched: ${searched}</small></td>
        </tr>`;
      }
      // last_synced_at is unix timestamp in seconds - use relTime for live updating
      const synced = p.last_synced_at ? relTime(p.last_synced_at) : 'Never';
      const link = p.provider_url ? `<a href="${escape(p.provider_url)}" target="_blank" rel="noopener">[open]</a>` : '—';
      return `<tr>
        <td><b>${escape(p.provider_name)}</b></td>
        <td><span style="color:#393">Found</span></td>
        <td>${link}</td>
        <td><small>${synced}</small></td>
      </tr>`;
    }).join('');
    el.innerHTML = `<table>
      <tr><th>Provider</th><th>Status</th><th>Link</th><th>Last Synced</th></tr>
      ${rows}
    </table>
    <small>Per-chapter provider availability and tier info is shown in the chapter list above. Manage trusted groups in Settings.</small>`;
  } catch(e) {
    el.innerHTML = `<p class="error">Error loading providers: ${escape(e.message)}</p>`;
  }
}

// setPreferredProvider — not yet implemented (no API endpoint)
function setPreferredProvider() {}

// ---------------------------------------------------------------------------
// Search — AniList search + add manga
// ---------------------------------------------------------------------------
async function viewSearch() {
  const preselectedLibId = new URLSearchParams(window.location.search).get('library_id');
  window._preselectedLibId = preselectedLibId || null;
  render(`<h2>Add Manga</h2>
    <div style="margin-bottom:0.8rem">
      <button id="tab-search" onclick="searchTab()" style="font-weight:bold;text-decoration:underline">AniList Search</button>
      &nbsp;|&nbsp;
      <button id="tab-manual" onclick="manualTab()">Manual Entry</button>
    </div>
    <div id="search-pane">
      <input type="text" id="sq" placeholder="Search AniList for manga..." onkeydown="if(event.key==='Enter')doSearch()">
      <button onclick="doSearch()">Search</button>
      <div id="results"></div>
    </div>
    <div id="manual-pane" style="display:none"></div>`);
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
      const synopsis = m.metadata?.synopsis ?? '';
      // Truncate synopsis to ~150 chars for search results
      const synopsisShort = synopsis.length > 150 ? synopsis.substring(0, 150) + '...' : synopsis;
      const thumb = m.thumbnail_url ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">` : '';
      return `<tr>
        <td>${thumb}</td>
        <td>
          <b><a href="https://anilist.co/manga/${id}" target="_blank">${escape(title)}</a></b><br>
          ${escape(year)} [${escape(status)}]<br>
          Other Titles: ${(m.metadata?.other_titles || []).map(t => `<span class="tag">${escape(t)}</span>`).join(' ')}
          ${synopsisShort ? `<br><small style="color:#666">${escape(synopsisShort)}</small>` : ''}
        </td>
        <td><button onclick='showAddManga(${id}, "${toPathSafe(title)}")'>Add to Library</button></td>
      </tr>`;
    }).join('');
    document.getElementById('results').innerHTML =
      `<table><tr><th></th><th>Title</th><th></th></tr>${rows}</table>`;
  } catch(e) {
    document.getElementById('results').innerHTML =
      `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

async function showAddManga(anilistId, pathSafeTitle) {
render('<p>Loading...</p>');
  try {
    // pathSafeTitle is already sanitized by toPathSafe() - no decoding needed
    const title = pathSafeTitle || 'Unknown';
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

function searchTab() {
  document.getElementById('tab-search').style.fontWeight = 'bold';
  document.getElementById('tab-search').style.textDecoration = 'underline';
  document.getElementById('tab-manual').style.fontWeight = '';
  document.getElementById('tab-manual').style.textDecoration = '';
  document.getElementById('search-pane').style.display = '';
  document.getElementById('manual-pane').style.display = 'none';
}

async function manualTab() {
  document.getElementById('tab-manual').style.fontWeight = 'bold';
  document.getElementById('tab-manual').style.textDecoration = 'underline';
  document.getElementById('tab-search').style.fontWeight = '';
  document.getElementById('tab-search').style.textDecoration = '';
  document.getElementById('search-pane').style.display = 'none';
  const pane = document.getElementById('manual-pane');
  pane.style.display = '';
  // Load libraries for the selector
  let libOptions = '<option value="">— select library —</option>';
  try {
    const libs = await api('GET', '/api/libraries');
    libOptions += libs.map(lib => {
      const sel = window._preselectedLibId === lib.uuid ? 'selected' : '';
      return `<option value="${lib.uuid}" ${sel}>${escape(lib.root_path)}</option>`;
    }).join('');
  } catch(e) {
    libOptions = '<option value="">Error loading libraries</option>';
  }
  pane.innerHTML = `
    <h3>Manual Entry</h3>
    <p><small>For series not on AniList. All fields except Title are optional.</small></p>
    <table style="width:100%">
      <tr><td style="width:160px;vertical-align:top;padding-top:0.4rem"><b>Title *</b></td>
          <td><input type="text" id="me-title" placeholder="English title" oninput="meAutoPath()"></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Other Titles</td>
          <td><input type="text" id="me-other-titles" placeholder="Comma-separated: 呪術廻戦, Jujutsu Kaisen">
              <small>Alternative names, separated by commas.</small></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Synopsis</td>
          <td><textarea id="me-synopsis" rows="4" style="width:100%;box-sizing:border-box;padding:0.3rem;font-family:monospace" placeholder="Series description..."></textarea></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Status</td>
          <td><select id="me-status" style="width:auto">
            <option value="Unknown">Unknown</option>
            <option value="Ongoing">Ongoing</option>
            <option value="Completed">Completed</option>
            <option value="Hiatus">Hiatus</option>
            <option value="Cancelled">Cancelled</option>
            <option value="NotYetReleased">Not Yet Released</option>
          </select></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Start Year</td>
          <td><input type="number" id="me-start-year" placeholder="e.g. 2019" style="width:120px"></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">End Year</td>
          <td><input type="number" id="me-end-year" placeholder="e.g. 2024 (blank if ongoing)" style="width:120px"></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Tags</td>
          <td><input type="text" id="me-tags" placeholder="Comma-separated: Action, Fantasy, Isekai">
              <small>Tags will be comma-split and trimmed.</small></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Cover URL</td>
          <td><input type="text" id="me-cover" placeholder="https://... (optional, will be downloaded)"></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem"><b>Library *</b></td>
          <td><select id="me-lib">${libOptions}</select></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem"><b>Folder Name *</b></td>
          <td><input type="text" id="me-path" placeholder="Series folder within library root" oninput="this.dataset.edited='1'"></td></tr>
    </table>
    <br>
    <button onclick="doAddManual()">+ Add to Library</button>
    <div id="me-status-msg"></div>`;
}

function meAutoPath() {
  const pathEl = document.getElementById('me-path');
  if (!pathEl || pathEl.dataset.edited) return;
  pathEl.value = toPathSafe(document.getElementById('me-title').value);
}

async function doAddManual() {
  const statusEl = document.getElementById('me-status-msg');
  const title = document.getElementById('me-title').value.trim();
  const lib = document.getElementById('me-lib').value;
  const path = document.getElementById('me-path').value.trim();
  if (!title) { statusEl.innerHTML = '<p class="error">Title is required.</p>'; return; }
  if (!lib) { statusEl.innerHTML = '<p class="error">Please select a library.</p>'; return; }
  if (!path) { statusEl.innerHTML = '<p class="error">Folder name is required.</p>'; return; }

  const startYearRaw = document.getElementById('me-start-year').value.trim();
  const endYearRaw   = document.getElementById('me-end-year').value.trim();
  const tagsRaw      = document.getElementById('me-tags').value.trim();
  const coverUrl     = document.getElementById('me-cover').value.trim();

  const otherTitlesRaw = document.getElementById('me-other-titles').value.trim();
  const other_titles = otherTitlesRaw ? otherTitlesRaw.split(',').map(t => t.trim()).filter(Boolean) : null;

  const body = {
    library_id: lib,
    relative_path: path,
    title,
    other_titles: other_titles,
    synopsis:    document.getElementById('me-synopsis').value.trim() || null,
    publishing_status: document.getElementById('me-status').value,
    tags: tagsRaw ? tagsRaw.split(',').map(t => t.trim()).filter(Boolean) : [],
    start_year: startYearRaw ? parseInt(startYearRaw, 10) : null,
    end_year:   endYearRaw   ? parseInt(endYearRaw, 10)   : null,
    cover_url:  coverUrl || null,
  };

  statusEl.innerHTML = '<p>Adding...</p>';
  try {
    const manga = await api('POST', '/api/manga/manual', body);
    navigate(`/series/${manga.id}`);
  } catch(e) {
    statusEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Settings — scan schedule, providers + links
// ---------------------------------------------------------------------------
async function viewSettings() {
  render('<p>Loading...</p>');
  try {
    const [providers, settings] = await Promise.all([
      api('GET', '/api/providers'),
      api('GET', '/api/settings'),
    ]);
    const pRows = providers.length === 0
      ? '<tr><td colspan="2">No providers loaded. Add YAML files to the providers/ directory.</td></tr>'
      : providers.map(p => `<tr>
          <td>${escape(p.name)}</td>
          <td>${p.needs_browser ? 'Yes (browser)' : 'No'}</td>
        </tr>`).join('');
    render(`<h2>Settings</h2>
      <h3>Scheduler</h3>
      <p>Rebarr periodically checks for new chapters on all monitored series.</p>
      <label>Scan interval (hours):<br>
        <input type="number" id="scan-interval" min="1" max="168" value="${escape(settings.scan_interval_hours)}" style="width:80px">
      </label>
      <br><br>
      <label>Preferred language (BCP 47, e.g. <code>en</code>):<br>
        <input type="text" id="preferred-language" value="${escape(settings.preferred_language || '')}" placeholder="Leave blank to accept any language" style="width:220px;padding:0.3rem">
      </label>
      <br><br>
      <button onclick="saveSettings()">Save</button>
      <div id="settings-status"></div>
      <hr>
      <h3>Providers</h3>
      <p><small>Providers are loaded from YAML files. Restart to pick up changes.</small></p>
      <table><tr><th>Name</th><th>Browser?</th></tr>${pRows}</table>
      <hr>
      <h3>Trusted Scanlation Groups</h3>
      <p><small>Groups listed here are Tier 2 (trusted). Chapters from these groups score higher than unknown groups (Tier 3), but lower than official releases (Tier 1, auto-detected via "Official" in the name). Re-scan a series after changing this list to update scores.</small></p>
      <div id="trusted-groups-list"><p>Loading...</p></div>
      <div style="margin-top:0.5rem">
        <input type="text" id="new-trusted-group" placeholder="Group name (exact)" style="width:220px;padding:0.3rem">
        &nbsp;<button onclick="addTrustedGroup()">Add</button>
      </div>
      <div id="trusted-groups-status"></div>
      <br>
      <h3>Libraries</h3>
      <p>Manage libraries (add, edit paths, delete) on the <a onclick="navigate('/library')" style="cursor:pointer;color:#06c">Libraries page</a>.</p>`);
    loadTrustedGroups();
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function loadTrustedGroups() {
  const el = document.getElementById('trusted-groups-list');
  if (!el) return;
  try {
    const groups = await api('GET', '/api/trusted-groups');
    if (groups.length === 0) {
      el.innerHTML = '<p><small>No trusted groups yet.</small></p>';
      return;
    }
    el.innerHTML = '<ul style="margin:0.3rem 0">' + groups.map(g =>
      `<li>${escape(g)} <button class="btn-sm" onclick='removeTrustedGroup("${escape(g)}")'>Remove</button></li>`
    ).join('') + '</ul>';
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

async function addTrustedGroup() {
  const input = document.getElementById('new-trusted-group');
  const status = document.getElementById('trusted-groups-status');
  const name = input ? input.value.trim() : '';
  if (!name) { status.innerHTML = '<p class="error">Enter a group name.</p>'; return; }
  try {
    await api('POST', '/api/trusted-groups', { name });
    input.value = '';
    status.innerHTML = '<p style="color:#393">Added!</p>';
    loadTrustedGroups();
  } catch(e) {
    status.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

async function removeTrustedGroup(name) {
  try {
    await api('DELETE', `/api/trusted-groups/${encodeURIComponent(name)}`);
    loadTrustedGroups();
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function saveSettings() {
  const hours = parseInt(document.getElementById('scan-interval').value, 10);
  const lang = (document.getElementById('preferred-language').value || '').trim();
  const statusEl = document.getElementById('settings-status');
  if (!hours || hours < 1 || hours > 168) {
    statusEl.innerHTML = '<p class="error">Interval must be 1–168 hours.</p>';
    return;
  }
  try {
    await api('PUT', '/api/settings', { scan_interval_hours: hours, preferred_language: lang });
    statusEl.innerHTML = '<p style="color:#393">Saved!</p>';
  } catch(e) {
    statusEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

// ---------------------------------------------------------------------------
// Queue — task history + active queue with live polling
// ---------------------------------------------------------------------------
async function viewQueue() {
  render('<h2>Queue</h2><div id="queue-controls"><p>Loading...</p></div><div id="queue-list"></div>');
  await refreshQueue();
  _pollHandle = setInterval(refreshQueue, 3000);
}

async function refreshQueue() {
  const listEl = document.getElementById('queue-list');
  const ctrlEl = document.getElementById('queue-controls');
  if (!listEl || !ctrlEl) return;
  try {
    const [tasks, settings] = await Promise.all([
      api('GET', '/api/tasks'),
      api('GET', '/api/settings'),
    ]);
    const paused = settings.queue_paused;
    const pauseLabel = paused ? '▶ Resume Queue' : '⏸ Pause Queue';
    const pauseStyle = paused ? 'color:#c70;font-weight:bold' : '';
    ctrlEl.innerHTML = `
      <button onclick="toggleQueuePause(${paused})" style="${pauseStyle}">${pauseLabel}</button>
      &nbsp;<button class="btn-sm btn-danger" onclick="cancelSelected()">Cancel Selected</button>
      ${paused ? '<span style="color:#c70;margin-left:0.8rem"><b>Queue paused — no new tasks will run.</b></span>' : ''}`;
    if (tasks.length === 0) {
      listEl.innerHTML = '<p>No tasks yet.</p>';
      return;
    }
    const rows = tasks.map(t => {
      const ts = new Date(t.created_at).toLocaleString();
      const manga = t.manga_title ? `<a onclick='navigate("/series/${t.manga_id}")' style="cursor:pointer;color:#06c">${escape(t.manga_title)}</a>` : '<small>—</small>';
      const err = t.last_error ? `<br><small class="error">${escape(t.last_error)}</small>` : '';
      const canCancel = t.status === 'Pending' || t.status === 'Running';
      const cb = canCancel ? `<input type="checkbox" class="task-cb" data-id="${t.id}">` : '';
      const cancelBtn = canCancel
        ? `<button class="btn-sm btn-danger" onclick='cancelTask("${t.id}")'>Cancel</button>`
        : '';
      return `<tr>
        <td>${cb}</td>
        <td><small>${escape(ts)}</small></td>
        <td>${escape(t.task_type)}</td>
        <td>${manga}</td>
        <td>${taskBadge(t.status)}${err}</td>
        <td>${cancelBtn}</td>
      </tr>`;
    }).join('');
    listEl.innerHTML = `<table>
      <tr><th><input type="checkbox" title="Select all cancelable" onchange="toggleSelectAllTasks(this.checked)"></th><th>Time</th><th>Type</th><th>Manga</th><th>Status</th><th></th></tr>
      ${rows}
    </table>`;
  } catch(e) {
    if (listEl) listEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

async function toggleQueuePause(currentlyPaused) {
  try {
    await api('PUT', '/api/settings', { queue_paused: !currentlyPaused });
    refreshQueue();
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

function toggleSelectAllTasks(checked) {
  document.querySelectorAll('.task-cb').forEach(cb => cb.checked = checked);
}

async function cancelSelected() {
  const checked = Array.from(document.querySelectorAll('.task-cb:checked'));
  if (checked.length === 0) { alert('Select at least one task to cancel.'); return; }
  for (const cb of checked) {
    try { await api('POST', `/api/tasks/${cb.dataset.id}/cancel`); } catch(_) {}
  }
  refreshQueue();
}

async function cancelTask(taskId) {
  try {
    await api('POST', `/api/tasks/${taskId}/cancel`);
    refreshQueue();
  } catch(e) {
    alert('Cancel failed: ' + e.message);
  }
}

// Boot
dispatch(window.location.pathname);

// Global timer to update all relative timestamps every 30 seconds (without API calls)
setInterval(updateRelTimes, 30000);
</script>
</body>
</html>"#;
