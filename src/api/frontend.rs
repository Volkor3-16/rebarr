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

    const monitoredChecked = m.monitored !== false ? 'checked' : '';
    render(`${thumb}<h2>${escape(meta.title)} ${aniLink}</h2>
      <label style="font-size:0.9em"><input type="checkbox" id="monitored-cb" ${monitoredChecked} onchange="toggleMonitored('${m.id}', this.checked)"> Monitored <small>(auto-download new chapters)</small></label>
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
    // Group by chapter_base, then within each group sort by chapter_variant ASC
    // so parent chapter (variant=0) always renders before its split parts.
    const groups = {};
    for (const ch of chapters) {
      const b = ch.chapter_base;
      if (!groups[b]) groups[b] = [];
      groups[b].push(ch);
    }
    const sortedBases = Object.keys(groups).map(Number).sort((a, b) => b - a);
    const orderedChapters = [];
    for (const b of sortedBases) {
      groups[b].sort((a, c) => a.chapter_variant - c.chapter_variant);
      orderedChapters.push(...groups[b]);
    }
    const rows = orderedChapters.map(ch => {
      const numFloat = ch.number_sort;
      const base = ch.chapter_base;
      const variant = ch.chapter_variant;
      const isExtra = ch.is_extra;
      // Build a human-friendly chapter label
      let chNum;
      if (variant === 0) {
        chNum = `Chapter ${base % 1 === 0 ? base.toFixed(0) : base}`;
      } else if (isExtra) {
        chNum = `Chapter ${base % 1 === 0 ? base.toFixed(0) : base} Extra`;
      } else {
        // split part: 1→a, 2→b, ...
        const partLetter = String.fromCharCode(96 + variant);
        chNum = `Chapter ${base % 1 === 0 ? base.toFixed(0) : base} Part ${partLetter.toUpperCase()}`;
      }
      let label = chNum;
      if (ch.title) label += ` - ${escape(ch.title)}`;
      if (ch.scanlator_group) label += ` [${escape(ch.scanlator_group)}]`;
      if (ch.language && ch.language !== 'en') {
        label += ` <span style="font-size:0.7em;padding:1px 4px;border-radius:3px;background:#555;color:#fff">${escape(ch.language.toUpperCase())}</span>`;
      }
      const vol = ch.volume ? `Vol.${escape(ch.volume)} ` : '';
      const status = ch.download_status;
      const canDl = status === 'Missing' || status === 'Failed';
      const dlBtn = canDl
        ? `<button class="btn-sm" onclick='doDownload("${mangaId}", ${numFloat})'>Download</button>`
        : '';
      const markBtn = canDl
        ? `&nbsp;<button class="btn-sm" onclick='doMarkDownloaded("${mangaId}", ${numFloat})'>Mark Done</button>`
        : '';
      const optBtn = status === 'Downloaded'
        ? `<button class="btn-sm" onclick='doOptimise("${mangaId}", ${numFloat})'>Optimise</button>`
        : '';
      const cb = canDl
        ? `<input type="checkbox" class="ch-checkbox" data-num="${numFloat}" data-status="${escape(status)}">`
        : '';
      const foundTitle = ch.created_at ? new Date(ch.created_at).toLocaleString() : '';
      const foundCell = ch.found_ago
        ? `<span title="${escape(foundTitle)}">${escape(ch.found_ago)} ago</span>`
        : '<span title="">—</span>';
      // Released date: prefer effective_date_released (best available from chapter or providers)
      let releasedCell = '—';
      if (ch.effective_date_released) {
        const rd = new Date(ch.effective_date_released);
        const now = new Date();
        const diffDays = Math.floor((now - rd) / 86400000);
        if (diffDays < 30) {
          releasedCell = `<span title="${rd.toLocaleString()}">${diffDays === 0 ? 'today' : diffDays + 'd ago'}</span>`;
        } else if (diffDays < 365) {
          releasedCell = `<span title="${rd.toLocaleString()}">${Math.floor(diffDays/30)}mo ago</span>`;
        } else {
          releasedCell = `<span title="${rd.toLocaleString()}">${rd.toLocaleDateString(undefined,{year:'numeric',month:'short'})}</span>`;
        }
      }
      // Indent split/extra variants slightly
      const indent = variant !== 0 ? ' style="padding-left:1.5rem;font-size:0.92em"' : '';
      // Per-chapter provider dropdown
      let provDetails = '';
      if (ch.providers && ch.providers.length > 0) {
        const pRows = ch.providers.map(p => {
          const tierLabel = {1:'T1:Official',2:'T2:Trusted',3:'T3:Unknown',4:'T4:NoGroup'}[p.tier] || 'T?';
          const tierColor = {1:'#393',2:'#06c',3:'#c70',4:'#888'}[p.tier] || '#888';
          const tierBadge = `<span style="font-size:0.7em;padding:1px 3px;border-radius:3px;background:${tierColor};color:#fff">${tierLabel}</span>`;
          const grp = p.scanlator_group ? ` <small style="color:#666">(${escape(p.scanlator_group)})</small>` : '';
          const langBadge = p.language && p.language !== 'en'
            ? ` <span style="font-size:0.7em;padding:1px 4px;border-radius:3px;background:#555;color:#fff">${escape(p.language.toUpperCase())}</span>`
            : '';
          let pDateStr = '';
          if (p.date_released) {
            const pd = new Date(p.date_released);
            const pdDiff = Math.floor((new Date() - pd) / 86400000);
            const pdLabel = pdDiff < 1 ? 'today' : pdDiff < 30 ? pdDiff + 'd ago' : pdDiff < 365 ? Math.floor(pdDiff/30) + 'mo ago' : pd.toLocaleDateString(undefined,{year:'numeric',month:'short'});
            pDateStr = ` <small style="color:#888" title="${pd.toLocaleString()}">${pdLabel}</small>`;
          }
          const link = `<a href="${escape(p.chapter_url)}" target="_blank" rel="noopener" style="font-size:0.85em">[read]</a>`;
          const pinned = p.is_preferred ? ' <span style="color:#393;font-size:0.8em">★ Pinned</span>' : '';
          const pinBtn = !p.is_preferred
            ? `<button class="btn-sm" onclick='setPreferredProvider("${mangaId}",${numFloat},"${p.provider_name.replace(/"/g,'\\"')}") '>Pin</button>`
            : `<button class="btn-sm" onclick='setPreferredProvider("${mangaId}",${numFloat},null)'>Unpin</button>`;
          return `<div style="margin-bottom:0.15rem">${tierBadge} <b>${escape(p.provider_name)}</b>${grp}${langBadge}${pDateStr} ${link}${pinned} ${pinBtn}</div>`;
        }).join('');
        const label2 = `${ch.providers.length} provider${ch.providers.length===1?'':'s'}${ch.preferred_provider?' (pinned: '+escape(ch.preferred_provider)+')':''}`;
        provDetails = `<details style="margin-top:0.25rem"><summary style="cursor:pointer;font-size:0.78em;color:#06c">${label2}</summary><div style="padding:0.2rem 0 0.1rem 0.5rem">${pRows}</div></details>`;
      }
      return `<tr>
        <td>${cb}</td>
        <td${indent}>${vol}${label}${provDetails}</td>
        <td>${statusBadge(status)}</td>
        <td><small>${releasedCell}</small></td>
        <td><small>${foundCell}</small></td>
        <td>${dlBtn}${markBtn}${optBtn}</td>
      </tr>`;
    }).join('');
    el.innerHTML = `<table><tr><th><input type="checkbox" title="Select all" onchange="toggleSelectAll(this.checked)"></th><th>Chapter</th><th>Status</th><th>Released</th><th>Found</th><th></th></tr>${rows}</table>`;
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

