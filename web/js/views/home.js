// Home view - shows all manga across all libraries

import { libraries, search as searchApi, manga as mangaApi } from '../api.js';
import { render, navigate } from '../router.js';
import { escape, skeleton, toPathSafe, relTime } from '../utils.js';

const SORT_KEY = 'rebarr_home_sort';
const VIEW_KEY = 'rebarr_home_view';
const SEARCH_INPUT_ID = 'home-search-input';

const SORT_OPTIONS = [
  { field: 'title',      label: 'A–Z',           defaultDir: 'asc' },
  { field: 'downloaded', label: 'Downloaded',     defaultDir: 'desc' },
  { field: 'chapters',   label: 'Chapters',       defaultDir: 'desc' },
  { field: 'latest',     label: 'Latest Chapter', defaultDir: 'desc' },
  { field: 'added',      label: 'Added',          defaultDir: 'desc' },
  { field: 'checked',    label: 'Checked',        defaultDir: 'desc' },
];

const VIEW_MODES = [
  { id: 'grid',  icon: 'mdi:view-grid',        title: 'Grid' },
  { id: 'large', icon: 'mdi:view-module',      title: 'Large thumbnails' },
  { id: 'small', icon: 'mdi:view-comfy',       title: 'Small thumbnails' },
  { id: 'table', icon: 'mdi:format-list-text', title: 'Table' },
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

function loadView() {
  return localStorage.getItem(VIEW_KEY) || 'grid';
}

function saveView(v) {
  try { localStorage.setItem(VIEW_KEY, v); } catch (_) {}
}

let homeSort = loadSort();
let homeView = loadView();
let cachedLibs = [];
let cachedMangaLists = [];
let currentSearchQuery = '';
let anilistDebounceTimer = null;
// null = not queried, 'loading' = in-flight, [] = no results, [...] = results
let anilistResults = null;

function normalizeSearchText(str) {
  return str.toLowerCase()
    .replace(/[^a-z0-9぀-ヿ㐀-䶿一-鿿豈-﫿ｦ-ﾟ]/g, '')
    .trim();
}

function filterManga(mangas, query) {
  if (!query || query.length === 0) return mangas;
  const nq = normalizeSearchText(query);
  return mangas.filter(m => {
    if (normalizeSearchText(m.metadata?.title ?? '').includes(nq)) return true;
    if (Array.isArray(m.metadata?.other_titles)) {
      for (const t of m.metadata.other_titles) {
        if (normalizeSearchText(t.title ?? '').includes(nq)) return true;
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
      case 'downloaded': return dir * ((a.downloaded_count ?? 0) - (b.downloaded_count ?? 0));
      case 'chapters':   return dir * ((a.chapter_count ?? 0) - (b.chapter_count ?? 0));
      case 'added':      return dir * ((a.created_at ?? 0) - (b.created_at ?? 0));
      case 'checked':    return dir * ((a.last_checked_at ?? 0) - (b.last_checked_at ?? 0));
      case 'latest':     return dir * ((a.last_chapter_at ?? 0) - (b.last_chapter_at ?? 0));
      default: return 0;
    }
  });
}

function buildToolbar() {
  const sortOpts = SORT_OPTIONS.map(opt => {
    const isActive = homeSort.field === opt.field;
    const label = isActive
      ? `${opt.label} ${homeSort.dir === 'asc' ? '↑' : '↓'}`
      : opt.label;
    return `<option value="${opt.field}" ${isActive ? 'selected' : ''}>${label}</option>`;
  }).join('');

  const viewBtns = VIEW_MODES.map(m =>
    `<button class="view-mode-btn${homeView === m.id ? ' active' : ''}" title="${m.title}" onclick="setHomeView('${m.id}')">
      <iconify-icon icon="${m.icon}" width="18" height="18"></iconify-icon>
    </button>`
  ).join('');

  return `<div class="home-toolbar" id="home-toolbar">
    <input
      type="text"
      id="${SEARCH_INPUT_ID}"
      class="home-search-input"
      placeholder="Search… (just start typing)"
      oninput="setHomeSearch(this.value)"
      onkeydown="if(event.key==='Escape'){clearHomeSearch();}"
      value="${escape(currentSearchQuery)}"
    >
    <div class="toolbar-right">
      <select class="sort-select" onchange="setHomeSortSelect(this.value)">${sortOpts}</select>
      <div class="view-mode-btns">${viewBtns}</div>
    </div>
  </div>`;
}

function buildLocalCards(mangas, multiLib) {
  const filtered = filterManga(mangas, currentSearchQuery);
  if (filtered.length === 0) return null;

  const sorted = sortManga(filtered);

  if (homeView === 'table') {
    const rows = sorted.map(m => {
      const title = m.metadata?.title ?? 'Unknown';
      const dl = m.downloaded_count ?? 0;
      const total = m.chapter_count != null ? m.chapter_count : '?';
      const status = m.metadata?.publishing_status ?? '';
      const thumb = m.thumbnail_url
        ? `<img class="thumb" src="${escape(m.thumbnail_url)}" alt="" loading="lazy">`
        : `<img class="thumb" src="/web/img/no-cover.svg" alt="" loading="lazy">`;
      const libBadge = multiLib && m._libName
        ? ` <span class="lib-badge">${escape(m._libName)}</span>`
        : '';
      return `<tr>
        <td><a href="/series/${m.id}" data-path="/series/${m.id}">${thumb}</a></td>
        <td><a href="/series/${m.id}" data-path="/series/${m.id}" class="manga-table-title">${escape(title)}</a>${libBadge}</td>
        <td class="text-muted">${escape(status)}</td>
        <td>${dl} / ${total}</td>
        <td>${relTime(m.created_at)}</td>
      </tr>`;
    }).join('');
    return `<table class="manga-table">
      <thead><tr><th></th><th>Title</th><th>Status</th><th>Downloaded</th><th>Added</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
  }

  const gridClass = homeView === 'large' ? 'card-grid card-grid--large'
    : homeView === 'small' ? 'card-grid card-grid--small'
    : 'card-grid';

  return `<div class="${gridClass}">${sorted.map(m => {
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    const extDl = m.extras_downloaded_count;
    const hasExtras = m.extras_count != null && m.extras_count > 0;
    const extrasStr = hasExtras ? (extDl != null && extDl > 0 ? ` + ${extDl}` : ' +') : '';
    const title = m.metadata?.title ?? 'Unknown';
    const thumb = m.thumbnail_url
      ? `<img src="${escape(m.thumbnail_url)}" alt="${escape(title)}" loading="lazy">`
      : `<img src="/web/img/no-cover.svg" alt="${escape(title)}" loading="lazy">`;
    const libBadge = multiLib && m._libName
      ? `<div class="lib-badge">${escape(m._libName)}</div>`
      : '';
    return `<a class="manga-card" href="/series/${m.id}" data-path="/series/${m.id}">
      ${thumb}
      <div class="info">
        <div class="title">${escape(title)}</div>
        ${homeView !== 'small' ? `<div class="meta">${dl} / ${total}${extrasStr} ch.</div>` : ''}
        ${libBadge}
      </div>
    </a>`;
  }).join('')}</div>`;
}

function buildAniListSection() {
  if (!currentSearchQuery || currentSearchQuery.length < 2) return '';
  if (anilistResults === null) return '';
  if (anilistResults === 'loading') {
    return `<div class="anilist-results"><p class="text-muted">Searching AniList…</p></div>`;
  }
  if (anilistResults.length === 0) {
    return `<div class="anilist-results"><p class="text-muted">No results on AniList either.</p></div>`;
  }

  const gridClass = homeView === 'large' ? 'card-grid card-grid--large'
    : homeView === 'small' ? 'card-grid card-grid--small'
    : 'card-grid';

  const cards = anilistResults.map(m => {
    const id = m.anilist_id ?? 0;
    const title = m.metadata?.title ?? 'Unknown';
    const year = m.metadata?.start_year ?? '';
    const status = m.metadata?.publishing_status ?? '';
    const pathSafe = toPathSafe(title);
    const thumb = m.thumbnail_url
      ? `<img src="${escape(m.thumbnail_url)}" alt="${escape(title)}" loading="lazy">`
      : `<img src="/web/img/no-cover.svg" alt="${escape(title)}" loading="lazy">`;
    const meta = [year, status].filter(Boolean).join(' · ');
    return `<div class="manga-card anilist-card" onclick="showAddMangaModal(${id}, '${escape(pathSafe)}')">
      ${thumb}
      <div class="add-overlay"><button class="add-btn" title="Add to library">+</button></div>
      <div class="info">
        <div class="title">${escape(title)}</div>
        ${homeView !== 'small' ? `<div class="meta">${escape(meta)}</div>` : ''}
      </div>
    </div>`;
  }).join('');

  return `<div class="anilist-results">
    <h4><iconify-icon icon="simple-icons:anilist" width="14" height="14"></iconify-icon> From AniList — click + to add</h4>
    <div class="${gridClass}">${cards}</div>
  </div>`;
}

function scheduleAnilistSearch(query) {
  clearTimeout(anilistDebounceTimer);
  if (!query || query.length < 2) {
    anilistResults = null;
    return;
  }
  anilistResults = 'loading';
  anilistDebounceTimer = setTimeout(async () => {
    try {
      const results = await searchApi.query(query);
      if (currentSearchQuery === query) {
        anilistResults = results;
        rerenderContent();
      }
    } catch (_) {
      if (currentSearchQuery === query) {
        anilistResults = [];
        rerenderContent();
      }
    }
  }, 600);
}

function getMangasWithLib() {
  const multiLib = cachedLibs.length > 1;
  return cachedLibs.flatMap((lib, i) => {
    const libName = lib.root_path.split('/').filter(Boolean).pop() || lib.root_path;
    return cachedMangaLists[i].map(m => ({ ...m, _libName: multiLib ? libName : null }));
  });
}

function rerenderContent() {
  const multiLib = cachedLibs.length > 1;
  const mangas = getMangasWithLib();
  const toolbar = buildToolbar();
  const cards = buildLocalCards(mangas, multiLib);

  let body;
  if (cards === null) {
    const aniSection = buildAniListSection();
    body = `<p class="text-muted no-results-msg">No series match your search.</p>${aniSection}`;
  } else {
    body = cards;
  }

  render(`<div class="home">${toolbar}${body}</div>`);

  const input = document.getElementById(SEARCH_INPUT_ID);
  if (input && currentSearchQuery.length > 0) {
    input.focus();
    const len = input.value.length;
    input.setSelectionRange(len, len);
  }
}

export async function viewHome() {
  render(`<div class="home">${skeleton(5)}</div>`);

  try {
    const libs = await libraries.list();

    if (libs.length === 0) {
      render(`
        <div class="welcome">
          <h2>Welcome to REBARR</h2>
          <p>No libraries configured yet. Add one in <a href="/settings" data-path="/settings">Settings</a>.</p>
        </div>
      `);
      return;
    }

    const mangaLists = await Promise.all(libs.map(lib => libraries.manga(lib.uuid)));
    cachedLibs = libs;
    cachedMangaLists = mangaLists;

    rerenderContent();
  } catch (e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
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
  if (cachedLibs.length > 0) rerenderContent();
};

window.setHomeSortSelect = function(field) {
  window.setHomeSort(field);
};

window.setHomeView = function(v) {
  homeView = v;
  saveView(v);
  if (cachedLibs.length > 0) rerenderContent();
};

window.setHomeSearch = function(query) {
  currentSearchQuery = query;
  if (cachedLibs.length === 0) return;

  const allMangas = cachedLibs.flatMap((_, i) => cachedMangaLists[i]);
  const filtered = filterManga(allMangas, query);

  if (filtered.length === 0 && query.length >= 2) {
    scheduleAnilistSearch(query);
  } else {
    clearTimeout(anilistDebounceTimer);
    anilistResults = null;
  }

  rerenderContent();
};

window.clearHomeSearch = function() {
  currentSearchQuery = '';
  clearTimeout(anilistDebounceTimer);
  anilistResults = null;
  if (cachedLibs.length > 0) rerenderContent();
  document.getElementById(SEARCH_INPUT_ID)?.focus();
};

// Global keydown: typing anywhere on home page focuses search and appends the character
document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  if (['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement.tagName)) return;
  if (e.key === 'Escape') { window.clearHomeSearch(); return; }
  if (e.key.length !== 1) return;
  const input = document.getElementById(SEARCH_INPUT_ID);
  if (!input) return;
  e.preventDefault();
  input.focus();
  input.value += e.key;
  window.setHomeSearch(input.value);
});

// ---- Add to first library immediately (no modal) ----

window.showAddMangaModal = async function(anilistId, pathSafeTitle) {
  // Find the card element and show a spinner while adding
  const card = document.querySelector(`.anilist-card[onclick*="${anilistId}"]`);
  if (card) {
    card.style.pointerEvents = 'none';
    card.style.opacity = '0.6';
  }

  try {
    const libs = await libraries.list();
    if (libs.length === 0) {
      alert('No libraries configured. Add one in Settings first.');
      if (card) { card.style.pointerEvents = ''; card.style.opacity = ''; }
      return;
    }
    const lib = libs[0];
    const m = await mangaApi.create({ anilist_id: anilistId, library_id: lib.uuid, relative_path: pathSafeTitle });
    navigate(`/series/${m.id}`);
  } catch (err) {
    if (card) { card.style.pointerEvents = ''; card.style.opacity = ''; }
    alert(`Failed to add: ${err.message}`);
  }
};

window.viewHome = viewHome;
