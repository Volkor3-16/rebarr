// Queue view - task history + active queue with live polling

import { tasks, settings } from '../api.js';
import { navigate, render } from '../router.js';
import { escape, taskBadge, showToast } from '../utils.js';
import * as sse from '../events.js';

// Track which cancelled groups are expanded (by index)
const expandedGroups = new Set();

// Track selected task IDs to preserve selection across refreshes
const selectedTaskIds = new Set();

let sseHandler = null;
const QUEUE_FILTER_KEY = 'rebarr_queue_task_filters';
const QUEUE_REFRESH_DELAY_MS = 250;
const DEFAULT_VISIBLE_TASKS = ['DownloadChapter'];
const KNOWN_TASK_TYPES = [
  'ScanLibrary',
  'BuildFullChapterList',
  'RefreshMetadata',
  'CheckNewChapter',
  'DownloadChapter',
  'ScanDisk',
  'OptimiseChapter',
  'Backup',
  'SyncProviderChapters',
];

function loadTaskTypeFilters() {
  try {
    const raw = localStorage.getItem(QUEUE_FILTER_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.length > 0) {
        return new Set(parsed.filter(Boolean));
      }
    }
  } catch (_) {}
  return new Set(DEFAULT_VISIBLE_TASKS);
}

function saveTaskTypeFilters() {
  try {
    localStorage.setItem(QUEUE_FILTER_KEY, JSON.stringify([...visibleTaskTypes]));
  } catch (_) {}
}

let visibleTaskTypes = loadTaskTypeFilters();
let refreshInFlight = false;
let refreshQueued = false;
let refreshTimer = null;
let currentMangaFilter = null;

export async function viewQueue() {
  const url = new URL(window.location.href);
  currentMangaFilter = url.searchParams.get('manga_id') || null;
  render(`
    <h2>Queue</h2>
    <div id="queue-controls">
      <div class="spinner"></div>
    </div>
    <div id="queue-list"></div>
  `);
  
  await refreshQueue();
  
  // Coalesce bursts of task updates into a single bounded refresh.
  sseHandler = () => scheduleQueueRefresh();
  sse.on('task_update', sseHandler);
}

async function refreshQueue() {
  if (refreshInFlight) {
    refreshQueued = true;
    return;
  }
  refreshInFlight = true;
  const listEl = document.getElementById('queue-list');
  const ctrlEl = document.getElementById('queue-controls');
  if (!listEl || !ctrlEl) {
    refreshInFlight = false;
    return;
  }
  
  // Save current checkbox states before rebuilding
  document.querySelectorAll('.task-cb:checked').forEach(cb => {
    selectedTaskIds.add(cb.dataset.id);
  });
  document.querySelectorAll('.task-cb:not(:checked)').forEach(cb => {
    selectedTaskIds.delete(cb.dataset.id);
  });
  
  try {
    const [queueData, appSettings] = await Promise.all([
      tasks.listQueue(currentMangaFilter ? { manga_id: currentMangaFilter } : {}),
      settings.get(),
    ]);
    const taskList = queueData.tasks || [];
    
    const paused = appSettings.queue_paused;
    const pauseLabel = paused ? '<span class="iconify" data-icon="mdi-play"></span> Resume Queue' : '<span class="iconify" data-icon="mdi-pause"></span> Pause Queue';
    const availableTaskTypes = getAvailableTaskTypes(taskList);
    visibleTaskTypes = normalizeVisibleTaskTypes(visibleTaskTypes, availableTaskTypes);
    saveTaskTypeFilters();
    const filteredTaskList = taskList.filter(t => visibleTaskTypes.has(t.task_type));

    const runningCount = taskList.filter(t => t.status === 'Running').length;
    const pendingCount = taskList.filter(t => t.status === 'Pending').length;
    document.title = `[${runningCount} - ${pendingCount}] REBARR`;

    // Check if there's a running task for the Jump button
    const hasRunning = filteredTaskList.some(t => t.status === 'Running');
    const jumpBtn = hasRunning
      ? `<button class="btn btn-sm btn-primary" onclick="jumpToActive()"><span class="iconify" data-icon="mdi-crosshairs-gps"></span> Jump to Active</button>`
      : '';
    
    ctrlEl.innerHTML = `
      <button class="btn btn-sm ${paused ? 'btn-success' : ''}" onclick="toggleQueuePause(${paused})">${pauseLabel}</button>
      <button class="btn btn-sm btn-accent btn-outline" onclick="prioritiseSelected()">Prioritise Selected</button>
      <button class="btn btn-sm btn-error btn-outline" onclick="cancelSelected()">Cancel Selected</button>
      ${jumpBtn}
      ${paused ? '<span class="badge badge-warning">Queue paused — no new tasks will run.</span>' : ''}
      ${currentMangaFilter ? '<span class="badge badge-info">Filtered to one series task history.</span><button class="btn btn-sm btn-ghost" onclick="clearQueueSeriesFilter()">Show all tasks</button>' : ''}
      ${queueData.has_more_history ? `<span class="badge badge-info">Showing all active tasks + latest ${queueData.terminal_limit} finished tasks.</span>` : ''}
      ${buildTaskTypeFilterBar(availableTaskTypes, taskList)}
    `;
    
    if (taskList.length === 0) {
      listEl.innerHTML = '<p>No tasks yet.</p>';
      return;
    }

    if (filteredTaskList.length === 0) {
      listEl.innerHTML = '<p>No tasks match the current filters.</p>';
      return;
    }
    
    // Build rows with cancelled task compaction
    const rows = buildCompactedRows(filteredTaskList);
    
    listEl.innerHTML = `
      <table>
        <thead>
          <tr>
            <th><input type="checkbox" title="Select all cancelable" onchange="toggleSelectAllTasks(this.checked)"></th>
            <th>Time</th>
            <th>Task</th>
            <th>Status</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          ${rows}
        </tbody>
      </table>
    `;
    
    // Restore checkbox states after rebuilding
    document.querySelectorAll('.task-cb').forEach(cb => {
      if (selectedTaskIds.has(cb.dataset.id)) {
        cb.checked = true;
      }
    });
    
    // Update "select all" checkbox state
    updateSelectAllCheckbox();
  } catch(e) {
    if (listEl) listEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  } finally {
    refreshInFlight = false;
    if (refreshQueued) {
      refreshQueued = false;
      scheduleQueueRefresh(0);
    }
  }
}

