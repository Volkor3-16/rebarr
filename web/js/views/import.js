// Import wizard view

import { importApi, libraries } from '../api.js';
import { render } from '../router.js';
import { escape, showToast } from '../utils.js';

// Scan result state — each entry is an ImportCandidate augmented with
// provider_name (default "Local") and is_extra (default false).
let _candidates = [];
// All manga in all libraries, loaded once per scan
let _libraryManga = [];

export async function viewImport() {
  render(`
    <h2>Import Chapters</h2>
    <p class="text-sm opacity-70 mb-4">
      Scan a directory for CBZ files and import them into your library.
      Add the series to your library first, then import chapters here.
    </p>

    <div class="card bg-base-200 p-4 mb-6">
      <div class="flex gap-3 items-end flex-wrap">
        <div class="flex-1 min-w-60">
          <label class="label pb-1"><span class="label-text">Source directory</span></label>
          <input id="import-dir" type="text" class="input input-bordered w-full"
            placeholder="/path/to/manga/files">
        </div>
        <button class="btn btn-primary" onclick="importScan()">
          <iconify-icon icon="mdi:folder-search-outline" width="18" height="18"></iconify-icon>
          Scan
        </button>
      </div>
    </div>

    <div id="import-results"></div>
  `);
}

window.viewImport = viewImport;

// ---------------------------------------------------------------------------
// Load library manga (called once per scan)
// ---------------------------------------------------------------------------

async function loadLibraryManga() {
  const libs = await libraries.list();
  const allManga = await Promise.all(libs.map(lib => libraries.manga(lib.uuid).catch(() => [])));
  _libraryManga = allManga.flat();
}

// Filter _libraryManga by query (case-insensitive substring match on title)
function filterLibraryManga(query) {
  const q = query.trim().toLowerCase();
  if (!q) return _libraryManga.slice(0, 8);
  return _libraryManga
    .filter(m => (m.metadata?.title ?? '').toLowerCase().includes(q))
    .slice(0, 8);
}

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------

window.importScan = async function () {
  const dir = document.getElementById('import-dir').value.trim();
  if (!dir) {
    showToast('Enter a source directory path.', 'warning');
    return;
  }

  const resultsEl = document.getElementById('import-results');
  resultsEl.innerHTML = '<div class="opacity-60">Scanning…</div>';
  _candidates = [];
  _libraryManga = [];

  try {
    let rawCandidates;
    [rawCandidates] = await Promise.all([
      importApi.scan(dir),
      loadLibraryManga(),
    ]);
    _candidates = rawCandidates.map(c => ({
      ...c,
      provider_name: c.provider_name ?? 'Local',
      is_extra: c.is_extra ?? false,
    }));
  } catch (e) {
    resultsEl.innerHTML = `<p class="text-error">Scan failed: ${escape(e.message)}</p>`;
    return;
  }

  if (_candidates.length === 0) {
    resultsEl.innerHTML = '<p class="opacity-60">No CBZ files found in that directory.</p>';
    return;
  }

  renderCandidateTable();
};

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function tierBadge(tier) {
  const map = {
    rebarr:    { label: 'Rebarr',    cls: 'badge-success' },
    comicinfo: { label: 'ComicInfo', cls: 'badge-info' },
    filename:  { label: 'Filename',  cls: 'badge-warning' },
  };
  const t = map[tier] ?? { label: tier, cls: 'badge-ghost' };
  return `<span class="badge ${t.cls} badge-sm">${t.label}</span>`;
}

function confBadge(confidence) {
  if (confidence == null) return '';
  const pct = Math.round(confidence * 100);
  const cls = confidence >= 0.85 ? 'badge-success' : confidence >= 0.6 ? 'badge-warning' : 'badge-error';
  return `<span class="badge ${cls} badge-xs ml-1" title="Match confidence">${pct}%</span>`;
}

function isRowReady(c) {
  return c.chapter_number != null && c.suggested_manga != null;
}

