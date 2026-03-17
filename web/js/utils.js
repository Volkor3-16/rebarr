// Utility functions used across the app

/**
 * Escape HTML special characters
 * @param {string|null} s - String to escape
 * @returns {string} Escaped string
 */
export function escape(s) {
  if (s == null) return '';
  return String(s)
    .replace(/&/g, '&')
    .replace(/</g, '<')
    .replace(/>/g, '>')
    .replace(/"/g, '"')
    .replace(/'/g, '&#039;');
}

/**
 * Get CSS class for status badge
 * @param {string} s - Status string
 * @returns {string} CSS class name
 */
export function statusBadgeClass(s) {
  const classes = {
    'Missing': 'st-missing',
    'Queued': 'st-queued',
    'Downloading': 'st-downloading',
    'Downloaded': 'st-downloaded',
    'Failed': 'st-failed'
  };
  return classes[s] || 'st-missing';
}

/**
 * Render status badge HTML with icon
 * @param {string} s - Status string
 * @returns {string} HTML span element
 */
export function statusBadge(s) {
  const icons = {
    'Missing': 'mdi:book-remove',
    'Queued': 'mdi:book-clock',
    'Downloading': 'mdi:book-arrow-down',
    'Downloaded': 'mdi:book-check',
    'Failed': 'mdi:book-alert'
  };
  const icon = icons[s] || 'mdi:book-remove';
  const cls = statusBadgeClass(s);
  return `<span class="status-icon ${cls.replace('st-', '')}" title="${escape(s)}"><iconify-icon icon="${icon}" width="20" height="20"></iconify-icon></span>`;
}

/**
 * Render task badge HTML
 * @param {string} s - Task status
 * @returns {string} HTML span element
 */
export function taskBadge(s) {
  const classes = {
    'Pending': 'task-pending',
    'Running': 'task-running',
    'Completed': 'task-completed',
    'Failed': 'task-failed',
    'Cancelled': 'task-cancelled'
  };
  const cls = classes[s] || 'task-pending';
  return `<span class="${cls}">${escape(s)}</span>`;
}

/**
 * Format unix timestamp (seconds) to relative time
 * @param {number|null} ts - Unix timestamp in seconds
 * @returns {string} Relative time string
 */
export function relTime(ts) {
  if (!ts) return '—';
  const now = Math.floor(Date.now() / 1000);
  const diff = now - ts;
  const title = new Date(ts * 1000).toLocaleString();
  
  if (diff < 60) return `<span class="rel-time" data-ts="${ts}" title="${title}">just now</span>`;
  if (diff < 3600) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 60)}m ago</span>`;
  if (diff < 86400) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 3600)}h ago</span>`;
  if (diff < 2592000) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 86400)}d ago</span>`;
  if (diff < 31536000) return `<span class="rel-time" data-ts="${ts}" title="${title}">${Math.floor(diff / 2592000)}mo ago</span>`;
  return `<span class="rel-time" data-ts="${ts}" title="${title}">${new Date(ts * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short' })}</span>`;
}

/**
 * Convert ISO date string to relative time
 * @param {string|null} str - ISO date string
 * @returns {string} Relative time string
 */
export function relDate(str) {
  if (!str) return '—';
  const d = new Date(str);
  if (isNaN(d)) return '—';
  const now = new Date();
  const diffDays = Math.floor((now - d) / 86400000);
  const title = escape(d.toLocaleString());
  if (diffDays === 0) return `<span title="${title}">today</span>`;
  if (diffDays < 30) return `<span title="${title}">${diffDays}d ago</span>`;
  if (diffDays < 365) return `<span title="${title}">${Math.floor(diffDays / 30)}mo ago</span>`;
  return `<span title="${title}">${d.toLocaleDateString(undefined, { year: 'numeric', month: 'short' })}</span>`;
}

/**
 * Update all .rel-time spans to show fresh relative times
 */
export function updateRelTimes() {
  document.querySelectorAll('.rel-time').forEach(el => {
    const ts = parseInt(el.dataset.ts, 10);
    if (ts) {
      const now = Math.floor(Date.now() / 1000);
      const diff = now - ts;
      let text;
      if (diff < 60) text = 'just now';
      else if (diff < 3600) text = Math.floor(diff / 60) + 'm ago';
      else if (diff < 86400) text = Math.floor(diff / 3600) + 'h ago';
      else if (diff < 2592000) text = Math.floor(diff / 86400) + 'd ago';
      else if (diff < 31536000) text = Math.floor(diff / 2592000) + 'mo ago';
      else text = new Date(ts * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short' });
      if (el.textContent !== text) el.textContent = text;
    }
  });
}

/**
 * Convert string to path-safe version
 * @param {string|null} s - String to convert
 * @returns {string} Path-safe string
 */
export function toPathSafe(s) {
  return (s || '').replace(/[\/\\:*?"<>|']/g, '').replace(/\s+/g, ' ').trim() || 'manga';
}

/**
 * Render score badge HTML (formerly tier)
 * @param {number|null} tier - Tier number (1-4), lower is better
 * @returns {string} HTML span element
 */
export function tierBadgeHtml(tier) {
  // Show score number (inverted: 1 = best, 4 = worst)
  const score = tier ? (5 - tier) : 0;
  return `<span class="ch-tier ch-tier-${tier || 4}">${score}</span>`;
}

/**
 * Show a toast notification
 * @param {string} message - Toast message
 * @param {string} type - Toast type: 'success', 'error', 'warning'
 * @param {number} duration - Duration in ms
 */
export function showToast(message, type = 'success', duration = 3000) {
  const container = document.getElementById('toast-container');
  if (!container) return;
  
  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  
  container.appendChild(toast);
  
  setTimeout(() => {
    toast.style.animation = 'slideIn 0.3s ease reverse';
    setTimeout(() => toast.remove(), 300);
  }, duration);
}

/**
 * Confirm dialog wrapper
 * @param {string} message - Confirmation message
 * @returns {boolean} User's choice
 */
export function confirm(message) {
  return window.confirm(message);
}

/**
 * Render loading skeleton
 * @param {number} count - Number of skeleton items
 * @returns {string} HTML string
 */
export function skeleton(count = 3) {
  return Array(count).fill('<div class="skeleton" style="height: 20px; margin: 10px 0;"></div>').join('');
}

/**
 * Render empty state
 * @param {string} title - Empty state title
 * @param {string} message - Empty state message
 * @param {string|null} action - Optional action button HTML
 * @returns {string} HTML string
 */
export function emptyState(title, message, action = null) {
  return `
    <div class="empty-state">
      <h3>${escape(title)}</h3>
      <p>${escape(message)}</p>
      ${action ? `<div class="mt-2">${action}</div>` : ''}
    </div>
  `;
}