function scheduleQueueRefresh(delay = QUEUE_REFRESH_DELAY_MS) {
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    refreshQueue();
  }, delay);
}

function updateSelectAllCheckbox() {
  const allCancelableCheckboxes = document.querySelectorAll('.task-cb');
  const selectAllCheckbox = document.querySelector('th input[type="checkbox"]');
  if (!selectAllCheckbox || allCancelableCheckboxes.length === 0) return;
  
  const allChecked = Array.from(allCancelableCheckboxes).every(cb => cb.checked);
  const someChecked = Array.from(allCancelableCheckboxes).some(cb => cb.checked);
  
  selectAllCheckbox.checked = allChecked;
  selectAllCheckbox.indeterminate = someChecked && !allChecked;
}

function getAvailableTaskTypes(taskList) {
  const taskTypes = new Set(KNOWN_TASK_TYPES);
  taskList.forEach(t => {
    if (t.task_type) taskTypes.add(t.task_type);
  });
  return [...taskTypes].sort();
}

function normalizeVisibleTaskTypes(current, availableTaskTypes) {
  const available = new Set(availableTaskTypes);
  const next = new Set([...current].filter(t => available.has(t)));
  if (next.size === 0) {
    if (available.has('DownloadChapter')) {
      next.add('DownloadChapter');
    } else if (availableTaskTypes.length > 0) {
      next.add(availableTaskTypes[0]);
    }
  }
  return next;
}

function buildTaskTypeFilterBar(taskTypes, taskList) {
  const counts = new Map();
  taskList.forEach(t => counts.set(t.task_type, (counts.get(t.task_type) || 0) + 1));
  const selectedCount = visibleTaskTypes.size;
  return `
    <div class="table-filter-bar" style="margin-top:0.75rem">
      <div class="filter-chips">
        <span class="filter-chip ${selectedCount === taskTypes.length ? 'active' : ''}" onclick="showAllQueueTaskTypes()">All</span>
        <span class="filter-chip ${selectedCount === 1 && visibleTaskTypes.has('DownloadChapter') ? 'active' : ''}" onclick="showOnlyDownloadTasks()">Downloads</span>
        ${taskTypes.map(type => {
          const active = visibleTaskTypes.has(type);
          const count = counts.get(type) || 0;
          return `<span class="filter-chip ${active ? 'active' : ''}" onclick="toggleQueueTaskType('${escape(type)}')">${escape(type)}${count > 0 ? ` (${count})` : ''}</span>`;
        }).join('')}
      </div>
    </div>
  `;
}

