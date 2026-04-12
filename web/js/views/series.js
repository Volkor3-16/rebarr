// Series detail view - manga info + chapters + live task status

import { manga as mangaApi, tasks, providerSettings, coverApi, qualityRules } from '../api.js';
import { render, setPoll, navigate } from '../router.js';
import { escape, relTime, statusBadge, taskBadge, skeleton, showToast, truncateMiddle, formatFileSize, renderTaskProgress } from '../utils.js';

let currentMangaId = null;
let chapterDataCache = [];
let chapterSlotsCache = [];
let allChapterGroupsCache = [];
let visibleChapterGroupsCache = [];
let providersCache = []; // Cache provider names for filtering
let currentSort = { field: 'chapter', direction: 'desc' };
let currentFilter = { search: '', status: '', provider: '', extrasOnly: false };
let lastCheckedIdx = -1;
let intersectionObserver = null;
let hoveredChapterRow = null;
let selectedSlotKeys = new Set();
let expandedSlotKeys = new Set();

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
  // Cleanup fixed UI elements from previous series visit
  document.getElementById('bulk-action-bar')?.remove();
  document.body.classList.remove('bulk-bar-active');
  render(`<div class="series">${skeleton(5)}</div>`);
  
  try {
    const m = await mangaApi.get(id);
    const meta = m.metadata ?? {};
    const year = meta.start_year ? (meta.end_year ? `${meta.start_year} - ${meta.end_year}` : `${meta.start_year} - ongoing`) : '?';
    const dl = m.downloaded_count ?? 0;
    const total = m.chapter_count != null ? m.chapter_count : '?';
    
    const thumb = m.thumbnail_url
      ? `<div class="series-cover-wrapper">
          <img class="cover-lg" src="${escape(m.thumbnail_url)}" alt="cover">
          <div class="cover-hover-overlay">
            <button class="cover-action-btn" onclick="showCoverUpload('${m.id}')">
              <iconify-icon icon="mdi:pencil" width="16" height="16"></iconify-icon>
              Change
            </button>
          </div>
        </div>`
      : `<div class="series-cover-wrapper">
          <img class="cover-lg" src="/web/img/no-cover.svg" alt="No cover">
          <div class="cover-hover-overlay">
            <label class="cover-action-btn">
              <iconify-icon icon="mdi:upload" width="16" height="16"></iconify-icon>
              Upload
              <input type="file" accept="image/jpeg,image/png,image/webp" style="display:none"
                     onchange="doDirectCoverUpload(event, '${m.id}')">
            </label>
            <button class="cover-action-btn" onclick="showCoverUpload('${m.id}')">
              <iconify-icon icon="mdi:link" width="16" height="16"></iconify-icon>
              From URL
            </button>
          </div>
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
      
      <div class="providers-header">
        <h3 style="margin:0">Providers</h3>
        <button class="providers-chevron-btn" id="providers-chevron"
                onclick="toggleProvidersSection()" title="Toggle providers">
          <iconify-icon icon="mdi:chevron-down" width="20" height="20"></iconify-icon>
        </button>
      </div>
      <div class="providers-collapsible" id="providers-collapsible">
        <div id="providers-list"><p>Loading...</p></div>
      </div>
      
      <div class="mt-3">
        <a href="/library" data-path="/library">[Back to Libraries]</a>
      </div>
    `);

    setupBulkBar(m.id);

    // Load chapters, providers, and tips, then start polling
    await Promise.all([loadChapters(m.id), loadTips()]);
    loadProviders(m.id);
    // Restore providers collapsed state from localStorage
    try {
      if (localStorage.getItem('rebarr-providers-collapsed') === '1') {
        document.getElementById('providers-collapsible')?.classList.add('collapsed');
        document.getElementById('providers-chevron')?.classList.add('collapsed');
      }
    } catch(e) {}

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

function makeSlotKey(base, variant, isExtra) {
  return `${base}:${variant}:${isExtra ? 'extra' : 'normal'}`;
}

function slotDomId(slotKey) {
  return `slot-${slotKey.replace(/[^A-Za-z0-9_-]/g, '_')}`;
}

function chapterNumberValue(ch) {
  return ch.chapter_base * 100 + (ch.chapter_variant || 0);
}

function compareRows(a, b) {
  // Simply compare chapter numbers - backend has already selected the best chapters
  const aVal = chapterNumberValue(a);
  const bVal = chapterNumberValue(b);
  return aVal - bVal;
}

function buildChapterSlots(chapters) {
  const slotMap = new Map();
  for (const ch of chapters) {
    const key = makeSlotKey(ch.chapter_base, ch.chapter_variant, !!ch.is_extra);
    if (!slotMap.has(key)) {
      slotMap.set(key, {
        key,
        chapter_base: ch.chapter_base,
        chapter_variant: ch.chapter_variant,
        is_extra: !!ch.is_extra,
        rows: [],
      });
    }
    slotMap.get(key).rows.push(ch);
  }

  return [...slotMap.values()].map(slot => {
    const rows = [...slot.rows].sort(compareRows);
    return {
      ...slot,
      rows,
      canonicalRow: rows.find(row => row.is_canonical) || null,
    };
  });
}

function rowMatchesSearch(row, search) {
  if (!search) return true;
  const chNum = `Chapter ${row.chapter_base}${row.chapter_variant > 0 ? '.' + row.chapter_variant : ''}`;
  return chNum.toLowerCase().includes(search)
    || (row.title && row.title.toLowerCase().includes(search))
    || (row.scanlator_group && row.scanlator_group.toLowerCase().includes(search))
    || (row.provider_name && row.provider_name.toLowerCase().includes(search));
}

function rowMatchesProvider(row, provider) {
  if (!provider) return true;
  return row.provider_name === provider;
}

function applySlotFilters(slots, filter = currentFilter) {
  const search = filter.search.trim().toLowerCase();

  return slots
    .filter(slot => !filter.extrasOnly || slot.is_extra)
    .map(slot => {
      const visibleRows = slot.rows.filter(row =>
        rowMatchesSearch(row, search) && rowMatchesProvider(row, filter.provider)
      );
      if (visibleRows.length === 0) return null;

      const sortedVisibleRows = [...visibleRows].sort(compareRows);
      const mainRow = sortedVisibleRows.find(row => row.is_canonical) || sortedVisibleRows[0];
      if (!mainRow) return null;
      if (filter.status && mainRow.download_status !== filter.status) return null;

      return {
        ...slot,
        visibleRows: sortedVisibleRows,
        mainRow,
        altRows: sortedVisibleRows.filter(row => row.id !== mainRow.id),
      };
    })
    .filter(Boolean);
}

