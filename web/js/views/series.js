// Series detail view - manga info + chapters + live task status

import { manga as mangaApi, tasks, trustedGroups, providerScores, coverApi } from '../api.js';
import { render, setPoll, navigate } from '../router.js';
import { escape, relTime, statusBadge, taskBadge, tierBadgeHtml, skeleton, showToast, truncateMiddle, formatFileSize, renderTaskProgress } from '../utils.js';

let currentMangaId = null;
let trustedGroupsCache = [];
let chapterDataCache = [];
let providersCache = []; // Cache provider names for filtering
let currentSort = { field: 'chapter', direction: 'desc' };
let currentFilter = { search: '', status: '', provider: '' };

// Loading overlay / banner state
let tipsCache = null;
let currentTipIndex = 0;
let tipTimer = null;
let loadingLogs = [];
let overlayRendered = false;
let chaptersEverLoaded = false;

// Friendly task names
const FRIENDLY_NAMES = {
  'BuildFullChapterList': 'Initial Provider Search',
  'CheckNewChapter': 'Checking for New Chapters',
  'DownloadChapter': 'Downloading Chapter',
  'RefreshMetadata': 'Refreshing Metadata',
  'ScanDisk': 'Scanning Disk',
  'OptimiseChapter': 'Optimising Chapter',
};

function friendlyName(taskType) {
  return FRIENDLY_NAMES[taskType] || taskType;
}

// SVG spinner helper - always spins endlessly
function spinnerSvg(percent, size = 64) {
  const r = (size - 6) / 2;
  const circ = 2 * Math.PI * r;
  return `<svg class="spinner-svg spinning" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
    <circle class="spinner-track" cx="${size/2}" cy="${size/2}" r="${r}"/>
    <circle class="spinner-fill" cx="${size/2}" cy="${size/2}" r="${r}"
      stroke-dasharray="${circ}" stroke-dashoffset="${circ * 0.25}"/>
  </svg>`;
}

// Load tips from JSON file
async function loadTips() {
  if (tipsCache) return tipsCache;
  try {
    const resp = await fetch('/web/js/tips.json');
    tipsCache = await resp.json();
  } catch(e) {
    tipsCache = [{ text: 'While you wait, why not check your provider settings?' }];
  }
  return tipsCache;
}

// Start cycling tips (returns cleanup function)
function startTipCycling(containerEl) {
  stopTipCycling();
  const tips = tipsCache || [];
  if (tips.length === 0) return () => {};

  currentTipIndex = Math.floor(Math.random() * tips.length);

  function showNext() {
    if (!containerEl || !document.contains(containerEl)) { stopTipCycling(); return; }
    containerEl.classList.add('fading');
    setTimeout(() => {
      currentTipIndex = (currentTipIndex + 1) % tips.length;
      containerEl.textContent = tips[currentTipIndex].text;
      containerEl.classList.remove('fading');
    }, 400);
  }

  // Show first tip immediately
  containerEl.textContent = tips[currentTipIndex].text;

  tipTimer = setInterval(showNext, 16000);
  return stopTipCycling;
}

function stopTipCycling() {
  if (tipTimer) { clearInterval(tipTimer); tipTimer = null; }
}

// Add a log entry (keeps last 6, deduplicates by message)
function addLog(message) {
  if (loadingLogs.length > 0 && loadingLogs[0].message === message) return;
  const now = new Date();
  const time = now.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  loadingLogs.unshift({ time, message });
  if (loadingLogs.length > 6) loadingLogs.pop();
}

function renderLogsHtml() {
  if (loadingLogs.length === 0) return '';
  return `<div class="loading-overlay-logs">${loadingLogs.map(l =>
    `<div class="loading-overlay-log-entry"><span class="log-time">${l.time}</span>${escape(l.message)}</div>`
  ).join('')}</div>`;
}

// Clear all loading UI
function clearLoadingUI() {
  stopTipCycling();
  loadingLogs = [];
  overlayRendered = false;
  const banner = document.getElementById('tasks-banner');
  if (banner) banner.innerHTML = '';
}

