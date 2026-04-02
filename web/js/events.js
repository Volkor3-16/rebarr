// Server-Sent Events client for real-time task updates

let eventSource = null;
const listeners = new Map(); // eventType -> Set<callback>

/**
 * Connect to the SSE endpoint and start dispatching events.
 * Safe to call multiple times — will reconnect only if needed.
 */
export function connect() {
  if (eventSource && eventSource.readyState !== EventSource.CLOSED) return;

  eventSource = new EventSource('/api/events');

  eventSource.onmessage = (e) => {
    // Ignore heartbeats / control messages
    if (e.data === 'ping' || e.data === 'reconnect') return;

    try {
      const data = JSON.parse(e.data);
      // All events have a `status` field — use it as the event type
      const eventType = data.status || 'unknown';
      dispatch(eventType, data);
      dispatch('task_update', data); // catch-all for any task change
    } catch {
      // ignore parse errors
    }
  };

  eventSource.onerror = () => {
    // EventSource auto-reconnects, but we log for visibility
    console.warn('[SSE] Connection lost, browser will reconnect...');
  };
}

/**
 * Disconnect and clean up.
 */
export function disconnect() {
  if (eventSource) {
    eventSource.close();
    eventSource = null;
  }
}

/**
 * Remove all registered listeners.  Call this when navigating away from
 * a view so old handlers don't fire after the view is gone.
 */
export function clearListeners() {
  listeners.clear();
}

/**
 * Subscribe to a specific event type.
 * @param {string} eventType - e.g. 'Running', 'Completed', 'Failed', 'task_update'
 * @param {Function} callback
 */
export function on(eventType, callback) {
  if (!listeners.has(eventType)) {
    listeners.set(eventType, new Set());
  }
  listeners.get(eventType).add(callback);
}

/**
 * Unsubscribe from an event type.
 */
export function off(eventType, callback) {
  const set = listeners.get(eventType);
  if (set) {
    set.delete(callback);
  }
}

/**
 * Dispatch an event to all registered listeners.
 */
function dispatch(eventType, data) {
  const set = listeners.get(eventType);
  if (set) {
    for (const cb of set) {
      try {
        cb(data);
      } catch {
        // don't let one listener break others
      }
    }
  }
}

// Auto-connect on module load
connect();