function renderCandidateTable() {
  const resultsEl = document.getElementById('import-results');

  const rows = _candidates.map((c, i) => {
    const chNum = c.chapter_number != null ? c.chapter_number : '';
    const mangaTitle = c.suggested_manga ? escape(c.suggested_manga.title) : '';
    const mangaId = c.suggested_manga ? c.suggested_manga.manga_id : '';
    const autoChecked = isRowReady(c) &&
      (c.import_tier === 'rebarr' || (c.suggested_manga?.confidence ?? 0) >= 0.85);
    const rowCls = !c.suggested_manga ? 'opacity-60' : '';

    return `
      <tr class="${rowCls}" data-idx="${i}">
        <td>
          <input type="checkbox" class="checkbox checkbox-sm candidate-check" data-idx="${i}"
            ${autoChecked ? 'checked' : ''}
            ${!isRowReady(c) ? 'disabled title="Needs series and chapter # set"' : ''}>
        </td>
        <td class="text-xs break-all max-w-xs" title="${escape(c.cbz_path)}">
          ${escape(c.file_name)}
        </td>
        <td>${tierBadge(c.import_tier)}</td>
        <td>
          <input type="number" class="input input-bordered input-xs w-16 chapter-num-input"
            data-idx="${i}" value="${chNum}" step="0.1" min="0" placeholder="Ch#"
            onchange="importUpdateChapterNum(${i}, this.value)">
        </td>
        <td style="position:relative">
          <div class="flex items-center gap-1">
            <input type="text" class="input input-bordered input-xs flex-1 manga-search-input"
              data-idx="${i}" value="${mangaTitle}" placeholder="Search library…"
              data-manga-id="${mangaId}"
              oninput="importSearchManga(${i}, this.value)">
            ${confBadge(c.suggested_manga?.confidence)}
          </div>
          <div id="manga-suggestions-${i}" class="absolute left-0 right-0 z-20 mt-1 menu p-1 shadow bg-base-100 rounded-box text-sm hidden" style="top:100%"></div>
        </td>
        <td>
          <input type="text" class="input input-bordered input-xs w-32"
            value="${escape(c.chapter_title ?? '')}" placeholder="Title…"
            onchange="importUpdateField(${i}, 'chapter_title', this.value)">
        </td>
        <td>
          <input type="text" class="input input-bordered input-xs w-24"
            value="${escape(c.scanlator_group ?? '')}" placeholder="Scanlator…"
            onchange="importUpdateField(${i}, 'scanlator_group', this.value)">
        </td>
        <td>
          <input type="text" class="input input-bordered input-xs w-20"
            value="${escape(c.provider_name ?? 'Local')}" placeholder="Local"
            onchange="importUpdateField(${i}, 'provider_name', this.value)">
        </td>
        <td class="text-center">
          <input type="checkbox" class="checkbox checkbox-sm"
            ${c.is_extra ? 'checked' : ''}
            onchange="importUpdateField(${i}, 'is_extra', this.checked)">
        </td>
      </tr>
    `;
  }).join('');

  resultsEl.innerHTML = `
    <div class="flex gap-2 mb-2 items-center flex-wrap">
      <span class="text-sm opacity-70">${_candidates.length} file(s) found</span>
      <button class="btn btn-xs btn-ghost" onclick="importSelectAll(true)">Select all</button>
      <button class="btn btn-xs btn-ghost" onclick="importSelectAll(false)">Deselect all</button>

      <!-- Bulk series picker -->
      <div class="flex items-center gap-2 ml-auto flex-wrap" style="position:relative">
        <span class="text-sm opacity-70">Set series for selected:</span>
        <div style="position:relative">
          <input type="text" id="bulk-series-input" class="input input-bordered input-sm w-52"
            placeholder="Search library…" oninput="importBulkSearch(this.value)">
          <div id="bulk-suggestions" class="absolute left-0 right-0 z-30 mt-1 menu p-1 shadow bg-base-100 rounded-box text-sm hidden" style="top:100%"></div>
        </div>
        <button class="btn btn-sm btn-secondary" onclick="importApplyBulkSeries()">Apply</button>
      </div>

      <button class="btn btn-sm btn-primary" onclick="importExecute()">
        <iconify-icon icon="mdi:import" width="16" height="16"></iconify-icon>
        Import Selected
      </button>
    </div>

    <!-- Bulk fill (collapsible) -->
    <details class="mb-3">
      <summary class="cursor-pointer select-none text-sm opacity-60 hover:opacity-100 w-fit mb-1">Bulk fill selected rows…</summary>
      <div class="flex gap-4 flex-wrap items-end p-3 bg-base-200 rounded-box">
        <div class="flex flex-col gap-1">
          <span class="text-xs opacity-50">Ch. Title</span>
          <div class="join">
            <input type="text" id="bulk-title-input" class="input input-bordered input-xs join-item w-32" placeholder="value…">
            <button class="btn btn-xs join-item" onclick="importBulkFill('chapter_title', document.getElementById('bulk-title-input').value)">Set</button>
            <button class="btn btn-xs btn-ghost join-item" title="Clear" onclick="importBulkClear('chapter_title')">✕</button>
          </div>
        </div>
        <div class="flex flex-col gap-1">
          <span class="text-xs opacity-50">Scanlator</span>
          <div class="join">
            <input type="text" id="bulk-scanlator-input" class="input input-bordered input-xs join-item w-32" placeholder="value…">
            <button class="btn btn-xs join-item" onclick="importBulkFill('scanlator_group', document.getElementById('bulk-scanlator-input').value)">Set</button>
            <button class="btn btn-xs btn-ghost join-item" title="Clear" onclick="importBulkClear('scanlator_group')">✕</button>
          </div>
        </div>
        <div class="flex flex-col gap-1">
          <span class="text-xs opacity-50">Provider</span>
          <div class="join">
            <input type="text" id="bulk-provider-input" class="input input-bordered input-xs join-item w-24" placeholder="Local">
            <button class="btn btn-xs join-item" onclick="importBulkFill('provider_name', document.getElementById('bulk-provider-input').value || 'Local')">Set</button>
          </div>
        </div>
        <div class="flex flex-col gap-1">
          <span class="text-xs opacity-50">Extra</span>
          <div class="join">
            <button class="btn btn-xs join-item" onclick="importBulkFill('is_extra', true)">Mark extra</button>
            <button class="btn btn-xs btn-ghost join-item" onclick="importBulkFill('is_extra', false)">Unmark</button>
          </div>
        </div>
      </div>
    </details>

    <div class="overflow-x-auto">
      <table class="table table-sm table-zebra w-full">
        <thead>
          <tr>
            <th></th>
            <th>File</th>
            <th class="text-xs">Metadata Source</th>
            <th style="width:7rem">Chapter #</th>
            <th style="min-width:14rem">Series</th>
            <th>Ch. Title</th>
            <th>Scanlator</th>
            <th>Provider</th>
            <th>Extra</th>
          </tr>
        </thead>
        <tbody id="candidate-tbody">${rows}</tbody>
      </table>
    </div>
    <div id="import-summary"></div>
  `;

  // Delegated click listener for bulk series picker suggestions
  document.getElementById('bulk-suggestions')?.addEventListener('click', (e) => {
    const el = e.target.closest('.bulk-pick');
    if (!el) return;
    e.preventDefault();
    importBulkPick(el.dataset.id, el.dataset.title);
  });

  // Delegated click listener for per-row manga suggestions
  document.getElementById('candidate-tbody')?.addEventListener('click', (e) => {
    const el = e.target.closest('.manga-pick');
    if (!el) return;
    e.preventDefault();
    importPickManga(parseInt(el.dataset.idx, 10), el.dataset.id, el.dataset.title);
  });
}