export async function viewSeries(id) {
  currentMangaId = id;
  chaptersEverLoaded = false; // Reset when viewing a new series
  overlayRendered = false; // Reset overlay state
  render(`<div class="series">${skeleton(5)}</div>`);
  
  try {
    // Load trusted groups for bubble UI
    try {
      trustedGroupsCache = await trustedGroups.list();
    } catch(e) {
      trustedGroupsCache = [];
    }
    
    const m = await mangaApi.get(id);
    const meta = m.metadata ?? {};
    const year = meta.start_year ? (meta.end_year ? `${meta.start_year} - ${meta.end_year}` : `${meta.start_year} - ongoing`) : '?';
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    
    const thumb = m.thumbnail_url 
      ? `<img class="cover-lg" src="${escape(m.thumbnail_url)}" alt="cover" onclick="showCoverUpload('${m.id}')" title="Click to change cover">`
      : `<div class="cover-placeholder" onclick="showCoverUpload('${m.id}')">
          <iconify-icon class="cover-placeholder-icon" icon="mdi:image-plus"></iconify-icon>
          <span class="cover-placeholder-text">Add Cover</span>
        </div>`;
    
    const tags = (meta.tags ?? []).map(t => `<span class="badge badge-neutral">${escape(t)}</span>`).join(' ');
    const aniLink = m.anilist_id 
      ? `<a href="https://anilist.co/manga/${m.anilist_id}" target="_blank" class="anilist-link"><iconify-icon icon="simple-icons:anilist" width="16" height="16"></iconify-icon><span>AniList</span></a>` 
      : '';
    
    document.title = `${meta.title ?? 'Manga'} — REBARR`;
    const isMonitored = m.monitored !== false;
    const monitoredClass = isMonitored ? 'monitored' : '';

    render(`
      <div class="series-header">
        <div class="series-cover">${thumb}</div>
        <div class="series-info">
          <h2>${escape(meta.title)}</h2>
          
          <div class="series-actions-row">
            <label class="monitored-toggle ${monitoredClass}" title="${isMonitored ? 'Monitored - click to unmonitor' : 'Not monitored - click to monitor'}">
              <input type="checkbox" id="monitored-cb" ${isMonitored ? 'checked' : ''} onchange="toggleMonitored('${m.id}', this.checked)"> 
              <iconify-icon icon="mdi:${isMonitored ? 'bookmark' : 'bookmark-outline'}" width="24" height="24"></iconify-icon>
            </label>
            <button class="btn btn-sm btn-danger" onclick='showDeleteSeriesModal("${m.id}", ${JSON.stringify(meta.title ?? "Series")})'>
              <iconify-icon icon="mdi:delete" width="18" height="18"></iconify-icon>
              Delete Series
            </button>
          </div>
          
          <div class="series-meta">
            <div class="series-meta-item">
              <span class="label">Years:</span>
              <span class="value">${escape(year)}</span>
            </div>
            <div class="series-meta-item">
              <span class="label">Status:</span>
              <span class="value">${escape(meta.publishing_status)}</span>
            </div>
            <div class="series-meta-item">
              <span class="label">Chapters:</span>
              <span class="value">${dl} / ${total} downloaded</span>
            </div>
            <div class="series-meta-item">
              <span class="label">Folder:</span>
              <span class="value">${escape(m.relative_path)}</span>
            </div>
            ${(meta.other_titles || []).length > 0 ? `
            <div class="series-meta-item">
              <span class="label">Aliases:</span>
              <span class="value synonyms-list" id="synonyms-list">${renderSynonyms(meta.other_titles || [])}</span>
              <button class="btn btn-sm btn-ghost" onclick="addSynonym()" title="Add alias">+</button>
            </div>
            ` : `
            <div class="series-meta-item">
              <span class="label">Aliases:</span>
              <button class="btn btn-sm btn-ghost" onclick="addSynonym()" title="Add alias">+ Add</button>
            </div>
            `}
          </div>
          
          <div class="series-synopsis" id="series-synopsis">
            <button class="synopsis-toggle" onclick="toggleSynopsis()">
              <iconify-icon class="synopsis-icon" icon="mdi-chevron-down" width="24" height="24"></iconify-icon>
              <span class="synopsis-text">Show Synopsis</span>
            </button>
            ${aniLink ? aniLink : ''}
            <div class="synopsis-content hidden" id="synopsis-content">
              ${escape(meta.synopsis ?? 'No synopsis available.')}
            </div>
          </div>
          
          ${tags ? `
          <div class="series-tags">
            <span class="label">Tags:</span>
            ${tags}
          </div>
          ` : ''}
        </div>
      </div>
      
      <div class="action-toolbar">
        <button class="btn btn-sm btn-primary" onclick='doScan("${m.id}")'>
          <iconify-icon icon="mdi-web-sync" width="18" height="18"></iconify-icon>
          Search All Providers
        </button>
        <button class="btn btn-sm" onclick='doCheckNew("${m.id}")'>
          <iconify-icon icon="mdi-book-search" width="18" height="18"></iconify-icon>
          Check new Chapters
        </button>
        <button class="btn btn-sm" onclick='doScanDisk("${m.id}")'>
          <iconify-icon icon="mdi-harddisk-plus" width="18" height="18"></iconify-icon>
          Scan Disk
        </button>
        <button class="btn btn-sm" onclick='doRefreshMetadata("${m.id}")'>
          <iconify-icon icon="mdi-database-refresh" width="18" height="18"></iconify-icon>
          Refresh Metadata
        </button>
        <button class="btn btn-sm btn-accent" onclick='doDownloadAllMissing("${m.id}")'>
          <iconify-icon icon="mdi-download" width="18" height="18"></iconify-icon>
          Download All Missing
        </button>
        <button class="btn btn-sm btn-outline" onclick='doDownloadSelected("${m.id}")'>
          <iconify-icon icon="mdi-checkbox-marked" width="18" height="18"></iconify-icon>
          Download Selected
        </button>
        <span id="scan-status"></span>
      </div>
      
      <div id="tasks-banner"></div>
      
      <h3>Chapters</h3>
      <div id="chapters-list"><p>Loading...</p></div>
      
      <h3>Providers</h3>
      <div id="providers-list"><p>Loading...</p></div>
      
      <div class="mt-3">
        <a href="/library" data-path="/library">[Back to Libraries]</a>
      </div>
    `);

    // Load chapters, providers, and tips, then start polling
    await Promise.all([loadChapters(m.id), loadTips()]);
    loadProviders(m.id);

    // Poll for active tasks every 3s
    let prevHadActive = false;
    const pollTasks = async () => {
      try {
        const taskList = await tasks.list({ manga_id: m.id, limit: 20 });
        const active = taskList.filter(t => t.status === 'Running' || t.status === 'Pending');
        const banner = document.getElementById('tasks-banner');
        const chaptersEl = document.getElementById('chapters-list');
        if (!banner || !chaptersEl) return;

        // Find relevant scan tasks for this manga - prioritize RUNNING over PENDING
        const scanTask = active.find(t =>
          t.status === 'Running' &&
          (t.task_type === 'BuildFullChapterList' || t.task_type === 'CheckNewChapter') &&
          t.manga_id === m.id
        ) || active.find(t =>
          t.status === 'Pending' &&
          (t.task_type === 'BuildFullChapterList' || t.task_type === 'CheckNewChapter') &&
          t.manga_id === m.id
        );

        if (active.length > 0) {
          if (scanTask && scanTask.status === 'Running') {
            // FANCY OVERLAY: Scan task is actively running
            banner.innerHTML = ''; // Clear tasks-banner
            const progress = scanTask.progress;
            const percent = progress?.current != null && progress?.total != null
              ? Math.round((progress.current / progress.total) * 100)
              : null;

            // Build activity log (only when running)
            let logsHtml = '';
            if (progress?.detail) {
              addLog(progress.detail);
              logsHtml = renderLogsHtml();
            }

            // Only render overlay HTML once, then update parts
            if (!overlayRendered) {
              chaptersEl.innerHTML = `
                <div class="loading-overlay-card">
                  ${spinnerSvg(percent)}
                  <div class="loading-overlay-title">${friendlyName(scanTask.task_type)}</div>
                  <div class="loading-overlay-subtitle" id="loading-subtitle">${progress?.label || 'Working...'}</div>
                  <div class="loading-overlay-tips" id="loading-tip"></div>
                  <div id="loading-logs">${logsHtml}</div>
                </div>
              `;

              // Start tip cycling once
              const tipContainer = document.getElementById('loading-tip');
              if (tipContainer && tipsCache) {
                startTipCycling(tipContainer);
              }
              overlayRendered = true;
            } else {
              // Update dynamic parts only
              const subtitle = document.getElementById('loading-subtitle');
              if (subtitle) {
                subtitle.textContent = progress?.label || 'Working...';
              }
              const logsDiv = document.getElementById('loading-logs');
              if (logsDiv) {
                logsDiv.innerHTML = logsHtml;
              }
            }
          } else if (scanTask && scanTask.status === 'Pending' && chapterDataCache.length > 0) {
            // COMPACT BANNER: Scan task is queued and there are existing chapters
            stopTipCycling();
            loadingLogs = [];
            const progress = scanTask.progress;
            const percent = progress?.current != null && progress?.total != null
              ? Math.round((progress.current / progress.total) * 100)
              : null;

            banner.innerHTML = `
              <div class="loading-banner">
                ${spinnerSvg(percent, 24)}
                <div class="loading-banner-info">
                  <div class="loading-banner-title">${friendlyName(scanTask.task_type)}</div>
                  <div class="loading-banner-detail">${progress?.detail || progress?.label || 'Working...'}</div>
                </div>
              </div>
            `;
          } else {
            // FALLBACK: Other tasks (downloads, etc.) — show original banner
            stopTipCycling();
            loadingLogs = [];
            banner.innerHTML = '';
            const lines = active.map(t => {
              let taskInfo = friendlyName(t.task_type);
              if (t.chapter_number_raw && (t.task_type === 'DownloadChapter' || t.task_type === 'CheckNewChapter')) {
                taskInfo += ` <small style="color:#888">(Ch. ${escape(t.chapter_number_raw)})</small>`;
              }
              return `
                <div class="task-banner-item">
                  <div><b>${taskInfo}</b>: ${taskBadge(t.status)}</div>
                  ${renderTaskProgress(t.progress)}
                </div>
              `;
            }).join('');
            banner.innerHTML = `<div class="task-banner">${lines}</div>`;
          }
          prevHadActive = true;
        } else {
          banner.innerHTML = '';
          stopTipCycling();
          loadingLogs = [];
          if (prevHadActive) { prevHadActive = false; loadChapters(m.id); }
        }
      } catch(e) { console.warn('Task poll error:', e); }
    };
    setPoll(pollTasks, 3000);
  } catch(e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// Build a compact colored-square overview of all canonical chapters.
// Each square = one chapter, color = download status. Click scrolls to that row.
function buildChapterOverview(chapters) {
  const canonical = chapters
    .filter(ch => ch.is_canonical)
    .sort((a, b) => a.chapter_base * 100 + (a.chapter_variant || 0) - (b.chapter_base * 100 + (b.chapter_variant || 0)));
  if (canonical.length === 0) return '';
  const dots = canonical.map(ch => {
    const base = ch.chapter_base;
    const variant = ch.chapter_variant;
    const chNum = variant === 0 ? `Chapter ${base}` : `Chapter ${base}.${variant}`;
    const titlePart = ch.title ? ` — ${ch.title}` : '';
    const tip = `${chNum}${titlePart} (${ch.download_status})`;
    const cls = `ch-dot ch-dot-${ch.download_status.toLowerCase()}`;
    return `<span class="${cls}" title="${escape(tip)}" data-base="${base}" data-variant="${variant}" onclick="scrollToChapter(${base}, ${variant})"></span>`;
  }).join('');
  return `<div class="ch-overview">${dots}</div>`;
}

// Chapter rendering helpers
function chapterRow(mangaId, ch, isVariant = false, altCount = 0, extraActions = '') {
  const base = ch.chapter_base;
  const variant = ch.chapter_variant;
  const chNum = variant === 0 ? `Chapter ${base}` : `Chapter ${base}.${variant}`;
  
  // Truncate long titles in the middle, keep full title as tooltip
  const rawTitle = ch.title || '';
  const truncatedTitle = truncateMiddle(rawTitle, 50);
  const titleHtml = rawTitle 
    ? ` — <span class="ch-title" title="${escape(rawTitle)}">${escape(truncatedTitle)}</span>` 
    : '';
  const chapterLabel = `<b>${chNum}</b>${titleHtml}`;

  const tierHtml = tierBadgeHtml(ch.tier || 4);

  const sourceUrl = ch.chapter_url;
  const sourceTitle = sourceUrl ? ` title="${escape(sourceUrl)}"` : '';
  const sourceName = ch.provider_name ? escape(ch.provider_name) : (ch.scanlator_group ? escape(ch.scanlator_group) : '—');
  
  // Show +N badge inline next to provider name when there are alternatives
  const expandId = `${base}-${variant}`;
  const altCountHtml = altCount > 0
    ? `<span class="alt-count-bubble" onclick="event.stopPropagation(); toggleChapterExpand('ch-${expandId}', '${expandId}')" title="Click to see ${altCount} alternative${altCount === 1 ? '' : 's'}">+${altCount}</span>`
    : '';

  // Provider name (as link) with alt count badge inline
  const sourceHtml = sourceUrl
    ? `<div class="provider-cell"><a href="${escape(sourceUrl)}" target="_blank" class="ch-source"${sourceTitle}>${sourceName}</a>${altCountHtml}</div>`
    : `<div class="provider-cell"><span class="ch-source">${sourceName}</span>${altCountHtml}</div>`;

  let langHtml = '';
  if (ch.language && ch.language.toLowerCase() !== 'en') {
    langHtml = ` <span style="font-size:0.7em;padding:1px 3px;border-radius:3px;background:#555;color:#fff">${ch.language.toUpperCase()}</span>`;
  }

  const status = ch.download_status;
  const canDl = status === 'Missing' || status === 'Failed';

  // File size label for downloaded chapters
  const fileSizeHtml = (status === 'Downloaded' && ch.file_size_bytes)
    ? `<div class="ch-filesize">${formatFileSize(ch.file_size_bytes)}</div>`
    : '';

  const cb = (!isVariant && canDl)
    ? `<input type="checkbox" class="ch-checkbox" data-base="${base}" data-variant="${variant}" onclick="event.stopPropagation()">`
    : '';

  // Scanlator bubble — click anywhere to toggle trusted state
  const scanlatorName = ch.scanlator_group || '—';
  const isTrusted = trustedGroupsCache.includes(scanlatorName);
  const trustedIndicator = isTrusted ? '<span class="trusted-indicator" title="Trusted scanlator"></span>' : '';
  const trustedClass = isTrusted ? ' scanlator-trusted' : '';
  const trustedTitle = isTrusted ? 'Trusted — click to remove from trusted' : 'Click to add to trusted';

  const scanlatorHtml = scanlatorName !== '—'
    ? `<span class="scanlator-bubble${trustedClass}" title="${trustedTitle}" onclick="event.stopPropagation(); ${isTrusted ? `removeTrustedFromBubble('${escape(scanlatorName)}')` : `addTrustedFromBubble('${escape(scanlatorName)}')`}">${trustedIndicator}${escape(scanlatorName)}</span>`
    : '—';

  // Action menu (three-dot dropdown)
  let actionMenuHtml = '';
  if (!isVariant) {
    const menuId = `menu-${base}-${variant}`;
    const dlBtn = canDl ? `<button onclick="event.stopPropagation(); doDownload('${mangaId}', ${base}, ${variant})">Download</button>` : '';
    const canReset = (status === 'Failed' || status === 'Queued' || status === 'Downloading') && ch.is_canonical;
    const resetBtn = canReset ? `<button onclick="event.stopPropagation(); doResetChapter('${mangaId}', ${base}, ${variant})">Reset</button>` : '';
    const extraBtn = ch.is_canonical ? `<button onclick="event.stopPropagation(); doToggleExtra('${mangaId}', ${base}, ${variant})">${ch.is_extra ? 'Un-extra' : 'Extra'}</button>` : '';
    const deleteBtn = (ch.is_canonical && status !== 'Missing') ? `<button class="danger" onclick="event.stopPropagation(); doDeleteChapter('${mangaId}', ${base}, ${variant})">Delete</button>` : '';
    
    actionMenuHtml = `<div class="action-menu">
      <button class="action-menu-btn" onclick="event.stopPropagation(); toggleActionMenu('${menuId}')"><iconify-icon icon="mdi:dots-vertical" width="18" height="18"></iconify-icon></button>
      <div class="action-menu-dropdown" id="${menuId}">
        ${dlBtn}${resetBtn}${extraBtn}${deleteBtn}
      </div>
    </div>`;
  }

  // Use button for variants
  const useBtn = (isVariant && !ch.is_canonical)
    ? `<button class="btn-sm" onclick='event.stopPropagation(); doSetCanonical("${mangaId}", ${base}, ${variant}, "${ch.id}")'>Use</button>`
    : '';

  const rowClass = isVariant
    ? 'ch-variant ch-row'
    : `ch-main ch-row ch-row-${status.toLowerCase()}`;
  const rowId = `ch-row-${base}-${variant}`;

  return {
    row: `<tr class="${rowClass}" id="${rowId}" onclick="toggleChapterExpand('${rowId}', '${base}-${variant}')">
      <td>${cb}</td>
      <td>${chapterLabel}${langHtml}</td>
      <td>${scanlatorHtml}</td>
      <td>${tierHtml}</td>
      <td>${sourceHtml}</td>
      <td>${statusBadge(status)}${fileSizeHtml}</td>
      <td><small>${relTime(ch.released_at)}</small></td>
      <td><small>${relTime(ch.scraped_at)}</small></td>
      <td>${useBtn}${actionMenuHtml}${extraActions}</td>
    </tr>`,
    base, variant, status, tier: ch.tier || 4, title: chNum, released: ch.released_at
  };
}

function chapterGroupHtml(mangaId, base, mainCh, v0alts, splitParts) {
  if (!mainCh) return '';

  let subRows = [];
  for (const alt of v0alts) {
    subRows.push(chapterRow(mangaId, alt, true));
  }
  for (const sp of splitParts) {
    if (sp.canonical) subRows.push(chapterRow(mangaId, sp.canonical, true));
    for (const alt of sp.alts) {
      subRows.push(chapterRow(mangaId, alt, true));
    }
  }

  const totalSub = v0alts.length + splitParts.reduce((n, sp) => n + 1 + sp.alts.length, 0);

  // Pass alt count to show "+N more" in provider column
  const mainRow = chapterRow(mangaId, mainCh, false, totalSub);

  if (subRows.length === 0) return mainRow.row;

  const groupId = `ch-${mainCh.chapter_base}-${mainCh.chapter_variant}`;
  const expandId = `${mainCh.chapter_base}-${mainCh.chapter_variant}`;

  // Build expandable section
  const subRowsHtml = subRows.map(s => s.row).join('');
  
  return mainRow.row + 
    `<tr class="ch-expandable" id="${groupId}">
      <td colspan="9" style="padding:0;border:0;background:var(--bg-tertiary)">
        <div class="ch-expandable-inner">
          <table style="width:100%">${subRowsHtml}</table>
        </div>
      </td>
    </tr>`;
}

window.scrollToChapter = function(base, variant) {
  document.getElementById(`ch-row-${base}-${variant}`)
    ?.scrollIntoView({ behavior: 'smooth', block: 'center' });
};

window.toggleChapterExpand = function(rowId, expandId) {
  const row = document.getElementById(rowId);
  const expandRow = document.getElementById(`ch-${expandId}`);
  const badge = row?.querySelector('.variant-badge');
  
  if (expandRow) {
    expandRow.classList.toggle('open');
    row?.classList.toggle('expanded');
    badge?.classList.toggle('open');
  }
};

window.toggleActionMenu = function(menuId) {
  // Close all other menus first
  document.querySelectorAll('.action-menu-dropdown.open').forEach(el => {
    if (el.id !== menuId) el.classList.remove('open');
  });
  
  const menu = document.getElementById(menuId);
  if (menu) {
    menu.classList.toggle('open');
  }
};

// Close menus when clicking outside
document.addEventListener('click', (e) => {
  if (!e.target.closest('.action-menu')) {
    document.querySelectorAll('.action-menu-dropdown.open').forEach(el => {
      el.classList.remove('open');
    });
  }
});

window.toggleVariants = function(groupId, toggleEl) {
  const row = document.getElementById(groupId);
  if (!row) return;
  const isOpen = row.classList.toggle('open');
  toggleEl.classList.toggle('open', isOpen);
};

// Filtering and sorting functions
function filterAndSortChapters(chapters) {
  let filtered = [...chapters];
  
  // Filter by search
  if (currentFilter.search) {
    const search = currentFilter.search.toLowerCase();
    filtered = filtered.filter(ch => {
      const chNum = `Chapter ${ch.chapter_base}${ch.chapter_variant > 0 ? '.' + ch.chapter_variant : ''}`;
      return chNum.toLowerCase().includes(search) || 
             (ch.title && ch.title.toLowerCase().includes(search)) ||
             (ch.scanlator_group && ch.scanlator_group.toLowerCase().includes(search)) ||
             (ch.provider_name && ch.provider_name.toLowerCase().includes(search));
    });
  }
  
  // Filter by status
  if (currentFilter.status) {
    if (currentFilter.status === 'Missing') {
      // Build a set of (base,variant) pairs that are NOT missing
      // (i.e. at least one provider has it downloaded/queued/downloading/failed)
      const hasAnyNonMissing = new Set();
      for (const ch of filtered) {
        if (ch.download_status !== 'Missing') {
          hasAnyNonMissing.add(`${ch.chapter_base}|${ch.chapter_variant}`);
        }
      }
      // Only show chapters where NO provider has a non-missing status
      filtered = filtered.filter(ch => !hasAnyNonMissing.has(`${ch.chapter_base}|${ch.chapter_variant}`));
    } else {
      filtered = filtered.filter(ch => ch.download_status === currentFilter.status);
    }
  }
  
  // Filter by provider
  if (currentFilter.provider) {
    filtered = filtered.filter(ch => ch.provider_name === currentFilter.provider);
  }
  
  // Sort
  filtered.sort((a, b) => {
    let aVal, bVal;
    switch (currentSort.field) {
      case 'chapter':
        aVal = a.chapter_base * 100 + (a.chapter_variant || 0);
        bVal = b.chapter_base * 100 + (b.chapter_variant || 0);
        return currentSort.direction === 'desc' ? bVal - aVal : aVal - bVal;
      case 'status':
        aVal = a.download_status;
        bVal = b.download_status;
        break;
      case 'tier':
        aVal = a.tier || 4;
        bVal = b.tier || 4;
        break;
      case 'released':
        aVal = new Date(a.released_at || 0).getTime();
        bVal = new Date(b.released_at || 0).getTime();
        break;
      default:
        return 0;
    }
    if (aVal < bVal) return currentSort.direction === 'asc' ? -1 : 1;
    if (aVal > bVal) return currentSort.direction === 'asc' ? 1 : -1;
    return 0;
  });
  
  return filtered;
}

// Build chapter table rows HTML from a flat chapters array.
// Groups by base number, separates extras, handles split variants.
function buildChapterRows(mangaId, chapters) {
  const baseMap = new Map();
  for (const ch of chapters) {
    if (!baseMap.has(ch.chapter_base)) baseMap.set(ch.chapter_base, new Map());
    const varMap = baseMap.get(ch.chapter_base);
    if (!varMap.has(ch.chapter_variant)) varMap.set(ch.chapter_variant, []);
    varMap.get(ch.chapter_variant).push(ch);
  }

  const sortedBases = [...baseMap.keys()].sort((a, b) => b - a);
  let rows = '';

  for (const base of sortedBases) {
    const varMap = baseMap.get(base);

    const extrasByVariant = new Map();
    for (const [variant, chs] of varMap) {
      const extraChs = chs.filter(ch => ch.is_extra);
      if (extraChs.length > 0) extrasByVariant.set(variant, extraChs);
    }

    const v0rows = (varMap.get(0) || []).filter(ch => !ch.is_extra);
    const v0canonical = v0rows.find(ch => ch.is_canonical) || null;
    const v0alts = v0rows.filter(ch => !ch.is_canonical).sort((a, b) => (a.tier || 4) - (b.tier || 4));

    const splitParts = [...varMap.keys()]
      .filter(v => v > 0)
      .sort((a, b) => b - a)
      .map(v => {
        const vrows = varMap.get(v).filter(ch => !ch.is_extra);
        return {
          canonical: vrows.find(ch => ch.is_canonical) || null,
          alts: vrows.filter(ch => !ch.is_canonical).sort((a, b) => (a.tier || 4) - (b.tier || 4)),
        };
      });

    let mainCh = v0canonical;
    let effectiveV0alts = v0alts;
    if (!mainCh) {
      if (v0alts.length > 0) {
        mainCh = v0alts[0];
        effectiveV0alts = v0alts.slice(1);
      }
    }

    const sortedExtraVariants = [...extrasByVariant.keys()].sort((a, b) => b - a);
    for (const v of sortedExtraVariants) {
      const vChs = extrasByVariant.get(v);
      const canonical = vChs.find(ch => ch.is_canonical) || vChs[0];
      const alts = vChs.filter(ch => ch !== canonical).sort((a, b) => (a.tier || 4) - (b.tier || 4));
      rows += chapterGroupHtml(mangaId, base, canonical, alts, []);
    }

    if (mainCh) {
      rows += chapterGroupHtml(mangaId, base, mainCh, effectiveV0alts, splitParts);
    } else {
      for (const sp of splitParts) {
        const spMain = sp.canonical || sp.alts[0];
        if (spMain) {
          const spAlts = sp.canonical ? sp.alts : sp.alts.slice(1);
          rows += chapterGroupHtml(mangaId, base, spMain, spAlts, []);
        }
      }
    }
  }
  return rows;
}

// Patch a canonical chapter entry in the cache and re-render without fetching.
function patchCachedChapter(base, variant, fields) {
  const idx = chapterDataCache.findIndex(
    ch => ch.chapter_base == base && ch.chapter_variant == variant && ch.is_canonical
  );
  if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], ...fields };
  renderFilteredChapters(filterAndSortChapters(chapterDataCache));
}

