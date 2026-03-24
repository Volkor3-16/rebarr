import { system } from '../api.js';
import { render } from '../router.js';
import { escape } from '../utils.js';

function iframeSrc(viewOnly) {
  const params = new URLSearchParams({
    autoconnect: '1',
    // x11vnc usually doesn't support dynamic remote resize reliably.
    // Use client-side scaling so the full desktop fits the iframe.
    resize: 'scale',
    view_only: viewOnly ? '1' : '0',
    path: '/desktop/vnc/websockify',
  });
  return `/desktop/vnc/vnc.html?${params.toString()}`;
}

function statusBadge(ok) {
  return ok
    ? '<span class="badge badge-success">Connected</span>'
    : '<span class="badge badge-error">Disconnected</span>';
}

export async function viewDesktop() {
  render('<p>Loading desktop...</p>');

  let health = { xvfb: false, vnc: false, novnc: false };
  try {
    health = await system.desktop();
  } catch (_) {
    // Keep defaults; page still renders for troubleshooting.
  }

  const stackOk = !!(health.xvfb && health.vnc && health.novnc);

  render(`
    <h2>Desktop</h2>
    <div class="settings-card" style="margin-bottom:1rem">
      <div class="settings-card-header">
        <iconify-icon icon="mdi:monitor-eye" width="20" height="20"></iconify-icon>
        <h3>Virtual Desktop</h3>
      </div>
      <p class="settings-card-desc">View-only by default. Unlock controls only when manual interaction is needed.</p>
      <div id="desktop-status" style="display:flex;gap:0.5rem;align-items:center;flex-wrap:wrap">
        <strong>Stack:</strong> ${statusBadge(stackOk)}
        <span id="viewer-status" class="badge badge-warning">Viewer: Loading</span>
        <span class="badge ${health.xvfb ? 'badge-success' : 'badge-error'}">Xvfb: ${escape(String(health.xvfb))}</span>
        <span class="badge ${health.vnc ? 'badge-success' : 'badge-error'}">VNC: ${escape(String(health.vnc))}</span>
        <span class="badge ${health.novnc ? 'badge-success' : 'badge-error'}">noVNC: ${escape(String(health.novnc))}</span>
      </div>
      <div style="margin-top:0.75rem;display:flex;gap:0.5rem;align-items:center;flex-wrap:wrap">
        <button id="desktop-toggle" class="btn btn-sm btn-warning">Unlock Controls</button>
        <button id="desktop-reload" class="btn btn-sm">Reload Viewer</button>
        <small id="desktop-mode-label">Mode: view-only</small>
      </div>
    </div>

    <div class="settings-card" style="padding:0.5rem">
      <iframe
        id="desktop-frame"
        title="Rebarr virtual desktop"
        src="${iframeSrc(true)}"
        style="width:100%;height:82vh;border:1px solid var(--border);border-radius:var(--radius);background:#111"
      ></iframe>
    </div>
  `);

  const frame = document.getElementById('desktop-frame');
  const toggle = document.getElementById('desktop-toggle');
  const reload = document.getElementById('desktop-reload');
  const modeLabel = document.getElementById('desktop-mode-label');
  const viewerStatus = document.getElementById('viewer-status');

  let viewOnly = true;

  const refreshFrame = () => {
    viewerStatus.className = 'badge badge-warning';
    viewerStatus.textContent = 'Viewer: Loading';
    frame.src = iframeSrc(viewOnly);
    modeLabel.textContent = `Mode: ${viewOnly ? 'view-only' : 'interactive'}`;
    toggle.textContent = viewOnly ? 'Unlock Controls' : 'Lock to View-Only';
    toggle.classList.toggle('btn-warning', viewOnly);
    toggle.classList.toggle('btn-success', !viewOnly);
  };

  toggle.addEventListener('click', () => {
    viewOnly = !viewOnly;
    refreshFrame();
  });

  reload.addEventListener('click', () => refreshFrame());

  frame.addEventListener('load', () => {
    viewerStatus.className = 'badge badge-success';
    viewerStatus.textContent = 'Viewer: Connected';
  });

  frame.addEventListener('error', () => {
    viewerStatus.className = 'badge badge-error';
    viewerStatus.textContent = 'Viewer: Disconnected';
  });
}

window.viewDesktop = viewDesktop;