// ---------------------------------------------------------------------------
// Row interactions
// ---------------------------------------------------------------------------

window.importSelectAll = function (checked) {
  document.querySelectorAll('.candidate-check:not([disabled])').forEach(cb => {
    cb.checked = checked;
  });
};

window.importUpdateChapterNum = function (idx, val) {
  const parsed = parseFloat(val);
  _candidates[idx].chapter_number = isNaN(parsed) ? null : parsed;
  syncCheckbox(idx);
};

window.importUpdateField = function (idx, field, value) {
  if (typeof value === 'boolean') {
    _candidates[idx][field] = value;
  } else {
    _candidates[idx][field] = value === '' ? null : value;
  }
};

function syncCheckbox(idx) {
  const cb = document.querySelector(`.candidate-check[data-idx="${idx}"]`);
  if (!cb) return;
  const ready = isRowReady(_candidates[idx]);
  cb.disabled = !ready;
  if (!ready) cb.checked = false;
  const row = document.querySelector(`tr[data-idx="${idx}"]`);
  if (row) row.classList.toggle('opacity-60', !_candidates[idx].suggested_manga);
}

// Per-row series search (local library)
let _searchTimers = {};
window.importSearchManga = function (idx, query) {
  clearTimeout(_searchTimers[idx]);
  _searchTimers[idx] = setTimeout(() => {
    const suggestEl = document.getElementById(`manga-suggestions-${idx}`);
    if (!suggestEl) return;
    const results = filterLibraryManga(query);
    if (results.length === 0) { suggestEl.classList.add('hidden'); return; }
    suggestEl.innerHTML = results.map(m => {
      const title = escape(m.metadata?.title ?? '');
      const id = escape(m.id);
      return `<a class="block px-3 py-1 hover:bg-base-200 cursor-pointer rounded manga-pick" data-idx="${idx}" data-id="${id}" data-title="${escape(m.metadata?.title ?? '')}">${title}</a>`;
    }).join('');
    suggestEl.classList.remove('hidden');
  }, 150);
};

