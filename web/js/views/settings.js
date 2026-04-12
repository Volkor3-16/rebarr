// Settings view

import { providers, settings, providerSettings, webhooks, qualityRules as qualityRulesApi, metadataRules as metadataRulesApi } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton, showToast } from '../utils.js';
import { showWizard } from './wizard.js';

let _settingsSaveSeq = 0;

export async function viewSettings() {
  render(`<div class="settings">${skeleton(5)}</div>`);

  try {
    const [providerList, appSettings] = await Promise.all([
      providers.list(),
      settings.get(),
    ]);

    // Provider rows with enabled toggle
    const pRows = providerList.length === 0
      ? '<tr><td colspan="3">No providers loaded. Add YAML files to the providers/ directory.</td></tr>'
      : providerList.map(p => `
          <tr data-provider-row="${escape(p.name)}">
            <td>${escape(p.name)}</td>
            <td>${p.needs_browser ? '<iconify-icon icon="mdi:google-chrome" width="16" height="16" title="Requires browser"></iconify-icon>' : '—'}</td>
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
            <input type="checkbox" id="disable-chapter-upgrades" class="checkbox checkbox-sm" ${appSettings.disable_chapter_upgrades ? 'checked' : ''}>
            <span>Disable chapter upgrades</span>
          </label>
          <label class="flex gap-1 align-center">
            <span>Download mode:</span>
            <select id="download-mode" class="select select-bordered select-sm" title="Best Only: try only the top-ranked release, fail immediately if unavailable. Must Have: try the best first, fall back to alternatives on failure.">
              <option value="must_have" ${appSettings.download_mode !== 'best_only' ? 'selected' : ''}>Must Have (fallback)</option>
              <option value="best_only" ${appSettings.download_mode === 'best_only' ? 'selected' : ''}>Best Only</option>
            </select>
          </label>
          <button type="submit" id="settings-save-btn" class="btn btn-primary btn-sm">Save now</button>
        </form>
        <p class="settings-card-desc" style="margin-top:0.75rem">When enabled, newly discovered chapters still download automatically, but rebarr will keep already-downloaded chapters as canonical until you replace them manually.</p>
        <div id="settings-status"></div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:power" width="20" height="20"></iconify-icon>
          <h3>Providers</h3>
        </div>
        <p class="settings-card-desc">Enable or disable providers globally. Disabled providers are excluded from all chapter lookups. Per-series overrides can be set on each series page.</p>
        <table>
          <thead><tr><th>Provider</th><th>Browser</th><th>Enabled</th></tr></thead>
          <tbody id="provider-settings-body">${pRows}</tbody>
        </table>
        <div id="provider-settings-status"></div>
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
          <iconify-icon icon="mdi:star-settings-outline" width="20" height="20"></iconify-icon>
          <h3>Quality Rules</h3>
        </div>
        <p class="settings-card-desc">Rules scored when choosing the best source for each chapter. Each rule's score is added when all its conditions match. Higher total score wins. Re-scan series after changing rules to apply.</p>
        <div id="quality-rules-list"><p>Loading...</p></div>
        <div class="mt-2">
          <button class="btn btn-sm" onclick="showAddQualityRuleModal()">+ Add rule</button>
        </div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:tag-text-outline" width="20" height="20"></iconify-icon>
          <h3>Metadata Rules</h3>
        </div>
        <p class="settings-card-desc">Clean up or override chapter metadata (title, scanlator group) from specific providers before it is displayed or used in merging.</p>
        <div id="metadata-rules-list"><p>Loading...</p></div>
        <div class="mt-2">
          <button class="btn btn-sm" onclick="showAddMetadataRuleModal()">+ Add rule</button>
        </div>
      </div>

      <div class="settings-card">
        <div class="settings-card-header">
          <iconify-icon icon="mdi:folder-multiple-outline" width="20" height="20"></iconify-icon>
          <h3>Libraries</h3>
        </div>
        <p class="settings-card-desc">Manage libraries (add, edit paths, delete) on the <a href="/library" data-path="/library">Libraries page</a>.</p>
      </div>
    `);

    // Load existing global enabled settings
    loadProviderSettings(providerList);

    bindSchedulerAutoSave();

    loadQualityRules();
    loadMetadataRules();
    loadWebhooks();
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

async function saveSchedulerSettings(showToastOnSuccess = false) {
  const hours = parseInt(document.getElementById('scan-interval').value, 10);
  const browserWorkers = parseInt(document.getElementById('browser-worker-count').value, 10);
  const lang = document.getElementById('preferred-language').value.trim();
  const autoUnmonitorCompleted = document.getElementById('auto-unmonitor-completed').checked;
  const disableChapterUpgrades = document.getElementById('disable-chapter-upgrades').checked;
  const downloadMode = document.getElementById('download-mode').value;
  const statusEl = document.getElementById('settings-status');
  const saveBtn = document.getElementById('settings-save-btn');
  const seq = ++_settingsSaveSeq;

  if (!hours || hours < 1 || hours > 168) {
    statusEl.innerHTML = '<p class="error">Interval must be 1–168 hours.</p>';
    return false;
  }
  if (!browserWorkers || browserWorkers < 1 || browserWorkers > 16) {
    statusEl.innerHTML = '<p class="error">Browser workers must be 1–16.</p>';
    return false;
  }

  statusEl.innerHTML = '<small style="color:var(--text-muted)">Saving…</small>';
  if (saveBtn) saveBtn.disabled = true;

  try {
    await settings.update({
      scan_interval_hours: hours,
      browser_worker_count: browserWorkers,
      preferred_language: lang || null,
      auto_unmonitor_completed: autoUnmonitorCompleted,
      disable_chapter_upgrades: disableChapterUpgrades,
      download_mode: downloadMode,
    });
    if (seq === _settingsSaveSeq) {
      statusEl.innerHTML = '<small style="color:var(--success)">Saved</small>';
      setTimeout(() => {
        if (_settingsSaveSeq === seq && statusEl) statusEl.innerHTML = '';
      }, 1500);
    }
    if (showToastOnSuccess) showToast('Settings saved');
    return true;
  } catch(err) {
    if (seq === _settingsSaveSeq) {
      statusEl.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
    }
    return false;
  } finally {
    if (saveBtn) saveBtn.disabled = false;
  }
}

function bindSchedulerAutoSave() {
  const form = document.getElementById('settings-form');
  if (!form) return;

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    await saveSchedulerSettings(true);
  });

  const immediateIds = [
    'auto-unmonitor-completed',
    'disable-chapter-upgrades',
    'download-mode',
  ];
  immediateIds.forEach((id) => {
    const el = document.getElementById(id);
    if (!el) return;
    el.addEventListener('change', () => { void saveSchedulerSettings(false); });
  });

  const delayedIds = [
    'scan-interval',
    'browser-worker-count',
    'preferred-language',
  ];
  delayedIds.forEach((id) => {
    const el = document.getElementById(id);
    if (!el) return;
    el.addEventListener('change', () => { void saveSchedulerSettings(false); });
    el.addEventListener('keydown', async (e) => {
      if (e.key !== 'Enter') return;
      e.preventDefault();
      await saveSchedulerSettings(true);
      el.blur();
    });
  });
}

