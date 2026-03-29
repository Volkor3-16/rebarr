// First-run setup wizard

import { importApi, libraries, providers, search, settings, providerScores } from '../api.js';
import { escape } from '../utils.js';

const TOTAL_STEPS = 5;

// ---------------------------------------------------------------------------
// Module-level series import state
// Needs to be module-level so window.wizSeries* handlers can access it.
// ---------------------------------------------------------------------------
const _series = {
  folders: [],        // FolderEntry[]
  matches: {},        // { folderName → { anilist_id, title, year } | null }
  overrides: {},      // manually chosen: { folderName → { anilist_id, title } }
  relPaths: {},       // { folderName → string }
  included: {},       // { folderName → bool }
  duplicates: {},     // { folderName → 'library' | 'scan' } — pre-import duplicate flags
  existingAnilistIds: new Set(), // anilist_ids already in the target library
  matchQueue: [],
  matchRunning: false,
  libraryId: null,
  result: null,       // SeriesImportSummary after execute
};

// ---------------------------------------------------------------------------
// Wizard
// ---------------------------------------------------------------------------

export function showWizard(onComplete) {
  let currentStep = 1;
  let pendingSettings = {};
  let libraryCreated = false;
  let providerList = [];
  let providerChanges = {};
  let seriesSubstep = 'intro'; // 'intro' | 'scanning' | 'review' | 'done' | 'skip'
  let step4Libraries = [];

  const overlay = document.createElement('div');
  overlay.id = 'wizard-overlay';
  Object.assign(overlay.style, {
    position: 'fixed',
    inset: '0',
    zIndex: '9999',
    background: 'rgba(0,0,0,0.65)',
    backdropFilter: 'blur(4px)',
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'center',
    overflowY: 'auto',
    padding: '2rem 1rem',
  });
  document.body.appendChild(overlay);

  // -------------------------------------------------------------------------
  // Core render
  // -------------------------------------------------------------------------

  function rerenderOverlay() {
    overlay.innerHTML = `
      <div style="max-width:660px;width:100%;margin:auto;
        background:var(--b1,#1d232a);border-radius:1rem;
        padding:1.75rem;box-shadow:0 24px 64px rgba(0,0,0,0.5);
        border:1px solid var(--b3,#374151)">
        ${stepIndicatorHtml()}
        <div id="wizard-body">${stepBodyHtml(currentStep)}</div>
        ${navHtml()}
      </div>
    `;
    wireHandlers();
    if (currentStep === 2) loadProviders();
    if (currentStep === 4 && seriesSubstep === 'intro') loadStep4Libraries();
  }

  function stepIndicatorHtml() {
    const labels = ['Library', 'Providers', 'Priority', 'Import', 'Tutorial'];
    const parts = [];
    labels.forEach((label, i) => {
      const n = i + 1;
      const active = n === currentStep;
      const done = n < currentStep;
      const bg = done
        ? 'var(--su, #36d399)'
        : active
        ? 'var(--p, #7c3aed)'
        : 'var(--b3, #374151)';
      const color = done || active ? '#fff' : 'var(--bc, #a6adbb)';
      const inner = done
        ? '<iconify-icon icon="mdi:check" width="14"></iconify-icon>'
        : n;
      parts.push(`
        <div style="display:flex;flex-direction:column;align-items:center;gap:0.2rem">
          <div style="width:1.75rem;height:1.75rem;border-radius:50%;display:flex;align-items:center;
            justify-content:center;background:${bg};color:${color};font-size:0.75rem;font-weight:600">
            ${inner}
          </div>
          <small style="color:${active ? 'var(--p)' : 'var(--bc, #a6adbb)'};font-size:0.6rem;white-space:nowrap">
            ${label}
          </small>
        </div>
      `);
      if (i < labels.length - 1) {
        parts.push(`<div style="flex:1;height:1px;background:var(--b3);margin:0.875rem 0.25rem 1.2rem"></div>`);
      }
    });
    return `<div style="display:flex;align-items:flex-start;margin-bottom:1.5rem">${parts.join('')}</div>`;
  }

  function navHtml() {
    const isFirst = currentStep === 1;
    const isLast = currentStep === TOTAL_STEPS;
    const hideNext = currentStep === 4 && seriesSubstep === 'scanning';
    const nextLabel = (currentStep === 4 && seriesSubstep === 'review') ? 'Skip import →' : 'Next →';
    return `
      <div style="display:flex;justify-content:space-between;margin-top:1.25rem">
        <button class="btn btn-ghost btn-sm" id="wizard-back-btn" ${isFirst ? 'disabled' : ''}>← Back</button>
        ${isLast
          ? `<button class="btn btn-primary btn-sm" id="wizard-finish-btn">Finish Setup</button>`
          : hideNext
          ? `<span></span>`
          : `<button class="btn btn-primary btn-sm" id="wizard-next-btn">${nextLabel}</button>`}
      </div>
    `;
  }

  function wireHandlers() {
    document.getElementById('wizard-back-btn')?.addEventListener('click', () => {
      if (currentStep === 4 && seriesSubstep !== 'intro') {
        if (seriesSubstep === 'scanning' || seriesSubstep === 'review') {
          seriesSubstep = 'intro';
          _series.matchQueue = [];
          _series.matchRunning = false;
        } else {
          seriesSubstep = 'intro';
        }
        rerenderOverlay();
        return;
      }
      if (currentStep > 1) { currentStep--; rerenderOverlay(); }
    });
    document.getElementById('wizard-next-btn')?.addEventListener('click', () => advanceStep());
    document.getElementById('wizard-finish-btn')?.addEventListener('click', () => finishWizard());
    document.getElementById('wizard-create-lib-btn')?.addEventListener('click', createLibrary);
    document.getElementById('wizard-default-monitored')?.addEventListener('change', e => {
      pendingSettings.default_monitored = e.target.checked;
    });
    document.getElementById('wizard-auto-unmonitor-completed')?.addEventListener('change', e => {
      pendingSettings.auto_unmonitor_completed = e.target.checked;
    });
    document.querySelectorAll('input[name="wizard-min-tier"]').forEach(radio => {
      radio.addEventListener('change', e => {
        pendingSettings.min_tier = parseInt(e.target.value, 10);
        highlightRadioGroup('input[name="wizard-min-tier"]', e.target.value);
      });
    });

    // Step 4 handlers
    document.getElementById('wiz-series-scan-btn')?.addEventListener('click', startSeriesScan);
    document.getElementById('wiz-series-skip')?.addEventListener('click', () => {
      seriesSubstep = 'skip'; rerenderOverlay();
    });
    document.getElementById('wiz-series-review-btn')?.addEventListener('click', () => {
      seriesSubstep = 'review'; rerenderOverlay();
    });
    document.getElementById('wiz-series-add-btn')?.addEventListener('click', executeSeriesImport);
    document.getElementById('wiz-series-review-skip')?.addEventListener('click', () => {
      seriesSubstep = 'skip'; rerenderOverlay();
    });
    document.getElementById('wiz-series-done-next')?.addEventListener('click', () => advanceStep());
  }

  function highlightRadioGroup(selector, selectedValue) {
    document.querySelectorAll(selector).forEach(radio => {
      const label = radio.closest('label');
      if (!label) return;
      const sel = radio.value === selectedValue;
      label.style.borderColor = sel ? 'var(--p)' : 'var(--b3, #374151)';
      label.style.background = sel ? 'var(--b2)' : 'transparent';
    });
  }

  async function advanceStep() {
    if (currentStep === 2) await saveProviders();
    if (currentStep === 4 && seriesSubstep !== 'done' && seriesSubstep !== 'skip') return;
    if (currentStep < TOTAL_STEPS) { currentStep++; rerenderOverlay(); }
  }

  async function finishWizard() {
    const btn = document.getElementById('wizard-finish-btn');
    if (btn) { btn.disabled = true; btn.textContent = 'Saving…'; }
    pendingSettings.wizard_completed = true;
    try {
      await settings.update(pendingSettings);
      overlay.remove();
      onComplete(false);
    } catch (e) {
      if (btn) { btn.disabled = false; btn.textContent = 'Finish Setup'; }
      const errEl = document.getElementById('wizard-finish-error');
      if (errEl) errEl.textContent = 'Error saving settings: ' + e.message;
    }
  }

  // -------------------------------------------------------------------------
  // Step content
  // -------------------------------------------------------------------------

  function stepBodyHtml(step) {
    switch (step) {
      case 1: return step1Html();
      case 2: return step2LoadingHtml();
      case 3: return step3Html();
      case 4: return step4Html();
      case 5: return step5Html();
      default: return '';
    }
  }

  // --- Step 1: Library Setup ---

  function step1Html() {
    const monitored = pendingSettings.default_monitored !== false;
    const autoUnmonitorCompleted = pendingSettings.auto_unmonitor_completed === true;
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:folder-plus-outline" width="20"></iconify-icon>
          <h3>Library Setup</h3>
        </div>
        <p class="settings-card-desc">
          Create the folder where your manga will be stored on disk.
          You can skip this and add libraries later from the Settings page.
        </p>
        ${libraryCreated
          ? `<p style="color:var(--su,#36d399);margin-bottom:0.75rem">
               <iconify-icon icon="mdi:check-circle" width="16"></iconify-icon> Library created.
             </p>`
          : ''}
        <div style="display:flex;flex-direction:column;gap:0.6rem;margin-bottom:0.75rem">
          <label class="flex gap-1 align-center">
            <span style="min-width:50px">Type:</span>
            <select id="wizard-lib-type" class="select select-bordered select-sm">
              <option value="Manga">Manga</option>
              <option value="Comics">Comics</option>
            </select>
          </label>
          <label class="flex gap-1 align-center">
            <span style="min-width:50px">Path:</span>
            <input type="text" id="wizard-lib-path" class="input input-bordered input-sm"
              style="flex:1" placeholder="/srv/manga">
          </label>
        </div>
        <div style="display:flex;align-items:center;gap:0.5rem;margin-bottom:1rem">
          <button id="wizard-create-lib-btn" class="btn btn-sm btn-outline">Create Library</button>
          <span id="wizard-lib-status" style="font-size:0.82rem"></span>
        </div>
        <div style="border-top:1px solid var(--b3);padding-top:0.75rem">
          <label style="display:flex;gap:0.5rem;align-items:center;cursor:pointer">
            <input type="checkbox" id="wizard-default-monitored" class="checkbox checkbox-sm"
              ${monitored ? 'checked' : ''}>
            <span>Monitor new series by default</span>
          </label>
          <p style="font-size:0.78rem;opacity:0.65;margin:0.2rem 0 0 1.6rem">
            When enabled, newly-added manga will automatically be checked for chapter updates.
          </p>
          <label style="display:flex;gap:0.5rem;align-items:center;cursor:pointer;margin-top:0.85rem">
            <input type="checkbox" id="wizard-auto-unmonitor-completed" class="checkbox checkbox-sm"
              ${autoUnmonitorCompleted ? 'checked' : ''}>
            <span>Automatically unmonitor completed AniList manga</span>
          </label>
          <p style="font-size:0.78rem;opacity:0.65;margin:0.2rem 0 0 1.6rem">
            Completed series stop scheduled checks and auto-download monitoring when they are added or refreshed.
          </p>
        </div>
      </div>
    `;
  }

  async function createLibrary() {
    const type = document.getElementById('wizard-lib-type')?.value;
    const path = document.getElementById('wizard-lib-path')?.value.trim();
    const status = document.getElementById('wizard-lib-status');
    if (!path) { status.textContent = 'Enter a path.'; status.style.color = 'var(--er)'; return; }
    const btn = document.getElementById('wizard-create-lib-btn');
    if (btn) btn.disabled = true;
    status.textContent = 'Creating…'; status.style.color = '';
    try {
      await libraries.create({ library_type: type, root_path: path });
      libraryCreated = true;
      status.textContent = 'Created!';
      status.style.color = 'var(--su, #36d399)';
    } catch (e) {
      status.textContent = 'Error: ' + e.message;
      status.style.color = 'var(--er)';
    } finally {
      if (btn) btn.disabled = false;
    }
  }

  // --- Step 2: Provider Configuration ---

  function step2LoadingHtml() {
    return `<div class="settings-card"><p>Loading providers…</p></div>`;
  }

  async function loadProviders() {
    try {
      providerList = await providers.list();
      const scoreResults = await Promise.allSettled(
        providerList.map(p => providerScores.getGlobal(p.name).then(s => ({ name: p.name, ...s })))
      );
      for (const r of scoreResults) {
        if (r.status === 'fulfilled') {
          const { name, score, enabled } = r.value;
          providerChanges[name] ??= { score: score ?? 0, enabled: enabled ?? true };
        }
      }

      const rows = providerList.map(p => {
        const state = providerChanges[p.name] ?? { score: 0, enabled: true };
        return `
          <tr>
            <td>${escape(p.name)}</td>
            <td>
              <input type="number" class="input input-bordered input-xs wiz-score"
                data-p="${escape(p.name)}" value="${state.score}" min="-100" max="100"
                style="width:65px">
            </td>
            <td>
              <input type="checkbox" class="checkbox checkbox-sm wiz-enabled"
                data-p="${escape(p.name)}" ${state.enabled ? 'checked' : ''}>
            </td>
          </tr>
        `;
      }).join('');

      document.getElementById('wizard-body').innerHTML = `
        <div class="settings-card">
          <div class="settings-card-header">
            <iconify-icon icon="mdi:server-outline" width="20"></iconify-icon>
            <h3>Provider Configuration</h3>
          </div>
          <p class="settings-card-desc">
            Providers are the sources Rebarr searches for chapters.
            Higher scores are preferred within the same tier.
          </p>
          ${providerList.length === 0
            ? '<p>No providers loaded. Add YAML files to the <code>providers/</code> directory and restart.</p>'
            : `<table style="width:100%">
                 <thead><tr><th>Provider</th><th>Score</th><th>Enabled</th></tr></thead>
                 <tbody>${rows}</tbody>
               </table>
               <p style="font-size:0.78rem;opacity:0.65;margin-top:0.5rem">
                 Disabled providers won't be searched.
               </p>`}
        </div>
      `;

      document.querySelectorAll('.wiz-score, .wiz-enabled').forEach(el => {
        el.addEventListener('change', () => syncProviderState(el.dataset.p));
        el.addEventListener('input', () => syncProviderState(el.dataset.p));
      });
    } catch (e) {
      document.getElementById('wizard-body').innerHTML =
        `<div class="settings-card"><p class="error">Failed to load providers: ${escape(e.message)}</p></div>`;
    }
  }

  function syncProviderState(name) {
    const scoreEl = document.querySelector(`.wiz-score[data-p="${CSS.escape(name)}"]`);
    const enabledEl = document.querySelector(`.wiz-enabled[data-p="${CSS.escape(name)}"]`);
    providerChanges[name] = {
      score: scoreEl ? (parseInt(scoreEl.value, 10) || 0) : 0,
      enabled: enabledEl ? enabledEl.checked : true,
    };
  }

  async function saveProviders() {
    document.querySelectorAll('.wiz-score').forEach(el => syncProviderState(el.dataset.p));
    await Promise.allSettled(
      Object.entries(providerChanges).map(([name, { score, enabled }]) =>
        providerScores.setGlobal(name, score, enabled)
      )
    );
  }

  // --- Step 3: Download Priority ---

  function step3Html() {
    const tier = pendingSettings.min_tier ?? 4;
    const options = [
      { value: 1, label: 'Official only (Tier 1)',        desc: 'Only official publisher releases.' },
      { value: 2, label: 'Trusted groups+ (Tier 1–2)',    desc: 'Official releases and named trusted scanlation groups.' },
      { value: 3, label: 'Known groups+ (Tier 1–3)',      desc: 'Official, trusted, and unverified scanlation groups.' },
      { value: 4, label: 'All sources (Tier 1–4)',        desc: 'Includes aggregator sites. Broadest coverage.' },
    ];
    const radios = options.map(opt => {
      const sel = tier == opt.value;
      return `
        <label style="display:flex;gap:0.75rem;align-items:flex-start;cursor:pointer;
          padding:0.6rem 0.75rem;border-radius:0.5rem;
          border:1px solid ${sel ? 'var(--p)' : 'var(--b3, #374151)'};
          background:${sel ? 'var(--b2)' : 'transparent'}">
          <input type="radio" name="wizard-min-tier" class="radio radio-sm radio-primary"
            value="${opt.value}" ${sel ? 'checked' : ''} style="margin-top:0.15rem">
          <div>
            <div style="font-weight:500">${opt.label}</div>
            <div style="font-size:0.8rem;opacity:0.7">${opt.desc}</div>
          </div>
        </label>
      `;
    }).join('');
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:sort-descending" width="20"></iconify-icon>
          <h3>Download Priority</h3>
        </div>
        <p class="settings-card-desc">
          Choose the minimum scanlation tier Rebarr will consider when selecting chapters.
        </p>
        <div style="display:flex;flex-direction:column;gap:0.5rem;margin:0.75rem 0">${radios}</div>
        <p style="font-size:0.78rem;opacity:0.65">
          Trusted scanlation groups (Tier 2) are managed on the Settings page.
        </p>
      </div>
    `;
  }

  // --- Step 4: Import Existing Library ---

  function step4Html() {
    switch (seriesSubstep) {
      case 'intro':    return step4IntroHtml();
      case 'scanning': return step4ScanningHtml();
      case 'review':   return step4ReviewHtml();
      case 'done':     return step4DoneHtml();
      case 'skip':     return step4SkipHtml();
      default:         return step4IntroHtml();
    }
  }

  async function loadStep4Libraries() {
    if (step4Libraries.length > 0) return;
    try {
      step4Libraries = await libraries.list();
      const body = document.getElementById('wizard-body');
      if (body && currentStep === 4 && seriesSubstep === 'intro') {
        body.innerHTML = step4IntroHtml();
        wireHandlers();
      }
    } catch (_) {}
  }

  function step4IntroHtml() {
    const libOptions = step4Libraries.length > 0
      ? step4Libraries.map(l => `<option value="${escape(l.uuid)}">${escape(l.root_path)}</option>`).join('')
      : `<option value="">Loading…</option>`;
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:import" width="20"></iconify-icon>
          <h3>Import Existing Library</h3>
        </div>
        <p class="settings-card-desc">
          If you have manga already organized as subfolders, Rebarr can scan them,
          match each folder to AniList, and bulk-add them to your library.
        </p>
        <div style="display:flex;flex-direction:column;gap:0.6rem;margin:0.75rem 0">
          <label class="flex gap-1 align-center">
            <span style="min-width:90px;flex-shrink:0">Library:</span>
            <select id="wiz-series-lib" class="select select-bordered select-sm flex-1">
              ${libOptions}
            </select>
          </label>
          <label class="flex gap-1 align-center">
            <span style="min-width:90px;flex-shrink:0">Source dir:</span>
            <input type="text" id="wiz-series-dir" class="input input-bordered input-sm flex-1"
              placeholder="/old/manga/library">
          </label>
        </div>
        <div id="wiz-series-scan-error" style="color:var(--er);font-size:0.82rem;margin-bottom:0.5rem"></div>
        <div style="display:flex;gap:0.75rem;align-items:center">
          <button id="wiz-series-scan-btn" class="btn btn-sm btn-primary"
            ${step4Libraries.length === 0 ? 'disabled' : ''}>
            <iconify-icon icon="mdi:folder-search-outline" width="16"></iconify-icon>
            Scan Folder
          </button>
          <button id="wiz-series-skip" class="btn btn-sm btn-ghost">Skip →</button>
        </div>
      </div>
    `;
  }

  async function startSeriesScan() {
    const dir = document.getElementById('wiz-series-dir')?.value.trim();
    const libId = document.getElementById('wiz-series-lib')?.value;
    const errEl = document.getElementById('wiz-series-scan-error');
    if (!dir) { if (errEl) errEl.textContent = 'Enter a source directory.'; return; }
    if (!libId) { if (errEl) errEl.textContent = 'Select a library.'; return; }

    seriesSubstep = 'scanning';
    _series.folders = [];
    _series.matches = {};
    _series.overrides = {};
    _series.relPaths = {};
    _series.included = {};
    _series.duplicates = {};
    _series.existingAnilistIds = new Set();
    _series.matchQueue = [];
    _series.matchRunning = false;
    _series.libraryId = libId;
    rerenderOverlay();

    try {
      _series.folders = await importApi.seriesScan(dir);
    } catch (e) {
      seriesSubstep = 'intro';
      rerenderOverlay();
      const err2 = document.getElementById('wiz-series-scan-error');
      if (err2) err2.textContent = 'Scan failed: ' + e.message;
      return;
    }

    if (_series.folders.length === 0) {
      seriesSubstep = 'intro';
      rerenderOverlay();
      const err2 = document.getElementById('wiz-series-scan-error');
      if (err2) err2.textContent = 'No subdirectories found.';
      return;
    }

    let existingPaths = new Set();
    try {
      const existing = await libraries.manga(libId);
      existingPaths = new Set(existing.map(m => m.relative_path));
      _series.existingAnilistIds = new Set(existing.map(m => m.anilist_id).filter(Boolean));
    } catch (_) {}

    for (const f of _series.folders) {
      _series.relPaths[f.folder_name] = f.folder_name;
      _series.included[f.folder_name] = !existingPaths.has(f.folder_name);
    }

    rerenderOverlay();
    startMatchQueue();
  }

  function step4ScanningHtml() {
    const rows = _series.folders.map(f => `
      <tr>
        <td class="text-sm" style="max-width:240px;word-break:break-word">${escape(f.folder_name)}</td>
        <td class="text-center opacity-60">${f.cbz_count}</td>
        <td id="wiz-match-${CSS.escape(f.folder_name)}" style="font-size:0.85rem">
          <span class="opacity-40">searching…</span>
        </td>
      </tr>
    `).join('');
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:magnify" width="20"></iconify-icon>
          <h3>Matching ${_series.folders.length} folder${_series.folders.length === 1 ? '' : 's'}…</h3>
        </div>
        <p class="settings-card-desc" style="font-size:0.8rem">
          Searching AniList one at a time. Results fill in as they arrive.
        </p>
        <div style="max-height:340px;overflow-y:auto;margin:0.75rem 0">
          <table class="table table-xs table-zebra w-full">
            <thead><tr><th>Folder</th><th>CBZs</th><th>AniList Match</th></tr></thead>
            <tbody>${rows}</tbody>
          </table>
        </div>
        <button id="wiz-series-review-btn" class="btn btn-sm btn-primary hidden">
          Review Matches →
        </button>
      </div>
    `;
  }

  function startMatchQueue() {
    _series.matchQueue = _series.folders.map(f => f.folder_name);
    if (_series.matchRunning) return;
    _series.matchRunning = true;
    drainNextMatch();
  }

  async function drainNextMatch() {
    if (_series.matchQueue.length === 0) {
      _series.matchRunning = false;
      const btn = document.getElementById('wiz-series-review-btn');
      if (btn) btn.classList.remove('hidden');
      return;
    }

    const folderName = _series.matchQueue.shift();
    const cell = document.getElementById(`wiz-match-${CSS.escape(folderName)}`);

    try {
      const results = await search.query(folderName);
      if (results && results.length > 0) {
        const top = results[0];
        _series.matches[folderName] = {
          anilist_id: top.anilist_id,
          title: top.metadata?.title ?? '',
          year: top.metadata?.start_year ?? null,
        };
        // Flag as library duplicate if this anilist_id is already in the library
        if (_series.existingAnilistIds.has(top.anilist_id)) {
          _series.duplicates[folderName] = 'library';
          _series.included[folderName] = false;
        }
      } else {
        _series.matches[folderName] = null;
      }
    } catch (_) {
      _series.matches[folderName] = null;
    }

    if (cell) {
      const match = _series.matches[folderName];
      if (_series.duplicates[folderName] === 'library') {
        cell.innerHTML = `<span>${escape(match.title)}</span>`
          + (match.year ? `<span class="opacity-50 text-xs ml-1">(${match.year})</span>` : '')
          + ` <span class="badge badge-warning badge-xs">In library</span>`;
      } else if (match) {
        cell.innerHTML = `<span>${escape(match.title)}</span>`
          + (match.year ? `<span class="opacity-50 text-xs ml-1">(${match.year})</span>` : '');
      } else {
        cell.innerHTML = `<span class="text-error text-xs">No match</span>`;
      }
    }

    setTimeout(drainNextMatch, 700);
  }

  function step4ReviewHtml() {
    // Build a set of anilist_ids seen so far to detect within-scan duplicates
    const seenAnilistIds = new Map(); // anilist_id → first folderName
    for (const f of _series.folders) {
      const match = _series.overrides[f.folder_name] ?? _series.matches[f.folder_name];
      if (match?.anilist_id && !_series.duplicates[f.folder_name]) {
        if (seenAnilistIds.has(match.anilist_id)) {
          _series.duplicates[f.folder_name] = 'scan';
          _series.included[f.folder_name] = false;
        } else {
          seenAnilistIds.set(match.anilist_id, f.folder_name);
        }
      }
    }

    const rows = _series.folders.map(f => {
      const match = _series.overrides[f.folder_name] ?? _series.matches[f.folder_name];
      const noMatch = !match || !match.anilist_id;
      const relPath = _series.relPaths[f.folder_name] ?? f.folder_name;
      const included = _series.included[f.folder_name] ?? !noMatch;
      const isOverride = !!_series.overrides[f.folder_name];
      const dupKind = _series.duplicates[f.folder_name];
      const isDisabled = noMatch || !!dupKind;

      let matchHtml;
      if (dupKind === 'library') {
        matchHtml = `${escape(match.title)}${match.year ? ` <span class="opacity-50 text-xs">(${match.year})</span>` : ''}
          <span class="badge badge-warning badge-xs">In library</span>`;
      } else if (dupKind === 'scan') {
        matchHtml = `${escape(match.title)}${match.year ? ` <span class="opacity-50 text-xs">(${match.year})</span>` : ''}
          <span class="badge badge-warning badge-xs">Duplicate</span>`;
      } else if (noMatch) {
        matchHtml = `<span class="text-error text-xs">No match</span>
          <button class="btn btn-xs btn-ghost ml-1" onclick="wizSeriesChange(${JSON.stringify(f.folder_name)})">Search</button>`;
      } else {
        matchHtml = `${escape(match.title)}${match.year ? ` <span class="opacity-50 text-xs">(${match.year})</span>` : ''}${isOverride ? ' <span class="badge badge-info badge-xs">manual</span>' : ''}
          <button class="btn btn-xs btn-ghost ml-1" onclick="wizSeriesChange(${JSON.stringify(f.folder_name)})">Change</button>`;
      }

      return `
        <tr class="${isDisabled && !dupKind ? 'opacity-50' : ''}">
          <td style="vertical-align:top;padding-top:0.55rem;width:1.5rem">
            <input type="checkbox" class="checkbox checkbox-xs wiz-series-check"
              data-folder="${escape(f.folder_name)}"
              ${included ? 'checked' : ''}
              ${isDisabled ? 'disabled' : ''}
              onchange="wizSeriesToggle(${JSON.stringify(f.folder_name)}, this.checked)">
          </td>
          <td style="padding:0.3rem 0.25rem">
            <div style="display:flex;justify-content:space-between;align-items:baseline;gap:0.5rem">
              <span style="font-weight:500;word-break:break-word;font-size:0.88rem">${escape(f.folder_name)}</span>
              <span style="font-size:0.72rem;opacity:0.5;flex-shrink:0">${f.cbz_count} CBZs</span>
            </div>
            <div style="display:flex;justify-content:space-between;align-items:center;gap:0.5rem;margin-top:0.2rem;flex-wrap:wrap">
              <input type="text" class="input input-bordered input-xs" style="flex:1;min-width:80px;max-width:160px"
                value="${escape(relPath)}"
                onchange="wizSeriesRelPath(${JSON.stringify(f.folder_name)}, this.value)">
              <div id="wiz-review-match-${CSS.escape(f.folder_name)}"
                style="font-size:0.8rem;text-align:right;flex-shrink:0;max-width:55%">
                ${matchHtml}
              </div>
            </div>
          </td>
        </tr>
      `;
    }).join('');

    const selectedCount = _series.folders.filter(f => _series.included[f.folder_name]).length;
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:format-list-checks" width="20"></iconify-icon>
          <h3>Review — ${selectedCount} of ${_series.folders.length} selected</h3>
        </div>
        <div style="display:flex;gap:0.5rem;margin-bottom:0.5rem;flex-wrap:wrap;align-items:center">
          <button class="btn btn-xs btn-ghost" onclick="wizSeriesAll(true)">Select all</button>
          <button class="btn btn-xs btn-ghost" onclick="wizSeriesAll(false)">Deselect all</button>
          <label style="display:flex;gap:0.4rem;align-items:center;margin-left:auto;font-size:0.82rem">
            <input type="checkbox" id="wiz-queue-chapter-scan" class="checkbox checkbox-xs">
            Also queue chapter scan for all added series
          </label>
        </div>
        <div style="max-height:360px;overflow-y:auto;margin-bottom:0.75rem">
          <table class="table table-xs table-zebra w-full">
            <tbody>${rows}</tbody>
          </table>
        </div>
        <div style="display:flex;gap:0.75rem">
          <button id="wiz-series-add-btn" class="btn btn-sm btn-primary"
            ${selectedCount === 0 ? 'disabled' : ''}>
            <iconify-icon icon="mdi:plus-circle-outline" width="16"></iconify-icon>
            Add to Library
          </button>
          <button id="wiz-series-review-skip" class="btn btn-sm btn-ghost">Skip</button>
        </div>
      </div>
    `;
  }

  async function executeSeriesImport() {
    const toImport = _series.folders.filter(f => _series.included[f.folder_name]);
    if (toImport.length === 0) return;

    const queueChapterScan = document.getElementById('wiz-queue-chapter-scan')?.checked ?? false;
    const imports = toImport.map(f => ({
      folder_path: f.folder_path,
      anilist_id: (_series.overrides[f.folder_name] ?? _series.matches[f.folder_name]).anilist_id,
      library_id: _series.libraryId,
      relative_path: _series.relPaths[f.folder_name] ?? f.folder_name,
    }));

    const body = document.getElementById('wizard-body');
    if (body) {
      body.innerHTML = `
        <div class="settings-card">
          <p class="opacity-60">Adding ${imports.length} series to library…</p>
        </div>
      `;
    }

    try {
      _series.result = await importApi.seriesExecute({ imports, queue_chapter_scan: queueChapterScan });
    } catch (e) {
      _series.result = { added: 0, skipped_duplicates: 0, errors: [e.message], manga_ids: [] };
    }

    seriesSubstep = 'done';
    rerenderOverlay();
  }

  function step4DoneHtml() {
    const r = _series.result ?? { added: 0, skipped_duplicates: 0, errors: [], manga_ids: [] };
    const errHtml = r.errors.length > 0
      ? `<ul style="margin-top:0.5rem;font-size:0.8rem;color:var(--er);list-style:disc;padding-left:1.25rem">
           ${r.errors.map(e => `<li>${escape(e)}</li>`).join('')}
         </ul>`
      : '';
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:check-circle-outline" width="20"
            style="color:var(--su,#36d399)"></iconify-icon>
          <h3>Import Complete</h3>
        </div>
        <div style="margin:0.75rem 0">
          <p>Added <strong>${r.added}</strong> series.${r.skipped_duplicates > 0
            ? ` Skipped <strong>${r.skipped_duplicates}</strong> duplicate(s).` : ''}</p>
          ${r.added > 0
            ? `<p style="font-size:0.82rem;opacity:0.7;margin-top:0.25rem">
                 ScanDisk queued for all added series to pick up existing chapters.
               </p>`
            : ''}
          ${errHtml}
        </div>
        <button id="wiz-series-done-next" class="btn btn-sm btn-primary">Continue →</button>
      </div>
    `;
  }

  function step4SkipHtml() {
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:import" width="20"></iconify-icon>
          <h3>Import Existing Library</h3>
        </div>
        <p class="settings-card-desc">
          Skipped. You can always add series from the Search page later.
        </p>
      </div>
    `;
  }

  // --- Step 5: Quick Tutorial ---

  function step5Html() {
    const items = [
      { icon: 'mdi:magnify',
        title: 'Search & Add Manga',
        desc: 'Use the <strong>Search</strong> page to find titles on AniList and add them to your library.' },
      { icon: 'mdi:book-multiple-outline',
        title: 'Series Page',
        desc: 'Click any series to see its chapters. Use <strong>Check New Chapters</strong> to find updates and <strong>Download All Missing</strong> to fetch them.' },
      { icon: 'mdi:clock-outline',
        title: 'Task Queue',
        desc: 'All downloads and scans run in the background. Monitor progress from the <strong>Queue</strong> page.' },
      { icon: 'mdi:cog-outline',
        title: 'Settings',
        desc: 'Adjust scan intervals, manage trusted scanlation groups, and configure providers.' },
    ];
    const listHtml = items.map(item => `
      <div style="display:flex;gap:0.75rem;align-items:flex-start">
        <iconify-icon icon="${item.icon}" width="22"
          style="color:var(--p);flex-shrink:0;margin-top:0.1rem"></iconify-icon>
        <div>
          <div style="font-weight:500">${item.title}</div>
          <div style="font-size:0.82rem;opacity:0.8">${item.desc}</div>
        </div>
      </div>
    `).join('');
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:check-decagram-outline" width="20"></iconify-icon>
          <h3>You're all set!</h3>
        </div>
        <p class="settings-card-desc">Here's a quick overview of Rebarr's main features.</p>
        <div style="display:flex;flex-direction:column;gap:0.9rem;margin:0.75rem 0">${listHtml}</div>
        <div id="wizard-finish-error" style="color:var(--er);margin-top:0.5rem;font-size:0.82rem"></div>
      </div>
    `;
  }

  // Initial render
  rerenderOverlay();
}

