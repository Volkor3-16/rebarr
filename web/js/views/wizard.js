// First-run setup wizard — full-page version at /setup

import { importApi, libraries, providers, search, settings, providerSettings, qualityRules as qualityRulesApi } from '../api.js';
import { escape } from '../utils.js';

const TOTAL_STEPS = 6;

// ---------------------------------------------------------------------------
// Module-level series import state
// ---------------------------------------------------------------------------
const _series = {
  folders: [],
  matches: {},
  matchAlternatives: {},  // { [folderName]: Array<{anilist_id, title, year, cover_url}> }
  matchStatus: {},        // { [folderName]: 'pending'|'auto_confirmed'|'needs_action'|'confirmed'|'skipped' }
  relPaths: {},
  matchQueue: [],
  matchRunning: false,
  failed: new Set(),
  libraryId: null,
  result: null,
  importQueue: [],       // [{folderPath, mangaId, folderName, match}]
  currentImportIdx: 0,
  seriesCandidates: null,
  importSummary: null,
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
let _currentStep = 1;
let _pendingSettings = {};
let _libraryCreated = false;
let _providerList = [];
let _providerChanges = {};
let _seriesSubstep = 'intro';
let _step4Libraries = [];
let _onComplete = null;
let _pickerFolderIdx = -1;
let _pickerResults = [];
let _qualityRules = null;

// ---------------------------------------------------------------------------
// viewSetup — entry point from router
// ---------------------------------------------------------------------------

export function viewSetup(onComplete) {
  _onComplete = onComplete || null;
  _currentStep = 1;
  _seriesSubstep = 'intro';

  const nav = document.getElementById('nav');
  if (nav) nav.style.display = 'none';
  const headerContainer = document.querySelector('.header-container');
  if (headerContainer) headerContainer.style.display = 'none';

  _pendingSettings = {};
  _providerChanges = {};
  _qualityRules = null;
  _providerSettingsLoaded = false;
  Promise.all([
    libraries.list().then(libs => { if (libs.length > 0) _libraryCreated = true; }).catch(() => {}),
    settings.get().then(s => {
      _pendingSettings = {
        default_monitored:        s.default_monitored,
        auto_unmonitor_completed: s.auto_unmonitor_completed,
        scan_interval_hours:      s.scan_interval_hours,
        browser_worker_count:     s.browser_worker_count,
        preferred_language:       s.preferred_language ?? '',
        disable_chapter_upgrades: s.disable_chapter_upgrades,
        download_mode:            s.download_mode,
      };
    }).catch(() => {}),
    providers.list().then(p => { _providerList = p; }).catch(() => {}),
  ]).then(() => render());
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

function render() {
  const content = document.getElementById('content');
  if (!content) return;

  content.innerHTML = `
    <div class="setup-page">
      <div class="setup-brand">
        <h1 class="setup-title">REBARR</h1>
        <p class="setup-subtitle">Let's get your manga library configured.</p>
      </div>
      ${stepsIndicatorHtml()}
      <div class="setup-card">
        <div class="card-body">
          ${stepBodyHtml(_currentStep)}
          ${navHtml()}
        </div>
      </div>
    </div>
  `;

  wireHandlers();
  if (_currentStep === 4 && _seriesSubstep === 'intro') loadStep4Libraries();
  if (_currentStep === 3) ensureProviderSettingsLoaded();
  if (_currentStep === 5 && !_qualityRules) loadQualityRules();
}

function stepsIndicatorHtml() {
  const labels = ['Library', 'Settings', 'Providers', 'Import', 'Quality', 'Ready'];
  const items = labels.map((label, i) => {
    const n = i + 1;
    const active = n === _currentStep;
    const done = n < _currentStep;
    let cls = '';
    if (done) cls = 'step-primary';
    else if (active) cls = '';
    return `<li class="step ${cls}">${label}</li>`;
  }).join('');
  return `<ul class="steps steps-horizontal mb-8">${items}</ul>`;
}

function navHtml() {
  const isFirst = _currentStep === 1;
  const isLast = _currentStep === TOTAL_STEPS;
  const hideNext = _currentStep === 4 &&
    (_seriesSubstep === 'matching' || _seriesSubstep === 'chapter_import' || _seriesSubstep === 'done');
  let nextLabel = 'Next';
  if (_currentStep === 4 && _seriesSubstep === 'intro') nextLabel = 'Skip';

  return `
    <div class="setup-nav">
      ${isFirst ? '<div></div>' : `<button class="btn btn-ghost" id="wizard-back-btn">Back</button>`}
      ${isLast
        ? `<button class="btn btn-primary" id="wizard-finish-btn">Finish Setup</button>`
        : hideNext ? '<div></div>' : `<button class="btn btn-primary" id="wizard-next-btn">${nextLabel} →</button>`}
    </div>
  `;
}

// ---------------------------------------------------------------------------
// Wire handlers
// ---------------------------------------------------------------------------

function wireHandlers() {
  document.getElementById('wizard-back-btn')?.addEventListener('click', () => {
    if (_currentStep === 4 && _seriesSubstep !== 'intro') {
      _seriesSubstep = 'intro';
      _series.matchQueue = [];
      _series.matchRunning = false;
      render();
      return;
    }
    if (_currentStep > 1) { _currentStep--; render(); }
  });

  document.getElementById('wizard-next-btn')?.addEventListener('click', () => advanceStep());
  document.getElementById('wizard-finish-btn')?.addEventListener('click', () => finishWizard());
  document.getElementById('wizard-create-lib-btn')?.addEventListener('click', createLibrary);
  document.getElementById('wizard-default-monitored')?.addEventListener('change', e => {
    _pendingSettings.default_monitored = e.target.checked;
  });
  document.getElementById('wizard-auto-unmonitor-completed')?.addEventListener('change', e => {
    _pendingSettings.auto_unmonitor_completed = e.target.checked;
  });
  // Step 2: Settings controls
  document.getElementById('wizard-scan-interval')?.addEventListener('input', e => {
    _pendingSettings.scan_interval_hours = parseInt(e.target.value, 10) || 6;
  });
  document.getElementById('wizard-browser-workers')?.addEventListener('input', e => {
    _pendingSettings.browser_worker_count = parseInt(e.target.value, 10) || 3;
  });
  document.getElementById('wizard-preferred-language')?.addEventListener('input', e => {
    _pendingSettings.preferred_language = e.target.value.trim();
  });
  document.getElementById('wizard-disable-upgrades')?.addEventListener('change', e => {
    _pendingSettings.disable_chapter_upgrades = e.target.checked;
  });
  document.getElementById('wizard-download-mode')?.addEventListener('change', e => {
    _pendingSettings.download_mode = e.target.value;
  });
  // Step 3: Provider toggles
  document.querySelectorAll('.wiz-provider-enabled').forEach(el => {
    el.addEventListener('change', () => {
      _providerChanges[el.dataset.p] = { enabled: el.checked };
    });
  });
  document.getElementById('wiz-series-scan-btn')?.addEventListener('click', startSeriesScan);
  document.getElementById('wiz-series-done-next')?.addEventListener('click', () => { _currentStep++; render(); });
}


async function advanceStep() {
  if (_currentStep === 1 && !_libraryCreated) {
    const status = document.getElementById('wizard-lib-status');
    if (status) { status.textContent = 'Create a library to continue.'; status.style.color = 'var(--er)'; }
    return;
  }
  if (_currentStep === 2) {
    try { await settings.update(_pendingSettings); } catch (_) {}
  }
  if (_currentStep === 3) {
    try { await saveProviders(); } catch (_) {}
  }
  if (_currentStep === 4 && _seriesSubstep === 'intro') { _seriesSubstep = 'skip'; render(); return; }
  if (_currentStep === 4 && _seriesSubstep !== 'skip') return;
  if (_currentStep < TOTAL_STEPS) { _currentStep++; render(); }
}

async function finishWizard() {
  const btn = document.getElementById('wizard-finish-btn');
  if (btn) { btn.disabled = true; btn.textContent = 'Saving…'; }
  _pendingSettings.wizard_completed = true;
  try {
    await settings.update(_pendingSettings);
    const nav = document.getElementById('nav');
    if (nav) nav.style.display = '';
    const headerContainer = document.querySelector('.header-container');
    if (headerContainer) headerContainer.style.display = '';
    if (_onComplete) _onComplete(false); else window.navigate('/');
  } catch (e) {
    if (btn) { btn.disabled = false; btn.textContent = 'Finish Setup'; }
    const errEl = document.getElementById('wizard-finish-error');
    if (errEl) errEl.textContent = 'Error saving settings: ' + e.message;
  }
}

// ---------------------------------------------------------------------------
// Step body dispatch
// ---------------------------------------------------------------------------

function stepBodyHtml(step) {
  switch (step) {
    case 1: return step1Html();
    case 2: return step2Html();
    case 3: return step3Html();
    case 4: return step4Html();
    case 5: return step5Html();
    case 6: return step6Html();
    default: return '';
  }
}

// ---------------------------------------------------------------------------
// Step 1: Library Setup
// ---------------------------------------------------------------------------

function step1Html() {
  const monitored = _pendingSettings.default_monitored !== false;
  const autoUnmonitorCompleted = _pendingSettings.auto_unmonitor_completed !== false;
  return `
    <div class="flex items-center gap-2 mb-4">
      <iconify-icon icon="mdi:folder-plus-outline" width="24" class="text-primary"></iconify-icon>
      <h3 class="text-lg font-semibold m-0">Library Setup</h3>
    </div>
    <p class="text-sm opacity-70 mb-4">Create the folder where your manga will be stored on disk.</p>
    ${_libraryCreated ? `<div class="alert alert-success alert-soft mb-4"><iconify-icon icon="mdi:check-circle" width="18"></iconify-icon><span>Library created successfully.</span></div>` : ''}
    <div class="flex flex-col gap-3 mb-4">
      <label class="form-control w-full"><div class="label"><span class="label-text font-medium">Library Type</span></div>
        <select id="wizard-lib-type" class="select select-bordered w-full"><option value="Manga">Manga</option><option value="Comics">Comics</option></select>
      </label>
      <label class="form-control w-full"><div class="label"><span class="label-text font-medium">Root Path</span></div>
        <input type="text" id="wizard-lib-path" class="input input-bordered w-full" placeholder="/srv/manga">
      </label>
    </div>
    <div class="flex items-center gap-3 mb-6">
      <button id="wizard-create-lib-btn" class="btn btn-outline btn-sm"><iconify-icon icon="mdi:folder-plus" width="16"></iconify-icon>Create Library</button>
      <span id="wizard-lib-status" class="text-sm"></span>
    </div>
    <div class="divider"></div>
    <div class="flex flex-col gap-4">
      <label class="flex gap-3 items-start cursor-pointer">
        <input type="checkbox" id="wizard-default-monitored" class="checkbox checkbox-primary checkbox-sm mt-0.5" ${monitored ? 'checked' : ''}>
        <div><span class="text-sm font-medium">Monitor new series by default</span><p class="text-xs opacity-60 mt-0.5">When enabled, newly-added manga will automatically be checked for chapter updates.</p></div>
      </label>
      <label class="flex gap-3 items-start cursor-pointer">
        <input type="checkbox" id="wizard-auto-unmonitor-completed" class="checkbox checkbox-primary checkbox-sm mt-0.5" ${autoUnmonitorCompleted ? 'checked' : ''}>
        <div><span class="text-sm font-medium">Automatically unmonitor completed AniList manga</span><p class="text-xs opacity-60 mt-0.5">Completed series stop scheduled checks and auto-download monitoring when they are added or refreshed.</p></div>
      </label>
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
  try { await libraries.create({ library_type: type, root_path: path }); _libraryCreated = true; render(); }
  catch (e) { status.textContent = 'Error: ' + e.message; status.style.color = 'var(--er)'; }
  finally { if (btn) btn.disabled = false; }
}

// ---------------------------------------------------------------------------
// Step 2: Scheduler & Download Settings
// ---------------------------------------------------------------------------

function step2Html() {
  const s = _pendingSettings;
  const interval = s.scan_interval_hours ?? 6;
  const workers = s.browser_worker_count ?? 3;
  const lang = s.preferred_language ?? '';
  const disableUpgrades = s.disable_chapter_upgrades ?? false;
  const dlMode = s.download_mode ?? 'must_have';
  return `
    <div class="flex items-center gap-2 mb-4">
      <iconify-icon icon="mdi:cog-outline" width="24" class="text-primary"></iconify-icon>
      <h3 class="text-lg font-semibold m-0">Scheduler & Downloads</h3>
    </div>
    <p class="text-sm opacity-70 mb-5">These can be changed at any time from Settings.</p>
    <div class="flex flex-col gap-4">
      <label class="form-control w-full max-w-xs">
        <div class="label"><span class="label-text font-medium">Scan interval (hours)</span></div>
        <input type="number" id="wizard-scan-interval" class="input input-bordered input-sm" value="${interval}" min="1" max="168">
        <div class="label"><span class="label-text-alt opacity-60">How often to check for new chapters.</span></div>
      </label>
      <label class="form-control w-full max-w-xs">
        <div class="label"><span class="label-text font-medium">Browser workers</span></div>
        <input type="number" id="wizard-browser-workers" class="input input-bordered input-sm" value="${workers}" min="1" max="16">
        <div class="label"><span class="label-text-alt opacity-60">Concurrent headless browser instances for scraping. Higher = faster but more RAM.</span></div>
      </label>
      <label class="form-control w-full max-w-xs">
        <div class="label"><span class="label-text font-medium">Preferred language</span></div>
        <input type="text" id="wizard-preferred-language" class="input input-bordered input-sm" value="${escape(lang)}" placeholder="en (BCP 47, leave blank for any)">
        <div class="label"><span class="label-text-alt opacity-60">Prefer chapters in this language when multiple are available.</span></div>
      </label>
      <div class="divider my-1"></div>
      <label class="flex gap-3 items-start cursor-pointer">
        <input type="checkbox" id="wizard-disable-upgrades" class="checkbox checkbox-primary checkbox-sm mt-0.5" ${disableUpgrades ? 'checked' : ''}>
        <div>
          <span class="text-sm font-medium">Disable chapter upgrades</span>
          <p class="text-xs opacity-60 mt-0.5">When enabled, already-downloaded chapters won't be replaced even if a better version is found.</p>
        </div>
      </label>
      <div>
        <div class="label px-0"><span class="label-text font-medium">Download mode</span></div>
        <select id="wizard-download-mode" class="select select-bordered select-sm w-full max-w-xs">
          <option value="must_have" ${dlMode === 'must_have' ? 'selected' : ''}>Must Have — try best, fall back on failure</option>
          <option value="best_only" ${dlMode === 'best_only' ? 'selected' : ''}>Best Only — try best, skip on failure</option>
        </select>
      </div>
    </div>
  `;
}

// ---------------------------------------------------------------------------
// Step 3: Provider Configuration
// ---------------------------------------------------------------------------

let _providerSettingsLoaded = false;

async function ensureProviderSettingsLoaded() {
  if (_providerSettingsLoaded) return;
  _providerSettingsLoaded = true;
  try {
    const results = await Promise.allSettled(
      _providerList.map(p => providerSettings.getGlobal(p.name).then(s => ({ name: p.name, enabled: s.enabled ?? true })))
    );
    for (const r of results) {
      if (r.status === 'fulfilled') {
        const { name, enabled } = r.value;
        _providerChanges[name] ??= { enabled };
      }
    }
  } catch (_) {}
  if (_currentStep === 3) render();
}

function step3Html() {
  if (_providerList.length === 0) {
    return `
      <div class="flex items-center gap-2 mb-4">
        <iconify-icon icon="mdi:server-outline" width="24" class="text-primary"></iconify-icon>
        <h3 class="text-lg font-semibold m-0">Providers</h3>
      </div>
      <p class="text-sm opacity-60">No providers loaded. Add YAML files to the <code>providers/</code> directory.</p>
    `;
  }
  const rows = _providerList.map(p => {
    const state = _providerChanges[p.name];
    const enabled = state !== undefined ? state.enabled : true;
    return `<tr>
      <td class="font-medium">${escape(p.name)}</td>
      <td>${p.needs_browser ? '<iconify-icon icon="mdi:google-chrome" width="16" title="Requires browser"></iconify-icon>' : '—'}</td>
      <td><input type="checkbox" class="checkbox checkbox-sm checkbox-primary wiz-provider-enabled" data-p="${escape(p.name)}" ${enabled ? 'checked' : ''}></td>
    </tr>`;
  }).join('');
  return `
    <div class="flex items-center gap-2 mb-4">
      <iconify-icon icon="mdi:server-outline" width="24" class="text-primary"></iconify-icon>
      <h3 class="text-lg font-semibold m-0">Providers</h3>
    </div>
    <p class="text-sm opacity-70 mb-4">Enable or disable chapter sources globally. Disabled providers won't be searched for any series. Per-series overrides can be set from the series page.</p>
    <div class="overflow-x-auto">
      <table class="table table-sm">
        <thead><tr><th>Provider</th><th>Browser</th><th>Enabled</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
  `;
}

async function saveProviders() {
  await Promise.allSettled(
    Object.entries(_providerChanges).map(([name, { enabled }]) => providerSettings.setGlobal(name, enabled))
  );
}

// ---------------------------------------------------------------------------
// Step 4: Import Existing Library
// ---------------------------------------------------------------------------

function step4Html() {
  switch (_seriesSubstep) {
    case 'intro': return step4IntroHtml();
    case 'matching': return step4MatchingHtml();
    case 'chapter_import': return step4ChapterImportHtml();
    case 'done': return step4DoneHtml();
    case 'skip': return step4SkipHtml();
    default: return step4IntroHtml();
  }
}

async function loadStep4Libraries() {
  if (_step4Libraries.length > 0) return;
  try {
    _step4Libraries = await libraries.list();
    if (_step4Libraries.length === 1) _series.libraryId = _step4Libraries[0].uuid;
    if (_currentStep === 2 && _seriesSubstep === 'intro') render();
  } catch (_) {}
}

function step4IntroHtml() {
  const singleLib = _step4Libraries.length === 1;
  let libControl;
  if (_step4Libraries.length === 0) {
    libControl = `<select id="wiz-series-lib" class="select select-bordered w-full" disabled><option value="">Loading…</option></select>`;
  } else if (singleLib) {
    const l = _step4Libraries[0];
    libControl = `<div class="tooltip tooltip-right w-full" data-tip="Only one library configured"><div class="input input-bordered w-full flex items-center opacity-70 cursor-not-allowed select-none">${escape(l.root_path)}</div></div><input type="hidden" id="wiz-series-lib" value="${escape(l.uuid)}">`;
  } else {
    libControl = `<select id="wiz-series-lib" class="select select-bordered w-full">${_step4Libraries.map(l => `<option value="${escape(l.uuid)}">${escape(l.root_path)}</option>`).join('')}</select>`;
  }
  return `<div class="flex items-center gap-2 mb-4"><iconify-icon icon="mdi:import" width="24" class="text-primary"></iconify-icon><h3 class="text-lg font-semibold m-0">Import Existing Library</h3></div><p class="text-sm opacity-70 mb-4">If you have manga already organized as subfolders, Rebarr can scan them, match each folder to AniList, and bulk-add them to your library.</p><div class="flex flex-col gap-3 mb-4"><label class="form-control w-full"><div class="label"><span class="label-text font-medium">Library</span></div>${libControl}</label><label class="form-control w-full"><div class="label"><span class="label-text font-medium">Source Directory</span></div><input type="text" id="wiz-series-dir" class="input input-bordered w-full" placeholder="/old/manga/library"></label></div><div id="wiz-series-scan-error" class="text-error text-sm mb-3"></div><button id="wiz-series-scan-btn" class="btn btn-primary btn-sm" ${_step4Libraries.length === 0 ? 'disabled' : ''}><iconify-icon icon="mdi:folder-search-outline" width="16"></iconify-icon>Scan Folder</button>`;
}

async function startSeriesScan() {
  const dir = document.getElementById('wiz-series-dir')?.value.trim();
  const libId = document.getElementById('wiz-series-lib')?.value;
  const errEl = document.getElementById('wiz-series-scan-error');
  if (!dir) { if (errEl) errEl.textContent = 'Enter a source directory.'; return; }
  if (!libId) { if (errEl) errEl.textContent = 'Select a library.'; return; }

  _seriesSubstep = 'matching';
  _series.folders = [];
  _series.matches = {};
  _series.matchAlternatives = {};
  _series.matchStatus = {};
  _series.relPaths = {};
  _series.matchQueue = [];
  _series.matchRunning = false;
  _series.failed = new Set();
  _series.libraryId = libId;
  render();

  try { _series.folders = await importApi.seriesScan(dir); }
  catch (e) { _seriesSubstep = 'intro'; render(); const err2 = document.getElementById('wiz-series-scan-error'); if (err2) err2.textContent = 'Scan failed: ' + e.message; return; }

  if (_series.folders.length === 0) { _seriesSubstep = 'intro'; render(); const err2 = document.getElementById('wiz-series-scan-error'); if (err2) err2.textContent = 'No subdirectories found.'; return; }

  for (const f of _series.folders) { _series.relPaths[f.folder_name] = f.folder_name; _series.matchStatus[f.folder_name] = 'pending'; }
  render();
  startMatchQueue();
}

// ---------------------------------------------------------------------------
// Step 4: Matching page
// ---------------------------------------------------------------------------

function step4MatchingHtml() {
  const total = _series.folders.length;
  if (total === 0) return `<div class="flex items-center gap-2 mb-4"><iconify-icon icon="mdi:magnify" width="24" class="text-primary"></iconify-icon><h3 class="text-lg font-semibold m-0">Matching…</h3></div><div class="flex items-center gap-2 opacity-60"><span class="loading loading-spinner loading-sm"></span><span>Scanning directory…</span></div>`;

  const cards = _series.folders.map(f => `<div id="wiz-card-${CSS.escape(f.folder_name)}" class="border border-base-300 rounded-lg p-3 mb-2">${matchCardHtml(f.folder_name)}</div>`).join('');
  const allDone = _series.folders.every(f => (_series.matchStatus[f.folder_name] ?? 'pending') !== 'pending');
  const confirmedCount = countConfirmed();

  return `<div class="flex items-center gap-2 mb-3"><iconify-icon icon="mdi:format-list-checks" width="24" class="text-primary"></iconify-icon><h3 class="text-lg font-semibold m-0">Match Folders (${total})</h3></div><p id="wiz-match-progress" class="text-sm opacity-60 mb-3">${matchProgressText()}</p><div class="mb-4">${cards}</div><div class="flex gap-3 mt-2 items-center"><button id="wiz-match-continue" class="btn btn-primary btn-sm" ${allDone ? '' : 'disabled'} onclick="wizStartSeriesExecute()"><iconify-icon icon="mdi:arrow-right-circle-outline" width="16"></iconify-icon>Continue to Import (${confirmedCount}) →</button><button class="btn btn-ghost btn-sm" onclick="wizMatchSkipAll()">Skip all →</button></div>`;
}

function matchCardHtml(folderName) {
  const fi = _series.folders.findIndex(x => x.folder_name === folderName);
  const status = _series.matchStatus[folderName] ?? 'pending';
  const match = _series.matches[folderName];
  const alts = _series.matchAlternatives[folderName] ?? [];
  const f = _series.folders[fi];
  const cbzCount = f?.cbz_count ?? 0;

  let statusBadge = '', bodyHtml = '';

  if (status === 'pending') {
    statusBadge = `<span class="flex items-center gap-1 opacity-40 text-xs"><span class="loading loading-spinner loading-xs"></span>searching…</span>`;
  } else if (status === 'needs_action') {
    statusBadge = `<span class="badge badge-warning badge-sm">needs action</span>`;
    if (alts.length === 0) {
      bodyHtml = `<p class="text-xs opacity-50 mt-2">No AniList results found.</p><div class="flex gap-2 mt-2"><button class="btn btn-xs btn-outline btn-primary" onclick="wizChangeMatch(${fi})">Search AniList…</button><button class="btn btn-xs btn-ghost" onclick="wizSkipMatch(${fi})">Skip</button></div>`;
    } else {
      const altRows = alts.map((alt, ai) => `<div class="flex items-center gap-2 p-1 rounded hover:bg-base-200">${alt.cover_url ? `<img src="${escape(alt.cover_url)}" alt="" class="w-8 h-11 object-cover rounded shrink-0">` : `<div class="w-8 h-11 bg-base-300 rounded shrink-0"></div>`}<div class="flex-1 min-w-0 text-sm">${escape(alt.title)}${alt.year ? `<span class="opacity-50 text-xs ml-1">(${alt.year})</span>` : ''}</div><button class="btn btn-xs btn-primary shrink-0" onclick="wizConfirmMatch(${fi}, ${ai})">Confirm</button></div>`).join('');
      bodyHtml = `<div class="mt-2 flex flex-col gap-1">${altRows}</div><div class="flex gap-2 mt-2"><button class="btn btn-xs btn-ghost btn-outline" onclick="wizChangeMatch(${fi})">Search AniList…</button><button class="btn btn-xs btn-ghost" onclick="wizSkipMatch(${fi})">Skip</button></div>`;
    }
  } else if (status === 'auto_confirmed' || status === 'confirmed') {
    const badgeCls = status === 'auto_confirmed' ? 'badge-success' : 'badge-info';
    const badgeLabel = status === 'auto_confirmed' ? '✓ auto-matched' : '✓ confirmed';
    statusBadge = `<span class="badge ${badgeCls} badge-sm">${badgeLabel}</span>`;
    if (match) {
      bodyHtml = `<div class="flex items-center gap-2 mt-2">${match.cover_url ? `<img src="${escape(match.cover_url)}" alt="" class="w-8 h-11 object-cover rounded shrink-0">` : `<div class="w-8 h-11 bg-base-300 rounded shrink-0"></div>`}<div class="flex-1 min-w-0 text-sm font-medium">${escape(match.title)}${match.year ? `<span class="opacity-50 font-normal text-xs ml-1">(${match.year})</span>` : ''}</div><div class="flex gap-1 shrink-0"><button class="btn btn-xs btn-ghost" onclick="wizChangeMatch(${fi})">Change</button></div></div>`;
    }
  } else if (status === 'skipped') {
    statusBadge = `<span class="badge badge-ghost badge-sm">skipped</span>`;
    bodyHtml = `<div class="flex items-center gap-2 mt-2"><span class="text-xs opacity-50">Will not be imported.</span><button class="btn btn-xs btn-ghost ml-auto" onclick="wizUndoMatch(${fi})">Undo</button></div>`;
  }

  return `<div class="flex items-center justify-between gap-2"><div class="flex items-center gap-2 min-w-0"><iconify-icon icon="mdi:folder-outline" width="16" class="opacity-40 shrink-0"></iconify-icon><span class="font-medium text-sm truncate">${escape(folderName)}</span><span class="text-xs opacity-40 shrink-0">${cbzCount} CBZs</span></div><div class="shrink-0">${statusBadge}</div></div>${bodyHtml}`;
}

function matchProgressText() {
  const total = _series.folders.length;
  const pending = _series.folders.filter(f => (_series.matchStatus[f.folder_name] ?? 'pending') === 'pending').length;
  const needsAction = _series.folders.filter(f => _series.matchStatus[f.folder_name] === 'needs_action').length;
  const confirmed = countConfirmed();
  const skipped = _series.folders.filter(f => _series.matchStatus[f.folder_name] === 'skipped').length;
  if (pending > 0) return `Searching… ${total - pending} / ${total} · ${needsAction} need action`;
  const parts = [];
  if (confirmed > 0) parts.push(`${confirmed} confirmed`);
  if (needsAction > 0) parts.push(`${needsAction} need action`);
  if (skipped > 0) parts.push(`${skipped} skipped`);
  return parts.join(' · ') || 'All done';
}

function countConfirmed() {
  return _series.folders.filter(f => {
    const s = _series.matchStatus[f.folder_name];
    return s === 'confirmed' || s === 'auto_confirmed';
  }).length;
}

function updateMatchCard(folderName) {
  const card = document.getElementById('wiz-card-' + CSS.escape(folderName));
  if (card) card.innerHTML = matchCardHtml(folderName);
}

function updateMatchProgress() {
  const el = document.getElementById('wiz-match-progress');
  if (el) el.textContent = matchProgressText();
  const allDone = _series.folders.every(f => (_series.matchStatus[f.folder_name] ?? 'pending') !== 'pending');
  const confirmed = countConfirmed();
  const btn = document.getElementById('wiz-match-continue');
  if (btn) { btn.disabled = !allDone; btn.innerHTML = `<iconify-icon icon="mdi:arrow-right-circle-outline" width="16"></iconify-icon>Continue to Import (${confirmed}) →`; }
}

function strSimilarity(a, b) {
  const norm = s => s.toLowerCase().replace(/[^a-z0-9 ]/g, '').replace(/\s+/g, ' ').trim();
  a = norm(a); b = norm(b);
  if (a === b) return 1.0; if (!a || !b) return 0.0;
  const maxLen = Math.max(a.length, b.length);
  const prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  const curr = new Array(b.length + 1);
  for (let i = 1; i <= a.length; i++) {
    curr[0] = i;
    for (let j = 1; j <= b.length; j++) curr[j] = a[i - 1] === b[j - 1] ? prev[j - 1] : 1 + Math.min(prev[j], curr[j - 1], prev[j - 1]);
    prev.splice(0, prev.length, ...curr);
  }
  return 1 - prev[b.length] / maxLen;
}

function startMatchQueue() {
  _series.matchQueue = _series.folders.filter(f => _series.matchStatus[f.folder_name] === 'pending').map(f => f.folder_name);
  if (_series.matchRunning) return;
  _series.matchRunning = true;
  drainNextMatch();
}

async function drainNextMatch() {
  if (_series.matchQueue.length === 0) { _series.matchRunning = false; updateMatchProgress(); return; }
  const folderName = _series.matchQueue.shift();
  try {
    const results = await search.query(folderName);
    if (results && results.length > 0) {
      _series.matchAlternatives[folderName] = results.slice(0, 3).map(r => ({ anilist_id: r.anilist_id, title: r.metadata?.title ?? '', year: r.metadata?.start_year ?? null, cover_url: r.thumbnail_url ?? null }));
      const top = results[0];
      _series.matches[folderName] = { anilist_id: top.anilist_id, title: top.metadata?.title ?? '', year: top.metadata?.start_year ?? null, cover_url: top.thumbnail_url ?? null, synopsis: top.metadata?.description ?? null };
      _series.failed.delete(folderName);
      const sim = strSimilarity(folderName, top.metadata?.title ?? '');
      _series.matchStatus[folderName] = sim >= 0.85 ? 'auto_confirmed' : 'needs_action';
    } else { _series.matchAlternatives[folderName] = []; _series.matches[folderName] = null; _series.matchStatus[folderName] = 'needs_action'; _series.failed.add(folderName); }
  } catch (_) { _series.matchAlternatives[folderName] = []; _series.matches[folderName] = null; _series.matchStatus[folderName] = 'needs_action'; _series.failed.add(folderName); }
  updateMatchCard(folderName); updateMatchProgress();
  setTimeout(drainNextMatch, 1000);
}

// ---------------------------------------------------------------------------
// Series execute
// ---------------------------------------------------------------------------

async function executeSeriesImport() {
  const confirmed = _series.folders.filter(f => {
    const s = _series.matchStatus[f.folder_name];
    return s === 'confirmed' || s === 'auto_confirmed';
  });

  if (confirmed.length === 0) { _seriesSubstep = 'done'; render(); return; }

  const imports = confirmed.map(f => ({
    folder_path: f.folder_path,
    anilist_id: _series.matches[f.folder_name].anilist_id,
    library_id: _series.libraryId,
    relative_path: _series.matches[f.folder_name]?.title ?? _series.relPaths[f.folder_name] ?? f.folder_name,
  }));

  const body = document.querySelector('.setup-card .card-body');
  if (body) body.innerHTML = `<div class="flex items-center gap-2 opacity-60"><span class="loading loading-spinner loading-sm"></span><span>Adding ${imports.length} series to library…</span></div>`;

  try { _series.result = await importApi.seriesExecute({ imports, queue_chapter_scan: false }); }
  catch (e) { _series.result = { added: 0, skipped_duplicates: 0, errors: [e.message], manga_ids: [] }; }

  const mangaIds = _series.result?.manga_ids ?? [];
  _series.importQueue = mangaIds.map((id, i) => (id && confirmed[i]) ? { folderPath: confirmed[i].folder_path, mangaId: id, folderName: confirmed[i].folder_name, match: _series.matches[confirmed[i].folder_name] } : null).filter(Boolean);

  _series.currentImportIdx = 0;
  _series.seriesCandidates = null;
  _series.importSummary = { moved: 0, errors: [] };

  if (_series.importQueue.length > 0) { _seriesSubstep = 'chapter_import'; render(); loadCurrentSeriesCandidates(); }
  else { _seriesSubstep = 'done'; render(); }
}

// ---------------------------------------------------------------------------
// Step 4: Chapter import
// ---------------------------------------------------------------------------

function wizTierBadge(tier) {
  const map = { rebarr: { label: 'Rebarr', cls: 'badge-success' }, comicinfo: { label: 'ComicInfo', cls: 'badge-info' }, filename: { label: 'Filename', cls: 'badge-warning' } };
  const t = map[tier] ?? { label: tier, cls: 'badge-ghost' };
  return `<span class="badge ${t.cls} badge-xs">${t.label}</span>`;
}

function step4ChapterImportHtml() {
  const queue = _series.importQueue, idx = _series.currentImportIdx, current = queue[idx];
  if (!current) return step4DoneHtml();

  const match = current.match;
  const coverHtml = match?.cover_url ? `<img src="${escape(match.cover_url)}" alt="" class="w-12 h-16 object-cover rounded shrink-0">` : `<div class="w-12 h-16 bg-base-300 rounded shrink-0"></div>`;

  let tableHtml;
  if (!_series.seriesCandidates) {
    tableHtml = `<div class="flex items-center gap-2 opacity-60 py-4"><span class="loading loading-spinner loading-sm"></span><span>Scanning ${escape(current.folderName)}…</span></div>`;
  } else if (_series.seriesCandidates.length === 0) {
    tableHtml = `<p class="text-sm opacity-60 py-4">No CBZ files found in this folder.</p>`;
  } else {
    const rows = _series.seriesCandidates.map((c, i) => {
      const chNum = c.chapter_number != null ? c.chapter_number : '';
      const autoChecked = c.chapter_number != null;
      return `<tr data-widx="${i}"><td><input type="checkbox" class="checkbox checkbox-xs wiz-ch-check" data-widx="${i}" ${autoChecked ? 'checked' : ''}></td><td class="text-xs break-all max-w-[180px]" title="${escape(c.cbz_path)}">${escape(c.file_name)}</td><td>${wizTierBadge(c.import_tier)}</td><td><input type="number" class="input input-bordered input-xs w-16" data-widx="${i}" value="${chNum}" step="0.1" min="0" placeholder="Ch#" onchange="wizChUpdateNum(${i}, this.value)"></td><td><input type="text" class="input input-bordered input-xs w-28" value="${escape(c.chapter_title ?? '')}" placeholder="Title…" onchange="wizChUpdateField(${i}, 'chapter_title', this.value)"></td><td><input type="text" class="input input-bordered input-xs w-24" value="${escape(c.scanlator_group ?? '')}" placeholder="Group…" onchange="wizChUpdateField(${i}, 'scanlator_group', this.value)"></td></tr>`;
    }).join('');
    tableHtml = `<div class="flex gap-2 mb-2 flex-wrap items-center"><button class="btn btn-xs btn-ghost" onclick="wizChSelectAll(true)">Select all</button><button class="btn btn-xs btn-ghost" onclick="wizChSelectAll(false)">Deselect all</button><span class="text-xs opacity-50">|</span><input type="text" class="input input-bordered input-xs w-32" id="wiz-bulk-group" placeholder="Bulk group…"><button class="btn btn-xs btn-outline" onclick="wizChBulkUpdate('scanlator_group')">Apply to selected</button><input type="text" class="input input-bordered input-xs w-32" id="wiz-bulk-title" placeholder="Bulk title…"><button class="btn btn-xs btn-outline" onclick="wizChBulkUpdate('chapter_title')">Apply to selected</button></div><div class="overflow-x-auto"><table class="table table-xs table-zebra w-full"><thead><tr><th></th><th>File</th><th>Tier</th><th>Ch #</th><th>Title</th><th>Scanlator Group</th></tr></thead><tbody>${rows}</tbody></table></div>`;
  }

  return `<div class="flex items-center gap-2 mb-3"><iconify-icon icon="mdi:book-arrow-down-outline" width="24" class="text-primary"></iconify-icon><h3 class="text-lg font-semibold m-0">Import Chapters</h3><span class="text-sm opacity-50 ml-auto">${idx + 1} of ${queue.length}</span></div><div class="flex gap-3 items-center mb-4 p-3 bg-base-200 rounded-lg">${coverHtml}<div class="flex-1 min-w-0"><div class="font-medium">${escape(match?.title ?? current.folderName)}</div>${match?.year ? `<div class="text-xs opacity-50">${match.year}</div>` : ''}<div class="text-xs opacity-50 mt-1">${escape(current.folderPath)}</div></div></div><p class="text-sm opacity-70 mb-3">Select chapters to import. <strong>Copy</strong> leaves originals in place; <strong>Move</strong> deletes them after import.</p>${tableHtml}<div id="wiz-ch-import-error" class="text-error text-sm mt-2"></div><div class="flex gap-2 mt-4 flex-wrap"><button class="btn btn-primary btn-sm" onclick="wizExecuteChapterImport(false)" ${!_series.seriesCandidates ? 'disabled' : ''}><iconify-icon icon="mdi:file-move-outline" width="16"></iconify-icon>Move</button><button class="btn btn-outline btn-sm" onclick="wizExecuteChapterImport(true)" ${!_series.seriesCandidates ? 'disabled' : ''}><iconify-icon icon="mdi:content-copy" width="16"></iconify-icon>Copy</button><button class="btn btn-ghost btn-sm ml-auto" onclick="wizSkipChapterImport()">Skip series →</button></div>`;
}

async function loadCurrentSeriesCandidates() {
  const current = _series.importQueue[_series.currentImportIdx];
  if (!current) return;
  try {
    const candidates = await importApi.scan(current.folderPath);
    _series.seriesCandidates = candidates.map(c => {
      // Clear title if it's just a chapter number (e.g. "Chapter 33", "Ch. 33", "Ch 33")
      let title = c.chapter_title ?? '';
      if (title && /^ch(apter)?\.?\s*\d/i.test(title.trim())) title = null;
      return { ...c, chapter_title: title, provider_name: c.provider_name ?? 'Local', is_extra: c.is_extra ?? false, suggested_manga: { manga_id: current.mangaId, title: current.match?.title ?? '', confidence: 1.0 } };
    });
  } catch (e) { _series.seriesCandidates = []; const errEl = document.getElementById('wiz-ch-import-error'); if (errEl) errEl.textContent = `Scan failed: ${e.message}`; }
  if (_seriesSubstep === 'chapter_import') render();
}

function advanceChapterImport() {
  _series.currentImportIdx++;
  if (_series.currentImportIdx >= _series.importQueue.length) { _seriesSubstep = 'done'; render(); }
  else { _series.seriesCandidates = null; render(); loadCurrentSeriesCandidates(); }
}

window.wizExecuteChapterImport = async function (copy) {
  const candidates = _series.seriesCandidates ?? [];
  const checked = [...document.querySelectorAll('.wiz-ch-check:checked')].map(cb => parseInt(cb.dataset.widx, 10));
  if (checked.length === 0) { advanceChapterImport(); return; }
  const current = _series.importQueue[_series.currentImportIdx];
  const imports = checked.filter(i => candidates[i]?.chapter_number != null).map(i => { const c = candidates[i]; return { cbz_path: c.cbz_path, manga_id: current.mangaId, chapter_number: c.chapter_number, chapter_title: c.chapter_title ?? null, scanlator_group: c.scanlator_group ?? null, language: c.language ?? null, provider_name: c.provider_name ?? 'Local', is_extra: c.is_extra ?? false, chapter_uuid: c.chapter_uuid ?? null, released_at: c.released_at ?? null, downloaded_at: c.downloaded_at ?? null, scraped_at: c.scraped_at ?? null, copy }; });
  if (imports.length === 0) { advanceChapterImport(); return; }
  document.querySelectorAll('.setup-card button').forEach(b => { b.disabled = true; });
  try { const result = await importApi.execute(imports); _series.importSummary.moved += result.moved ?? 0; _series.importSummary.errors.push(...(result.errors ?? [])); }
  catch (e) { _series.importSummary.errors.push(`${current.folderName}: ${e.message}`); }
  advanceChapterImport();
};

window.wizSkipChapterImport = function () { advanceChapterImport(); };
window.wizChSelectAll = function (checked) { document.querySelectorAll('.wiz-ch-check').forEach(cb => { cb.checked = checked; }); };
window.wizChUpdateNum = function (idx, val) { if (!_series.seriesCandidates) return; const parsed = parseFloat(val); _series.seriesCandidates[idx].chapter_number = isNaN(parsed) ? null : parsed; };
window.wizChUpdateField = function (idx, field, value) { if (!_series.seriesCandidates) return; _series.seriesCandidates[idx][field] = value === '' ? null : value; };
window.wizChBulkUpdate = function (field) {
  const inputId = field === 'scanlator_group' ? 'wiz-bulk-group' : 'wiz-bulk-title';
  const bulkInput = document.getElementById(inputId);
  if (!bulkInput) return;
  const value = bulkInput.value;
  const placeholder = field === 'scanlator_group' ? 'Group…' : 'Title…';
  const checked = [...document.querySelectorAll('.wiz-ch-check:checked')];
  checked.forEach(cb => { const idx = parseInt(cb.dataset.widx, 10); if (_series.seriesCandidates && _series.seriesCandidates[idx]) _series.seriesCandidates[idx][field] = value === '' ? null : value; const row = cb.closest('tr'); if (!row) return; const input = row.querySelector(`input[placeholder="${placeholder}"]`); if (input) input.value = value; });
  bulkInput.value = '';
};

function step4DoneHtml() {
  const r = _series.result ?? { added: 0, skipped_duplicates: 0, errors: [], manga_ids: [] };
  const ch = _series.importSummary ?? { moved: 0, errors: [] };
  const allErrors = [...r.errors, ...ch.errors];
  const errHtml = allErrors.length > 0 ? `<ul class="text-error text-sm mt-2 list-disc pl-5">${allErrors.map(e => `<li>${escape(e)}</li>`).join('')}</ul>` : '';
  return `<div class="flex items-center gap-2 mb-4"><iconify-icon icon="mdi:check-circle-outline" width="24" class="text-success"></iconify-icon><h3 class="text-lg font-semibold m-0">Import Complete</h3></div><div class="mb-4"><p class="text-sm">Added <strong>${r.added}</strong> series.${ch.moved > 0 ? ` Imported <strong>${ch.moved}</strong> chapter(s).` : ''}</p>${r.added > 0 ? `<p class="text-xs opacity-60 mt-1">ScanDisk queued for all added series.</p>` : ''}${errHtml}</div><button id="wiz-series-done-next" class="btn btn-primary btn-sm">Continue →</button>`;
}

function step4SkipHtml() {
  return `<div class="flex items-center gap-2 mb-4"><iconify-icon icon="mdi:import" width="24" class="text-primary"></iconify-icon><h3 class="text-lg font-semibold m-0">Import Existing Library</h3></div><p class="text-sm opacity-70">Skipped. You can always add series from the Search page later.</p>`;
}

// ---------------------------------------------------------------------------
// Step 5: Quality Rules & Concepts
// ---------------------------------------------------------------------------

async function loadQualityRules() {
  try {
    _qualityRules = await qualityRulesApi.list();
  } catch (_) {
    _qualityRules = [];
  }
  if (_currentStep === 5) render();
}

function step5Html() {
  const rulesSection = (() => {
    if (!_qualityRules) {
      return `<div class="flex items-center gap-2 opacity-60 py-3"><span class="loading loading-spinner loading-sm"></span><span>Loading quality rules…</span></div>`;
    }
    if (_qualityRules.length === 0) {
      return `<p class="text-sm opacity-60">No quality rules configured yet. You can add them from Settings.</p>`;
    }
    const rows = _qualityRules.map(r => {
      const condSummary = r.conditions.length === 0
        ? '<span class="opacity-40">—</span>'
        : r.conditions.map(c => {
            const neg = c.negate ? 'NOT ' : '';
            const val = c.value ? ` "${escape(c.value)}"` : '';
            return `<code class="text-xs">${neg}${escape(c.field)} ${escape(c.op)}${val}</code>`;
          }).join(', ');
      const scoreColour = r.score > 0 ? 'text-success' : r.score < 0 ? 'text-error' : '';
      return `<tr>
        <td class="font-medium text-sm">${escape(r.name)}</td>
        <td class="font-mono ${scoreColour}">${r.score > 0 ? '+' : ''}${r.score}</td>
        <td class="text-xs">${condSummary}</td>
      </tr>`;
    }).join('');
    return `
      <div class="overflow-x-auto">
        <table class="table table-sm">
          <thead><tr><th>Rule</th><th>Score</th><th>Conditions</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    `;
  })();

  return `
    <div class="flex items-center gap-2 mb-4">
      <iconify-icon icon="mdi:star-cog-outline" width="24" class="text-primary"></iconify-icon>
      <h3 class="text-lg font-semibold m-0">Quality & Concepts</h3>
    </div>

    <div class="mb-5">
      <h4 class="font-semibold text-sm mb-1">Quality Rules</h4>
      <p class="text-sm opacity-70 mb-3">When multiple versions of a chapter are available, Rebarr scores each one using these rules. The highest-scoring version becomes the <strong>canonical chapter</strong> — the one that gets downloaded. Rules can be customised from Settings → Quality Rules.</p>
      ${rulesSection}
    </div>

    <div class="divider my-3"></div>

    <div class="flex flex-col gap-4">
      <div class="flex gap-3 items-start">
        <iconify-icon icon="mdi:google-chrome" width="20" class="text-primary shrink-0 mt-0.5"></iconify-icon>
        <div>
          <div class="font-medium text-sm">Browser Workers</div>
          <div class="text-xs opacity-70">Rebarr drives headless browser instances to scrape chapter pages. The worker count controls how many run concurrently — more workers means faster downloads, but higher RAM usage.</div>
        </div>
      </div>
      <div class="flex gap-3 items-start">
        <iconify-icon icon="mdi:clock-outline" width="20" class="text-primary shrink-0 mt-0.5"></iconify-icon>
        <div>
          <div class="font-medium text-sm">Task Queue</div>
          <div class="text-xs opacity-70">All scans, metadata refreshes, and downloads run as background tasks. You can see what's running, pause the queue, or cancel individual tasks from the <strong>Queue</strong> page.</div>
        </div>
      </div>
      <div class="flex gap-3 items-start">
        <iconify-icon icon="mdi:crown-outline" width="20" class="text-primary shrink-0 mt-0.5"></iconify-icon>
        <div>
          <div class="font-medium text-sm">Canonical Chapters</div>
          <div class="text-xs opacity-70">For each chapter number, Rebarr picks one "canonical" version to download based on quality scores. If a better version appears later (e.g. an official release replaces a fan translation), Rebarr can upgrade it automatically — unless you've turned off chapter upgrades.</div>
        </div>
      </div>
    </div>
  `;
}

// ---------------------------------------------------------------------------
// Step 6: You're All Set
// ---------------------------------------------------------------------------

function step6Html() {
  const items = [
    { icon: 'mdi:magnify', title: 'Search & Add Manga', desc: 'Use the <strong>Search</strong> page to find titles on AniList and add them to your library.' },
    { icon: 'mdi:book-multiple-outline', title: 'Series Page', desc: 'Click any series to see its chapters. Use <strong>Check New Chapters</strong> to find updates and <strong>Download All Missing</strong> to fetch them.' },
    { icon: 'mdi:clock-outline', title: 'Task Queue', desc: 'All downloads and scans run in the background. Monitor progress from the <strong>Queue</strong> page.' },
    { icon: 'mdi:cog-outline', title: 'Settings', desc: 'Adjust scan intervals, configure providers, and manage quality rules.' },
  ];
  return `<div class="flex items-center gap-2 mb-4"><iconify-icon icon="mdi:check-decagram-outline" width="24" class="text-success"></iconify-icon><h3 class="text-lg font-semibold m-0">You're all set!</h3></div><p class="text-sm opacity-70 mb-4">Here's a quick overview of Rebarr's main features.</p><div class="flex flex-col gap-4 mb-4">${items.map(item => `<div class="flex gap-3 items-start"><iconify-icon icon="${item.icon}" width="22" class="text-primary shrink-0 mt-0.5"></iconify-icon><div><div class="font-medium text-sm">${item.title}</div><div class="text-xs opacity-70">${item.desc}</div></div></div>`).join('')}</div><div id="wizard-finish-error" class="text-error text-sm mt-2"></div>`;
}

// ---------------------------------------------------------------------------
// Exports and handlers
// ---------------------------------------------------------------------------

export function showWizard(onComplete) { viewSetup(onComplete); }
window.showWizard = showWizard;
window.viewSetup = viewSetup;

window.wizConfirmMatch = function (fi, ai) {
  const folder = _series.folders[fi]; if (!folder) return;
  const folderName = folder.folder_name;
  const alt = (_series.matchAlternatives[folderName] ?? [])[ai];
  if (alt) _series.matches[folderName] = { anilist_id: alt.anilist_id, title: alt.title, year: alt.year, cover_url: alt.cover_url, synopsis: null };
  _series.matchStatus[folderName] = 'confirmed';
  updateMatchCard(folderName); updateMatchProgress();
};

window.wizSkipMatch = function (fi) { const folderName = _series.folders[fi]?.folder_name; if (!folderName) return; _series.matchStatus[folderName] = 'skipped'; updateMatchCard(folderName); updateMatchProgress(); };
window.wizUndoMatch = function (fi) { const folderName = _series.folders[fi]?.folder_name; if (!folderName) return; _series.matchStatus[folderName] = 'needs_action'; updateMatchCard(folderName); updateMatchProgress(); };
window.wizChangeMatch = function (fi) { const folderName = _series.folders[fi]?.folder_name; if (!folderName) return; wizOpenPicker(fi, folderName); };
window.wizStartSeriesExecute = function () { executeSeriesImport(); };
window.wizMatchSkipAll = function () { _seriesSubstep = 'skip'; render(); };

function wizOpenPicker(fi, folderName) {
  document.getElementById('wiz-picker-modal')?.remove();
  _pickerFolderIdx = fi; _pickerResults = [];
  const modal = document.createElement('div'); modal.id = 'wiz-picker-modal'; modal.className = 'modal-overlay';
  modal.innerHTML = `<div class="modal-box w-full max-w-lg"><div class="text-sm opacity-60 mb-2">Matching: <strong>${escape(folderName)}</strong></div><input id="wiz-picker-input" type="text" class="input input-bordered w-full" placeholder="Search AniList…" value="${escape(folderName)}" autocomplete="off"><div id="wiz-picker-results" class="mt-3 max-h-96 overflow-y-auto flex flex-col gap-1"><p class="opacity-50 text-sm">Searching…</p></div><div class="modal-action"><button class="btn btn-ghost btn-sm" id="wiz-picker-cancel">Cancel</button></div></div>`;
  document.body.appendChild(modal);
  let debounceTimer = null;
  function renderResults(results) {
    const container = document.getElementById('wiz-picker-results'); if (!container) return;
    if (!results || results.length === 0) { container.innerHTML = '<p class="opacity-50 text-sm">No results found.</p>'; return; }
    _pickerResults = results.slice(0, 10);
    container.innerHTML = _pickerResults.map((r, pi) => { const title = escape(r.metadata?.title ?? ''); const yr = r.metadata?.start_year ? `<span class="opacity-55 text-xs ml-1">(${r.metadata.start_year})</span>` : ''; const cover = r.thumbnail_url ? `<img src="${escape(r.thumbnail_url)}" alt="" class="w-9 h-[52px] object-cover rounded shrink-0">` : `<div class="w-9 h-[52px] bg-base-300 rounded shrink-0"></div>`; return `<div class="flex gap-2 items-center p-2 rounded cursor-pointer hover:bg-base-200 border border-transparent" onclick="wizSeriesPick(${pi})">${cover}<div class="flex-1 min-w-0"><div class="font-medium text-sm">${title}${yr}</div></div></div>`; }).join('');
  }
  async function doSearch(q) { const container = document.getElementById('wiz-picker-results'); if (!container) return; if (!q.trim()) { container.innerHTML = ''; return; } container.innerHTML = '<p class="opacity-50 text-sm">Searching…</p>'; try { await new Promise(resolve => setTimeout(resolve, 200)); const results = await search.query(q); if (document.getElementById('wiz-picker-modal')) renderResults(results); } catch (e) { if (container) container.innerHTML = `<p class="text-error text-sm">Search failed: ${escape(e.message)}</p>`; } }
  const input = document.getElementById('wiz-picker-input');
  input?.addEventListener('input', () => { clearTimeout(debounceTimer); debounceTimer = setTimeout(() => doSearch(input.value), 300); });
  document.getElementById('wiz-picker-cancel')?.addEventListener('click', () => modal.remove());
  modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });
  doSearch(folderName); setTimeout(() => input?.select(), 0);
}

window.wizSeriesPick = function (pi) {
  document.getElementById('wiz-picker-modal')?.remove();
  const folder = _series.folders[_pickerFolderIdx]; const r = _pickerResults[pi]; if (!folder || !r) return;
  const folderName = folder.folder_name;
  _series.matches[folderName] = { anilist_id: r.anilist_id, title: r.metadata?.title ?? '', year: r.metadata?.start_year ?? null, cover_url: r.thumbnail_url ?? null, synopsis: null };
  _series.matchStatus[folderName] = 'confirmed';
  updateMatchCard(folderName); updateMatchProgress();
};