window.importPickManga = function (idx, mangaId, mangaTitle) {
  _candidates[idx].suggested_manga = { manga_id: mangaId, title: mangaTitle, confidence: 1.0 };
  const input = document.querySelector(`.manga-search-input[data-idx="${idx}"]`);
  if (input) { input.value = mangaTitle; input.dataset.mangaId = mangaId; }
  document.getElementById(`manga-suggestions-${idx}`)?.classList.add('hidden');
  syncCheckbox(idx);
  // Update confidence badge
  const confEl = input?.nextElementSibling;
  if (confEl) confEl.outerHTML = confBadge(1.0);
};

// ---------------------------------------------------------------------------
// Bulk series picker
// ---------------------------------------------------------------------------

let _bulkTimer = null;
let _bulkManga = null; // { id, title } chosen via bulk picker
let _bulkPicking = false; // suppress clearing _bulkManga while setting input value

window.importBulkSearch = function (query) {
  clearTimeout(_bulkTimer);
  _bulkTimer = setTimeout(() => {
    if (!_bulkPicking) _bulkManga = null;
    const suggestEl = document.getElementById('bulk-suggestions');
    if (!suggestEl) return;
    const results = filterLibraryManga(query);
    if (results.length === 0) { suggestEl.classList.add('hidden'); return; }
    suggestEl.innerHTML = results.map(m => {
      const title = escape(m.metadata?.title ?? '');
      const id = escape(m.id);
      return `<a class="block px-3 py-1 hover:bg-base-200 cursor-pointer rounded bulk-pick" data-id="${id}" data-title="${escape(m.metadata?.title ?? '')}">${title}</a>`;
    }).join('');
    suggestEl.classList.remove('hidden');
  }, 150);
};

window.importBulkPick = function (mangaId, mangaTitle) {
  _bulkPicking = true;
  _bulkManga = { manga_id: mangaId, title: mangaTitle };
  const input = document.getElementById('bulk-series-input');
  if (input) input.value = mangaTitle;
  document.getElementById('bulk-suggestions')?.classList.add('hidden');
  _bulkPicking = false;
};

window.importApplyBulkSeries = function () {
  if (!_bulkManga) { showToast('Select a series from the dropdown first.', 'warning'); return; }

  const checked = [...document.querySelectorAll('.candidate-check:checked')].map(cb =>
    parseInt(cb.dataset.idx, 10)
  );
  if (checked.length === 0) { showToast('No rows selected.', 'warning'); return; }

  for (const idx of checked) {
    importPickManga(idx, _bulkManga.manga_id, _bulkManga.title);
  }
  showToast(`Set series to "${_bulkManga.title}" for ${checked.length} row(s).`, 'success');
};

// ---------------------------------------------------------------------------
// Bulk fill / clear for any column
// ---------------------------------------------------------------------------