window.showWizard = showWizard;

window.viewSetup = function() {
  // Placeholder — the wizard overlay renders on top of this.
  const content = document.getElementById('content');
  if (content) content.innerHTML = '<div style="padding:2rem;opacity:0.4">Setup in progress…</div>';
};

// ---------------------------------------------------------------------------
// Module-level handlers for inline series review interactions.
// These reference the module-level _series object directly.
// ---------------------------------------------------------------------------

window.wizSeriesToggle = function(folderName, checked) {
  _series.included[folderName] = checked;
};

window.wizSeriesRelPath = function(folderName, value) {
  _series.relPaths[folderName] = value.trim() || folderName;
};

window.wizSeriesAll = function(checked) {
  document.querySelectorAll('.wiz-series-check:not([disabled])').forEach(cb => {
    cb.checked = checked;
    _series.included[cb.dataset.folder] = checked;
  });
};

window.wizSeriesChange = function(folderName) {
  // Remove any existing picker modal
  document.getElementById('wiz-picker-modal')?.remove();

  const modal = document.createElement('div');
  modal.id = 'wiz-picker-modal';
  Object.assign(modal.style, {
    position: 'fixed', inset: '0', zIndex: '10001',
    background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(3px)',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    padding: '1rem',
  });
  modal.innerHTML = `
    <div style="background:var(--b1,#1d232a);color:var(--bc);border:1px solid var(--b3,#374151);
      border-radius:0.75rem;padding:1.25rem;width:100%;max-width:540px;
      box-shadow:0 20px 60px rgba(0,0,0,0.5)">
      <div style="font-size:0.85rem;opacity:0.6;margin-bottom:0.5rem">
        Matching: <strong>${escape(folderName)}</strong>
      </div>
      <input id="wiz-picker-input" type="text" class="input input-bordered input-sm w-full"
        placeholder="Search AniList…" value="${escape(folderName)}" autocomplete="off">
      <div id="wiz-picker-results" style="margin-top:0.75rem;max-height:340px;overflow-y:auto;
        display:flex;flex-direction:column;gap:0.4rem">
        <p style="opacity:0.5;font-size:0.85rem">Searching…</p>
      </div>
      <div style="display:flex;justify-content:flex-end;margin-top:0.75rem">
        <button class="btn btn-sm btn-ghost" id="wiz-picker-cancel">Cancel</button>
      </div>
    </div>
  `;
  document.body.appendChild(modal);

  let debounceTimer = null;

  function renderResults(results) {
    const container = document.getElementById('wiz-picker-results');
    if (!container) return;
    if (!results || results.length === 0) {
      container.innerHTML = '<p style="opacity:0.5;font-size:0.85rem">No results found.</p>';
      return;
    }
    container.innerHTML = results.slice(0, 10).map(r => {
      const title = escape(r.metadata?.title ?? '');
      const native = r.metadata?.native_title && r.metadata.native_title !== r.metadata?.title
        ? `<div style="font-size:0.75rem;opacity:0.55">${escape(r.metadata.native_title)}</div>` : '';
      const yr = r.metadata?.start_year ? `<span style="opacity:0.55;font-size:0.78rem;margin-left:0.35rem">(${r.metadata.start_year})</span>` : '';
      const cover = r.thumbnail_url
        ? `<img src="${escape(r.thumbnail_url)}" alt="" style="width:36px;height:52px;object-fit:cover;border-radius:3px;flex-shrink:0">`
        : `<div style="width:36px;height:52px;background:var(--b3);border-radius:3px;flex-shrink:0"></div>`;
      const alLink = `<a href="https://anilist.co/manga/${r.anilist_id}" target="_blank" rel="noopener"
        style="font-size:0.72rem;opacity:0.55;white-space:nowrap;margin-left:0.25rem"
        onclick="event.stopPropagation()">AniList ↗</a>`;
      return `
        <div style="display:flex;gap:0.6rem;align-items:center;padding:0.4rem 0.5rem;
          border-radius:0.4rem;cursor:pointer;border:1px solid transparent"
          class="wiz-picker-result hover:bg-base-200"
          onclick="wizSeriesPick(${JSON.stringify(folderName)}, ${r.anilist_id}, ${JSON.stringify(r.metadata?.title ?? '')})">
          ${cover}
          <div style="flex:1;min-width:0">
            <div style="font-weight:500;font-size:0.88rem">${title}${yr}</div>
            ${native}
          </div>
          ${alLink}
        </div>
      `;
    }).join('');
  }

  async function doSearch(q) {
    const container = document.getElementById('wiz-picker-results');
    if (!container) return;
    if (!q.trim()) { container.innerHTML = ''; return; }
    container.innerHTML = '<p style="opacity:0.5;font-size:0.85rem">Searching…</p>';
    try {
      const results = await search.query(q);
      if (document.getElementById('wiz-picker-modal')) renderResults(results);
    } catch (e) {
      if (container) container.innerHTML = `<p style="color:var(--er);font-size:0.85rem">Search failed: ${escape(e.message)}</p>`;
    }
  }

  const input = document.getElementById('wiz-picker-input');
  input?.addEventListener('input', () => {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => doSearch(input.value), 300);
  });
  document.getElementById('wiz-picker-cancel')?.addEventListener('click', () => modal.remove());
  modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });

  // Kick off initial search with the folder name
  doSearch(folderName);
  setTimeout(() => input?.select(), 0);
};

window.wizSeriesPick = function(folderName, anilistId, title) {
  // Close picker modal
  document.getElementById('wiz-picker-modal')?.remove();

  _series.overrides[folderName] = { anilist_id: anilistId, title };
  // Clear any duplicate flag that was set by auto-match — user has chosen a different entry
  if (_series.duplicates[folderName]) delete _series.duplicates[folderName];

  const cell = document.getElementById(`wiz-review-match-${CSS.escape(folderName)}`);
  if (cell) {
    cell.innerHTML = `${escape(title)} <span class="badge badge-info badge-xs">manual</span>
      <button class="btn btn-xs btn-ghost ml-1"
        onclick="wizSeriesChange(${JSON.stringify(folderName)})">Change</button>`;
  }

  // Enable checkbox if it was disabled (no match or duplicate)
  const cb = document.querySelector(`.wiz-series-check[data-folder="${CSS.escape(folderName)}"]`);
  if (cb) { cb.disabled = false; cb.checked = true; _series.included[folderName] = true; }
};
