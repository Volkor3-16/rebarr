// REBARR - Main Application Entry Point

// Import views to attach them to window
import './views/home.js';
import './views/library.js';
import './views/series.js';
import './views/search.js';
import './views/settings.js';
import './views/queue.js';
import './views/import.js';
import './views/desktop.js';
import { showWizard } from './views/wizard.js';

import { initTheme } from './theme.js';
import { initRouter, navigate } from './router.js';
import { updateRelTimes } from './utils.js';
import { tasks, settings, system } from './api.js';

// Activity console state
let activityLog = [];
const MAX_ACTIVITY_ENTRIES = 20;

// Track task states to detect changes
let taskStates = {}; // { [taskId]: { status, manga_title, chapter_number_raw, task_type } }
let lastPendingCount = 0;
let isFirstPoll = true;

// Header scroll state
let lastScrollY = 0;
let headerCollapsed = false;

// Format task description with manga title and chapter
function formatTaskDescription(task) {
  const parts = [];
  
  // Add manga title if available
  if (task.manga_title) {
    parts.push(task.manga_title);
  }
  
  // Add chapter number for chapter-specific tasks
  if (task.chapter_number_raw && (task.task_type === 'DownloadChapter' || task.task_type === 'CheckNewChapter')) {
    parts.push(`(Ch. ${task.chapter_number_raw})`);
  }
  
  return parts.join(' ');
}

// Add entry to activity console
function addActivity(message, type = 'info') {
  const now = new Date();
  const time = now.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false });
  
  activityLog.unshift({ time, message, type });
  if (activityLog.length > MAX_ACTIVITY_ENTRIES) {
    activityLog.pop();
  }
  
  renderActivityConsole();
}

// Render activity console (full version for expanded header)
function renderActivityConsole() {
  const container = document.getElementById('console-entries');
  if (!container) return;
  
  if (activityLog.length === 0) {
    container.innerHTML = '<div class="console-empty">No recent activity</div>';
    return;
  }
  
  container.innerHTML = activityLog.map(entry => `
    <div class="console-entry ${entry.type}">
      <span class="console-time">${entry.time}</span>
      <span class="console-message">${entry.message}</span>
    </div>
  `).join('');
  
  // Also update mini ticker
  renderMiniTicker();
}

// Render mini ticker for collapsed header
function renderMiniTicker() {
  const ticker = document.getElementById('mini-ticker');
  if (!ticker) return;
  
  if (activityLog.length === 0) {
    ticker.innerHTML = '<span class="console-empty">No recent activity</span>';
    return;
  }
  
  // Show latest 2 entries in mini ticker
  const recent = activityLog.slice(0, 2);
  ticker.innerHTML = recent.map(entry => `
    <span class="mini-ticker-entry ${entry.type}">
      <span class="console-time">${entry.time}</span>
      <span class="console-message">${entry.message}</span>
    </span>
  `).join('<span class="ticker-sep">|</span>');
}

// Handle scroll for header collapse/expand
function handleScroll() {
  const currentScrollY = window.scrollY;
  const headerContainer = document.querySelector('.header-container');
  const nav = document.getElementById('nav');
  
  if (!headerContainer || !nav) return;
  
  // Collapse when scrolled down more than 50px, expand when near top
  if (currentScrollY > 50 && !headerCollapsed) {
    headerCollapsed = true;
    headerContainer.classList.add('collapsed');
    nav.classList.add('header-collapsed');
  } else if (currentScrollY <= 50 && headerCollapsed) {
    headerCollapsed = false;
    headerContainer.classList.remove('collapsed');
    nav.classList.remove('header-collapsed');
  }
  
  lastScrollY = currentScrollY;
}

