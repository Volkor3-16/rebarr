// Settings view

import { providers, settings, trustedGroups } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton, showToast } from '../utils.js';

export async function viewSettings() {
  render(`<div class="settings">${skeleton(5)}</div>`);
  
  try {
    const [providerList, appSettings] = await Promise.all([
      providers.list(),
      settings.get(),
    ]);
    
    const pRows = providerList.length === 0
      ? '<tr><td colspan="2">No providers loaded. Add YAML files to the providers/ directory.</td></tr>'
      : providerList.map(p => `
          <tr>
            <td>${escape(p.name)}</td>
            <td>${p.needs_browser ? 'Yes (browser)' : 'No'}</td>
          </tr>
        `).join('');

    // Parse filter languages for display (default to empty if not set)
    const filterLangs = (appSettings.synonym_filter_languages || '')
      .split(',')
      .map(s => s.trim().toLowerCase())
      .filter(s => s);
    const filterLangsHtml = filterLangs.map(lang => 
      `<span class="badge badge-neutral">${escape(lang)} <button class="synonym-remove btn btn-xs btn-ghost" style="padding:0;margin-left:4px;min-height:auto;line-height:1" onclick="removeFilterLanguage('${escape(lang)}')" title="Remove">×</button></span>`
    ).join('');

    render(`
      <h2>Settings</h2>
      
      <h3>Scheduler</h3>
      <p>Rebarr periodically checks for new chapters on all monitored series.</p>
      <form id="settings-form">
        <label>Scan interval (hours):
          <input type="number" id="scan-interval" min="1" max="168" value="${escape(appSettings.scan_interval_hours)}" style="width:80px">
        </label>
        <button type="submit" class="btn btn-primary">Save</button>
      </form>
      <div id="settings-status"></div>
      
      <hr>
      <h3>Synonym Language Filter</h3>
      <p><small>Synonyms in these languages are disabled from provider searches. <a href="https://github.com/greyblake/whatlang-rs/blob/master/SUPPORTED_LANGUAGES.md">Use whatlang codes.</a></small></p>
      <div id="filter-languages-list">${filterLangsHtml || '<p><small>No languages configured.</small></p>'}</div>
      <div class="mt-2 flex gap-1">
        <input type="text" id="new-filter-language" placeholder="Language code (e.g. cmn)" style="width:120px">
        <button class="btn btn-sm" onclick="addFilterLanguage()">Add</button>
      </div>
      <div id="filter-languages-status"></div>
      
      <hr>
      <h3>Providers</h3>
      <p><small>Providers are loaded from YAML files. Restart to pick up changes.</small></p>
      <table>
        <thead><tr><th>Name</th><th>Browser?</th></tr></thead>
        <tbody>${pRows}</tbody>
      </table>
      
      <hr>
      <h3>Trusted Scanlation Groups</h3>
      <p><small>Groups listed here are Tier 2 (trusted). Chapters from these groups score higher than unknown groups (Tier 3), but lower than official releases (Tier 1, auto-detected via "Official" in the name). Re-scan a series after changing this list to update scores.</small></p>
      <div id="trusted-groups-list"><p>Loading...</p></div>
      <div class="mt-2 flex gap-1">
        <input type="text" id="new-trusted-group" placeholder="Group name (exact)" style="width:220px">
        <button class="btn btn-sm" onclick="addTrustedGroup()">Add</button>
      </div>
      <div id="trusted-groups-status"></div>
      
      <hr>
      <h3>Libraries</h3>
      <p>Manage libraries (add, edit paths, delete) on the <a onclick="navigate('/library')">Libraries page</a>.</p>
    `);
    
    // Settings form handler
    document.getElementById('settings-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const hours = parseInt(document.getElementById('scan-interval').value, 10);
      const statusEl = document.getElementById('settings-status');
      
      if (!hours || hours < 1 || hours > 168) {
        statusEl.innerHTML = '<p class="error">Interval must be 1–168 hours.</p>';
        return;
      }
      
      try {
        await settings.update({ scan_interval_hours: hours });
        showToast('Settings saved');
      } catch(err) {
        statusEl.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
      }
    });
    
    loadTrustedGroups();
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

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

async function loadTrustedGroups() {
  const el = document.getElementById('trusted-groups-list');
  if (!el) return;
  try {
    const groups = await trustedGroups.list();
    if (groups.length === 0) {
      el.innerHTML = '<p><small>No trusted groups yet.</small></p>';
      return;
    }
    el.innerHTML = '<ul style="margin:0.3rem 0">' + groups.map(g =>
      `<li style="margin:0.25rem 0">${escape(g)} <button class="btn btn-xs btn-error btn-outline" onclick='removeTrustedGroup("${escape(g)}")'>Remove</button></li>`
    ).join('') + '</ul>';
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

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
