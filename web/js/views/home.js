// Home view - shows all manga across all libraries

import { libraries } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton } from '../utils.js';

// Sort state persisted to localStorage
const SORT_KEY = 'rebarr_home_sort';
const SEARCH_INPUT_ID = 'home-search-input';

const SORT_OPTIONS = [
  { field: 'title',      label: 'A–Z',             defaultDir: 'asc' },
  { field: 'downloaded', label: 'Downloaded',       defaultDir: 'desc' },
  { field: 'chapters',   label: 'Chapters',         defaultDir: 'desc' },
  { field: 'latest',     label: 'Latest Chapter',   defaultDir: 'desc' },
  { field: 'added',      label: 'Added',            defaultDir: 'desc' },
  { field: 'checked',    label: 'Checked',          defaultDir: 'desc' },
];

function loadSort() {
  try {
    const saved = localStorage.getItem(SORT_KEY);
    if (saved) return JSON.parse(saved);
  } catch (_) {}
  return { field: 'title', dir: 'asc' };
}

function saveSort(s) {
  try { localStorage.setItem(SORT_KEY, JSON.stringify(s)); } catch (_) {}
}

let homeSort = loadSort();
let cachedLibs = [];
let cachedMangaLists = [];
let currentSearchQuery = '';

function normalizeSearchText(str) {
  return str.toLowerCase()
    .replace(/[^a-z0-9\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\uff66-\uff9f]/g, '')
    .trim();
}

function filterManga(mangas, query) {
  if (!query || query.length === 0) return mangas;
  
  const normalizedQuery = normalizeSearchText(query);
  
  return mangas.filter(manga => {
    // Check main title
    const mainTitle = normalizeSearchText(manga.metadata?.title ?? '');
    if (mainTitle.includes(normalizedQuery)) return true;
    
    // Check all alternative/other titles
    if (manga.metadata?.other_titles && Array.isArray(manga.metadata.other_titles)) {
      for (const altTitle of manga.metadata.other_titles) {
        if (normalizeSearchText(altTitle.title ?? '').includes(normalizedQuery)) {
          return true;
        }
      }
    }
    
    return false;
  });
}

function sortManga(mangas) {
  return [...mangas].sort((a, b) => {
    const dir = homeSort.dir === 'asc' ? 1 : -1;
    switch (homeSort.field) {
      case 'title': {
        const ta = (a.metadata?.title ?? '').toLowerCase();
        const tb = (b.metadata?.title ?? '').toLowerCase();
        return ta < tb ? -dir : ta > tb ? dir : 0;
      }
      case 'downloaded':
        return dir * ((a.downloaded_count ?? 0) - (b.downloaded_count ?? 0));
      case 'chapters':
        return dir * ((a.chapter_count ?? 0) - (b.chapter_count ?? 0));
      case 'added':
        return dir * ((a.created_at ?? 0) - (b.created_at ?? 0));
      case 'checked':
        return dir * ((a.last_checked_at ?? 0) - (b.last_checked_at ?? 0));
      case 'latest':
        return dir * ((a.last_chapter_at ?? 0) - (b.last_chapter_at ?? 0));
      default:
        return 0;
    }
  });
}

function buildSearchBar() {
  return `<div class="search-bar flex gap-1 mb-2">
    <input 
      type="text" 
      id="${SEARCH_INPUT_ID}" 
      placeholder="Search series... (Press / to focus)" 
      oninput="setHomeSearch(this.value)"
      value="${escape(currentSearchQuery)}"
      style="flex-grow: 1"
    >
    ${currentSearchQuery.length > 0 ? `<button class="btn btn-ghost" onclick="clearHomeSearch()">✕</button>` : ''}
  </div>`;
}

function buildSortBar() {
  return `<div class="sort-bar">
    <span class="label">Sort:</span>
    ${SORT_OPTIONS.map(opt => {
      const isActive = homeSort.field === opt.field;
      const arrow = isActive ? (homeSort.dir === 'asc' ? ' ↑' : ' ↓') : '';
      return `<button class="sort-btn${isActive ? ' active' : ''}" onclick="setHomeSort('${opt.field}')">${opt.label}${arrow}</button>`;
    }).join('')}
  </div>`;
}