// Poll for active tasks and queue status - live event stream
async function pollActivity() {
  try {
    // Fetch more tasks to track state changes (recent + pending)
    const [taskList, appSettings] = await Promise.all([
      tasks.list({ limit: 20 }),
      settings.get()
    ]);
    
    // Process each task and detect state changes.
    // On first poll: iterate oldest-first so history appears newest-at-top after unshift.
    // On subsequent polls: API order is fine (usually newest-first, one event at a time).
    const orderedTasks = isFirstPoll ? [...taskList].reverse() : taskList;
    const currentTaskIds = new Set();

    for (const task of orderedTasks) {
      currentTaskIds.add(task.id);
      const prevState = taskStates[task.id];
      const taskDesc = formatTaskDescription(task);

      // New task appeared
      if (!prevState) {
        if (task.status === 'Pending') {
          addActivity(`${task.task_type}: ${taskDesc} pending`, 'info');
        } else if (task.status === 'Running') {
          addActivity(`${task.task_type}: ${taskDesc} started`, 'info');
        } else if (task.status === 'Completed') {
          addActivity(`${task.task_type}: ${taskDesc} completed`, 'success');
        } else if (task.status === 'Failed') {
          const err = task.last_error ? ` - ${task.last_error}` : '';
          addActivity(`${task.task_type}: ${taskDesc} failed${err}`, 'error');
        }
      } else {
        // Task status changed
        if (prevState.status !== task.status) {
          if (task.status === 'Completed') {
            addActivity(`${task.task_type}: ${taskDesc} completed`, 'success');
          } else if (task.status === 'Failed') {
            const err = task.last_error ? ` - ${task.last_error}` : '';
            addActivity(`${task.task_type}: ${taskDesc} failed${err}`, 'error');
          } else if (task.status === 'Cancelled') {
            addActivity(`${task.task_type}: ${taskDesc} cancelled`, 'warning');
          } else if (task.status === 'Running' && prevState.status === 'Pending') {
            addActivity(`${task.task_type}: ${taskDesc} started`, 'info');
          }
        }
      }
      
      // Update state
      taskStates[task.id] = {
        status: task.status,
        manga_title: task.manga_title,
        chapter_number_raw: task.chapter_number_raw,
        task_type: task.task_type
      };
    }
    
    // Clean up old tasks that are no longer in the list
    for (const taskId of Object.keys(taskStates)) {
      if (!currentTaskIds.has(taskId)) {
        delete taskStates[taskId];
      }
    }
    
    // Show pending queue overflow if pending count changed
    const pendingTasks = taskList.filter(t => t.status === 'Pending');
    const pendingCount = pendingTasks.length;
    if (pendingCount !== lastPendingCount) {
      if (pendingCount > 1) {
        addActivity(`+${pendingCount - 1} items pending in Task Queue`, 'info');
      }
      lastPendingCount = pendingCount;
    }
    
    // Check for queue paused
    if (appSettings.queue_paused) {
      addActivity('Queue is paused', 'warning');
    }

    isFirstPoll = false;
  } catch(e) {
    // Silently fail - activity console is non-critical
  }
}

// Update system stats in header
async function updateSystemStats() {
  const textEl = document.getElementById('system-stats-text');
  if (!textEl) return;
  try {
    const info = await system.info();
    const mb = info.process_mem_mb;
    const memText = mb > 0
      ? (mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`)
      : '';
    const queueText = info.tasks_pending > 0 || info.tasks_running > 0
      ? [
          info.tasks_running > 0 ? `${info.tasks_running} running` : '',
          info.tasks_pending > 0 ? `${info.tasks_pending} queued` : '',
        ].filter(Boolean).join(', ')
      : 'idle';
    textEl.textContent = [memText, queueText].filter(Boolean).join(' | ');
  } catch (_) {
    // Non-critical
  }
}

// Initialize the application
async function init() {
  // Initialize theme
  initTheme();

  // Initialize router (which will load the initial view)
  initRouter();

  // Show first-run wizard if setup has not been completed.
  // The wizard renders as a full-screen overlay above the initial route.
  try {
    const appSettings = await settings.get();
    if (!appSettings.wizard_completed) {
      await new Promise(resolve => {
        showWizard((goImport) => {
          resolve();
          if (goImport) navigate('/import');
        });
      });
    }
  } catch (_) {
    // Non-critical — proceed without wizard if the settings fetch fails.
  }

  // Start the relative time updater (updates every 30 seconds)
  setInterval(updateRelTimes, 30000);

  // Activity console polling runs independently — not through setPoll so navigation
  // can't kill it. Poll every 3s to catch fast tasks.
  pollActivity();
  setInterval(pollActivity, 3000);

  // System stats polling every 30s
  updateSystemStats();
  setInterval(updateSystemStats, 30000);

  // Add scroll listener for header collapse
  window.addEventListener('scroll', handleScroll, { passive: true });

  // Add initial activity
  addActivity('REBARR started', 'success');

  console.log('REBARR initialized');
}

// Start the app when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}

// Export for external use
window.addActivity = addActivity;
