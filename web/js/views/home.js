// Home view - shows all manga across all libraries

import { libraries } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton } from '../utils.js';

// Sort state persisted to localStorage
const SORT_KEY = 'rebarr_home_sort';

const SORT_OPTIONS = [
  { field: 'title',      label: 'A–Z',       defaultDir: 'asc' },
  { field: 'downloaded', label: 'Downloaded', defaultDir: 'desc' },
  { field: 'chapters',   label: 'Chapters',   defaultDir: 'desc' },
  { field: 'added',      label: 'Added',      defaultDir: 'desc' },
  { field: 'checked',    label: 'Checked',    defaultDir: 'desc' },
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
      default:
        return 0;
    }
  });
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
  if (mangas.length === 0) return '<p><small>No manga yet.</small></p>';
  return `<div class="card-grid">${sortManga(mangas).map(m => {
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    const title = m.metadata?.title ?? 'Unknown';
    const thumb = m.thumbnail_url
      ? `<img src="${escape(m.thumbnail_url)}" alt="${escape(title)}" loading="lazy">`
      : `<img src="/web/img/no-cover.svg" alt="${escape(title)}" loading="lazy">`;
    return `<a class="manga-card" href="/series/${m.id}" data-path="/series/${m.id}">
      ${thumb}
      <div class="info">
        <div class="title">${escape(title)}</div>
        <div class="meta">${dl} / ${total} chapters</div>
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
  let html = buildSortBar();
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

// Make viewHome available for router
window.viewHome = viewHome;
