// Queue view - task history + active queue with live polling

import { tasks, settings } from '../api.js';
import { render } from '../router.js';
import { escape, taskBadge, renderTaskProgress, showToast } from '../utils.js';
import * as sse from '../events.js';

// Track which cancelled groups are expanded (by index)
const expandedGroups = new Set();

// Track selected task IDs to preserve selection across refreshes
const selectedTaskIds = new Set();

let sseHandler = null;

export async function viewQueue() {
  render(`
    <h2>Queue</h2>
    <div id="queue-controls">
      <div class="spinner"></div>
    </div>
    <div id="queue-list"></div>
  `);
  
  await refreshQueue();
  
  // Use SSE instead of polling — refresh on any task update
  sseHandler = () => refreshQueue();
  sse.on('task_update', sseHandler);
}

async function refreshQueue() {
  const listEl = document.getElementById('queue-list');
  const ctrlEl = document.getElementById('queue-controls');
  if (!listEl || !ctrlEl) return;
  
  // Save current checkbox states before rebuilding
  document.querySelectorAll('.task-cb:checked').forEach(cb => {
    selectedTaskIds.add(cb.dataset.id);
  });
  document.querySelectorAll('.task-cb:not(:checked)').forEach(cb => {
    selectedTaskIds.delete(cb.dataset.id);
  });
  
  try {
    const [taskList, appSettings] = await Promise.all([
      tasks.list(),
      settings.get(),
    ]);
    
    const paused = appSettings.queue_paused;
    const pauseLabel = paused ? '<span class="iconify" data-icon="mdi-play"></span> Resume Queue' : '<span class="iconify" data-icon="mdi-pause"></span> Pause Queue';
    
    // Check if there's a running task for the Jump button
    const hasRunning = taskList.some(t => t.status === 'Running');
    const jumpBtn = hasRunning
      ? `<button class="btn btn-sm btn-primary" onclick="jumpToActive()"><span class="iconify" data-icon="mdi-crosshairs-gps"></span> Jump to Active</button>`
      : '';
    
    ctrlEl.innerHTML = `
      <button class="btn btn-sm ${paused ? 'btn-success' : ''}" onclick="toggleQueuePause(${paused})">${pauseLabel}</button>
      <button class="btn btn-sm btn-error btn-outline" onclick="cancelSelected()">Cancel Selected</button>
      ${jumpBtn}
      ${paused ? '<span class="badge badge-warning">Queue paused — no new tasks will run.</span>' : ''}
    `;
    
    if (taskList.length === 0) {
      listEl.innerHTML = '<p>No tasks yet.</p>';
      return;
    }
    
    // Build rows with cancelled task compaction
    const rows = buildCompactedRows(taskList);
    
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
  }
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
  
  let taskDesc = escape(t.task_type);
  if (t.manga_title) {
    taskDesc += ` ${escape(t.manga_title)}`;
  }
  if (t.chapter_number_raw && (t.task_type === 'DownloadChapter' || t.task_type === 'CheckNewChapter')) {
    taskDesc += ` <small style="color:#888">(Ch. ${escape(t.chapter_number_raw)})</small>`;
  }
  
  const progress = renderTaskProgress(t.progress);
  const err = t.last_error ? `<br><small class="error">${escape(t.last_error)}</small>` : '';
  const canCancel = t.status === 'Pending' || t.status === 'Running';
  const cb = canCancel ? `<input type="checkbox" class="task-cb" data-id="${t.id}">` : '';
  const cancelBtn = canCancel
    ? `<button class="btn btn-xs btn-error btn-outline" onclick='cancelTask("${t.id}")'>Cancel</button>`
    : '';
  const rowId = t.status === 'Running' ? `id="active-task-${t.id}"` : '';
  const highlightClass = t.status === 'Running' ? ' class="task-active-row"' : '';
  
  return `
    <tr${highlightClass} ${rowId}>
      <td>${cb}</td>
      <td><small>${escape(ts)}</small></td>
      <td>${taskDesc}${progress}</td>
      <td>${taskBadge(t.status)}${err}</td>
      <td>${cancelBtn}</td>
    </tr>
  `;
}

window.toggleQueuePause = async function(currentlyPaused) {
  try {
    await settings.update({ queue_paused: !currentlyPaused });
    refreshQueue();
    showToast(currentlyPaused ? 'Queue resumed' : 'Queue paused');
  } catch(e) {
    showToast('Error: ' + e.message, 'error');
  }
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
  refreshQueue();
};

window.cancelTask = async function(taskId) {
  try {
    await tasks.cancel(taskId);
    selectedTaskIds.delete(taskId);
    showToast('Task cancelled');
    refreshQueue();
  } catch(e) {
    showToast('Cancel failed: ' + e.message, 'error');
  }
};

window.toggleCancelledGroup = function(groupIndex) {
  if (expandedGroups.has(groupIndex)) {
    expandedGroups.delete(groupIndex);
  } else {
    expandedGroups.add(groupIndex);
  }
  refreshQueue();
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

window.viewQueue = viewQueue;