async function doMarkDownloaded(mangaId, chapterNum) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${chapterNum}/mark-downloaded`);
    loadChapters(mangaId);
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

async function doOptimise(mangaId, chapterNum) {
  try {
    await api('POST', `/api/manga/${mangaId}/chapters/${chapterNum}/optimise`);
    alert('Optimise task queued.');
  } catch(e) {
    alert('Error: ' + e.message);
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

async function toggleMonitored(mangaId, checked) {
  try {
    await api('PATCH', `/api/manga/${mangaId}`, { monitored: checked });
  } catch(e) {
    alert('Error updating monitored: ' + e.message);
  }
}

function toggleSelectAll(checked) {
  document.querySelectorAll('.ch-checkbox').forEach(cb => cb.checked = checked);
}

async function doDownloadSelected(mangaId) {
  const checked = Array.from(document.querySelectorAll('.ch-checkbox:checked'));
  if (checked.length === 0) { alert('Select at least one chapter.'); return; }
  let count = 0;
  for (const cb of checked) {
    try { await api('POST', `/api/manga/${mangaId}/chapters/${cb.dataset.num}/download`); count++; } catch(_) {}
  }
  if (count > 0) loadChapters(mangaId);
}

async function doDownloadAllMissing(mangaId) {
  const all = Array.from(document.querySelectorAll('.ch-checkbox'));
  if (all.length === 0) { alert('No missing chapters to download.'); return; }
  for (const cb of all) {
    try { await api('POST', `/api/manga/${mangaId}/chapters/${cb.dataset.num}/download`); } catch(_) {}
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
        const searched = p.search_attempted_at ? new Date(p.search_attempted_at).toLocaleString() : 'never';
        return `<tr style="opacity:0.5">
          <td><b>${escape(p.provider_name)}</b></td>
          <td><span style="color:#a55">Not found</span></td>
          <td></td>
          <td><small>searched: ${escape(searched)}</small></td>
        </tr>`;
      }
      const synced = p.last_synced_at ? new Date(p.last_synced_at).toLocaleString() : 'Never';
      const link = p.provider_url ? `<a href="${escape(p.provider_url)}" target="_blank" rel="noopener">[open]</a>` : '—';
      return `<tr>
        <td><b>${escape(p.provider_name)}</b></td>
        <td><span style="color:#393">Found</span></td>
        <td>${link}</td>
        <td><small>${escape(synced)}</small></td>
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

async function setPreferredProvider(mangaId, chapterNum, providerName) {
  try {
    await api('PATCH', `/api/manga/${mangaId}/chapters/${chapterNum}/preferred-provider`, { provider_name: providerName });
    loadChapters(mangaId);
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

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
      <tr><td style="vertical-align:top;padding-top:0.4rem">Native Title</td>
          <td><input type="text" id="me-title-og" placeholder="e.g. 呪術廻戦"></td></tr>
      <tr><td style="vertical-align:top;padding-top:0.4rem">Romanised Title</td>
          <td><input type="text" id="me-title-roman" placeholder="e.g. Jujutsu Kaisen"></td></tr>
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

  const body = {
    library_id: lib,
    relative_path: path,
    title,
    title_og:    document.getElementById('me-title-og').value.trim() || null,
    title_roman: document.getElementById('me-title-roman').value.trim() || null,
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
</script>
</body>
</html>"#;
