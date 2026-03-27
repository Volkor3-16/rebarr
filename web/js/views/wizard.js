// First-run setup wizard

import { libraries, providers, settings, providerScores } from '../api.js';
import { escape } from '../utils.js';

const TOTAL_STEPS = 5;

export function showWizard(onComplete) {
  let currentStep = 1;
  let pendingSettings = {};
  let redirectToImport = false;
  let libraryCreated = false;
  let providerList = [];
  let providerChanges = {};

  const overlay = document.createElement('div');
  overlay.id = 'wizard-overlay';
  Object.assign(overlay.style, {
    position: 'fixed',
    inset: '0',
    zIndex: '9999',
    background: 'var(--b1, #1d232a)',
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
      <div style="max-width:660px;width:100%;margin:auto">
        ${stepIndicatorHtml()}
        <div id="wizard-body">${stepBodyHtml(currentStep)}</div>
        ${navHtml()}
      </div>
    `;
    wireHandlers();
    if (currentStep === 2) loadProviders();
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
    return `
      <div style="display:flex;justify-content:space-between;margin-top:1.25rem">
        <button class="btn btn-ghost btn-sm" id="wizard-back-btn" ${isFirst ? 'disabled' : ''}>← Back</button>
        ${isLast
          ? `<button class="btn btn-primary btn-sm" id="wizard-finish-btn">Finish Setup</button>`
          : `<button class="btn btn-primary btn-sm" id="wizard-next-btn">Next →</button>`}
      </div>
    `;
  }

  function wireHandlers() {
    document.getElementById('wizard-back-btn')?.addEventListener('click', () => {
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

    document.querySelectorAll('input[name="wizard-import"]').forEach(radio => {
      radio.addEventListener('change', e => {
        redirectToImport = e.target.value === 'yes';
        highlightRadioGroup('input[name="wizard-import"]', e.target.value);
      });
    });
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
    if (currentStep < TOTAL_STEPS) { currentStep++; rerenderOverlay(); }
  }

  async function finishWizard() {
    const btn = document.getElementById('wizard-finish-btn');
    if (btn) { btn.disabled = true; btn.textContent = 'Saving…'; }
    pendingSettings.wizard_completed = true;
    try {
      await settings.update(pendingSettings);
      overlay.remove();
      onComplete(redirectToImport);
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
    if (!path) {
      status.textContent = 'Enter a path.';
      status.style.color = 'var(--er)';
      return;
    }
    const btn = document.getElementById('wizard-create-lib-btn');
    if (btn) btn.disabled = true;
    status.textContent = 'Creating…';
    status.style.color = '';
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
    const mkOpt = (value, label, desc) => {
      const sel = (value === 'yes') === redirectToImport;
      return `
        <label style="display:flex;gap:0.75rem;align-items:flex-start;cursor:pointer;
          padding:0.6rem 0.75rem;border-radius:0.5rem;
          border:1px solid ${sel ? 'var(--p)' : 'var(--b3, #374151)'};
          background:${sel ? 'var(--b2)' : 'transparent'}">
          <input type="radio" name="wizard-import" class="radio radio-sm radio-primary"
            value="${value}" ${sel ? 'checked' : ''} style="margin-top:0.15rem">
          <div>
            <div style="font-weight:500">${label}</div>
            <div style="font-size:0.8rem;opacity:0.7">${desc}</div>
          </div>
        </label>
      `;
    };
    return `
      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:import" width="20"></iconify-icon>
          <h3>Import Existing Library</h3>
        </div>
        <p class="settings-card-desc">
          Do you have existing CBZ files you'd like to import into Rebarr?
        </p>
        <div style="display:flex;flex-direction:column;gap:0.5rem;margin:0.75rem 0">
          ${mkOpt('no',  'Start fresh',
            'Add manga via the Search page.')}
          ${mkOpt('yes', 'Import existing CBZ files',
            "After finishing setup you'll be taken to the Import page to scan a directory and match files to AniList entries.")}
        </div>
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
