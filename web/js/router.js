// Client-side router

import * as sse from './events.js';

// Route definitions: [pattern, viewFunction]
// Views are attached to window by their respective modules
const routes = [
  [/^\/$/, 'viewHome'],
  [/^\/library$/, 'viewLibraries'],
  [/^\/series\/([^/]+)$/, 'viewSeries'],
  [/^\/search$/, 'viewSearch'],
  [/^\/settings$/, 'viewSettings'],
  [/^\/desktop$/, 'viewDesktop'],
  [/^\/queue$/, 'viewQueue'],
  [/^\/logs$/, 'viewQueue'], // Logs uses queue view
  [/^\/workers$/, 'viewWorkers'],
  [/^\/import$/, 'viewImport'],
  [/^\/suggested$/, 'viewSuggested'],
  [/^\/setup$/, 'viewSetup'],
];

// Current poll handle for live updates
let _pollHandle = null;

/**
 * Stop any active polling and SSE listeners
 */
export function stopPolling() {
  if (_pollHandle) {
    clearInterval(_pollHandle);
    _pollHandle = null;
  }
  sse.clearListeners();
}

/**
 * Set a poll interval
 * @param {Function} fn - Function to call
 * @param {number} interval - Interval in ms
 */
export function setPoll(fn, interval) {
  stopPolling();
  fn(); // Call immediately
  _pollHandle = setInterval(fn, interval);
}

/**
 * Navigate to a path
 * @param {string} path - URL path
 */
export function navigate(path) {
  stopPolling();
  history.pushState({}, '', path);
  dispatch(path);
}

/**
 * Dispatch to the appropriate view handler
 * @param {string} path - URL path
 */
export function dispatch(path) {
  // Update nav active state
  document.querySelectorAll('#nav a').forEach(a => {
    const pathMatch = path === a.dataset.path || 
      (a.dataset.path !== '/' && path.startsWith(a.dataset.path));
    a.classList.toggle('active', pathMatch);
  });

  // Find matching route
  for (const [pat, viewName] of routes) {
    const m = path.match(pat);
    if (m) {
      const viewFn = window[viewName];
      if (viewFn) {
        viewFn(...m.slice(1));
      } else {
        render('<p class="error">View not loaded yet.</p>');
      }
      return;
    }
  }

  // 404
  render('<p class="error">404 — page not found</p>');
}

/**
 * Render HTML to the main content area
 * @param {string} html - HTML to render
 */
export function render(html) {
  const content = document.getElementById('content');
  if (content) {
    content.innerHTML = html;
  }
}

/**
 * Initialize the router
 */
export function initRouter() {
  // Handle browser back/forward
  window.addEventListener('popstate', () => {
    stopPolling();
    dispatch(window.location.pathname);
  });

  // Handle navigation clicks
  document.addEventListener('click', (e) => {
    const link = e.target.closest('a[data-path]');
    if (link && !link.target) {
      e.preventDefault();
      const path = link.dataset.path;
      navigate(path);
    }
  });

  // Initial dispatch
  dispatch(window.location.pathname);
}

// Make navigate available globally for onclick handlers
window.navigate = navigate;
