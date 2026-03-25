// Search view - AniList search + add manga

import { search, libraries, manga as mangaApi } from '../api.js';
import { render, navigate } from '../router.js';
import { escape, toPathSafe, skeleton, showToast } from '../utils.js';

let preselectedLibId = null;

export async function viewSearch() {
  const params = new URLSearchParams(window.location.search);
  preselectedLibId = params.get('library_id');

  render(`
    <h2>Add Manga</h2>
    <div class="search-tabs mb-2">
      <button id="tab-search" class="btn btn-sm btn-primary">AniList Search</button>
      <button id="tab-manual" class="btn btn-sm">Manual Entry</button>
    </div>
    
    <div id="search-pane">
      <div class="search-box flex gap-1">
        <input type="text" id="sq" placeholder="Search AniList for manga..." onkeydown="if(event.key==='Enter')doSearch()">
        <button class="btn btn-primary" onclick="doSearch()">Search</button>
      </div>
      <div id="results"></div>
    </div>
    
    <div id="manual-pane" class="hidden"></div>
  `);

  // Tab handlers
  document.getElementById('tab-search').addEventListener('click', () => {
    document.getElementById('tab-search').classList.add('btn-primary');
    document.getElementById('tab-manual').classList.remove('btn-primary');
    document.getElementById('search-pane').classList.remove('hidden');
    document.getElementById('manual-pane').classList.add('hidden');
  });

  document.getElementById('tab-manual').addEventListener('click', () => {
    document.getElementById('tab-manual').classList.add('btn-primary');
    document.getElementById('tab-search').classList.remove('btn-primary');
    document.getElementById('manual-pane').classList.remove('hidden');
    document.getElementById('search-pane').classList.add('hidden');
    loadManualForm();
  });
}

async function loadManualForm() {
  const pane = document.getElementById('manual-pane');
  let libOptions = '<option value="">— select library —</option>';
  try {
    const libs = await libraries.list();
    libOptions += libs.map(lib => {
      const sel = preselectedLibId === lib.uuid ? 'selected' : '';
      return `<option value="${lib.uuid}" ${sel}>${escape(lib.root_path)}</option>`;
    }).join('');
  } catch(e) {
    libOptions = '<option value="">Error loading libraries</option>';
  }

  pane.innerHTML = `
    <h3>Manual Entry</h3>
    <p><small>For series not on AniList. All fields except Title are optional.</small></p>
    <form id="manual-form">
      <label>Title *</label>
      <input type="text" id="me-title" placeholder="English title" required>
      
      <label>Other Titles</label>
      <input type="text" id="me-other-titles" placeholder="Comma-separated: 呪術廻戦, Jujutsu Kaisen">
      
      <label>Synopsis</label>
      <textarea id="me-synopsis" rows="4" placeholder="Series description..."></textarea>
      
      <label>Status</label>
      <select id="me-status">
        <option value="Unknown">Unknown</option>
        <option value="Ongoing">Ongoing</option>
        <option value="Completed">Completed</option>
        <option value="Hiatus">Hiatus</option>
        <option value="Cancelled">Cancelled</option>
        <option value="NotYetReleased">Not Yet Released</option>
      </select>
      
      <label>Start Year</label>
      <input type="number" id="me-start-year" placeholder="e.g. 2019">
      
      <label>End Year</label>
      <input type="number" id="me-end-year" placeholder="e.g. 2024 (blank if ongoing)">
      
      <label>Tags</label>
      <input type="text" id="me-tags" placeholder="Comma-separated: Action, Fantasy">
      
      <label>Cover URL</label>
      <input type="text" id="me-cover" placeholder="https://...">
      
      <label>Library *</label>
      <select id="me-lib" required>${libOptions}</select>
      
      <label>Folder Name *</label>
      <input type="text" id="me-path" placeholder="Series folder within library root">
      
        <button type="submit" class="btn btn-primary">+ Add to Library</button>
    </form>
    <div id="me-status-msg"></div>
  `;

  // Auto-fill path from title
  document.getElementById('me-title').addEventListener('input', (e) => {
    const pathEl = document.getElementById('me-path');
    if (!pathEl.dataset.edited) {
      pathEl.value = toPathSafe(e.target.value);
    }
  });

  document.getElementById('me-path').addEventListener('input', (e) => {
    e.target.dataset.edited = '1';
  });

  // Form submit
  document.getElementById('manual-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const title = document.getElementById('me-title').value.trim();
    const lib = document.getElementById('me-lib').value;
    const path = document.getElementById('me-path').value.trim();
    const statusEl = document.getElementById('me-status-msg');

    if (!title) { statusEl.innerHTML = '<p class="error">Title is required.</p>'; return; }
    if (!lib) { statusEl.innerHTML = '<p class="error">Please select a library.</p>'; return; }
    if (!path) { statusEl.innerHTML = '<p class="error">Folder name is required.</p>'; return; }

    const body = {
      library_id: lib,
      relative_path: path,
      title,
      other_titles: document.getElementById('me-other-titles').value.trim() 
        ? document.getElementById('me-other-titles').value.trim().split(',').map(t => t.trim()).filter(Boolean)
        : null,
      synopsis: document.getElementById('me-synopsis').value.trim() || null,
      publishing_status: document.getElementById('me-status').value,
      tags: document.getElementById('me-tags').value.trim() 
        ? document.getElementById('me-tags').value.trim().split(',').map(t => t.trim()).filter(Boolean)
        : [],
      start_year: document.getElementById('me-start-year').value ? parseInt(document.getElementById('me-start-year').value, 10) : null,
      end_year: document.getElementById('me-end-year').value ? parseInt(document.getElementById('me-end-year').value, 10) : null,
      cover_url: document.getElementById('me-cover').value.trim() || null,
    };

    statusEl.innerHTML = '<p>Adding...</p>';
    try {
      const manga = await mangaApi.createManual(body);
      navigate(`/series/${manga.id}`);
    } catch(err) {
      statusEl.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
    }
  });
}

