/**
 * Workers Dashboard View
 * Shows all provider queues, workers, rate limits, and task progress.
 * Auto-refreshes every 3 seconds.
 */

import { tasks } from '../api.js';
import { render } from '../router.js';
import * as sse from '../events.js';

function escapeHtml(s) {
    if (!s) return '';
    const replacements = {
        38: 'amp',
        60: 'lt',
        62: 'gt',
        34: 'quot',
        39: '#39'
    };
    return String(s).replace(/./g, function(c) {
        const code = c.charCodeAt(0);
        return replacements[code] ? '&' + replacements[code] + ';' : c;
    });
}

function timeAgo(dateStr) {
    if (!dateStr) return '';
    const diff = (Date.now() - new Date(dateStr).getTime()) / 1000;
    if (diff < 60) return `${Math.max(0, Math.floor(diff))}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
}

function statusIcon(status) {
    switch (status) {
        case 'Running': return '\u{1f7e2}';
        case 'Pending': return '\u23f8';
        case 'Failed':  return '\u{1f534}';
        case 'Completed': return '\u2705';
        default: return '\u26aa';
    }
}

function progressBar(task) {
    if (!task.progress || !task.progress.total) return '';
    const pct = Math.min(100, Math.round((task.progress.current / task.progress.total) * 100));
    const label = task.progress.label || '';
    return `<div class="progress-bar-wrap" title="${escapeHtml(label)}">
        <div class="progress-bar" style="width:${pct}%">${pct}%</div>
    </div>
    ${task.progress.detail ? `<div class="progress-detail text-muted">${escapeHtml(task.progress.detail)}</div>` : ''}`;
}

function taskRow(task) {
    const mangaLink = task.manga_id
        ? `<a href="/series/${task.manga_id}">${escapeHtml(task.manga_title || task.manga_id)}</a>`
        : escapeHtml(task.manga_title || '\u2014');
    const chapterLabel = task.chapter_number_raw ? `Ch. ${task.chapter_number_raw}` : '';
    const errorInfo = task.last_error
        ? `<div class="error small mt-1">Error: ${escapeHtml(task.last_error)}</div>` : '';

    return `<div class="task-row p-2" data-id="${task.id}">
        <div class="flex justify-between items-start gap-1">
            <div class="flex items-start gap-2 flex-grow-1">
                <span class="flex-shrink-0">${statusIcon(task.status)}</span>
                <div class="flex-grow-1">
                    <div class="queue-task-main">
                        ${mangaLink}
                        ${chapterLabel ? `<span class="text-muted small ms-1">${chapterLabel}</span>` : ''}
                    </div>
                    <div class="queue-task-meta mt-05">
                        ${task.task_type} &bull; ${task.attempt}/${task.max_attempts} attempts &bull; ${timeAgo(task.updated_at)}
                    </div>
                </div>
            </div>
            ${task.status === 'Running' || task.status === 'Pending'
                ? `<button class="btn btn-sm btn-danger cancel-btn flex-shrink-0" data-id="${task.id}">\u2715</button>`
                : ''}
        </div>
        ${progressBar(task)}
        ${errorInfo}
    </div>`;
}

function queueSection(queue) {
    const qid = queue.display_name.replace(/[^a-zA-Z0-9]/g, '-');
    const totalRunning = queue.running_count;
    const totalPending = queue.pending_count;
    const hasTasks = totalRunning > 0 || totalPending > 0;
    const isDisabled = queue.is_provider && queue.disabled;
    const isRateLimited = queue.is_provider && queue.rate_limited;
    const isUnhealthy = isDisabled || isRateLimited;

    let statusBadge = '';
    let statusMessage = '';

    if (isDisabled) {
        statusBadge = '<span class="badge bg-secondary">Disabled</span>';
        statusMessage = '<div class="small text-muted">Provider is manually disabled</div>';
    } else if (isRateLimited) {
        statusBadge = '<span class="badge bg-danger">Rate Limited</span>';
        const resetSeconds = Math.max(0, Math.ceil((new Date(queue.rate_limit_resets_at) - Date.now()) / 1000));
        statusMessage = `<div class="small text-muted">Rate limited, resets in ${resetSeconds}s</div>`;
    }

    let extraClasses = [];
    if (hasTasks) extraClasses.push('has-tasks');
    if (isUnhealthy) extraClasses.push('disabled');

    return `<div class="queue-box ${extraClasses.join(' ')}">
        <div class="queue-header"
             data-bs-toggle="collapse" data-bs-target="#queue-${qid}"
             style="cursor:pointer">
            <div class="queue-name">${escapeHtml(queue.display_name)}</div>
            <div class="queue-badges">
                ${statusBadge}
                ${totalRunning > 0 ? `<span class="badge bg-success">${totalRunning}</span>` : ''}
                ${totalPending > 0 ? `<span class="badge bg-warning text-dark">${totalPending}</span>` : ''}
                ${!hasTasks && !isUnhealthy ? '<span class="badge bg-light text-muted">idle</span>' : ''}
            </div>
        </div>
        <div id="queue-${qid}" class="collapse ${hasTasks && !isUnhealthy ? 'show' : ''}">
            <div class="queue-body">
                ${isUnhealthy ? statusMessage : ''}
                ${queue.tasks.length > 0 ? `
                <div class="task-list">
                    ${queue.tasks.map(taskRow).join('')}
                </div>
                ` : `<div class="p-2 text-muted small">${isUnhealthy ? '' : 'Queue is idle'}</div>`}
            </div>
        </div>
    </div>`;
}

async function refreshQueues() {
    const listEl = document.getElementById('queues-list');
    if (!listEl) return;

    try {
        const queues = await tasks.listGrouped();
        // Sort: queues with tasks first, then by name
        queues.sort((a, b) => {
            const aHasTasks = a.running_count > 0 || a.pending_count > 0;
            const bHasTasks = b.running_count > 0 || b.pending_count > 0;
            if (aHasTasks && !bHasTasks) return -1;
            if (!aHasTasks && bHasTasks) return 1;
            return a.display_name.localeCompare(b.display_name);
        });
        listEl.innerHTML = queues.length
            ? `<div class="queue-grid">${queues.map(queueSection).join('')}</div>`
            : '<div class="alert alert-info">No queues configured.</div>';

        const totalRunning = queues.reduce((sum, q) => sum + q.running_count, 0);
        const totalPending = queues.reduce((sum, q) => sum + q.pending_count, 0);
        document.title = `[${totalRunning} - ${totalPending}] REBARR`;

        // Re-attach cancel handlers
        listEl.querySelectorAll('.cancel-btn').forEach(btn => {
            btn.addEventListener('click', async (e) => {
                const id = e.currentTarget.dataset.id;
                if (confirm('Cancel this task?')) {
                    try {
                        await tasks.cancel(id);
                    } catch (err) {
                        console.error('Failed to cancel:', err);
                    }
                }
            });
        });
    } catch (e) {
        console.error('Failed to refresh queues:', e);
    }
}

let sseHandler = null;

export function view() {
    // Render initial HTML structure
    render(`
        <h2>Workers</h2>
        <div id="queues-list">
            <div class="spinner"></div>
        </div>
    `);
    
    // Initial load
    refreshQueues();
    
    // Use SSE for live updates - register AFTER router has finished clearing listeners
    setTimeout(() => {
        sseHandler = () => refreshQueues();
        sse.on('task_update', sseHandler);
    }, 0);
}

// Register as window function for router
window.viewWorkers = view;