function compareVisibleSlots(a, b) {
  let aVal;
  let bVal;

  switch (currentSort.field) {
    case 'chapter':
      aVal = chapterNumberValue(a.mainRow) + (a.is_extra ? 0.001 : 0);
      bVal = chapterNumberValue(b.mainRow) + (b.is_extra ? 0.001 : 0);
      return currentSort.direction === 'desc' ? bVal - aVal : aVal - bVal;
    case 'status':
      aVal = a.mainRow.download_status;
      bVal = b.mainRow.download_status;
      break;
    case 'score':
      aVal = a.mainRow.score ?? 0;
      bVal = b.mainRow.score ?? 0;
      return currentSort.direction === 'desc' ? bVal - aVal : aVal - bVal;
    case 'released':
      aVal = a.mainRow.released_at || 0;
      bVal = b.mainRow.released_at || 0;
      break;
    default:
      return 0;
  }

  if (aVal < bVal) return currentSort.direction === 'asc' ? -1 : 1;
  if (aVal > bVal) return currentSort.direction === 'asc' ? 1 : -1;

  const chapterDiff = chapterNumberValue(b.mainRow) - chapterNumberValue(a.mainRow);
  if (chapterDiff !== 0) return chapterDiff;
  return a.key.localeCompare(b.key);
}

function buildChapterGroups(slots) {
  const byBase = new Map();
  for (const slot of slots) {
    if (!byBase.has(slot.chapter_base)) byBase.set(slot.chapter_base, []);
    byBase.get(slot.chapter_base).push(slot);
  }

  const groups = [];
  for (const slotsForBase of byBase.values()) {
    const extras = slotsForBase
      .filter(slot => slot.is_extra)
      .sort(compareVisibleSlots);

    for (const slot of extras) {
      groups.push({
        key: slot.key,
        mainSlot: slot,
        mainRow: slot.mainRow,
        subRows: slot.altRows,
      });
    }

    const normalBase = slotsForBase.find(slot => !slot.is_extra && slot.chapter_variant === 0) || null;
    const splitSlots = slotsForBase
      .filter(slot => !slot.is_extra && slot.chapter_variant > 0)
      .sort((a, b) => a.chapter_variant - b.chapter_variant);

    // Use normalBase as the main layout only if it actually has a canonical row.
    // When splits are canonical (normalBase is not), each split is its own group entry.
    const normalBaseIsCanonical = normalBase?.canonicalRow != null;

    if (normalBaseIsCanonical) {
      // Full chapter is canonical — show it as main, splits as sub-rows.
      const subRows = [...normalBase.altRows];
      for (const splitSlot of splitSlots) {
        subRows.push(splitSlot.mainRow, ...splitSlot.altRows);
      }

      groups.push({
        key: normalBase.key,
        mainSlot: normalBase,
        mainRow: normalBase.mainRow,
        subRows: subRows.filter(Boolean),
      });
    } else {
      // Splits are canonical (or there's no variant-0 at all) — each split is its own entry.
      // Attach the non-canonical full chapter rows to the first split as alternatives.
      const normalBaseRows = normalBase
        ? [normalBase.mainRow, ...normalBase.altRows].filter(Boolean)
        : [];

      for (let i = 0; i < splitSlots.length; i++) {
        const splitSlot = splitSlots[i];
        const subRows = [...splitSlot.altRows];
        if (i === 0 && normalBaseRows.length > 0) {
          subRows.push(...normalBaseRows);
        }
        groups.push({
          key: splitSlot.key,
          mainSlot: splitSlot,
          mainRow: splitSlot.mainRow,
          subRows: subRows.filter(Boolean),
        });
      }

      // If there's only a non-canonical normalBase and no splits, still show it.
      if (splitSlots.length === 0 && normalBase) {
        groups.push({
          key: normalBase.key,
          mainSlot: normalBase,
          mainRow: normalBase.mainRow,
          subRows: normalBase.altRows,
        });
      }
    }
  }

  return groups.sort((a, b) => compareVisibleSlots(a.mainSlot, b.mainSlot));
}

function getVisibleSelectableGroupKeys() {
  return visibleChapterGroupsCache
    .filter(group => {
      const status = group.mainRow?.download_status;
      return status === 'Missing' || status === 'Failed';
    })
    .map(group => group.key);
}

function getGroupByKey(slotKey) {
  return allChapterGroupsCache.find(group => group.key === slotKey) || null;
}

function getUniqueProviders() {
  const providers = new Set();
  for (const slot of chapterSlotsCache) {
    for (const row of slot.rows) {
      if (row.provider_name) providers.add(row.provider_name);
    }
  }
  return [...providers].sort();
}

function hasActiveChapterFilters() {
  return !!(currentFilter.search || currentFilter.status || currentFilter.provider || currentFilter.extrasOnly);
}

function renderChapterOverview() {
  const canonical = chapterSlotsCache
    .map(slot => slot.canonicalRow)
    .filter(Boolean)
    .sort((a, b) => chapterNumberValue(a) - chapterNumberValue(b));

  if (canonical.length === 0) return '';

  const visibleKeys = new Set();
  for (const group of visibleChapterGroupsCache) {
    visibleKeys.add(group.mainSlot.key);
    for (const row of group.subRows) {
      visibleKeys.add(makeSlotKey(row.chapter_base, row.chapter_variant, !!row.is_extra));
    }
  }
  const visibleCount = visibleChapterGroupsCache.length;
  const dots = canonical.map(ch => {
    const slotKey = makeSlotKey(ch.chapter_base, ch.chapter_variant, !!ch.is_extra);
    const chNum = ch.chapter_variant === 0 ? `Chapter ${ch.chapter_base}` : `Chapter ${ch.chapter_base}.${ch.chapter_variant}`;
    const titlePart = ch.title ? ` — ${ch.title}` : '';
    const tip = `${chNum}${titlePart} (${ch.download_status})`;
    const cls = `ch-dot ch-dot-${ch.download_status.toLowerCase()}${visibleKeys.has(slotKey) ? '' : ' ch-dot-dimmed'}`;
    return `<span class="${cls}" title="${escape(tip)}" onclick="scrollToChapter('${ch.id}', '${slotKey}')"></span>`;
  }).join('');

  return `<div class="chapter-overview-wrap">
    <div class="chapter-overview-meta">
      <span class="chapter-overview-count">${visibleCount} visible / ${canonical.length} total canonicals</span>
    </div>
    <div class="ch-overview">${dots}</div>
  </div>`;
}

