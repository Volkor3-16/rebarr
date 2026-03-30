// Settings view

import { providers, settings, trustedGroups, providerScores, webhooks } from '../api.js';
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
            <span>Browser workers:</span>
            <input type="number" id="browser-worker-count" class="input input-bordered input-sm" min="1" max="16" value="${escape(appSettings.browser_worker_count || 3)}" style="width:80px"
              title="Maximum number of concurrent browser-backed provider jobs. Higher values are faster but use more RAM/CPU.">
          </label>
          <label class="flex gap-1 align-center">
            <span>Preferred language (BCP 47):</span>
            <input type="text" id="preferred-language" class="input input-bordered input-sm" placeholder="e.g. en" value="${escape(appSettings.preferred_language || '')}" style="width:80px"
              title="Chapters in this language are preferred. Leave blank to accept any language.">
          </label>
          <label class="flex gap-1 align-center">
            <input type="checkbox" id="auto-unmonitor-completed" class="checkbox checkbox-sm" ${appSettings.auto_unmonitor_completed ? 'checked' : ''}>
            <span>Auto-unmonitor completed AniList series</span>
          </label>
          <label class="flex gap-1 align-center">
            <span>Download mode:</span>
            <select id="download-mode" class="select select-bordered select-sm" title="Best Only: try only the top-ranked release, fail immediately if unavailable. Must Have: try the best first, fall back to alternatives on failure.">
              <option value="must_have" ${appSettings.download_mode !== 'best_only' ? 'selected' : ''}>Must Have (fallback)</option>
              <option value="best_only" ${appSettings.download_mode === 'best_only' ? 'selected' : ''}>Best Only</option>
            </select>
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
          <iconify-icon icon="mdi:webhook" width="20" height="20"></iconify-icon>
          <h3>Task Webhooks</h3>
        </div>
        <p class="settings-card-desc">Send task lifecycle events to external services. Each endpoint can subscribe to specific task types and statuses.</p>
        <div id="webhooks-list"><p>Loading...</p></div>
        <div style="display:grid;gap:0.6rem;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));margin-top:0.75rem">
          <input type="hidden" id="webhook-edit-id">
          <label>
            <div style="font-size:0.8rem;opacity:0.75;margin-bottom:0.2rem">Webhook URL</div>
            <input type="url" id="webhook-url" class="input input-bordered input-sm" placeholder="https://example.com/rebarr">
          </label>
          <label>
            <div style="font-size:0.8rem;opacity:0.75;margin-bottom:0.2rem">Task Types</div>
            <select id="webhook-task-types" class="select select-bordered select-sm" multiple size="5">
              <option value="BuildFullChapterList">BuildFullChapterList</option>
              <option value="RefreshMetadata">RefreshMetadata</option>
              <option value="CheckNewChapter">CheckNewChapter</option>
              <option value="DownloadChapter">DownloadChapter</option>
              <option value="ScanDisk">ScanDisk</option>
              <option value="OptimiseChapter">OptimiseChapter</option>
              <option value="Backup">Backup</option>
            </select>
          </label>
          <label>
            <div style="font-size:0.8rem;opacity:0.75;margin-bottom:0.2rem">Task Statuses</div>
            <select id="webhook-task-statuses" class="select select-bordered select-sm" multiple size="5">
              <option value="Pending">Pending</option>
              <option value="Running">Running</option>
              <option value="Completed">Completed</option>
              <option value="Failed">Failed</option>
              <option value="Cancelled">Cancelled</option>
            </select>
          </label>
        </div>
        <div style="margin-top:0.75rem">
          <label>
            <div style="font-size:0.8rem;opacity:0.75;margin-bottom:0.2rem">Body template <span style="opacity:0.6">(optional — leave blank to send raw JSON)</span></div>
            <textarea id="webhook-body-template" class="input input-bordered input-sm" rows="4" style="width:100%;font-family:monospace;font-size:0.78rem" placeholder='{"embeds":[{"title":"{{task_type}} — {{status}}","description":"{{manga_title}} Ch.{{chapter_number_raw}}"}]}'></textarea>
            <div style="font-size:0.72rem;opacity:0.55;margin-top:0.2rem">Variables: {{task_id}} {{task_type}} {{status}} {{queue}} {{priority}} {{attempt}} {{max_attempts}} {{last_error}} {{manga_id}} {{manga_title}} {{chapter_id}} {{chapter_number_raw}} {{created_at}} {{updated_at}}</div>
          </label>
        </div>
        <label style="display:flex;gap:0.5rem;align-items:center;margin-top:0.75rem">
          <input type="checkbox" id="webhook-enabled" class="checkbox checkbox-sm" checked>
          <span>Enabled</span>
        </label>
        <div class="mt-2 flex gap-1">
          <button class="btn btn-sm btn-primary" onclick="saveWebhook()">Save Webhook</button>
          <button class="btn btn-sm btn-ghost" onclick="resetWebhookForm()">Clear</button>
        </div>
        <div id="webhooks-status"></div>
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
        <p class="settings-card-desc">Manage libraries (add, edit paths, delete) on the <a href="/library" data-path="/library">Libraries page</a>.</p>
      </div>
    `);

    // Load existing global scores into inputs
    loadGlobalScores(providerList);

    // Settings form handler
    document.getElementById('settings-form').addEventListener('submit', async (e) => {
      e.preventDefault();
        const hours = parseInt(document.getElementById('scan-interval').value, 10);
        const browserWorkers = parseInt(document.getElementById('browser-worker-count').value, 10);
        const lang = document.getElementById('preferred-language').value.trim();
        const autoUnmonitorCompleted = document.getElementById('auto-unmonitor-completed').checked;
        const downloadMode = document.getElementById('download-mode').value;
      const statusEl = document.getElementById('settings-status');

      if (!hours || hours < 1 || hours > 168) {
        statusEl.innerHTML = '<p class="error">Interval must be 1–168 hours.</p>';
        return;
      }
      if (!browserWorkers || browserWorkers < 1 || browserWorkers > 16) {
        statusEl.innerHTML = '<p class="error">Browser workers must be 1–16.</p>';
        return;
      }

      try {
        await settings.update({
          scan_interval_hours: hours,
          browser_worker_count: browserWorkers,
          preferred_language: lang || null,
          auto_unmonitor_completed: autoUnmonitorCompleted,
          download_mode: downloadMode,
        });
        showToast('Settings saved');
        statusEl.innerHTML = '';
      } catch(err) {
        statusEl.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
      }
    });

    loadTrustedGroups();
    loadWebhooks();
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
let _webhookCache = [];

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

function selectedOptions(id) {
  const el = document.getElementById(id);
  return el ? [...el.selectedOptions].map(opt => opt.value) : [];
}

function setSelectedOptions(id, values) {
  const selected = new Set(values || []);
  const el = document.getElementById(id);
  if (!el) return;
  [...el.options].forEach(opt => {
    opt.selected = selected.has(opt.value);
  });
}

window.resetWebhookForm = function() {
  const ids = ['webhook-edit-id', 'webhook-url', 'webhook-body-template'];
  ids.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
  const enabled = document.getElementById('webhook-enabled');
  if (enabled) enabled.checked = true;
  setSelectedOptions('webhook-task-types', []);
  setSelectedOptions('webhook-task-statuses', []);
};

async function loadWebhooks() {
  const el = document.getElementById('webhooks-list');
  if (!el) return;
  try {
    _webhookCache = await webhooks.list();
    if (_webhookCache.length === 0) {
      el.innerHTML = '<p><small>No webhooks configured yet.</small></p>';
      return;
    }
    el.innerHTML = `
      <table>
        <thead><tr><th>URL</th><th>Task Types</th><th>Statuses</th><th>Enabled</th><th></th></tr></thead>
        <tbody>
          ${_webhookCache.map(hook => `
            <tr>
              <td style="max-width:280px;word-break:break-word">${escape(hook.target_url)}</td>
              <td>${escape(hook.task_types.join(', '))}</td>
              <td>${escape(hook.task_statuses.join(', '))}</td>
              <td>${hook.enabled ? 'Yes' : 'No'}</td>
              <td style="white-space:nowrap">
                <button class="btn btn-xs" onclick="editWebhook('${hook.id}')">Edit</button>
                <button class="btn btn-xs btn-error" onclick="deleteWebhook('${hook.id}')">Delete</button>
              </td>
            </tr>
          `).join('')}
        </tbody>
      </table>
    `;
  } catch (e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

window.editWebhook = function(id) {
  const hook = _webhookCache.find(entry => entry.id === id);
  if (!hook) return;
  document.getElementById('webhook-edit-id').value = hook.id;
  document.getElementById('webhook-url').value = hook.target_url;
  document.getElementById('webhook-enabled').checked = !!hook.enabled;
  document.getElementById('webhook-body-template').value = hook.body_template || '';
  setSelectedOptions('webhook-task-types', hook.task_types);
  setSelectedOptions('webhook-task-statuses', hook.task_statuses);
};

window.saveWebhook = async function() {
  const status = document.getElementById('webhooks-status');
  const id = document.getElementById('webhook-edit-id').value;
  const bodyTemplate = document.getElementById('webhook-body-template').value.trim();
  const payload = {
    target_url: document.getElementById('webhook-url').value.trim(),
    enabled: document.getElementById('webhook-enabled').checked,
    task_types: selectedOptions('webhook-task-types'),
    task_statuses: selectedOptions('webhook-task-statuses'),
    body_template: bodyTemplate || null,
  };

  try {
    if (id) {
      await webhooks.update(id, payload);
      showToast('Webhook updated');
    } else {
      await webhooks.create(payload);
      showToast('Webhook created');
    }
    if (status) status.innerHTML = '';
    resetWebhookForm();
    loadWebhooks();
  } catch (e) {
    if (status) status.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};

window.deleteWebhook = async function(id) {
  try {
    await webhooks.delete(id);
    showToast('Webhook deleted');
    loadWebhooks();
  } catch (e) {
    const status = document.getElementById('webhooks-status');
    if (status) status.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
};
