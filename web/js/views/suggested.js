import { libraries, manga as mangaApi } from '../api.js';
import { render, navigate } from '../router.js';
import { escape, relTime, showToast, skeleton, toPathSafe } from '../utils.js';

let currentLibraryId = null;
let currentSuggestions = [];

function relationLabel(kind) {
  const labels = {
    Adaptation: 'Adaptation',
    Prequel: 'Prequel',
    Sequel: 'Sequel',
    Parent: 'Parent story',
    SideStory: 'Side story',
    Character: 'Character link',
    Summary: 'Summary',
    Alternative: 'Alternative version',
    SpinOff: 'Spin-off',
    Other: 'Related work',
    Source: 'Source material',
    Compilation: 'Compilation',
    Contains: 'Contains',
  };
  return labels[kind] || kind || 'Relation';
}

function buildReasonSummary(item) {
  const rels = item.sources.filter(s => s.source_kind === 'Relation');
  const recs = item.sources.filter(s => s.source_kind === 'Recommendation');
  const bits = [];
  if (rels.length > 0) {
    const grouped = rels.map(s => `${relationLabel(s.relation_type)}: ${s.source_title}`);
    bits.push(grouped.slice(0, 3).join(' • '));
  }
  if (recs.length > 0) {
    bits.push(`Recommended by ${recs.slice(0, 3).map(s => s.source_title).join(', ')}`);
  }
  return bits.join(' | ');
}

function chipTooltip(item, kind) {
  if (kind === 'hits') {
    return item.sources
      .map(source => source.context || `${source.source_kind}: ${source.source_title}`)
      .join('\n');
  }
  if (kind === 'recs') {
    return item.sources
      .filter(source => source.source_kind === 'Recommendation')
      .map(source => {
        const rating = source.rating != null ? ` (${source.rating})` : '';
        return `${source.source_title}${rating}`;
      })
      .join('\n');
  }
  return item.sources
    .filter(source => source.source_kind === 'Relation')
    .map(source => `${relationLabel(source.relation_type)}: ${source.source_title}`)
    .join('\n');
}

function suggestionCard(item) {
  const cover = item.cover_url
    ? `<img class="cover-lg" src="${escape(item.cover_url)}" alt="${escape(item.title)}">`
    : `<img class="cover-lg" src="/web/img/no-cover.svg" alt="No cover">`;
  const tags = (item.tags || []).slice(0, 5).map(tag => `<span class="badge badge-outline">${escape(tag)}</span>`).join(' ');
  const synopsis = item.synopsis
    ? (item.synopsis.length > 320 ? `${item.synopsis.slice(0, 320)}...` : item.synopsis)
    : '';
  const extra = [
    item.media_format || '',
    item.publishing_status || '',
    item.community_rating != null ? `Score ${escape(item.community_rating)}` : '',
    item.popularity != null ? `Popularity ${escape(item.popularity)}` : '',
    item.favourites != null ? `Favs ${escape(item.favourites)}` : '',
    `Weight ${Number(item.weighted_score || 0).toFixed(2)}`,
  ].filter(Boolean).join(' • ');

  return `
    <article class="settings-card" style="display:grid;grid-template-columns:120px 1fr;gap:1rem;max-width:none">
      <div>${cover}</div>
      <div>
        <div class="settings-card-header" style="margin-bottom:0.5rem">
          <h3 style="margin:0">${escape(item.title)}</h3>
          <div style="display:flex;gap:0.4rem;flex-wrap:wrap">
            <span class="badge badge-primary" title="${escape(chipTooltip(item, 'hits'))}">${escape(item.total_occurrences)} hits</span>
            ${item.recommendation_occurrences > 0 ? `<span class="badge badge-secondary" title="${escape(chipTooltip(item, 'recs'))}">${escape(item.recommendation_occurrences)} recs</span>` : ''}
            ${item.relation_occurrences > 0 ? `<span class="badge badge-accent" title="${escape(chipTooltip(item, 'relations'))}">${escape(item.relation_occurrences)} relations</span>` : ''}
          </div>
        </div>
        ${extra ? `<p class="settings-card-desc" style="margin-bottom:0.5rem">${escape(extra)}</p>` : ''}
        ${synopsis ? `<p style="margin:0 0 0.6rem 0;color:var(--text-secondary)">${escape(synopsis)}</p>` : ''}
        ${tags ? `<div style="display:flex;gap:0.35rem;flex-wrap:wrap;margin-bottom:0.8rem">${tags}</div>` : ''}
        <div style="display:flex;gap:0.5rem;flex-wrap:wrap">
          <button
            class="btn btn-sm btn-primary suggested-add-btn"
            data-anilist-id="${item.anilist_id}"
            data-title="${escape(item.title)}"
          >Add to Library</button>
          <a class="btn btn-sm btn-outline" href="https://anilist.co/manga/${item.anilist_id}" target="_blank">Open AniList</a>
          <button class="btn btn-sm btn-ghost suggested-hide-btn" data-anilist-id="${item.anilist_id}">Hide</button>
        </div>
      </div>
    </article>
  `;
}