function checkedIndices() {
  return [...document.querySelectorAll('.candidate-check:checked')].map(cb =>
    parseInt(cb.dataset.idx, 10)
  );
}

window.importBulkFill = function (field, value) {
  const indices = checkedIndices();
  if (indices.length === 0) { showToast('No rows selected.', 'warning'); return; }
  for (const idx of indices) {
    importUpdateField(idx, field, value);
    // Sync DOM input/checkbox for the affected row
    const row = document.querySelector(`tr[data-idx="${idx}"]`);
    if (!row) continue;
    if (field === 'is_extra') {
      row.querySelectorAll('input[type="checkbox"]:not(.candidate-check)').forEach(el => {
        el.checked = value;
      });
    } else {
      // td order: 0=check, 1=file, 2=meta, 3=ch#, 4=series, 5=title, 6=scanlator, 7=provider, 8=extra
      const fieldInputMap = {
        chapter_title:   5,
        scanlator_group: 6,
        provider_name:   7,
      };
      const tdIdx = fieldInputMap[field];
      if (tdIdx != null) {
        const input = row.querySelectorAll('td')[tdIdx]?.querySelector('input[type="text"], input[type="number"]');
        if (input) input.value = value == null ? '' : value;
      }
    }
  }
  const label = field.replace(/_/g, ' ');
  showToast(`Set ${label} for ${indices.length} row(s).`, 'success');
};

window.importBulkClear = function (field) {
  const indices = checkedIndices();
  if (indices.length === 0) { showToast('No rows selected.', 'warning'); return; }
  for (const idx of indices) {
    importUpdateField(idx, field, null);
    const row = document.querySelector(`tr[data-idx="${idx}"]`);
    const tdIdx = { chapter_title: 5, scanlator_group: 6, provider_name: 7 }[field];
    if (row && tdIdx != null) {
      const input = row.querySelectorAll('td')[tdIdx]?.querySelector('input[type="text"]');
      if (input) input.value = '';
    }
  }
  showToast(`Cleared ${field.replace('_', ' ')} for ${indices.length} row(s).`, 'success');
};

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

window.importExecute = async function () {
  const checked = [...document.querySelectorAll('.candidate-check:checked')].map(cb =>
    parseInt(cb.dataset.idx, 10)
  );
  if (checked.length === 0) { showToast('No files selected.', 'warning'); return; }

  const imports = checked.map(idx => {
    const c = _candidates[idx];
    return {
      cbz_path: c.cbz_path,
      manga_id: c.suggested_manga.manga_id,
      chapter_number: c.chapter_number,
      chapter_title: c.chapter_title ?? null,
      scanlator_group: c.scanlator_group ?? null,
      language: c.language ?? null,
      provider_name: c.provider_name ?? 'Local',
      is_extra: c.is_extra ?? false,
      chapter_uuid: c.chapter_uuid ?? null,
      released_at: c.released_at ?? null,
      downloaded_at: c.downloaded_at ?? null,
      scraped_at: c.scraped_at ?? null,
    };
  });

  const summaryEl = document.getElementById('import-summary');
  if (summaryEl) summaryEl.innerHTML = '<div class="opacity-60 mt-3">Importing…</div>';

  try {
    const result = await importApi.execute(imports);
    const errHtml = result.errors.length > 0
      ? `<ul class="mt-2 text-sm text-error list-disc list-inside">${result.errors.map(e => `<li>${escape(e)}</li>`).join('')}</ul>`
      : '';
    if (summaryEl) {
      summaryEl.innerHTML = `
        <div class="alert mt-4 ${result.moved > 0 ? 'alert-success' : 'alert-warning'}">
          <iconify-icon icon="mdi:check-circle" width="20" height="20"></iconify-icon>
          <span>Imported <strong>${result.moved}</strong> file(s).${result.skipped > 0 ? ` Skipped <strong>${result.skipped}</strong>.` : ''}</span>
        </div>
        ${errHtml}
      `;
    }
    if (result.moved > 0) showToast(`Imported ${result.moved} chapter(s). ScanDisk queued.`, 'success');
  } catch (e) {
    if (summaryEl) summaryEl.innerHTML = `<p class="text-error mt-3">Import failed: ${escape(e.message)}</p>`;
  }
};