function buildCompactedRows(taskList) {
  const result = [];
  let groupIndex = 0;
  let i = 0;
  
  while (i < taskList.length) {
    const t = taskList[i];
    
    // Check if this is a cancelled task
    if (t.status === 'Cancelled') {
      // Count consecutive cancelled tasks
      let cancelledCount = 0;
      let j = i;
      while (j < taskList.length && taskList[j].status === 'Cancelled') {
        cancelledCount++;
        j++;
      }
      
      // If multiple consecutive cancelled tasks, compact them
      if (cancelledCount > 1) {
        const currentGroupIndex = groupIndex++;
        const isExpanded = expandedGroups.has(currentGroupIndex);
        
        if (isExpanded) {
          // Show all cancelled tasks in this group
          for (let k = i; k < j; k++) {
            result.push(buildTaskRow(taskList[k]));
          }
          // Add collapse toggle
          result.push(`
            <tr class="cancelled-group-toggle">
              <td colspan="5">
                <span class="cancelled-toggle" onclick="toggleCancelledGroup(${currentGroupIndex})">
                  <span class="iconify" data-icon="mdi-chevron-up"></span>
                  Hide ${cancelledCount} cancelled tasks
                </span>
              </td>
            </tr>
          `);
        } else {
          // Show compacted row
          const firstTs = new Date(taskList[i].created_at).toLocaleString();
          const lastTs = new Date(taskList[j - 1].created_at).toLocaleString();
          result.push(`
            <tr class="cancelled-group-row">
              <td></td>
              <td><small>${escape(firstTs)}</small></td>
              <td colspan="2">
                <span class="cancelled-toggle" onclick="toggleCancelledGroup(${currentGroupIndex})">
                  <span class="iconify" data-icon="mdi-chevron-down"></span>
                  ${cancelledCount} cancelled tasks
                </span>
              </td>
              <td></td>
            </tr>
          `);
        }
        
        i = j; // Skip past all cancelled tasks in this group
      } else {
        // Single cancelled task, show normally
        result.push(buildTaskRow(t));
        i++;
      }
    } else {
      // Non-cancelled task, show normally
      result.push(buildTaskRow(t));
      i++;
    }
  }
  
  return result.join('');
}

function buildTaskRow(t) {
  const ts = new Date(t.created_at).toLocaleString();
  const taskDesc = buildCompactTaskLabel(t);
  const progress = buildCompactTaskProgress(t.progress);
  const err = t.last_error ? `<br><small class="error">${escape(t.last_error)}</small>` : '';
  const canCancel = t.status === 'Pending' || t.status === 'Running';
  const canPrioritise = t.status === 'Pending' && t.task_type === 'DownloadChapter';
  const cb = canCancel
    ? `<input type="checkbox" class="task-cb" data-id="${t.id}" data-can-prioritise="${canPrioritise ? '1' : '0'}">`
    : '';
  const prioritiseBtn = canPrioritise
    ? `<button class="btn btn-xs btn-accent btn-outline" onclick='prioritiseTask("${t.id}")'>Run next</button>`
    : '';
  const cancelBtn = canCancel
    ? `<button class="btn btn-xs btn-error btn-outline" onclick='cancelTask("${t.id}")'>Cancel</button>`
    : '';
  const rowId = t.status === 'Running' ? `id="active-task-${t.id}"` : '';
  const highlightClass = t.status === 'Running' ? ' class="task-active-row"' : '';
  
  return `
    <tr${highlightClass} ${rowId}>
      <td>${cb}</td>
      <td><small>${escape(ts)}</small></td>
      <td><div class="queue-task-main">${taskDesc}</div>${progress}</td>
      <td>${taskBadge(t.status)}${err}</td>
      <td>${prioritiseBtn}${cancelBtn}</td>
    </tr>
  `;
}

function buildCompactTaskLabel(t) {
  const title = t.manga_title ? escape(t.manga_title) : '';
  const chapter = t.chapter_number_raw ? `Ch. ${escape(t.chapter_number_raw)}` : '';
  const page = compactPageSummary(t.progress);

  const details = [chapter, page].filter(Boolean).join(' - ');
  if (title && details) return `${escape(t.task_type)}: ${title} <small class="queue-task-subtle">(${details})</small>`;
  if (title) return `${escape(t.task_type)}: ${title}`;
  if (details) return `${escape(t.task_type)} <small class="queue-task-subtle">(${details})</small>`;
  return escape(t.task_type);
}

function compactPageSummary(progress) {
  const current = Number(progress?.current);
  const total = Number(progress?.total);
  const unit = String(progress?.unit || '').toLowerCase();
  if (!Number.isFinite(current) || !Number.isFinite(total) || total <= 0) return '';
  if (unit && unit !== 'page') return `${current} / ${total} ${unit}${total === 1 ? '' : 's'}`;
  return `p. ${current} / ${total}`;
}

function compactProgressPercent(progress) {
  const current = Number(progress?.current);
  const total = Number(progress?.total);
  if (!Number.isFinite(current) || !Number.isFinite(total) || total <= 0) return null;
  return Math.max(0, Math.min(100, (current / total) * 100));
}

