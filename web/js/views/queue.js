// Queue view - task history + active queue with live polling

import { tasks, settings } from '../api.js';
import { render, setPoll } from '../router.js';
import { escape, taskBadge, relTime, skeleton, showToast } from '../utils.js';

export async function viewQueue() {
  render(`
    <h2>Queue</h2>
    <div id="queue-controls">
      <div class="spinner"></div>
    </div>
    <div id="queue-list"></div>
  `);
  
  await refreshQueue();
  setPoll(refreshQueue, 3000);
}

async function refreshQueue() {
  const listEl = document.getElementById('queue-list');
  const ctrlEl = document.getElementById('queue-controls');
  if (!listEl || !ctrlEl) return;
  
  try {
    const [taskList, appSettings] = await Promise.all([
      tasks.list(),
      settings.get(),
    ]);
    
    const paused = appSettings.queue_paused;
    const pauseLabel = paused ? '<span class="iconify" data-icon="mdi-play"></span> Resume Queue' : '<span class="iconify" data-icon="mdi-pause"></span> Pause Queue';
    const pauseStyle = paused ? 'color:#c70;font-weight:bold' : '';
    
    ctrlEl.innerHTML = `
      <button onclick="toggleQueuePause(${paused})" style="${pauseStyle}">${pauseLabel}</button>
      <button class="btn-sm btn-danger" onclick="cancelSelected()">Cancel Selected</button>
      ${paused ? '<span style="color:#c70;margin-left:0.8rem"><b>Queue paused — no new tasks will run.</b></span>' : ''}
    `;
    
    if (taskList.length === 0) {
      listEl.innerHTML = '<p>No tasks yet.</p>';
      return;
    }
    
    const rows = taskList.map(t => {
      const ts = new Date(t.created_at).toLocaleString();
      
      // Build task description similar to app.js
      let taskDesc = escape(t.task_type);
      if (t.manga_title) {
        taskDesc += ` ${escape(t.manga_title)}`;
      }
      if (t.chapter_number_raw && (t.task_type === 'DownloadChapter' || t.task_type === 'CheckNewChapter')) {
        taskDesc += ` <small style="color:#888">(Ch. ${escape(t.chapter_number_raw)})</small>`;
      }
      
      const err = t.last_error ? `<br><small class="error">${escape(t.last_error)}</small>` : '';
      const canCancel = t.status === 'Pending' || t.status === 'Running';
      const cb = canCancel ? `<input type="checkbox" class="task-cb" data-id="${t.id}">` : '';
      const cancelBtn = canCancel
        ? `<button class="btn-sm btn-danger" onclick='cancelTask("${t.id}")'>Cancel</button>`
        : '';
      
      return `
        <tr>
          <td>${cb}</td>
          <td><small>${escape(ts)}</small></td>
          <td>${taskDesc}</td>
          <td>${taskBadge(t.status)}${err}</td>
          <td>${cancelBtn}</td>
        </tr>
      `;
    }).join('');
    
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
  } catch(e) {
    if (listEl) listEl.innerHTML = `<p class="error">Error: ${escape(e.message)}</p>`;
  }
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
  document.querySelectorAll('.task-cb').forEach(cb => cb.checked = checked);
};

window.cancelSelected = async function() {
  const checked = Array.from(document.querySelectorAll('.task-cb:checked'));
  if (checked.length === 0) { showToast('Select at least one task to cancel.', 'warning'); return; }
  for (const cb of checked) {
    try { 
      await tasks.cancel(cb.dataset.id); 
    } catch(_) {}
  }
  showToast('Cancelled ' + checked.length + ' task(s)');
  refreshQueue();
};

window.cancelTask = async function(taskId) {
  try {
    await tasks.cancel(taskId);
    showToast('Task cancelled');
    refreshQueue();
  } catch(e) {
    showToast('Cancel failed: ' + e.message, 'error');
  }
};

window.viewQueue = viewQueue;