function buildCards(mangas) {
  const filtered = filterManga(mangas, currentSearchQuery);
  
  if (filtered.length === 0) {
    if (currentSearchQuery.length > 0) {
      return '<p><small>No series match your search.</small></p>';
    }
    return '<p><small>No manga yet.</small></p>';
  }
  
  return `<div class="card-grid">${sortManga(filtered).map(m => {
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    const extDl = m.extras_downloaded_count;
    const hasExtras = m.extras_count != null && m.extras_count > 0;
    const extrasStr = hasExtras ? (extDl != null && extDl > 0 ? ` + ${extDl}` : ' +') : '';
    const title = m.metadata?.title ?? 'Unknown';
    const thumb = m.thumbnail_url
      ? `<img src="${escape(m.thumbnail_url)}" alt="${escape(title)}" loading="lazy">`
      : `<img src="/web/img/no-cover.svg" alt="${escape(title)}" loading="lazy">`;
    return `<a class="manga-card" href="/series/${m.id}" data-path="/series/${m.id}">
      ${thumb}
      <div class="info">
        <div class="title">${escape(title)}</div>
        <div class="meta">${dl} / ${total}${extrasStr} chapters</div>
      </div>
    </a>`;
  }).join('')}</div>`;
}

export async function viewHome() {
  render(`<div class="home">${skeleton(5)}</div>`);

  try {
    const libs = await libraries.list();

    if (libs.length === 0) {
      render(`
        <div class="welcome">
          <h2>Welcome to REBARR</h2>
          <p>No libraries configured yet.</p>
          <a href="/library" data-path="/library" class="btn">Add a Library</a>
        </div>
      `);
      return;
    }

    const mangaLists = await Promise.all(libs.map(lib => libraries.manga(lib.uuid)));
    cachedLibs = libs;
    cachedMangaLists = mangaLists;

    renderHome(libs, mangaLists);
  } catch (e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

function renderHome(libs, mangaLists) {
  let html = buildSearchBar();
  html += buildSortBar();
  
  libs.forEach((lib, i) => {
    const mangas = mangaLists[i];
    const type = lib.type === 'Comics' ? 'Comics' : 'Manga';
    html += `<section class="library-section mt-3">`;
    html += `<h3>${escape(lib.root_path)} <small>[${type}]</small></h3>`;
    if (mangas.length === 0) {
      html += `<p><small>No manga yet. <a href="/search?library_id=${lib.uuid}" data-path="/search?library_id=${lib.uuid}">Add some!</a></small></p>`;
    } else {
      html += buildCards(mangas);
    }
    html += `</section>`;
  });
  render(`<div class="home">${html}</div>`);
}

window.setHomeSort = function(field) {
  const opt = SORT_OPTIONS.find(o => o.field === field);
  if (!opt) return;
  if (homeSort.field === field) {
    homeSort.dir = homeSort.dir === 'asc' ? 'desc' : 'asc';
  } else {
    homeSort = { field, dir: opt.defaultDir };
  }
  saveSort(homeSort);
  if (cachedLibs.length > 0) {
    renderHome(cachedLibs, cachedMangaLists);
  }
};

window.setHomeSearch = function(query) {
  currentSearchQuery = query;
  if (cachedLibs.length > 0) {
    renderHome(cachedLibs, cachedMangaLists);
    // Preserve focus and cursor position after re-render
    const input = document.getElementById(SEARCH_INPUT_ID);
    if (input && document.activeElement === input || query.length > 0) {
      input.focus();
      // Move cursor to end of input
      const len = input.value.length;
      input.setSelectionRange(len, len);
    }
  }
};

window.clearHomeSearch = function() {
  currentSearchQuery = '';
  if (cachedLibs.length > 0) {
    renderHome(cachedLibs, cachedMangaLists);
  }
  document.getElementById(SEARCH_INPUT_ID)?.focus();
};

// Global / hotkey handler
document.addEventListener('keydown', (e) => {
  if (e.key === '/' && document.activeElement.tagName !== 'INPUT' && document.activeElement.tagName !== 'TEXTAREA') {
    e.preventDefault();
    const input = document.getElementById(SEARCH_INPUT_ID);
    if (input) {
      input.focus();
      input.select();
    }
  }
});

// Make viewHome available for router
window.viewHome = viewHome;