function renderSuggestedPage(libOptions, refreshedAt) {
  const cards = currentSuggestions.length > 0
    ? currentSuggestions.map(suggestionCard).join('')
    : `<section class="settings-card" style="max-width:760px">
         <div class="settings-card-header">
           <h3>No suggestions yet</h3>
         </div>
         <p class="settings-card-desc">Refresh this library to fetch AniList recommendations and relation links.</p>
       </section>`;

  render(`
    <section class="settings-card" style="max-width:none">
      <div class="settings-card-header">
        <iconify-icon icon="mdi:lightbulb-on-outline" width="20" height="20"></iconify-icon>
        <h3>Suggested Manga</h3>
      </div>
      <p class="settings-card-desc">
        Cached AniList recommendations and related works for the selected library.
      </p>
      <div style="display:flex;gap:0.75rem;flex-wrap:wrap;align-items:end">
        <label style="display:flex;flex-direction:column;gap:0.35rem;min-width:260px">
          <span>Library</span>
          <select id="suggested-library-select">${libOptions}</select>
        </label>
        <button class="btn btn-primary" onclick="refreshSuggestedLibrary()">Refresh suggestions</button>
        ${refreshedAt ? `<span style="opacity:0.8">Last refresh: ${relTime(refreshedAt)}</span>` : '<span style="opacity:0.8">Never refreshed</span>'}
      </div>
    </section>
    <div style="display:grid;gap:1rem;margin-top:1rem">${cards}</div>
  `);

  const select = document.getElementById('suggested-library-select');
  if (select) {
    select.value = currentLibraryId || '';
    select.addEventListener('change', () => {
      currentLibraryId = select.value || null;
      if (currentLibraryId) {
        loadSuggestions();
      }
    });
  }

  document.querySelectorAll('.suggested-add-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      const anilistId = Number(btn.dataset.anilistId);
      const title = btn.dataset.title || 'manga';
      window.addSuggestedToLibrary?.(anilistId, title);
    });
  });

  document.querySelectorAll('.suggested-hide-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      const anilistId = Number(btn.dataset.anilistId);
      window.hideSuggestion?.(anilistId);
    });
  });
}

async function loadSuggestions() {
  render(`<div class="libraries">${skeleton(5)}</div>`);
  try {
    const libList = await libraries.list();
    if (libList.length === 0) {
      render(`<section class="settings-card" style="max-width:760px"><h3>No libraries</h3><p>Create a library first to use suggestions.</p></section>`);
      return;
    }
    if (!currentLibraryId || !libList.some(lib => lib.uuid === currentLibraryId)) {
      currentLibraryId = libList[0].uuid;
    }

    const payload = await libraries.suggestions(currentLibraryId);

    currentSuggestions = payload.suggestions || [];
    const libOptions = libList.map(lib => `<option value="${lib.uuid}">${escape(lib.root_path)}</option>`).join('');
    renderSuggestedPage(libOptions, payload.refreshed_at || null);
  } catch (e) {
    render(`<p class="error">Error loading suggestions: ${escape(e.message)}</p>`);
  }
}

export async function viewSuggested() {
  await loadSuggestions();
  document.title = 'Suggested - REBARR';
}

window.refreshSuggestedLibrary = async function refreshSuggestedLibrary() {
  if (!currentLibraryId) return;
  try {
    const payload = await libraries.refreshSuggestions(currentLibraryId);
    showToast('Suggestions refreshed', 'success');
    currentSuggestions = payload.suggestions || [];
    const libList = await libraries.list();
    const libOptions = libList.map(lib => `<option value="${lib.uuid}">${escape(lib.root_path)}</option>`).join('');
    renderSuggestedPage(libOptions, payload.refreshed_at || null);
  } catch (e) {
    showToast(`Refresh failed: ${e.message}`, 'error');
  }
};

window.hideSuggestion = async function hideSuggestion(anilistId) {
  if (!currentLibraryId) return;
  try {
    const payload = await libraries.setSuggestionHidden(currentLibraryId, anilistId, true);
    currentSuggestions = payload.suggestions || [];
    const libList = await libraries.list();
    const libOptions = libList.map(lib => `<option value="${lib.uuid}">${escape(lib.root_path)}</option>`).join('');
    renderSuggestedPage(libOptions, payload.refreshed_at || null);
    showToast('Suggestion hidden', 'success');
  } catch (e) {
    showToast(`Hide failed: ${e.message}`, 'error');
  }
};

window.addSuggestedToLibrary = async function addSuggestedToLibrary(anilistId, title) {
  if (!currentLibraryId) return;
  try {
    const manga = await mangaApi.create({
      anilist_id: anilistId,
      library_id: currentLibraryId,
      relative_path: toPathSafe(title),
    });
    showToast('Added to library', 'success');
    await loadSuggestions();
    navigate(`/series/${manga.id}`);
  } catch (e) {
    showToast(`Add failed: ${e.message}`, 'error');
  }
};

window.viewSuggested = viewSuggested;