export async function loadChapters(mangaId) {
  const el = document.getElementById('chapters-list');
  if (!el) return;
  
  // Save scroll position before updating content to prevent scroll jump
  const savedScrollY = window.scrollY;
  
  // Save current content to prevent height collapse
  const originalContent = el.innerHTML;
  el.innerHTML = '<div id="chapters-loading-overlay" style="min-height:50px;padding:1rem;text-align:center;background:var(--bg-secondary)">Loading...</div>' + originalContent;
  
  try {
    const chapters = await mangaApi.chapters(mangaId);
    chapterDataCache = chapters; // Cache for filtering
    chaptersEverLoaded = true; // Mark that we've loaded chapters at least once
    
    if (chapters.length === 0) {
      el.innerHTML = `
        <div class="banner banner-info" style="margin: 1rem 0; padding: 1rem; border-radius: 8px; background: var(--bg-secondary); border: 1px solid var(--border-color);">
          <h4 style="margin: 0 0 0.5rem 0;">No chapters yet!</h4>
          <p style="margin: 0 0 0.75rem 0; color: var(--text-muted);">To get started, you'll need to:</p>
          <ol style="margin: 0; padding-left: 1.25rem; color: var(--text-muted);">
            <li>Enable/Disable providers for this series</li>
            <li>Enable/Disable aliases (alternative titles)</li>
            <li>Run 'Search All Providers' to discover chapters</li>
          </ol>
          <p style="margin: 0.75rem 0 0 0; font-size: 0.875rem; color: var(--text-muted);">
            Tip: More aliases = slower searches. Each one is tried on every provider, so only include the best. (I personally keep 3)
          </p>
        </div>
      `;
      // Restore scroll position
      window.scrollTo(0, savedScrollY);
      return;
    }

    const rows = buildChapterRows(mangaId, chapters);

    // Build filter bar
    const sortIndicator = (field) => {
      if (currentSort.field !== field) return '↕';
      return currentSort.direction === 'desc' ? '↓' : '↑';
    };

    // Get unique providers for filter chips
    const uniqueProviders = getUniqueProviders();
    const providerFilterHtml = buildProviderChipsHtml(uniqueProviders);

    el.innerHTML = `
      <div class="table-filter-bar">
        <input type="text" class="search-input" placeholder="Search chapters..." value="${escape(currentFilter.search)}" oninput="filterChapters(this.value)">
        <select class="sort-select" onchange="sortChapters(this.value)">
          <option value="chapter-desc" ${currentSort.field === 'chapter' && currentSort.direction === 'desc' ? 'selected' : ''}>Newest first</option>
          <option value="chapter-asc" ${currentSort.field === 'chapter' && currentSort.direction === 'asc' ? 'selected' : ''}>Oldest first</option>
          <option value="released-desc" ${currentSort.field === 'released' && currentSort.direction === 'desc' ? 'selected' : ''}>Recently released</option>
          <option value="released-asc" ${currentSort.field === 'released' && currentSort.direction === 'asc' ? 'selected' : ''}>Oldest released</option>
          <option value="tier-asc" ${currentSort.field === 'tier' ? 'selected' : ''}>Best score first</option>
        </select>
        <div class="filter-chips">
          <span class="filter-chip ${currentFilter.status === '' ? 'active' : ''}" onclick="filterByStatus('')">All</span>
          <span class="filter-chip ${currentFilter.status === 'Missing' ? 'active' : ''}" onclick="filterByStatus('Missing')">Missing</span>
          <span class="filter-chip ${currentFilter.status === 'Downloaded' ? 'active' : ''}" onclick="filterByStatus('Downloaded')">Downloaded</span>
          <span class="filter-chip ${currentFilter.status === 'Queued' ? 'active' : ''}" onclick="filterByStatus('Queued')">Queued</span>
          <span class="filter-chip ${currentFilter.status === 'Failed' ? 'active' : ''}" onclick="filterByStatus('Failed')">Failed</span>
        </div>
        ${providerFilterHtml}
      </div>
      ${buildChapterOverview(chapters)}
      <div class="chapters-table">
        <table>
          <thead>
            <tr>
              <th style="width:30px"><input type="checkbox" title="Select all" onchange="toggleSelectAll(this.checked)"></th>
              <th>Chapter </th>
              <th>Scanlator</th>
              <th title="Scanlator tier: Official (verified release), Trusted (added by you), Unknown, or No Group">Score</th>
              <th>Provider</th>
              <th><iconify-icon icon="mdi:tray-download" width="24" height="24"></iconify-icon></th>
              <th>Released</th>
              <th>Scraped</th>
              <th></th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    `;
    
    // Restore scroll position after content update to prevent scroll jump
    window.scrollTo(0, savedScrollY);
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
    // Restore scroll position on error too
    window.scrollTo(0, savedScrollY);
  }
}