function buildProviderChipsHtml(uniqueProviders) {
  if (uniqueProviders.length === 0) return '';
  return `<div class="filter-chips provider-filter-chips">
    <span class="filter-chip ${currentFilter.provider === '' ? 'active' : ''}" onclick="filterByProvider('')">All providers</span>
    ${uniqueProviders.map(p => `<span class="filter-chip ${currentFilter.provider === p ? 'active' : ''}" onclick='filterByProvider(${JSON.stringify(p)})'>${escape(p)}</span>`).join('')}
  </div>`;
}

function getFilterSummaryText() {
  const parts = [];
  if (currentFilter.search) parts.push(`search: "${currentFilter.search}"`);
  if (currentFilter.status) parts.push(`status: ${currentFilter.status}`);
  if (currentFilter.provider) parts.push(`provider: ${currentFilter.provider}`);
  if (currentFilter.extrasOnly) parts.push('extras only');
  return parts.join(' • ');
}

function chapterRow(mangaId, ch, {
  groupKey,
  isSubrow = false,
  subRowCount = 0,
  isExpanded = false,
  isSelected = false,
} = {}) {
  const base = ch.chapter_base;
  const variant = ch.chapter_variant;
  const chNum = variant === 0 ? `Chapter ${base}` : `Chapter ${base}.${variant}`;
  const rawTitle = ch.title || '';
  const truncatedTitle = truncateMiddle(rawTitle, 50);
  const titleHtml = rawTitle
    ? ` — <span class="ch-title" title="${escape(rawTitle)}">${escape(truncatedTitle)}</span>`
    : '';
  const langHtml = (ch.language && ch.language.toLowerCase() !== 'en')
    ? ` <span style="font-size:0.7em;padding:1px 3px;border-radius:3px;background:#555;color:#fff">${ch.language.toUpperCase()}</span>`
    : '';

  const expanderHtml = (!isSubrow && subRowCount > 0)
    ? `<button class="alt-count-bubble alt-count-button" type="button"
         aria-expanded="${isExpanded}"
         aria-controls="${slotDomId(groupKey)}-expand"
         onclick="toggleChapterExpand('${groupKey}')"
         title="Show ${subRowCount} alternative${subRowCount === 1 ? '' : 's'}">+${subRowCount}</button>`
    : '';

  let chips = '';
  if (ch.is_extra) {
    chips += `<span class="chip chip-extra">EXTRA</span>`;
  }
  if (ch.is_canonical && ch.has_canonical_override) {
    chips += `<span class="chip chip-canonical" style="cursor: pointer" onclick="event.stopPropagation(); doClearCanonicalOverride('${mangaId}', ${base}, ${variant})" title="Click to reset to auto selection">Override</span>`;
  }

  const chapterLabel = `<div class="chapter-cell">
    ${expanderHtml}
    <span class="${ch.is_canonical ? 'canonical-chapter' : ''}">
      <b title="A comic reader will be coming soon">${chNum}</b>${titleHtml}${langHtml}
    </span>
    <span style="margin-left: auto; display: inline-flex; gap: 4px;">
      ${chips}
    </span>
  </div>`;

  let scoreTooltip = 'Quality Score Breakdown:\n';
  if (ch.matched_rules && ch.matched_rules.length > 0) {
    const maxLength = Math.max(...ch.matched_rules.map(([name]) => name.length));
    ch.matched_rules.forEach(([name, score]) => {
      const sign = score >= 0 ? '+' : '';
      scoreTooltip += `  ${name.padEnd(maxLength)}  ${sign}${score}\n`;
    });
    scoreTooltip += `\nTotal: ${ch.score ?? 0}`;
  } else {
    scoreTooltip = `Quality score: ${ch.score ?? 0}`;
  }
  const scoreHtml = `<span class="score-badge" title="${escape(scoreTooltip)}">${ch.score ?? 0}</span>`;
  const sourceUrl = ch.chapter_url;
  const sourceTitle = sourceUrl ? ` title="${escape(sourceUrl)}"` : '';
  const sourceName = ch.provider_name ? escape(ch.provider_name) : (ch.scanlator_group ? escape(ch.scanlator_group) : '—');
  const sourceHtml = sourceUrl
    ? `<div class="provider-cell"><a href="${escape(sourceUrl)}" target="_blank" rel="noopener" class="ch-source"${sourceTitle}>${sourceName}</a></div>`
    : `<div class="provider-cell"><span class="ch-source">${sourceName}</span></div>`;

   const scanlatorName = ch.scanlator_group || null;
   const scanlatorHtml = scanlatorName
     ? `<span class="badge badge-neutral synonym-pill scanlator-pill" style="cursor: pointer" onclick="openScanlatorRuleModal('${escape(scanlatorName)}')" title="Click to set quality score for this scanlator group">${escape(scanlatorName)}</span>`
     : `<span class="badge badge-neutral synonym-pill scanlator-pill" style="opacity: 0.6">unknown</span>`;

  const status = ch.download_status;
  const canDl = status === 'Missing' || status === 'Failed';
  const fileSizeHtml = (status === 'Downloaded' && ch.file_size_bytes)
    ? ` <span class="ch-filesize">${formatFileSize(ch.file_size_bytes)}</span>`
    : '';
  const checkboxHtml = (!isSubrow && canDl)
    ? `<input type="checkbox" class="ch-checkbox" data-slot-key="${groupKey}" ${isSelected ? 'checked' : ''} onclick="handleCheckboxClick(event, this)">`
    : '';
  const quickDlBtn = (canDl && !isSubrow)
    ? `<button class="ch-status-overlay-btn" onclick="event.stopPropagation(); doDownload('${mangaId}', ${base}, ${variant})" title="Download">
         <iconify-icon icon="mdi:download" width="18" height="18"></iconify-icon>
       </button>`
    : '';

  let actionMenuHtml = '';
  if (!isSubrow) {
    const menuId = `${slotDomId(groupKey)}-menu`;
    const dlBtn = canDl ? `<button onclick="doDownload('${mangaId}', ${base}, ${variant})">Download</button>` : '';
    const canReset = (status === 'Failed' || status === 'Queued' || status === 'Downloading') && ch.is_canonical;
    const resetBtn = canReset ? `<button onclick="doResetChapter('${mangaId}', ${base}, ${variant})">Reset</button>` : '';
    const extraBtn = ch.is_canonical ? `<button onclick="doToggleExtra('${mangaId}', ${base}, ${variant})">${ch.is_extra ? 'Un-extra' : 'Extra'}</button>` : '';
    const clearOverrideBtn = (ch.is_canonical && ch.has_canonical_override)
      ? `<button onclick="doClearCanonicalOverride('${mangaId}', ${base}, ${variant})">Reset to auto</button>`
      : '';
    const deleteBtn = (ch.is_canonical && status !== 'Missing')
      ? `<button class="danger" onclick="doDeleteChapter('${mangaId}', ${base}, ${variant})" title="Delete the downloaded CBZ from disk, but keep this chapter entry in the database. The chapter will be marked Missing.">Delete</button>`
      : '';
    const deleteEntryBtn = ch.is_canonical
      ? `<button class="danger" onclick="doDeleteChapterEntry('${mangaId}', ${base}, ${variant})" title="Delete this chapter entry from the database entirely. Use this when you want to remove the record, not just the downloaded file.">Delete Chapter Entry</button>`
      : '';

    actionMenuHtml = `<div class="action-menu">
      <button class="action-menu-btn" type="button" aria-haspopup="menu" aria-expanded="false"
        onclick="toggleActionMenu('${menuId}')"><iconify-icon icon="mdi:dots-vertical" width="18" height="18"></iconify-icon></button>
      <div class="action-menu-dropdown" id="${menuId}">
        ${dlBtn}${resetBtn}${extraBtn}${clearOverrideBtn}${deleteBtn}${deleteEntryBtn}
      </div>
    </div>`;
  }

  const useBtn = (isSubrow && !ch.is_canonical)
    ? `<button class="btn-sm" onclick='doSetCanonical("${mangaId}", ${base}, ${variant}, "${ch.id}")'>Use</button>`
    : '';

  const rowClass = isSubrow
    ? 'ch-variant ch-row'
    : `ch-main ch-row ch-row-${status.toLowerCase()}`;
  const rowId = `${slotDomId(groupKey)}-${isSubrow ? ch.id : 'main'}`;

  return `<tr class="${rowClass}" id="${rowId}">
    <td>${checkboxHtml}</td>
    <td>${chapterLabel}</td>
    <td>${scanlatorHtml}</td>
    <td>${scoreHtml}</td>
    <td>${sourceHtml}</td>
    <td>
       <div class="ch-status-cell">
         ${statusBadge(status, ch.downloaded_at)}${fileSizeHtml}
         ${quickDlBtn}
       </div>
    </td>
    <td><small>${relTime(ch.released_at)}</small></td>
    <td><small>${relTime(ch.scraped_at)}</small></td>
    <td><div style="display: inline-flex; align-items: center; gap: 4px;">${useBtn}${actionMenuHtml}</div></td>
  </tr>`;
}

