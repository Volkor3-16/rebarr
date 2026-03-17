// Theme management - handles dark/light mode switching

const THEME_KEY = 'rebarr-theme';

/**
 * Get the preferred theme from localStorage or system preference
 */
export function getPreferredTheme() {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved && saved !== 'auto') return saved;
  // No saved preference, use system preference
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/**
 * Apply theme to the document
 * @param {string} theme - 'dark' or 'light'
 */
export function applyTheme(theme) {
  if (theme === 'dark') {
    document.documentElement.setAttribute('data-theme', 'dark');
  } else {
    document.documentElement.removeAttribute('data-theme');
  }
  updateThemeButton();
}

/**
 * Update the theme toggle button icon
 */
export function updateThemeButton() {
  const btn = document.getElementById('theme-toggle');
  if (!btn) return;
  const current = document.documentElement.getAttribute('data-theme');
  // When dark mode is active, show "lightbulb-on" to switch to light mode
  // When light mode is active, show "lightbulb-night" to switch to dark mode
  const icon = current === 'dark' ? 'mdi:lightbulb-on' : 'mdi:moon-waxing-crescent';
  btn.innerHTML = `<iconify-icon icon="${icon}" width="24" height="24"></iconify-icon>`;
  btn.title = current === 'dark' ? 'Switch to light mode' : 'Switch to dark mode';
}

/**
 * Cycle through themes: light -> dark -> light
 */
export function cycleTheme() {
  const current = document.documentElement.getAttribute('data-theme');
  let next;
  if (current === 'dark') {
    next = 'light';
  } else {
    next = 'dark';
  }
  localStorage.setItem(THEME_KEY, next);
  applyTheme(next);
}

/**
 * Initialize theme on page load
 */
export function initTheme() {
  const theme = getPreferredTheme();
  applyTheme(theme);
  
  // Listen for system theme changes
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', e => {
    if (!localStorage.getItem(THEME_KEY) || localStorage.getItem(THEME_KEY) === 'auto') {
      applyTheme(e.matches ? 'dark' : 'light');
    }
  });
}

// Make cycleTheme available globally for the onclick handler in HTML
window.cycleTheme = cycleTheme;