// Filter functions
window.filterChapters = function(search) {
  currentFilter.search = search;
  // Re-render with current data
  const filtered = filterAndSortChapters(chapterDataCache);
  renderFilteredChapters(filtered);
};

window.filterByStatus = function(status) {
  currentFilter.status = status;
  const filtered = filterAndSortChapters(chapterDataCache);
  renderFilteredChapters(filtered);
};

window.filterByProvider = function(provider) {
  // Toggle off if already selected
  if (currentFilter.provider === provider) {
    currentFilter.provider = '';
  } else {
    currentFilter.provider = provider;
  }
  const filtered = filterAndSortChapters(chapterDataCache);
  renderFilteredChapters(filtered);
};

window.sortChapters = function(value) {
  const [field, direction] = value.split('-');
  currentSort = { field, direction };
  const filtered = filterAndSortChapters(chapterDataCache);
  renderFilteredChapters(filtered);
};

// Get unique providers from chapter data for filter chips
function getUniqueProviders() {
  const providers = new Set();
  for (const ch of chapterDataCache) {
    if (ch.provider_name) {
      providers.add(ch.provider_name);
    }
  }
  return [...providers].sort();
}

// Build the filter chips HTML for providers
function buildProviderChipsHtml(uniqueProviders) {
  if (uniqueProviders.length === 0) return '';
  return `<span class="filter-separator">|</span>
          <span class="filter-chip ${currentFilter.provider === '' ? 'active' : ''}" onclick="filterByProvider('')">All</span>
          ${uniqueProviders.map(p => `<span class="filter-chip ${currentFilter.provider === p ? 'active' : ''}" onclick="filterByProvider('${escape(p)}')">${escape(p)}</span>`).join('')}`;
}