async function loadProviderSettings(providerList) {
  for (const p of providerList) {
    try {
      const res = await providerSettings.getGlobal(p.name);
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

window.saveGlobalEnabled = async function(providerName, enabled) {
  const statusEl = document.getElementById('provider-settings-status');
  try {
    await providerSettings.setGlobal(providerName, enabled);
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

let _webhookCache = [];

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

// ---------------------------------------------------------------------------
// Quality Rules
// ---------------------------------------------------------------------------

let _qualityRulesCache = [];
let _qualityRuleFields = [];

async function loadQualityRules() {
  const el = document.getElementById('quality-rules-list');
  if (!el) return;
  try {
    [_qualityRulesCache, _qualityRuleFields] = await Promise.all([
      qualityRulesApi.list(),
      qualityRulesApi.fields(),
    ]);
    renderQualityRules();
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

function renderQualityRules() {
  const el = document.getElementById('quality-rules-list');
  if (!el) return;
  if (_qualityRulesCache.length === 0) {
    el.innerHTML = '<p><small>No rules yet.</small></p>';
    return;
  }
  el.innerHTML = `
    <table class="table table-sm" style="width:100%">
      <thead><tr><th style="width:2rem"></th><th>Name</th><th>Score</th><th>Conditions</th><th></th></tr></thead>
      <tbody id="quality-rules-tbody">
        ${_qualityRulesCache.map(rule => `
          <tr data-rule-id="${escape(rule.id)}">
            <td style="cursor:grab;color:#666">⠿</td>
            <td>${escape(rule.name)}</td>
            <td>
              <span class="badge ${rule.score >= 0 ? 'badge-success' : 'badge-error'}" style="font-variant-numeric:tabular-nums">
                ${rule.score >= 0 ? '+' : ''}${rule.score}
              </span>
            </td>
            <td><small style="color:#888">${formatConditions(rule.conditions)}</small></td>
            <td>
              <button class="btn btn-xs btn-ghost" onclick='editQualityRule(${JSON.stringify(rule)})'>Edit</button>
              <button class="btn btn-xs btn-ghost" style="color:var(--error)" onclick="deleteQualityRule('${escape(rule.id)}')">Delete</button>
            </td>
          </tr>
        `).join('')}
      </tbody>
    </table>`;
}

function formatConditions(conditions) {
  if (!conditions || conditions.length === 0) return '<em>always</em>';
  return conditions.map(c => {
    const neg = c.negate ? 'NOT ' : '';
    if (c.op === 'present') return `${neg}${c.field} present`;
    if (c.op === 'not_present') return `${c.field} not present`;
    return `${neg}${c.field} ${c.op} "${c.value || ''}"`;
  }).join(' AND ');
}

window.showAddQualityRuleModal = function() {
  showQualityRuleModal(null);
};

window.editQualityRule = function(rule) {
  showQualityRuleModal(rule);
};

function showQualityRuleModal(rule) {
  const isEdit = rule !== null;
  const conditionsJson = isEdit ? JSON.stringify(rule.conditions) : '[]';
  document.getElementById('quality-rule-modal')?.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'quality-rule-modal';
  overlay.innerHTML = `
    <div class="modal" style="max-width:560px">
      <h2>${isEdit ? 'Edit' : 'Add'} Quality Rule</h2>
      <div style="margin-top:0.75rem">
        <label>Rule name</label>
        <input type="text" id="qr-name" class="input input-sm" value="${escape(rule?.name || '')}" placeholder="e.g. Official">
      </div>
      <div style="margin-top:0.75rem">
        <label>Score <small style="color:#888">(positive = preferred, negative = penalised)</small></label>
        <input type="number" id="qr-score" class="input input-sm" value="${rule?.score ?? 0}" placeholder="e.g. 500">
      </div>
      <div style="margin-top:0.75rem">
        <label>Sort order</label>
        <input type="number" id="qr-sort" class="input input-sm" value="${rule?.sort_order ?? 50}">
      </div>
      <div style="margin-top:0.75rem">
        <label>Conditions <small style="color:#888">(all must match — leave empty to always apply)</small></label>
        <div id="qr-conditions"></div>
        <button class="btn btn-xs btn-ghost" style="margin-top:0.25rem" onclick="addQrCondition()">+ Add condition</button>
      </div>
      <div class="modal-actions">
        <button class="btn btn-sm btn-ghost" onclick="document.getElementById('quality-rule-modal').remove()">Cancel</button>
        <button class="btn btn-sm btn-primary" onclick="saveQualityRule('${escape(rule?.id || '')}')">Save</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  // Populate conditions
  const condContainer = document.getElementById('qr-conditions');
  const conditions = JSON.parse(conditionsJson);
  conditions.forEach(c => addQrConditionRow(condContainer, c));
}

window.addQrCondition = function() {
  const container = document.getElementById('qr-conditions');
  if (container) addQrConditionRow(container, null);
};

function addQrConditionRow(container, cond) {
  const row = document.createElement('div');
  row.className = 'qr-condition-row';
  row.style.cssText = 'display:flex;gap:0.25rem;margin-top:0.25rem;align-items:center';
  const fieldOpts = _qualityRuleFields.map(f =>
    `<option value="${escape(f.field)}" ${cond?.field === f.field ? 'selected' : ''}>${escape(f.label)}</option>`
  ).join('');
  const opOpts = ['eq','contains','regex','present','not_present'].map(op =>
    `<option value="${op}" ${cond?.op === op ? 'selected' : ''}>${op}</option>`
  ).join('');
  row.innerHTML = `
    <select class="input input-sm qr-field" onchange="updateQrOps(this)">${fieldOpts}</select>
    <select class="input input-sm qr-op">${opOpts}</select>
    <input class="input input-sm qr-value" style="flex:1" value="${escape(cond?.value || '')}" placeholder="value">
    <label style="display:flex;align-items:center;gap:0.25rem;font-size:.8rem;white-space:nowrap"><input type="checkbox" class="qr-negate" ${cond?.negate ? 'checked' : ''}> NOT</label>
    <button class="btn btn-xs btn-ghost" style="color:var(--error)" onclick="this.closest('.qr-condition-row').remove()">×</button>`;
  container.appendChild(row);
}

window.updateQrOps = function(fieldSel) {
  const row = fieldSel.closest('.qr-condition-row');
  const field = fieldSel.value;
  const fieldDef = _qualityRuleFields.find(f => f.field === field);
  if (!fieldDef) return;
  const opSel = row.querySelector('.qr-op');
  const current = opSel.value;
  opSel.innerHTML = fieldDef.ops.map(op =>
    `<option value="${op}" ${op === current ? 'selected' : ''}>${op}</option>`
  ).join('');
};

window.saveQualityRule = async function(existingId) {
  const name = document.getElementById('qr-name')?.value.trim();
  const score = parseInt(document.getElementById('qr-score')?.value, 10);
  const sortOrder = parseInt(document.getElementById('qr-sort')?.value, 10);
  if (!name) { showToast('Rule name is required', 'error'); return; }
  if (isNaN(score)) { showToast('Score must be a number', 'error'); return; }
  const conditions = [];
  document.querySelectorAll('.qr-condition-row').forEach(row => {
    const field = row.querySelector('.qr-field')?.value;
    const op = row.querySelector('.qr-op')?.value;
    const value = row.querySelector('.qr-value')?.value || undefined;
    const negate = row.querySelector('.qr-negate')?.checked || false;
    if (field && op) conditions.push({ field, op, value, negate });
  });
  try {
    if (existingId) {
      await qualityRulesApi.update(existingId, { name, score, sort_order: sortOrder, conditions });
    } else {
      await qualityRulesApi.create({ name, score, sort_order: sortOrder, conditions });
    }
    document.getElementById('quality-rule-modal')?.remove();
    showToast('Rule saved');
    loadQualityRules();
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.deleteQualityRule = async function(id) {
  try {
    await qualityRulesApi.delete(id);
    showToast('Rule deleted');
    loadQualityRules();
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

// ---------------------------------------------------------------------------
// Metadata Rules
// ---------------------------------------------------------------------------

let _metadataRulesCache = [];

async function loadMetadataRules() {
  try {
    _metadataRulesCache = await metadataRulesApi.list();
    renderMetadataRules();
  } catch(e) {
    const el = document.getElementById('metadata-rules-list');
    if (el) el.innerHTML = `<p class="error">Error loading rules: ${escape(e.message)}</p>`;
  }
}

function renderMetadataRules() {
  const el = document.getElementById('metadata-rules-list');
  if (!el) return;
  if (_metadataRulesCache.length === 0) {
    el.innerHTML = '<p style="color:#888;font-size:.875rem">No rules defined.</p>';
    return;
  }
  el.innerHTML = `
    <table class="table table-xs w-full">
      <thead><tr><th>Name</th><th>Provider</th><th>Field</th><th>Action</th><th>Pattern / Value</th><th></th></tr></thead>
      <tbody>
        ${_metadataRulesCache.map(rule => `
          <tr data-mr-id="${escape(rule.id)}">
            <td>${escape(rule.name)}</td>
            <td><small style="color:#888">${rule.provider_name ? escape(rule.provider_name) : '<em>all</em>'}</small></td>
            <td><code>${escape(rule.field)}</code></td>
            <td><span class="badge badge-neutral badge-sm">${escape(rule.action)}</span></td>
            <td><small style="color:#888;word-break:break-all">${rule.pattern ? escape(rule.pattern) : ''}${rule.value ? (rule.pattern ? ' → ' : '') + escape(rule.value) : ''}</small></td>
            <td>
              <button class="btn btn-xs btn-ghost" onclick='editMetadataRule(${JSON.stringify(rule)})'>Edit</button>
              <button class="btn btn-xs btn-ghost" style="color:var(--error)" onclick="deleteMetadataRule('${escape(rule.id)}')">Delete</button>
            </td>
          </tr>
        `).join('')}
      </tbody>
    </table>`;
}

window.showAddMetadataRuleModal = function() {
  showMetadataRuleModal(null);
};

window.editMetadataRule = function(rule) {
  showMetadataRuleModal(rule);
};

function showMetadataRuleModal(rule) {
  const isEdit = rule !== null;
  document.getElementById('metadata-rule-modal')?.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'metadata-rule-modal';
  overlay.innerHTML = `
    <div class="modal" style="max-width:500px">
      <h2>${isEdit ? 'Edit' : 'Add'} Metadata Rule</h2>
      <div style="margin-top:0.75rem">
        <label>Rule name</label>
        <input type="text" id="mr-name" class="input input-sm" value="${escape(rule?.name || '')}" placeholder="e.g. Strip generic titles">
      </div>
      <div style="margin-top:0.75rem">
        <label>Sort order</label>
        <input type="number" id="mr-sort" class="input input-sm" value="${rule?.sort_order ?? 50}">
      </div>
      <div style="margin-top:0.75rem">
        <label>Provider <small style="color:#888">(leave blank to apply to all providers)</small></label>
        <input type="text" id="mr-provider" class="input input-sm" value="${escape(rule?.provider_name || '')}" placeholder="e.g. WeebCentral">
      </div>
      <div style="margin-top:0.75rem">
        <label>Field</label>
        <select id="mr-field" class="input input-sm" onchange="updateMrActionHelp()">
          <option value="title" ${(rule?.field || 'title') === 'title' ? 'selected' : ''}>title</option>
          <option value="scanlator_group" ${rule?.field === 'scanlator_group' ? 'selected' : ''}>scanlator_group</option>
        </select>
      </div>
      <div style="margin-top:0.75rem">
        <label>Action</label>
        <select id="mr-action" class="input input-sm" onchange="updateMrActionHelp()">
          <option value="clear" ${(rule?.action || '') === 'clear' ? 'selected' : ''}>clear — remove value (if pattern matches)</option>
          <option value="set" ${rule?.action === 'set' ? 'selected' : ''}>set — override with fixed value</option>
          <option value="replace" ${rule?.action === 'replace' ? 'selected' : ''}>replace — regex substitution</option>
        </select>
      </div>
      <div class="form-group" id="mr-pattern-row">
        <label>Pattern (regex) — required for clear/replace</label>
        <input type="text" id="mr-pattern" class="input input-sm" style="font-family:monospace" value="${escape(rule?.pattern || '')}" placeholder="e.g. ^Chapter\\s*\\d+$">
      </div>
      <div class="form-group" id="mr-value-row">
        <label>Value — required for set/replace</label>
        <input type="text" id="mr-value" class="input input-sm" value="${escape(rule?.value || '')}" placeholder="e.g. Official">
      </div>
      <div class="modal-actions">
        <button class="btn btn-sm btn-ghost" onclick="document.getElementById('metadata-rule-modal').remove()">Cancel</button>
        <button class="btn btn-sm btn-primary" onclick="saveMetadataRule('${escape(rule?.id || '')}')">Save</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  updateMrActionHelp();
}

window.updateMrActionHelp = function() {
  const action = document.getElementById('mr-action')?.value;
  const patternRow = document.getElementById('mr-pattern-row');
  const valueRow = document.getElementById('mr-value-row');
  if (!action || !patternRow || !valueRow) return;
  patternRow.style.display = action === 'set' ? 'none' : '';
  valueRow.style.display = action === 'clear' ? 'none' : '';
};

window.saveMetadataRule = async function(existingId) {
  const name = document.getElementById('mr-name')?.value.trim();
  const sortOrder = parseInt(document.getElementById('mr-sort')?.value, 10);
  const providerName = document.getElementById('mr-provider')?.value.trim() || null;
  const field = document.getElementById('mr-field')?.value;
  const action = document.getElementById('mr-action')?.value;
  const pattern = document.getElementById('mr-pattern')?.value.trim() || null;
  const value = document.getElementById('mr-value')?.value.trim() || null;

  if (!name) { showToast('Rule name is required', 'error'); return; }
  if (!field) { showToast('Field is required', 'error'); return; }
  if (!action) { showToast('Action is required', 'error'); return; }
  if ((action === 'clear' || action === 'replace') && !pattern) {
    showToast('Pattern is required for clear/replace actions', 'error'); return;
  }
  if ((action === 'set' || action === 'replace') && !value) {
    showToast('Value is required for set/replace actions', 'error'); return;
  }

  const payload = { name, sort_order: sortOrder, provider_name: providerName, field, action, pattern, value };
  try {
    if (existingId) {
      await metadataRulesApi.update(existingId, payload);
    } else {
      await metadataRulesApi.create(payload);
    }
    document.getElementById('metadata-rule-modal')?.remove();
    showToast('Rule saved');
    loadMetadataRules();
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.deleteMetadataRule = async function(id) {
  try {
    await metadataRulesApi.delete(id);
    showToast('Rule deleted');
    loadMetadataRules();
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};