window.doSearch = async function() {
  const q = document.getElementById('sq').value.trim();
  if (!q) return;
  document.getElementById('results').innerHTML = '<p>Searching...</p>';
  
  try {
    const results = await search.query(q);
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
      const synopsisShort = synopsis.length > 150 ? synopsis.substring(0, 150) + '...' : synopsis;
      const thumb = m.thumbnail_url 
        ? `<img class="cover" src="${escape(m.thumbnail_url)}" alt="">` 
        : '';
      
      return `
        <tr>
          <td>${thumb}</td>
          <td>
            <b><a href="https://anilist.co/manga/${id}" target="_blank">${escape(title)}</a></b><br>
            ${escape(year)} [${escape(status)}]<br>
            ${(m.metadata?.other_titles || []).map(t => `<span class="badge badge-neutral">${escape(t.title)}</span>`).join(' ')}
            ${synopsisShort ? `<br><small style="color:var(--text-secondary)">${escape(synopsisShort)}</small>` : ''}
          </td>
          <td><button class="btn btn-sm btn-primary" onclick='showAddManga(${id}, "${toPathSafe(title)}")'>Add to Library</button></td>
        </tr>
      `;
    }).join('');
    
    document.getElementById('results').innerHTML = `
      <table>
        <tr><th></th><th>Title</th><th></th></tr>
        ${rows}
      </table>
    `;
  } catch(e) {
    document.getElementById('results').innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.showAddManga = async function(anilistId, pathSafeTitle) {
  render('<p>Loading...</p>');
  try {
    const libs = await libraries.list();
    if (libs.length === 0) {
      render('<p class="error">No libraries found. <a href="/library" data-path="/library">Add one first.</a></p>');
      return;
    }
    
    const libOptions = libs.map(lib => {
      const sel = preselectedLibId === lib.uuid ? 'selected' : '';
      return `<option value="${lib.uuid}" ${sel}>${escape(lib.root_path)}</option>`;
    }).join('');
    
    render(`
      <h2>Add Manga (AniList #${anilistId})</h2>
      <p>Choose a destination library and folder name. Metadata will be fetched on add.</p>
      <form id="add-manga-form">
        <label>Library:</label>
        <select id="am-lib">${libOptions}</select>
        
        <label>Folder name:</label>
        <input type="text" id="am-path" value="${escape(pathSafeTitle)}">
        
        <button type="submit" class="btn btn-primary">Add to Library</button>
        <a href="/search" data-path="/search" class="btn btn-ghost">Cancel</a>
      </form>
      <div id="am-status"></div>
    `);

    document.getElementById('add-manga-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const libId = document.getElementById('am-lib').value;
      const path = document.getElementById('am-path').value.trim();
      const status = document.getElementById('am-status');
      
      if (!path) {
        status.innerHTML = '<p class="error">Folder name required.</p>';
        return;
      }
      
      status.innerHTML = '<p>Adding... (downloading cover, fetching metadata)</p>';
      try {
        const manga = await mangaApi.create({ anilist_id: anilistId, library_id: libId, relative_path: path });
        navigate(`/series/${manga.id}`);
      } catch(err) {
        status.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
      }
    });
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
};

window.viewSearch = viewSearch;