function renderFilteredChapters(filteredChapters) {
  const el = document.getElementById('chapters-list');
  if (!el || !currentMangaId) return;
  
  // Get unique providers for filter chips
  const uniqueProviders = getUniqueProviders();
  
  // Check if filter bar already exists in DOM
  const existingFilterBar = el.querySelector('.table-filter-bar');
  
  if (filteredChapters.length === 0) {
    // Update existing filter bar or create new one
    if (existingFilterBar) {
      // Update the chips and input without destroying them
      const searchInput = existingFilterBar.querySelector('.search-input');
      const sortSelect = existingFilterBar.querySelector('.sort-select');
      
      if (searchInput) searchInput.value = currentFilter.search;
      if (sortSelect) sortSelect.value = `${currentSort.field}-${currentSort.direction}`;
      
      // Update status chips (first 5 filter-chip elements are always status chips)
      const allFilterChips = existingFilterBar.querySelectorAll('.filter-chip');
      const statuses = ['', 'Missing', 'Downloaded', 'Queued', 'Failed'];
      for (let i = 0; i < Math.min(5, allFilterChips.length); i++) {
        allFilterChips[i].classList.toggle('active', currentFilter.status === statuses[i]);
      }
      
      // Check if provider filter already exists
      const existingSeparator = existingFilterBar.querySelector('.filter-separator');
      if (!existingSeparator && uniqueProviders.length > 0) {
        // Add provider chips after status chips
        const providerHtml = buildProviderChipsHtml(uniqueProviders);
        const filterChipsDivs = existingFilterBar.querySelectorAll('.filter-chips');
        const lastFilterChips = filterChipsDivs[filterChipsDivs.length - 1];
        if (lastFilterChips) {
          lastFilterChips.insertAdjacentHTML('afterend', providerHtml);
        }
      }
    }
    
    // Show empty message
    let emptyMsg = el.querySelector('.empty-message');
    if (!emptyMsg) {
      emptyMsg = document.createElement('p');
      emptyMsg.className = 'empty-message';
      emptyMsg.style.cssText = 'padding:1rem;text-align:center;color:var(--text-muted)';
      emptyMsg.textContent = 'No chapters match your filters.';
      el.appendChild(emptyMsg);
    }
    
    // Hide table
    const tableContainer = el.querySelector('.chapters-table');
    if (tableContainer) tableContainer.style.display = 'none';
    
    const overview = el.querySelector('.ch-overview');
    if (overview) overview.style.display = 'none';
    
    return;
  }
  
  // We have results - build the table body from filtered data
  const rows = buildChapterRows(currentMangaId, filteredChapters);

  // Update existing filter bar to preserve focus
  if (existingFilterBar) {
    const searchInput = existingFilterBar.querySelector('.search-input');
    const sortSelect = existingFilterBar.querySelector('.sort-select');
    
    if (searchInput) searchInput.value = currentFilter.search;
    if (sortSelect) sortSelect.value = `${currentSort.field}-${currentSort.direction}`;
    
    // Update status chips (first 5 filter-chip elements are always status chips)
    const allFilterChips = existingFilterBar.querySelectorAll('.filter-chip');
    const statuses = ['', 'Missing', 'Downloaded', 'Queued', 'Failed'];
    for (let i = 0; i < Math.min(5, allFilterChips.length); i++) {
      allFilterChips[i].classList.toggle('active', currentFilter.status === statuses[i]);
    }
    
    // Check if provider filter already exists (from initial load)
    const existingSeparator = existingFilterBar.querySelector('.filter-separator');
    if (!existingSeparator && uniqueProviders.length > 0) {
      // Add provider chips after status chips
      const providerHtml = buildProviderChipsHtml(uniqueProviders);
      // Find the last filter-chips div and add provider chips after it
      const filterChipsDivs = existingFilterBar.querySelectorAll('.filter-chips');
      const lastFilterChips = filterChipsDivs[filterChipsDivs.length - 1];
      if (lastFilterChips) {
        lastFilterChips.insertAdjacentHTML('afterend', providerHtml);
      }
    } else if (existingSeparator) {
      // Update active state for provider chips - they're siblings with status chips
      const allChips = existingFilterBar.querySelectorAll('.filter-chip');
      allChips.forEach(chip => {
        if (chip.classList.contains('filter-separator')) return;
        if (chip.textContent === 'All' && chip.previousElementSibling?.classList.contains('filter-separator')) {
          // This is the "All" provider chip
          chip.classList.toggle('active', currentFilter.provider === '');
        } else if (chip.previousElementSibling?.classList.contains('filter-separator') || 
                   chip.previousElementSibling?.textContent === 'All') {
          // This is a provider chip
          chip.classList.toggle('active', chip.textContent === currentFilter.provider);
        }
      });
    }
  }
  
  // Update table body only
  const tbody = el.querySelector('.chapters-table tbody');
  if (tbody) {
    tbody.innerHTML = rows;
    el.querySelector('.chapters-table').style.display = '';
  } else {
    // Table doesn't exist, create it
    const tableHtml = `
      <div class="chapters-table">
        <table>
          <thead>
            <tr>
              <th style="width:30px"><input type="checkbox" title="Select all" onchange="toggleSelectAll(this.checked)"></th>
              <th>Chapter </th>
              <th>Scanlator</th>
              <th title="Scanlator tier: Official (verified release), Trusted (added by you), Unknown, or No Group">Score</th>
              <th>Provider</th>
              <th><iconify-icon icon="mdi:tray-download" width="24" height="24"></iconify-icon></th>
              <th>Released</th>
              <th>Scraped</th>
              <th></th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>`;
    
    // Append table after filter bar
    if (existingFilterBar) {
      existingFilterBar.insertAdjacentHTML('afterend', tableHtml);
    }
  }
  
  // Show/hide overview
  const overview = el.querySelector('.ch-overview');
  if (overview) overview.style.display = '';
  
  // Remove empty message if present
  const emptyMsg = el.querySelector('.empty-message');
  if (emptyMsg) emptyMsg.remove();
}