function chapterGroupHtml(mangaId, group) {
  if (!group.mainRow) return '';
  const isExpanded = expandedSlotKeys.has(group.key);
  const mainRowHtml = chapterRow(mangaId, group.mainRow, {
    groupKey: group.key,
    isSubrow: false,
    subRowCount: group.subRows.length,
    isExpanded,
    isSelected: selectedSlotKeys.has(group.key),
  });

  if (group.subRows.length === 0) return mainRowHtml;

  const subRowsHtml = group.subRows
    .map(row => chapterRow(mangaId, row, { groupKey: group.key, isSubrow: true }))
    .join('');

  return `${mainRowHtml}
    <tr class="ch-expandable${isExpanded ? ' open' : ''}" id="${slotDomId(group.key)}-expand">
      <td colspan="9" style="padding:0;border:0;background:var(--bg-tertiary)">
        <div class="ch-expandable-inner">
          <table style="width:100%"><tbody>${subRowsHtml}</tbody></table>
        </div>
      </td>
    </tr>`;
}

window.scrollToChapter = function(chapterId, slotKey) {
  const containingGroup = allChapterGroupsCache.find(group =>
    group.mainRow?.id === chapterId || group.subRows.some(row => row.id === chapterId)
  );

  if (containingGroup && containingGroup.subRows.some(row => row.id === chapterId)) {
    expandedSlotKeys.add(containingGroup.key);
    renderChapterSection();
  }

  const targetId = containingGroup?.mainRow?.id === chapterId
    ? `${slotDomId(containingGroup.key)}-main`
    : `${slotDomId(containingGroup?.key || slotKey)}-${chapterId}`;
  document.getElementById(targetId)
    ?.scrollIntoView({ behavior: 'smooth', block: 'center' });
};

window.toggleChapterExpand = function(slotKey) {
  if (expandedSlotKeys.has(slotKey)) {
    expandedSlotKeys.delete(slotKey);
  } else {
    // Clear all other expanded slots - only allow one expanded at a time
    expandedSlotKeys.clear();
    expandedSlotKeys.add(slotKey);
  }

  // Re-render entire chapter section to update all expanded states
  renderChapterSection();
};

window.toggleActionMenu = function(menuId) {
  // Close all existing open menus
  document.querySelectorAll('.action-menu-dropdown.open, .action-menu-portal.open').forEach(el => {
    el.classList.remove('open');
    el.classList.remove('flip-up');
    if (el.classList.contains('action-menu-portal')) {
      el.remove();
    } else {
      const btn = el.parentElement?.querySelector('.action-menu-btn');
      btn?.setAttribute('aria-expanded', 'false');
    }
  });
  
  const menu = document.getElementById(menuId);
  if (menu) {
    // Create portal element to move menu outside table overflow context
    const portal = document.createElement('div');
    portal.id = `${menuId}-portal`;
    portal.className = 'action-menu-dropdown action-menu-portal open';
    portal.innerHTML = menu.innerHTML;
    
    // Position portal fixed relative to button
    const btn = menu.parentElement?.querySelector('.action-menu-btn');
    const btnRect = btn.getBoundingClientRect();
    
    portal.style.position = 'fixed';
    portal.style.right = `${window.innerWidth - btnRect.right}px`;
    portal.style.top = `${btnRect.bottom + 4}px`;
    portal.style.zIndex = '9999';
    
    document.body.appendChild(portal);
    
    // Check if portal would go off bottom of viewport
    const portalRect = portal.getBoundingClientRect();
    if (portalRect.bottom > window.innerHeight) {
      portal.style.top = 'auto';
      portal.style.bottom = `${window.innerHeight - btnRect.top + 4}px`;
    }
    
    btn.setAttribute('aria-expanded', 'true');
    
    // Close when clicking anywhere outside
    const closeHandler = (e) => {
      if (!portal.contains(e.target) && !btn.contains(e.target)) {
        portal.remove();
        btn.setAttribute('aria-expanded', 'false');
        document.removeEventListener('click', closeHandler);
      }
    };
    setTimeout(() => document.addEventListener('click', closeHandler), 0);
  }
};