function buildCompactTaskProgress(progress) {
  if (!progress) return '';

  const lines = [];
  if (progress.provider) {
    lines.push(`<div class="queue-task-meta">Provider: ${escape(progress.provider)}</div>`);
  }

  const percent = compactProgressPercent(progress);
  if (percent != null) {
    lines.push(`
      <div class="task-progress queue-task-progress" aria-label="Task progress">
        <div class="task-progress-bar">
          <div class="task-progress-fill" style="width:${percent.toFixed(1)}%"></div>
        </div>
      </div>
    `);
  }

  return lines.join('');
}

window.toggleQueuePause = async function(currentlyPaused) {
  try {
    await settings.update({ queue_paused: !currentlyPaused });
    scheduleQueueRefresh(0);
    showToast(currentlyPaused ? 'Queue resumed' : 'Queue paused');
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
};

window.toggleQueueTaskType = function(taskType) {
  if (visibleTaskTypes.has(taskType)) {
    if (visibleTaskTypes.size === 1) return;
    visibleTaskTypes.delete(taskType);
  } else {
    visibleTaskTypes.add(taskType);
  }
  saveTaskTypeFilters();
  scheduleQueueRefresh(0);
};

window.showAllQueueTaskTypes = function() {
  visibleTaskTypes = new Set(KNOWN_TASK_TYPES);
  saveTaskTypeFilters();
  scheduleQueueRefresh(0);
};

window.showOnlyDownloadTasks = function() {
  visibleTaskTypes = new Set(DEFAULT_VISIBLE_TASKS);
  saveTaskTypeFilters();
  scheduleQueueRefresh(0);
};

window.toggleSelectAllTasks = function(checked) {
  document.querySelectorAll('.task-cb').forEach(cb => {
    cb.checked = checked;
    if (checked) {
      selectedTaskIds.add(cb.dataset.id);
    } else {
      selectedTaskIds.delete(cb.dataset.id);
    }
  });
};

window.cancelSelected = async function() {
  const checked = Array.from(document.querySelectorAll('.task-cb:checked'));
  if (checked.length === 0) { showToast('Select at least one task to cancel.', 'warning'); return; }
  for (const cb of checked) {
    try { 
      await tasks.cancel(cb.dataset.id); 
      selectedTaskIds.delete(cb.dataset.id);
    } catch(_) {}
  }
  showToast('Cancelled ' + checked.length + ' task(s)');
  scheduleQueueRefresh(0);
};

window.prioritiseSelected = async function() {
  const checked = Array.from(document.querySelectorAll('.task-cb:checked'));
  const pendingIds = checked
    .filter(cb => cb.dataset.canPrioritise === '1')
    .map(cb => cb.dataset.id);

  if (pendingIds.length === 0) {
    showToast('Select at least one pending task.', 'warning');
    return;
  }

  let count = 0;
  for (const id of pendingIds.reverse()) {
    try {
      await tasks.prioritise(id);
      count++;
    } catch(_) {}
  }

  if (count === 0) {
    showToast('No selected tasks could be prioritised.', 'warning');
    return;
  }

  showToast(`Moved ${count} task${count === 1 ? '' : 's'} to the front`);
  scheduleQueueRefresh(0);
};

window.cancelTask = async function(taskId) {
  try {
    await tasks.cancel(taskId);
    selectedTaskIds.delete(taskId);
    showToast('Task cancelled');
    scheduleQueueRefresh(0);
  } catch(e) {
    showToast('Cancel failed: ' + e.message, 'error');
  }
};

window.prioritiseTask = async function(taskId) {
  try {
    await tasks.prioritise(taskId);
    showToast('Task moved to the front of the queue');
    scheduleQueueRefresh(0);
  } catch(e) {
    showToast('Prioritise failed: ' + e.message, 'error');
  }
};

window.toggleCancelledGroup = function(groupIndex) {
  if (expandedGroups.has(groupIndex)) {
    expandedGroups.delete(groupIndex);
  } else {
    expandedGroups.add(groupIndex);
  }
  scheduleQueueRefresh(0);
};

window.jumpToActive = function() {
  const activeRow = document.querySelector('.task-active-row');
  if (activeRow) {
    activeRow.scrollIntoView({ behavior: 'smooth', block: 'center' });
    activeRow.classList.add('task-highlight');
    setTimeout(() => activeRow.classList.remove('task-highlight'), 3000);
  } else {
    showToast('No active task found', 'warning');
  }
};

window.clearQueueSeriesFilter = function() {
  navigate('/queue');
};

window.viewQueue = viewQueue;