export async function loadProviders(mangaId) {
  const el = document.getElementById('providers-list');
  if (!el) return;
  try {
    const provList = await mangaApi.providers(mangaId);
    if (provList.length === 0) {
      el.innerHTML = '<p><small>No providers found yet. Scan this manga to discover providers.</small></p>';
      return;
    }

    // Fetch per-series scores in parallel
    const scoreResults = await Promise.allSettled(
      provList.map(p => providerScores.getSeries(mangaId, p.provider_name))
    );

    const rows = provList.map((p, i) => {
      const statusClass = p.found ? 'found' : 'not-found';
      const statusText = p.found ? 'Found' : 'Not found';
      const searched = p.search_attempted_at ? relTime(p.search_attempted_at) : 'never';
      const synced = p.last_synced_at ? relTime(p.last_synced_at) : 'Never';

      const scoreData = scoreResults[i].status === 'fulfilled' ? scoreResults[i].value : null;
      const currentScore = scoreData?.score ?? 0;
      const isEnabled = scoreData?.enabled ?? true;

      const linkBtn = p.provider_url
        ? `<button onclick="window.open('${escape(p.provider_url)}', '_blank')">Open</button>`
        : '';

      const enableToggle = `<label title="${isEnabled ? 'Enabled: click to disable (only for checking new chapters)' : 'Disabled — click to enable'}">
        <input type="checkbox" ${isEnabled ? 'checked' : ''} onchange="setProviderEnabled('${mangaId}', '${escape(p.provider_name)}', this.checked)">
        ${isEnabled ? 'Enabled' : 'Disabled'}
      </label>`;

      const scoreInput = `<input type="number" class="score-input" value="${currentScore}" min="-100" max="100"
        title="Per-series score override for ${escape(p.provider_name)}"
        data-manga="${mangaId}" data-provider="${escape(p.provider_name)}"
        onchange="setSeriesScore('${mangaId}', '${escape(p.provider_name)}', this.value)"
        onblur="setSeriesScore('${mangaId}', '${escape(p.provider_name)}', this.value)">`;

      const pickBtn = `<button class="btn btn-xs btn-ghost" onclick="pickProvider('${mangaId}', '${escape(p.provider_name)}')" title="Search this provider and pick the correct match">Pick</button>`;

      return `<tr>
        <td><span class="provider-bubble">
          <span class="status-dot ${statusClass}"></span>
          ${escape(p.provider_name)}
          <span class="actions">${linkBtn}</span>
        </span></td>
        <td>${statusText}</td>
        <td><small>${synced}</small></td>
        <td><small>searched: ${searched}</small></td>
        <td>${enableToggle}</td>
        <td>${scoreInput}</td>
        <td>${pickBtn}</td>
      </tr>`;
    }).join('');

    el.innerHTML = `<div class="chapters-table">
      <table>
        <thead>
          <tr><th>Provider</th><th>Status</th><th>Last Synced</th><th>Searched</th><th>Enabled</th><th>Score</th><th></th></tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    </div>`;
  } catch(e) {
    el.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
}

window.setProviderEnabled = async function(mangaId, providerName, enabled) {
  try {
    const current = await providerScores.getSeries(mangaId, providerName);
    await providerScores.setSeries(mangaId, providerName, current?.score ?? 0, enabled);
    showToast(`${providerName} ${enabled ? 'enabled' : 'disabled'}`);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.setSeriesScore = async function(mangaId, providerName, value) {
  const score = parseInt(value, 10);
  if (isNaN(score)) return;
  try {
    const current = await providerScores.getSeries(mangaId, providerName);
    await providerScores.setSeries(mangaId, providerName, score, current?.enabled ?? true);
  } catch(e) {
    showToast('Score save failed: ' + e.message, 'error');
  }
};

window.pickProvider = async function(mangaId, providerName) {
  // Create and show modal immediately with loading state
  const existingModal = document.getElementById('pick-provider-modal');
  if (existingModal) existingModal.remove();

  const modal = document.createElement('div');
  modal.id = 'pick-provider-modal';
  modal.className = 'modal-overlay';
  modal.innerHTML = `
    <div class="modal-box">
      <h3 class="modal-title">Pick match for <strong>${escape(providerName)}</strong></h3>
      <div id="pick-modal-results"><p class="modal-loading">Searching…</p></div>
      <div class="modal-custom-url">
        <label>Custom URL</label>
        <div class="modal-custom-url-row">
          <input type="url" id="pick-custom-url" placeholder="https://..." class="input input-sm">
          <button class="btn btn-sm btn-primary" onclick="pickProviderSaveCustom('${escape(mangaId)}', '${escape(providerName)}')">Save</button>
        </div>
      </div>
      <div class="modal-footer">
        <button class="btn btn-sm btn-ghost" onclick="document.getElementById('pick-provider-modal').remove()">Cancel</button>
      </div>
    </div>
  `;
  document.body.appendChild(modal);

  // Close on backdrop click
  modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });

  // Fetch candidates
  try {
    const candidates = await mangaApi.providerCandidates(mangaId, providerName);
    const resultsEl = document.getElementById('pick-modal-results');
    if (!resultsEl) return;

    if (candidates.length === 0) {
      resultsEl.innerHTML = '<p class="modal-empty">No results found on this provider.</p>';
      return;
    }

    const rows = candidates.map(c => {
      const pct = Math.round(c.score * 100);
      const scoreClass = pct >= 85 ? 'score-good' : pct >= 60 ? 'score-mid' : 'score-low';
      const coverHtml = c.cover
        ? `<img class="pick-cover" src="${escape(c.cover)}" alt="" loading="lazy">`
        : `<div class="pick-cover pick-cover-empty"></div>`;
      return `<div class="pick-result-row">
        ${coverHtml}
        <div class="pick-result-info">
          <a class="pick-result-title" href="${escape(c.url)}" target="_blank" rel="noopener">${escape(c.title)}</a>
          <span class="pick-result-url">${escape(c.url)}</span>
        </div>
        <span class="pick-score ${scoreClass}">${pct}%</span>
        <button class="btn btn-xs btn-primary" onclick="pickProviderSelect('${escape(mangaId)}', '${escape(providerName)}', '${escape(c.url)}')">Select</button>
      </div>`;
    }).join('');

    resultsEl.innerHTML = `<div class="pick-results-list">${rows}</div>`;
  } catch(e) {
    const resultsEl = document.getElementById('pick-modal-results');
    if (resultsEl) resultsEl.innerHTML = `<p class="error">Search failed: ${escape(e.message)}</p>`;
  }
};

window.pickProviderSelect = async function(mangaId, providerName, url) {
  try {
    await mangaApi.setProviderUrl(mangaId, providerName, url);
    document.getElementById('pick-provider-modal')?.remove();
    showToast(`${providerName} → matched`);
    loadProviders(mangaId);
  } catch(e) {
    showToast('Failed to save: ' + e.message, 'error');
  }
};

window.pickProviderSaveCustom = async function(mangaId, providerName) {
  const url = document.getElementById('pick-custom-url')?.value?.trim();
  if (!url) { showToast('Please enter a URL', 'error'); return; }
  await window.pickProviderSelect(mangaId, providerName, url);
};

// Trusted group functions for bubble UI
window.addTrustedFromBubble = async function(groupName) {
  try {
    await trustedGroups.add(groupName);
    trustedGroupsCache.push(groupName);
    showToast(`"${groupName}" added to trusted`);
    loadChapters(currentMangaId);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.removeTrustedFromBubble = async function(groupName) {
  if (!confirm(`Remove "${groupName}" from trusted scanlators?`)) return;
  try {
    await trustedGroups.remove(groupName);
    trustedGroupsCache = trustedGroupsCache.filter(g => g !== groupName);
    showToast(`"${groupName}" removed from trusted`);
    loadChapters(currentMangaId);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

// Action handlers
window.doScan = async function(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing scan...';
  try {
    await mangaApi.scan(mangaId);
    if (statusEl) statusEl.textContent = ' Scan queued!';
    showToast('Scan queued');
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
    showToast(e.message, 'error');
  }
};

window.doCheckNew = async function(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing chapter check...';
  try {
    await mangaApi.checkNew(mangaId);
    if (statusEl) statusEl.textContent = ' Chapter check queued!';
    showToast('Chapter check queued');
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
};

window.doScanDisk = async function(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing disk scan...';
  try {
    await mangaApi.scanDisk(mangaId);
    if (statusEl) statusEl.textContent = ' Disk scan queued!';
    showToast('Disk scan queued');
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
};

window.doRefreshMetadata = async function(mangaId) {
  const statusEl = document.getElementById('scan-status');
  if (statusEl) statusEl.textContent = ' Queueing metadata refresh...';
  try {
    await mangaApi.refresh(mangaId);
    if (statusEl) statusEl.textContent = ' Metadata refresh queued!';
    showToast('Metadata refresh queued');
  } catch(e) {
    if (statusEl) statusEl.textContent = ` Error: ${escape(e.message)}`;
  }
};

window.doDownload = async function(mangaId, base, variant) {
  try {
    await mangaApi.downloadChapter(mangaId, base, variant);
    patchCachedChapter(base, variant, { download_status: 'Queued' });
    showToast('Download queued');
  } catch(e) {
    showToast('Download error: ' + e.message, 'error');
  }
};

window.doResetChapter = async function(mangaId, base, variant) {
  try {
    await mangaApi.resetChapter(mangaId, base, variant);
    patchCachedChapter(base, variant, { download_status: 'Missing' });
    showToast('Chapter reset');
  } catch(e) {
    showToast('Reset failed: ' + e.message, 'error');
  }
};

window.doDeleteChapter = async function(mangaId, base, variant) {
  if (!confirm('Delete this chapter? This will also remove downloaded files from disk.')) return;
  try {
    await mangaApi.deleteChapter(mangaId, base, variant);
    loadChapters(mangaId);
    showToast('Chapter deleted');
  } catch(e) {
    showToast('Delete error: ' + e.message, 'error');
  }
};

window.showDeleteSeriesModal = function(mangaId, title) {
  const existingModal = document.getElementById('delete-series-modal');
  if (existingModal) existingModal.remove();

  const modal = document.createElement('div');
  modal.id = 'delete-series-modal';
  modal.className = 'modal-overlay';
  modal.innerHTML = `
    <div class="modal-box">
      <h3 class="modal-title">Delete Series</h3>
      <div class="delete-series-modal">
        <p>Choose how you want to delete <strong>${escape(title || 'this series')}</strong>.</p>
        <p>You can remove only the database records, or remove the database records and delete the whole series folder on disk.</p>
        <p class="delete-series-warning">Deleting files will remove the entire series folder, including any extra or manual files inside it.</p>
      </div>
      <div class="modal-footer delete-series-actions">
        <button class="btn btn-sm btn-danger" data-delete-mode="db" onclick="confirmDeleteSeries('${escape(mangaId)}', false)">Delete from DB</button>
        <button class="btn btn-sm btn-danger" data-delete-mode="files" onclick="confirmDeleteSeries('${escape(mangaId)}', true)">Delete DB + Files</button>
        <button class="btn btn-sm btn-ghost" data-delete-mode="cancel" onclick="document.getElementById('delete-series-modal')?.remove()">Cancel</button>
      </div>
    </div>
  `;
  document.body.appendChild(modal);

  modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });
};

window.confirmDeleteSeries = async function(mangaId, deleteFiles) {
  const modal = document.getElementById('delete-series-modal');
  if (!modal) return;

  const buttons = modal.querySelectorAll('button');
  buttons.forEach(btn => { btn.disabled = true; });

  try {
    await mangaApi.delete(mangaId, { delete_files: deleteFiles });
    modal.remove();
    showToast(deleteFiles ? 'Series and files deleted' : 'Series deleted from database');
    navigate('/library');
  } catch(e) {
    buttons.forEach(btn => { btn.disabled = false; });
    showToast('Delete failed: ' + e.message, 'error');
  }
};

window.doToggleExtra = async function(mangaId, base, variant) {
  try {
    await mangaApi.toggleExtra(mangaId, base, variant);
    const ch = chapterDataCache.find(c => c.chapter_base == base && c.chapter_variant == variant && c.is_canonical);
    if (ch) patchCachedChapter(base, variant, { is_extra: !ch.is_extra });
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.doSetCanonical = async function(mangaId, base, variant, chapterId) {
  try {
    await mangaApi.setCanonical(mangaId, base, variant, chapterId);
    loadChapters(mangaId);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.toggleMonitored = async function(mangaId, checked) {
  try {
    await mangaApi.update(mangaId, { monitored: checked });
    // Update the visual styling
    const label = document.querySelector('.monitored-toggle');
    if (label) {
      label.classList.toggle('monitored', checked);
      label.title = checked ? 'Monitored - click to unmonitor' : 'Not monitored - click to monitor';
      
      const iconName = checked ? 'bookmark' : 'bookmark-outline';
      const fullIconName = `mdi:${iconName}`;
      
      // Replace both icon elements with fresh ones to ensure proper re-rendering
      const iconSpan = label.querySelector('.monitored-icon');
      const iconifyEl = label.querySelector('iconify-icon');
      
      if (iconSpan) {
        iconSpan.setAttribute('data-icon', fullIconName);
      }
      
      if (iconifyEl) {
        // The most reliable way to update iconify-icon is to replace the element
        const newIconifyEl = document.createElement('iconify-icon');
        newIconifyEl.setAttribute('icon', fullIconName);
        newIconifyEl.setAttribute('width', '24');
        newIconifyEl.setAttribute('height', '24');
        iconifyEl.replaceWith(newIconifyEl);
      }
    }
  } catch(e) {
    showToast('Error updating monitored: ' + e.message, 'error');
  }
};

// Toggle synopsis visibility
window.toggleSynopsis = function() {
  const content = document.getElementById('synopsis-content');
  const btn = document.querySelector('.synopsis-toggle');
  const icon = btn?.querySelector('.synopsis-icon');
  const text = btn?.querySelector('.synopsis-text');
  
  if (content && btn) {
    const isHidden = content.classList.contains('hidden');
    content.classList.toggle('hidden');
    
    if (icon && text) {
      if (isHidden) {
        // Expand: show chevron-up and "Hide Synopsis"
        const newIcon = document.createElement('iconify-icon');
        newIcon.setAttribute('icon', 'mdi-chevron-up');
        newIcon.setAttribute('width', '24');
        newIcon.setAttribute('height', '24');
        newIcon.classList.add('synopsis-icon');
        icon.replaceWith(newIcon);
        text.textContent = 'Hide Synopsis';
      } else {
        // Collapse: show chevron-down and "Show Synopsis"
        const newIcon = document.createElement('iconify-icon');
        newIcon.setAttribute('icon', 'mdi-chevron-down');
        newIcon.setAttribute('width', '24');
        newIcon.setAttribute('height', '24');
        newIcon.classList.add('synopsis-icon');
        icon.replaceWith(newIcon);
        text.textContent = 'Show Synopsis';
      }
    }
  }
};

window.toggleSelectAll = function(checked) {
  document.querySelectorAll('.ch-checkbox').forEach(cb => cb.checked = checked);
};

window.doDownloadSelected = async function(mangaId) {
  const checked = Array.from(document.querySelectorAll('.ch-checkbox:checked'));
  if (checked.length === 0) { showToast('Select at least one chapter.', 'warning'); return; }
  let count = 0, errors = 0;
  for (const cb of checked) {
    try {
      await mangaApi.downloadChapter(mangaId, cb.dataset.base, cb.dataset.variant);
      count++;
    } catch(e) { errors++; }
  }
  for (const cb of checked) {
    const idx = chapterDataCache.findIndex(ch => ch.chapter_base == cb.dataset.base && ch.chapter_variant == cb.dataset.variant && ch.is_canonical);
    if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], download_status: 'Queued' };
  }
  renderFilteredChapters(filterAndSortChapters(chapterDataCache));
  if (count > 0) {
    showToast(`Queued ${count} download${count === 1 ? '' : 's'}${errors > 0 ? `, ${errors} failed` : ''}`);
  } else {
    showToast(`${errors} download${errors === 1 ? '' : 's'} failed`, 'error');
  }
};

window.doDownloadAllMissing = async function(mangaId) {
  const cbs = Array.from(document.querySelectorAll('.ch-checkbox'));
  if (cbs.length === 0) { showToast('No missing chapters to download.', 'warning'); return; }
  let count = 0, errors = 0;
  for (const cb of cbs) {
    try {
      await mangaApi.downloadChapter(mangaId, cb.dataset.base, cb.dataset.variant);
      count++;
    } catch(e) { errors++; }
  }
  for (const cb of cbs) {
    const idx = chapterDataCache.findIndex(ch => ch.chapter_base == cb.dataset.base && ch.chapter_variant == cb.dataset.variant && ch.is_canonical);
    if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], download_status: 'Queued' };
  }
  renderFilteredChapters(filterAndSortChapters(chapterDataCache));
  if (errors > 0) {
    showToast(`Queued ${count}, ${errors} failed`, 'error');
  } else {
    showToast(`Queued ${count} chapter${count === 1 ? '' : 's'}`);
  }
};

// ---------------------------------------------------------------------------
// Synonym management functions
// ---------------------------------------------------------------------------

// Render synonyms with source indicators and remove buttons
function renderSynonyms(synonyms) {
  if (!synonyms || synonyms.length === 0) return '';
  
  return synonyms.map(syn => {
    const isManual = syn.source === 'Manual';
    const isHidden = syn.hidden;
    
    // Build tooltip based on hidden state and filter reason
    let title;
    if (isHidden) {
      if (syn.filter_reason) {
        title = `Hidden: ${syn.filter_reason}`;
      } else {
        title = 'Hidden from search';
      }
    } else {
      title = isManual ? 'Manual synonym - always used for search' : 'AniList synonym - click to hide from search';
    }
    
    const badgeClass = isHidden ? 'badge badge-neutral opacity-50 line-through synonym-pill' : 'badge badge-neutral synonym-pill';

    return `<span class="${badgeClass}" title="${title}" data-title="${escape(syn.title)}" data-manual="${isManual}" data-hidden="${isHidden}">${escape(syn.title)}</span>`;
  }).join(' ');
}

// Add a new synonym — inserts an inline input row instead of using prompt()
window.addSynonym = function() {
  if (document.getElementById('add-synonym-row')) return;
  const addBtn = document.querySelector('[onclick="addSynonym()"]');
  if (!addBtn) return;

  const row = document.createElement('span');
  row.id = 'add-synonym-row';
  row.style.cssText = 'display:inline-flex;gap:4px;align-items:center;margin-left:4px';
  row.innerHTML = `<input id="add-synonym-input" type="text" class="input input-xs" placeholder="New alias…" style="width:10rem">` +
    `<button class="btn btn-xs btn-primary" onclick="confirmAddSynonym()">Add</button>` +
    `<button class="btn btn-xs btn-ghost" onclick="document.getElementById('add-synonym-row')?.remove()">✕</button>`;
  addBtn.parentElement.insertBefore(row, addBtn);

  const input = document.getElementById('add-synonym-input');
  input.focus();
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') confirmAddSynonym();
    if (e.key === 'Escape') row.remove();
  });
};

// Refresh only the synonym list in-place (no full page reload)
async function refreshSynonyms(mangaId) {
  try {
    const m = await mangaApi.get(mangaId);
    const meta = m.metadata ?? {};
    const synonyms = meta.other_titles || [];
    const el = document.getElementById('synonyms-list');
    if (!el) return;
    el.innerHTML = renderSynonyms(synonyms);
  } catch(e) {
    showToast('Error refreshing synonyms: ' + e.message, 'error');
  }
}

window.confirmAddSynonym = async function() {
  const input = document.getElementById('add-synonym-input');
  const title = input?.value?.trim();
  if (!title) return;
  try {
    await mangaApi.updateSynonyms(currentMangaId, { add: [title] });
    showToast('Synonym added');
    document.getElementById('add-synonym-row')?.remove();
    refreshSynonyms(currentMangaId);
  } catch(e) {
    showToast('Error adding synonym: ' + e.message, 'error');
  }
};

// Remove a synonym (unhide for AniList if hidden, delete for Manual)
window.removeSynonym = async function(title, isManual, isHidden) {
  if (isHidden) {
    // Already hidden - unhide it
    try {
      await mangaApi.updateSynonyms(currentMangaId, {
        remove: [title]
      });
      showToast('Synonym shown in search');
    } catch(e) {
      showToast('Error showing synonym: ' + e.message, 'error');
      return;
    }
  } else {
    // Not hidden - hide/remove it
    try {
      if (isManual) {
        // For manual synonyms, remove entirely
        await mangaApi.updateSynonyms(currentMangaId, {
          remove: [title]
        });
        showToast('Synonym removed');
      } else {
        // For AniList synonyms, just hide
        await mangaApi.updateSynonyms(currentMangaId, {
          hide: [title]
        });
        showToast('Synonym hidden from search');
      }
    } catch(e) {
      showToast('Error hiding synonym: ' + e.message, 'error');
      return;
    }
  }
  // Refresh synonyms in-place (no full page reload)
  refreshSynonyms(currentMangaId);
};

// Cover upload modal
window.showCoverUpload = function(mangaId) {
  const existingModal = document.getElementById('cover-upload-modal');
  if (existingModal) existingModal.remove();

  const modal = document.createElement('div');
  modal.id = 'cover-upload-modal';
  modal.className = 'modal-overlay';
  modal.innerHTML = `
    <div class="modal-box">
      <h3 class="modal-title">Change Cover</h3>
      <div class="cover-upload-modal">
        <label>Download from URL</label>
        <div class="cover-url-row">
          <input type="url" id="cover-url-input" placeholder="https://example.com/cover.jpg" class="input input-sm">
          <button class="btn btn-sm btn-primary" onclick="doCoverUploadUrl('${escape(mangaId)}')">Download</button>
        </div>
        <div class="cover-upload-divider">— or —</div>
        <label>Upload from device</label>
        <input type="file" id="cover-file-input" class="cover-file-input" accept="image/jpeg,image/png,image/webp">
        <button class="btn btn-sm" onclick="document.getElementById('cover-file-input').click()">
          <iconify-icon icon="mdi:upload" width="16" height="16"></iconify-icon>
          Choose File
        </button>
        <span id="cover-file-name" style="font-size:0.8rem;color:var(--text-muted);margin-left:0.5rem"></span>
      </div>
      <div class="modal-footer">
        <button class="btn btn-sm btn-ghost" onclick="document.getElementById('cover-upload-modal').remove()">Cancel</button>
      </div>
    </div>
  `;
  document.body.appendChild(modal);

  // Close on backdrop click
  modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });

  // File input change handler
  const fileInput = document.getElementById('cover-file-input');
  fileInput.addEventListener('change', async () => {
    const file = fileInput.files[0];
    if (!file) return;
    document.getElementById('cover-file-name').textContent = file.name;
    try {
      showToast('Uploading cover...');
      await coverApi.uploadFile(mangaId, file);
      document.getElementById('cover-upload-modal')?.remove();
      showToast('Cover updated');
      viewSeries(mangaId);
    } catch(e) {
      showToast('Upload failed: ' + e.message, 'error');
    }
  });
};

window.doCoverUploadUrl = async function(mangaId) {
  const url = document.getElementById('cover-url-input')?.value?.trim();
  if (!url) { showToast('Please enter a URL', 'error'); return; }
  try {
    showToast('Downloading cover...');
    await coverApi.uploadUrl(mangaId, url);
    document.getElementById('cover-upload-modal')?.remove();
    showToast('Cover updated');
    viewSeries(mangaId);
  } catch(e) {
    showToast('Download failed: ' + e.message, 'error');
  }
};

window.viewSeries = viewSeries;

// Event delegation for synonym pills — click anywhere on pill to toggle
document.addEventListener('click', (e) => {
  const pill = e.target.closest('.synonym-pill');
  if (!pill) return;

  e.stopPropagation();
  const title = pill.dataset.title;
  const isManual = pill.dataset.manual === 'true';
  const isHidden = pill.dataset.hidden === 'true';

  if (title && currentMangaId) {
    window.removeSynonym(title, isManual, isHidden);
  }
});