// Close menus when clicking outside
document.addEventListener('click', (e) => {
  if (!e.target.closest('.action-menu')) {
    document.querySelectorAll('.action-menu-dropdown.open').forEach(el => {
      el.classList.remove('open');
      el.parentElement?.querySelector('.action-menu-btn')?.setAttribute('aria-expanded', 'false');
    });
  }
});

window.toggleVariants = function(groupId, toggleEl) {
  const row = document.getElementById(groupId);
  if (!row) return;
  const isOpen = row.classList.toggle('open');
  toggleEl.classList.toggle('open', isOpen);
};

function rebuildChapterDerivedState() {
  chapterSlotsCache = buildChapterSlots(chapterDataCache);
  allChapterGroupsCache = buildChapterGroups(applySlotFilters(chapterSlotsCache, {
    search: '',
    status: '',
    provider: '',
    extrasOnly: false,
  }));
  visibleChapterGroupsCache = buildChapterGroups(applySlotFilters(chapterSlotsCache));

  const validKeys = new Set(allChapterGroupsCache.map(group => group.key));
  selectedSlotKeys = new Set([...selectedSlotKeys].filter(key => validKeys.has(key)));
  expandedSlotKeys = new Set([...expandedSlotKeys].filter(key => validKeys.has(key)));
}

