// Settings view

import { providers, settings, trustedGroups, providerScores } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton, showToast } from '../utils.js';
import { showWizard } from './wizard.js';

export async function viewSettings() {
  render(`<div class="settings">${skeleton(5)}</div>`);

  try {
    const [providerList, appSettings] = await Promise.all([
      providers.list(),
      settings.get(),
    ]);

    // Parse filter languages for display
    const filterLangs = (appSettings.synonym_filter_languages || '')
      .split(',')
      .map(s => s.trim().toLowerCase())
      .filter(s => s);
    const filterLangsHtml = filterLangs.map(lang =>
      `<span class="badge badge-neutral">${escape(lang)} <button class="synonym-remove btn btn-xs btn-ghost" style="padding:0;margin-left:4px;min-height:auto;line-height:1" onclick="removeFilterLanguage('${escape(lang)}')" title="Remove">×</button></span>`
    ).join('');

    // Provider rows with global score inputs and enabled toggle
    const pRows = providerList.length === 0
      ? '<tr><td colspan="4">No providers loaded. Add YAML files to the providers/ directory.</td></tr>'
      : providerList.map(p => `
          <tr data-provider-row="${escape(p.name)}">
            <td>${escape(p.name)}</td>
            <td>${p.needs_browser ? '<iconify-icon icon="mdi:google-chrome" width="16" height="16" title="Requires browser"></iconify-icon>' : '—'}</td>
            <td>
              <input type="number" class="score-input" data-provider="${escape(p.name)}"
                value="0" min="-100" max="100"
                title="Global score override for ${escape(p.name)} (used as tiebreaker within same tier)"
                onchange="saveGlobalScore('${escape(p.name)}', this.value)"
                onblur="saveGlobalScore('${escape(p.name)}', this.value)">
            </td>
            <td>
              <input type="checkbox" class="enabled-toggle" data-provider="${escape(p.name)}"
                checked title="Globally enable/disable ${escape(p.name)}"
                onchange="saveGlobalEnabled('${escape(p.name)}', this.checked)">
            </td>
          </tr>
        `).join('');

    render(`
      <h2>Settings</h2>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:magic-staff" width="20" height="20"></iconify-icon>
          <h3>Setup Wizard</h3>
        </div>
        <p class="settings-card-desc">Re-run the guided setup to configure your library, providers, and download preferences.</p>
        <button class="btn btn-sm btn-outline" onclick="runSetupWizard()">Run Setup Wizard</button>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:clock-outline" width="20" height="20"></iconify-icon>
          <h3>Scheduler</h3>
        </div>
        <p class="settings-card-desc">Rebarr periodically checks for new chapters on all monitored series.</p>
        <form id="settings-form" class="flex gap-2 align-center flex-wrap">
          <label class="flex gap-1 align-center">
            <span>Scan interval (hours):</span>
            <input type="number" id="scan-interval" class="input input-bordered input-sm" min="1" max="168" value="${escape(appSettings.scan_interval_hours)}" style="width:80px">
          </label>
          <label class="flex gap-1 align-center">
            <span>Preferred language (BCP 47):</span>
            <input type="text" id="preferred-language" class="input input-bordered input-sm" placeholder="e.g. en" value="${escape(appSettings.preferred_language || '')}" style="width:80px"
              title="Chapters in this language are preferred. Leave blank to accept any language.">
          </label>
          <button type="submit" class="btn btn-primary btn-sm">Save</button>
        </form>
        <div id="settings-status"></div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:translate" width="20" height="20"></iconify-icon>
          <h3>Synonym Language Filter</h3>
        </div>
        <p class="settings-card-desc">Synonyms in these languages are excluded from provider searches. <a href="https://github.com/greyblake/whatlang-rs/blob/master/SUPPORTED_LANGUAGES.md" target="_blank">Use whatlang codes.</a></p>
        <div id="filter-languages-list">${filterLangsHtml || '<p><small>No languages configured.</small></p>'}</div>
        <div class="mt-2 flex gap-1">
          <input type="text" id="new-filter-language" class="input input-bordered input-sm" placeholder="Language code (e.g. cmn)" style="width:140px">
          <button class="btn btn-sm" onclick="addFilterLanguage()">Add</button>
        </div>
        <div id="filter-languages-status"></div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:star-outline" width="20" height="20"></iconify-icon>
          <h3>Provider Scores</h3>
        </div>
        <p class="settings-card-desc">Global score overrides — used as a tiebreaker within the same tier. Higher scores are preferred. Score never promotes a lower tier over a higher one. Per-series overrides are set on each series page.</p>
        <table>
          <thead><tr><th>Provider</th><th>Browser</th><th>Global Score</th><th>Enabled</th></tr></thead>
          <tbody id="provider-scores-body">${pRows}</tbody>
        </table>
        <div id="provider-scores-status"></div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:account-group-outline" width="20" height="20"></iconify-icon>
          <h3>Trusted Scanlation Groups</h3>
        </div>
        <p class="settings-card-desc">Groups listed here are Tier 2 (Trusted). They rank above unknown groups (Tier 3) but below official releases (Tier 1). Re-scan a series after changing this list to update scores.</p>
        <input type="search" id="trusted-groups-filter" class="input input-bordered input-sm" placeholder="Filter groups…" style="width:220px;margin-bottom:0.5rem" oninput="filterTrustedGroups(this.value)">
        <div id="trusted-groups-list"><p>Loading...</p></div>
        <div class="mt-2 flex gap-1">
          <input type="text" id="new-trusted-group" class="input input-bordered input-sm" placeholder="Group name (exact)" style="width:220px">
          <button class="btn btn-sm" onclick="addTrustedGroup()">Add</button>
        </div>
        <div id="trusted-groups-status"></div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:folder-multiple-outline" width="20" height="20"></iconify-icon>
          <h3>Libraries</h3>
        </div>
        <p class="settings-card-desc">Manage libraries (add, edit paths, delete) on the <a onclick="navigate('/library')">Libraries page</a>.</p>
      </div>
    `);

    // Load existing global scores into inputs
    loadGlobalScores(providerList);

    // Settings form handler
    document.getElementById('settings-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const hours = parseInt(document.getElementById('scan-interval').value, 10);
      const lang = document.getElementById('preferred-language').value.trim();
      const statusEl = document.getElementById('settings-status');

      if (!hours || hours < 1 || hours > 168) {
        statusEl.innerHTML = '<p class="error">Interval must be 1–168 hours.</p>';
        return;
      }

      try {
        await settings.update({ scan_interval_hours: hours, preferred_language: lang || null });
        showToast('Settings saved');
        statusEl.innerHTML = '';
      } catch(err) {
        statusEl.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
      }
    });

    loadTrustedGroups();
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function loadGlobalScores(providerList) {
  for (const p of providerList) {
    try {
      const res = await providerScores.getGlobal(p.name);
      const scoreInput = document.querySelector(`.score-input[data-provider="${CSS.escape(p.name)}"]`);
      if (scoreInput && res.score != null) {
        scoreInput.value = res.score;
      }
      const toggle = document.querySelector(`.enabled-toggle[data-provider="${CSS.escape(p.name)}"]`);
      if (toggle) {
        toggle.checked = res.enabled;
        _applyProviderRowStyle(p.name, res.enabled);
      }
    } catch (_) {}
  }
}

function _applyProviderRowStyle(providerName, enabled) {
  const row = document.querySelector(`tr[data-provider-row="${CSS.escape(providerName)}"]`);
  if (row) row.style.opacity = enabled ? '' : '0.5';
}

window.saveGlobalScore = async function(providerName, value) {
  const score = parseInt(value, 10);
  if (isNaN(score)) return;
  const toggle = document.querySelector(`.enabled-toggle[data-provider="${CSS.escape(providerName)}"]`);
  const enabled = toggle ? toggle.checked : true;
  const statusEl = document.getElementById('provider-scores-status');
  try {
    await providerScores.setGlobal(providerName, score, enabled);
    if (statusEl) {
      statusEl.innerHTML = `<small style="color:var(--success)">Score saved for ${escape(providerName)}</small>`;
      setTimeout(() => { if (statusEl) statusEl.innerHTML = ''; }, 2000);
    }
  } catch(e) {
    if (statusEl) statusEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.saveGlobalEnabled = async function(providerName, enabled) {
  const scoreInput = document.querySelector(`.score-input[data-provider="${CSS.escape(providerName)}"]`);
  const score = scoreInput ? (parseInt(scoreInput.value, 10) || 0) : 0;
  const statusEl = document.getElementById('provider-scores-status');
  try {
    await providerScores.setGlobal(providerName, score, enabled);
    _applyProviderRowStyle(providerName, enabled);
    if (statusEl) {
      const state = enabled ? 'enabled' : 'disabled';
      statusEl.innerHTML = `<small style="color:var(--success)">${escape(providerName)} ${state}</small>`;
      setTimeout(() => { if (statusEl) statusEl.innerHTML = ''; }, 2000);
    }
  } catch(e) {
    if (statusEl) statusEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.addFilterLanguage = async function() {
  const input = document.getElementById('new-filter-language');
  const status = document.getElementById('filter-languages-status');
  const code = input ? input.value.trim().toLowerCase() : '';
  if (!code) { status.innerHTML = '<p class="error">Enter a language code.</p>'; return; }
  if (code.length > 5) { status.innerHTML = '<p class="error">Invalid language code.</p>'; return; }
  
  try {
    // Get current settings
    const appSettings = await settings.get();
    const currentLangs = (appSettings.synonym_filter_languages || '')
      .split(',')
      .map(s => s.trim().toLowerCase())
      .filter(s => s);
    
    if (currentLangs.includes(code)) {
      status.innerHTML = '<p class="error">Language already in list.</p>';
      return;
    }
    
    currentLangs.push(code);
    await settings.update({ synonym_filter_languages: currentLangs.join(',') });
    input.value = '';
    showToast('Language added');
    viewSettings(); // Reload to show updated list
  } catch(e) {
    status.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.removeFilterLanguage = async function(code) {
  try {
    const appSettings = await settings.get();
    const currentLangs = (appSettings.synonym_filter_languages || '')
      .split(',')
      .map(s => s.trim().toLowerCase())
      .filter(s => s);
    
    const newLangs = currentLangs.filter(l => l !== code);
    await settings.update({ synonym_filter_languages: newLangs.join(',') });
    showToast('Language removed');
    viewSettings(); // Reload to show updated list
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

let _trustedGroupsCache = [];

function renderTrustedGroupPills(filter = '') {
  const el = document.getElementById('trusted-groups-list');
  if (!el) return;
  const q = filter.trim().toLowerCase();
  const visible = q ? _trustedGroupsCache.filter(g => g.toLowerCase().includes(q)) : _trustedGroupsCache;
  if (visible.length === 0) {
    el.innerHTML = q ? '<p><small>No groups match.</small></p>' : '<p><small>No trusted groups yet.</small></p>';
    return;
  }
  el.innerHTML = `<div style="display:flex;flex-wrap:wrap;gap:0.4rem;margin:0.3rem 0">${
    visible.map(g =>
      `<span class="badge badge-neutral" style="cursor:default;gap:0.35rem">${escape(g)}<button class="btn btn-xs btn-ghost" style="padding:0;min-height:auto;line-height:1;color:var(--error)" onclick='removeTrustedGroup("${escape(g)}")' title="Remove">×</button></span>`
    ).join('')
  }</div>`;
}

async function loadTrustedGroups() {
  const el = document.getElementById('trusted-groups-list');
  if (!el) return;
  try {
    _trustedGroupsCache = await trustedGroups.list();
    const filterInput = document.getElementById('trusted-groups-filter');
    renderTrustedGroupPills(filterInput ? filterInput.value : '');
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

window.filterTrustedGroups = function(value) {
  renderTrustedGroupPills(value);
};

window.addTrustedGroup = async function() {
  const input = document.getElementById('new-trusted-group');
  const status = document.getElementById('trusted-groups-status');
  const name = input ? input.value.trim() : '';
  if (!name) { status.innerHTML = '<p class="error">Enter a group name.</p>'; return; }
  try {
    await trustedGroups.add(name);
    input.value = '';
    showToast('Group added');
    loadTrustedGroups();
  } catch(e) {
    status.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.removeTrustedGroup = async function(name) {
  try {
    await trustedGroups.remove(name);
    showToast('Group removed');
    loadTrustedGroups();
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.viewSettings = viewSettings;

window.runSetupWizard = function() {
  showWizard(() => viewSettings());
};