function renderChapterTable(groups) {
  if (groups.length === 0) return '';
  const visibleSelectable = getVisibleSelectableGroupKeys();
  const allVisibleSelected = visibleSelectable.length > 0 && visibleSelectable.every(key => selectedSlotKeys.has(key));
  const rows = groups.map(group => chapterGroupHtml(currentMangaId, group)).join('');
  return `<div class="chapters-table">
    <table class="table table-xs">
      <thead>
        <tr>
          <th style="width:30px"><input type="checkbox" title="Select all visible" ${allVisibleSelected ? 'checked' : ''} onchange="toggleSelectAll(this.checked)"></th>
          <th>Chapter</th>
          <th>Scanlator</th>
          <th title="Quality score computed from quality rules">Score</th>
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
}

function renderChapterFilters() {
  const uniqueProviders = getUniqueProviders();
  const shortcutHint = selectedSlotKeys.size > 0 ? 'Shortcuts: D download selected, A toggle all visible, Esc clear selection' : 'Shortcuts: A toggle all visible, D download selected, Esc clear selection';
  const clearFiltersBtn = hasActiveChapterFilters()
    ? `<button class="btn btn-sm btn-ghost" onclick="clearChapterFilters()">Clear filters</button>`
    : '';

  return `<div class="table-filter-bar">
    <input id="chapter-search-input" type="text" class="search-input" placeholder="Search chapters..." value="${escape(currentFilter.search)}" oninput="filterChapters(this.value)">
    <select class="sort-select" onchange="sortChapters(this.value)">
      <option value="chapter-desc" ${currentSort.field === 'chapter' && currentSort.direction === 'desc' ? 'selected' : ''}>Newest first</option>
      <option value="chapter-asc" ${currentSort.field === 'chapter' && currentSort.direction === 'asc' ? 'selected' : ''}>Oldest first</option>
      <option value="released-desc" ${currentSort.field === 'released' && currentSort.direction === 'desc' ? 'selected' : ''}>Recently released</option>
      <option value="released-asc" ${currentSort.field === 'released' && currentSort.direction === 'asc' ? 'selected' : ''}>Oldest released</option>
      <option value="score-desc" ${currentSort.field === 'score' && currentSort.direction === 'desc' ? 'selected' : ''}>Best score first</option>
      <option value="status-asc" ${currentSort.field === 'status' && currentSort.direction === 'asc' ? 'selected' : ''}>Status A-Z</option>
      <option value="status-desc" ${currentSort.field === 'status' && currentSort.direction === 'desc' ? 'selected' : ''}>Status Z-A</option>
    </select>
    <div class="filter-chips">
      <span class="filter-chip ${currentFilter.status === '' ? 'active' : ''}" onclick="filterByStatus('')">All</span>
      <span class="filter-chip ${currentFilter.status === 'Missing' ? 'active' : ''}" onclick="filterByStatus('Missing')">Missing</span>
      <span class="filter-chip ${currentFilter.status === 'Downloaded' ? 'active' : ''}" onclick="filterByStatus('Downloaded')">Downloaded</span>
      <span class="filter-chip ${currentFilter.status === 'Queued' ? 'active' : ''}" onclick="filterByStatus('Queued')">Queued</span>
      <span class="filter-chip ${currentFilter.status === 'Failed' ? 'active' : ''}" onclick="filterByStatus('Failed')">Failed</span>
      <span class="filter-chip ${currentFilter.extrasOnly ? 'active' : ''}" onclick="toggleExtrasFilter()">Extras</span>
    </div>
    ${buildProviderChipsHtml(uniqueProviders)}
    <button class="btn btn-sm btn-ghost" onclick="selectAllMissing()" title="Check all visible missing/failed chapters">
      <iconify-icon icon="mdi:select-all" width="16" height="16"></iconify-icon>
      Select Missing
    </button>
    ${clearFiltersBtn}
    <span class="chapter-shortcut-hint">${shortcutHint}</span>
  </div>`;
}

function renderChapterEmptyState() {
  const summary = getFilterSummaryText();
  return `<div class="chapter-empty-state">
    <h4>No chapters match your filters.</h4>
    ${summary ? `<p>${escape(summary)}</p>` : '<p>Try a different search or filter combination.</p>'}
    <button class="btn btn-sm btn-primary" onclick="clearChapterFilters()">Clear filters</button>
  </div>`;
}

function restoreChapterInputState(previousFocus) {
  if (!previousFocus) return;
  const el = document.getElementById(previousFocus.id);
  if (!el) return;
  el.focus();
  if (typeof previousFocus.start === 'number' && typeof previousFocus.end === 'number' && typeof el.setSelectionRange === 'function') {
    el.setSelectionRange(previousFocus.start, previousFocus.end);
  }
}

function renderChapterSection({ preserveFocus = false } = {}) {
  const el = document.getElementById('chapters-list');
  if (!el) return;

  rebuildChapterDerivedState();

  const activeEl = document.activeElement;
  const previousFocus = preserveFocus && activeEl?.id === 'chapter-search-input'
    ? { id: activeEl.id, start: activeEl.selectionStart, end: activeEl.selectionEnd }
    : null;

  const content = chapterDataCache.length === 0
    ? `
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
      </div>`
    : `${renderChapterFilters()}
      ${renderChapterOverview()}
      ${visibleChapterGroupsCache.length > 0 ? renderChapterTable(visibleChapterGroupsCache) : renderChapterEmptyState()}`;

  el.innerHTML = content;
  restoreChapterInputState(previousFocus);
  updateBulkBar();
}

// Patch a canonical chapter entry in the cache and re-render without fetching.
function patchCachedChapter(base, variant, fields) {
  const idx = chapterDataCache.findIndex(
    ch => ch.chapter_base == base && ch.chapter_variant == variant && ch.is_canonical
  );
  if (idx === -1) return;

  const current = chapterDataCache[idx];
  const oldKey = makeSlotKey(current.chapter_base, current.chapter_variant, !!current.is_extra);
  const next = { ...current, ...fields };
  const newKey = makeSlotKey(next.chapter_base, next.chapter_variant, !!next.is_extra);
  chapterDataCache[idx] = next;

  if (oldKey !== newKey) {
    if (selectedSlotKeys.has(oldKey)) {
      selectedSlotKeys.delete(oldKey);
      selectedSlotKeys.add(newKey);
    }
    if (expandedSlotKeys.has(oldKey)) {
      expandedSlotKeys.delete(oldKey);
      expandedSlotKeys.add(newKey);
    }
  }

  renderChapterSection();
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
    renderChapterSection();
    
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
  renderChapterSection({ preserveFocus: true });
};

window.filterByStatus = function(status) {
  currentFilter.status = status;
  renderChapterSection();
};

window.filterByProvider = function(provider) {
  if (currentFilter.provider === provider) {
    currentFilter.provider = '';
  } else {
    currentFilter.provider = provider;
  }
  renderChapterSection();
};

window.sortChapters = function(value) {
  const [field, direction] = value.split('-');
  currentSort = { field, direction };
  renderChapterSection();
};

window.toggleExtrasFilter = function() {
  currentFilter.extrasOnly = !currentFilter.extrasOnly;
  renderChapterSection();
};

window.clearChapterFilters = function() {
  currentFilter = { search: '', status: '', provider: '', extrasOnly: false };
  renderChapterSection();
};

export async function loadProviders(mangaId) {
  const el = document.getElementById('providers-list');
  if (!el) return;
  try {
    const provList = await mangaApi.providers(mangaId);
    if (provList.length === 0) {
      el.innerHTML = '<p><small>No providers found yet. Scan this manga to discover providers.</small></p>';
      return;
    }

    // Fetch per-series settings in parallel
    const settingsResults = await Promise.allSettled(
      provList.map(p => providerSettings.getSeries(mangaId, p.provider_name))
    );

    const rows = provList.map((p, i) => {
      const statusClass = p.found ? 'found' : 'not-found';
      const statusText = p.found ? 'Found' : 'Not found';
      const searched = p.search_attempted_at ? relTime(p.search_attempted_at) : 'never';
      const synced = p.last_synced_at ? relTime(p.last_synced_at) : 'Never';

      const settingsData = settingsResults[i].status === 'fulfilled' ? settingsResults[i].value : null;
      const isEnabled = settingsData?.effective_enabled ?? true;
      const hasOverride = settingsData?.enabled != null;

      const linkBtn = p.provider_url
        ? `<button onclick="window.open('${escape(p.provider_url)}', '_blank')">Open</button>`
        : '';

      const overrideLabel = hasOverride ? '' : ' <small style="opacity:0.6">(global)</small>';
      const enableToggle = `<label title="${isEnabled ? 'Enabled — click to disable for this series' : 'Disabled — click to enable for this series'}">
        <input type="checkbox" ${isEnabled ? 'checked' : ''} onchange="setProviderEnabled('${mangaId}', '${escape(p.provider_name)}', this.checked)">
        ${isEnabled ? 'Enabled' : 'Disabled'}${overrideLabel}
      </label>`;

      const resetBtn = hasOverride
        ? `<button class="btn btn-xs btn-ghost" onclick="resetProviderEnabled('${mangaId}', '${escape(p.provider_name)}')" title="Reset to global setting">Reset</button>`
        : '';

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
        <td>${enableToggle}${resetBtn}</td>
        <td>${pickBtn}</td>
      </tr>`;
    }).join('');

    el.innerHTML = `<div class="chapters-table">
      <table>
        <thead>
          <tr><th>Provider</th><th>Status</th><th>Last Synced</th><th>Searched</th><th>Enabled</th><th></th></tr>
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
    await providerSettings.setSeries(mangaId, providerName, enabled);
    showToast(`${providerName} ${enabled ? 'enabled' : 'disabled'} for this series`);
    loadProviders(mangaId);
    await loadChapters(mangaId);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.resetProviderEnabled = async function(mangaId, providerName) {
  try {
    await providerSettings.deleteSeries(mangaId, providerName);
    showToast(`${providerName} reset to global setting`);
    loadProviders(mangaId);
    await loadChapters(mangaId);
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
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
  if (!confirm('Delete the downloaded file for this chapter? The chapter entry will stay in the database and be marked Missing.')) return;
  try {
    await mangaApi.deleteChapter(mangaId, base, variant);
    loadChapters(mangaId);
    showToast('Chapter file deleted');
  } catch(e) {
    showToast('Delete error: ' + e.message, 'error');
  }
};

window.doDeleteChapterEntry = async function(mangaId, base, variant) {
  if (!confirm('Delete this chapter entry from the database? This removes the chapter record itself.')) return;
  try {
    await mangaApi.deleteChapterEntry(mangaId, base, variant);
    loadChapters(mangaId);
    showToast('Chapter entry deleted');
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

window.doClearCanonicalOverride = async function(mangaId, base, variant) {
  try {
    await mangaApi.clearCanonicalOverride(mangaId, base, variant);
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
  for (const key of getVisibleSelectableGroupKeys()) {
    if (checked) selectedSlotKeys.add(key);
    else selectedSlotKeys.delete(key);
  }
  renderChapterSection();
  updateBulkBar();
};

function getSelectedGroups() {
  return [...selectedSlotKeys]
    .map(key => getGroupByKey(key))
    .filter(Boolean);
}

window.doDownloadSelected = async function(mangaId) {
  const selectedGroups = getSelectedGroups().reverse();
  if (selectedGroups.length === 0) { showToast('Select at least one chapter.', 'warning'); return; }
  let count = 0, errors = 0;
  for (const group of selectedGroups) {
    try {
      await mangaApi.downloadChapter(mangaId, group.mainRow.chapter_base, group.mainRow.chapter_variant);
      count++;
    } catch(e) { errors++; }
  }
  for (const group of selectedGroups) {
    const idx = chapterDataCache.findIndex(ch =>
      ch.chapter_base == group.mainRow.chapter_base &&
      ch.chapter_variant == group.mainRow.chapter_variant &&
      ch.is_canonical
    );
    if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], download_status: 'Queued' };
  }
  renderChapterSection();
  updateBulkBar();
  if (count > 0) {
    showToast(`Queued ${count} download${count === 1 ? '' : 's'}${errors > 0 ? `, ${errors} failed` : ''}`);
  } else {
    showToast(`${errors} download${errors === 1 ? '' : 's'} failed`, 'error');
  }
};

window.doDownloadAllMissing = async function(mangaId) {
  const groups = visibleChapterGroupsCache
    .filter(group => {
      const status = group.mainRow?.download_status;
      return status === 'Missing' || status === 'Failed';
    })
    .reverse();
  if (groups.length === 0) { showToast('No missing chapters to download.', 'warning'); return; }
  let count = 0, errors = 0;
  for (const group of groups) {
    try {
      await mangaApi.downloadChapter(mangaId, group.mainRow.chapter_base, group.mainRow.chapter_variant);
      count++;
    } catch(e) { errors++; }
  }
  for (const group of groups) {
    const idx = chapterDataCache.findIndex(ch =>
      ch.chapter_base == group.mainRow.chapter_base &&
      ch.chapter_variant == group.mainRow.chapter_variant &&
      ch.is_canonical
    );
    if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], download_status: 'Queued' };
  }
  renderChapterSection();
  updateBulkBar();
  if (errors > 0) {
    showToast(`Queued ${count}, ${errors} failed`, 'error');
  } else {
    showToast(`Queued ${count} chapter${count === 1 ? '' : 's'}`);
  }
};

function setupBulkBar(mangaId) {
  document.getElementById('bulk-action-bar')?.remove();
  const bar = document.createElement('div');
  bar.id = 'bulk-action-bar';
  bar.className = 'bulk-action-bar';
  bar.innerHTML = `
    <span class="bulk-action-count" id="bulk-count">0 selected</span>
    <button class="btn btn-sm btn-accent" onclick="doDownloadSelected('${mangaId}')">
      <iconify-icon icon="mdi:download" width="16" height="16"></iconify-icon>
      Download
    </button>
    <button class="btn btn-sm btn-outline" onclick="doBulkMarkDownloaded('${mangaId}')">
      <iconify-icon icon="mdi:check-bold" width="16" height="16"></iconify-icon>
      Mark Downloaded
    </button>
    <button class="btn btn-sm btn-ghost" onclick="clearBulkSelection()">
      <iconify-icon icon="mdi:close" width="16" height="16"></iconify-icon>
      Clear
    </button>
  `;
  document.body.appendChild(bar);
}

function updateBulkBar() {
  const bar = document.getElementById('bulk-action-bar');
  if (!bar) return;
  const n = selectedSlotKeys.size;
  if (n > 0) {
    bar.classList.add('visible');
    document.body.classList.add('bulk-bar-active');
    const el = document.getElementById('bulk-count');
    if (el) {
      const hidden = [...selectedSlotKeys].filter(key => !visibleChapterGroupsCache.some(group => group.key === key)).length;
      el.textContent = hidden > 0
        ? `${n} chapter${n === 1 ? '' : 's'} selected (${hidden} hidden)`
        : `${n} chapter${n === 1 ? '' : 's'} selected`;
    }
  } else {
    bar.classList.remove('visible');
    document.body.classList.remove('bulk-bar-active');
  }
}

window.toggleProvidersSection = function() {
  const collapsed = document.getElementById('providers-collapsible')?.classList.toggle('collapsed');
  document.getElementById('providers-chevron')?.classList.toggle('collapsed', collapsed);
  try { localStorage.setItem('rebarr-providers-collapsed', collapsed ? '1' : '0'); } catch(e) {}
};

window.clearBulkSelection = function() {
  selectedSlotKeys.clear();
  renderChapterSection();
  updateBulkBar();
};

window.selectAllMissing = function() {
  for (const key of getVisibleSelectableGroupKeys()) {
    selectedSlotKeys.add(key);
  }
  renderChapterSection();
  updateBulkBar();
};

window.doBulkMarkDownloaded = async function(mangaId) {
  const selectedGroups = getSelectedGroups();
  if (!selectedGroups.length) { showToast('Select at least one chapter.', 'warning'); return; }
  let count = 0, errors = 0;
  for (const group of selectedGroups) {
    try { await mangaApi.markDownloaded(mangaId, group.mainRow.chapter_base, group.mainRow.chapter_variant); count++; }
    catch(e) { errors++; }
  }
  for (const group of selectedGroups) {
    const idx = chapterDataCache.findIndex(ch =>
      ch.chapter_base == group.mainRow.chapter_base &&
      ch.chapter_variant == group.mainRow.chapter_variant &&
      ch.is_canonical);
    if (idx !== -1) chapterDataCache[idx] = { ...chapterDataCache[idx], download_status: 'Downloaded' };
  }
  renderChapterSection();
  updateBulkBar();
  showToast(`Marked ${count} as downloaded${errors > 0 ? `, ${errors} failed` : ''}`);
};

window.handleCheckboxClick = function(e, cb) {
  const visibleKeys = getVisibleSelectableGroupKeys();
  const slotKey = cb.dataset.slotKey;
  const currentIdx = visibleKeys.indexOf(slotKey);
  const nextChecked = !selectedSlotKeys.has(slotKey);

  if (nextChecked) selectedSlotKeys.add(slotKey);
  else selectedSlotKeys.delete(slotKey);

  if (e.shiftKey && lastCheckedIdx !== -1 && currentIdx !== -1) {
    const lo = Math.min(lastCheckedIdx, currentIdx);
    const hi = Math.max(lastCheckedIdx, currentIdx);
    for (let i = lo; i <= hi; i++) {
      const key = visibleKeys[i];
      if (nextChecked) selectedSlotKeys.add(key);
      else selectedSlotKeys.delete(key);
    }
  }
  if (currentIdx !== -1) lastCheckedIdx = currentIdx;
  renderChapterSection();
  updateBulkBar();
};

window.doDirectCoverUpload = async function(event, mangaId) {
  const file = event.target.files[0];
  if (!file) return;
  try {
    showToast('Uploading cover...');
    await coverApi.uploadFile(mangaId, file);
    showToast('Cover updated');
    viewSeries(mangaId);
  } catch(e) {
    showToast('Upload failed: ' + e.message, 'error');
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

// Keyboard shortcuts (series page only)
document.addEventListener('keydown', (e) => {
  if (!currentMangaId) return;
  const tag = document.activeElement?.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
  switch(e.key) {
    case 'd': {
      const n = selectedSlotKeys.size;
      if (n === 0) showToast('Select chapters first, or use "Download All Missing"', 'info');
      else window.doDownloadSelected(currentMangaId);
      break;
    }
    case 'a': {
      const visibleKeys = getVisibleSelectableGroupKeys();
      const anyUnchecked = visibleKeys.some(key => !selectedSlotKeys.has(key));
      window.toggleSelectAll(anyUnchecked);
      break;
    }
    case 's': {
      const cb = hoveredChapterRow?.querySelector('.ch-checkbox');
      if (cb) window.handleCheckboxClick({ shiftKey: false }, cb);
      break;
    }
    case 'Escape':
      window.clearBulkSelection();
      break;
  }
});

// Track hovered chapter row for the 's' shortcut
document.addEventListener('mouseover', (e) => {
  const row = e.target.closest('.ch-row');
  if (row) hoveredChapterRow = row;
});
document.addEventListener('mouseout', (e) => {
  if (e.target.closest('.ch-row') && !e.relatedTarget?.closest('.ch-row')) hoveredChapterRow = null;
});

// ---------------------------------------------------------------------------
// Scanlator Quality Rule Modal
// ---------------------------------------------------------------------------

window.openScanlatorRuleModal = async function(scanlatorName) {
  // Remove any existing modal first
  const existingModal = document.getElementById('scanlator-rule-modal');
  if (existingModal) existingModal.remove();

  try {
    // Load all quality rules to check if we already have one for this scanlator
    const allRules = await qualityRules.list();
    
    // Find exact matching rule: single condition, scanlator_group eq scanlatorName
    let existingRule = null;
    for (const rule of allRules) {
      if (rule.conditions.length === 1 &&
          rule.conditions[0].field === 'scanlator_group' &&
          rule.conditions[0].op === 'eq' &&
          rule.conditions[0].value === scanlatorName &&
          !rule.conditions[0].negate) {
        existingRule = rule;
        break;
      }
    }

    // Create modal
    const modal = document.createElement('div');
    modal.id = 'scanlator-rule-modal';
    modal.className = 'modal-overlay';
    
    const currentScore = existingRule ? existingRule.score : 0;
    
    modal.innerHTML = `
      <div class="modal-box">
        <h3 class="modal-title">Quality Rule for <strong>${escape(scanlatorName)}</strong></h3>
        
        <div style="margin: 1rem 0;">
          <label style="display: block; margin-bottom: 0.5rem;">Score adjustment for this scanlator group:</label>
          <div style="display: flex; gap: 0.75rem; align-items: center;">
            <input type="range" id="scanlator-score-slider" min="-100" max="100" value="${currentScore}" 
                   style="flex: 1;" oninput="document.getElementById('scanlator-score-input').value = this.value">
            <input type="number" id="scanlator-score-input" min="-100" max="100" value="${currentScore}" 
                   style="width: 5rem;" oninput="document.getElementById('scanlator-score-slider').value = this.value">
          </div>
          <div style="display: flex; justify-content: space-between; font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem;">
            <span>Worst (-100)</span>
            <span>Default (0)</span>
            <span>Best (+100)</span>
          </div>
        </div>
        
        <p style="font-size: 0.875rem; color: var(--text-muted); margin-bottom: 1rem;">
          ${existingRule 
            ? 'This will update the existing quality rule for this scanlator.' 
            : 'A new quality rule will be created that matches chapters from this scanlator group.'}
        </p>
        
        <div class="modal-footer">
          <button class="btn btn-sm btn-ghost" onclick="document.getElementById('scanlator-rule-modal').remove()">Cancel</button>
          <button class="btn btn-sm btn-primary" onclick="saveScanlatorRule('${escape(scanlatorName)}', ${existingRule ? `'${existingRule.id}'` : 'null'})">Save</button>
        </div>
      </div>
    `;
    
    document.body.appendChild(modal);
    
    // Close on backdrop click
    modal.addEventListener('click', e => { if (e.target === modal) modal.remove(); });
    
  } catch(e) {
    showToast('Error loading quality rules: ' + e.message, 'error');
  }
};

window.saveScanlatorRule = async function(scanlatorName, existingRuleId) {
  const score = parseInt(document.getElementById('scanlator-score-input').value, 10);
  
  if (isNaN(score) || score < -100 || score > 100) {
    showToast('Please enter a valid score between -100 and 100', 'error');
    return;
  }
  
  const ruleData = {
    name: `Scanlator: ${scanlatorName}`,
    score: score,
    sort_order: 100,
    conditions: [
      {
        field: 'scanlator_group',
        op: 'eq',
        value: scanlatorName,
        negate: false
      }
    ]
  };
  
  try {
    if (existingRuleId) {
      await qualityRules.update(existingRuleId, ruleData);
      showToast(`Updated quality rule for ${scanlatorName}`);
    } else {
      await qualityRules.create(ruleData);
      showToast(`Created quality rule for ${scanlatorName}`);
    }
    
    // Close modal
    document.getElementById('scanlator-rule-modal')?.remove();
    
    // Refresh chapters to apply new score
    if (currentMangaId) {
      await loadChapters(currentMangaId);
    }
    
  } catch(e) {
    showToast('Error saving quality rule: ' + e.message, 'error');
  }
};
