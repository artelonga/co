// ===== Dashboard view =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, isOverdue, formatDate, relativeTime } from '../helpers.js';
import { STATUSES, STATUS_LABELS } from '../constants.js';

let _openTaskModal = () => {};

export function injectDashboardCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
}

export function svgVelocityChart(velocity) {
    const W = 380, H = 160, PAD = { top: 10, right: 10, bottom: 36, left: 32 };
    const chartW = W - PAD.left - PAD.right;
    const chartH = H - PAD.top - PAD.bottom;
    const maxCount = Math.max(1, ...velocity.map(v => v.count));
    const barCount = velocity.length || 1;
    const gap = 6;
    const barW = Math.max(4, (chartW - gap * (barCount - 1)) / barCount);

    let bars = '';
    let xLabels = '';
    velocity.forEach((v, i) => {
        const x = PAD.left + i * (barW + gap);
        const barH = Math.max(2, (v.count / maxCount) * chartH);
        const y = PAD.top + chartH - barH;
        bars += `<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${barH.toFixed(1)}" fill="#22c55e" rx="2"/>`;
        if (v.count > 0) {
            bars += `<text x="${(x + barW / 2).toFixed(1)}" y="${(y - 3).toFixed(1)}" text-anchor="middle" font-size="10" fill="#64748b">${v.count}</text>`;
        }
        const label = v.week.slice(-3);
        xLabels += `<text x="${(x + barW / 2).toFixed(1)}" y="${(H - 6).toFixed(1)}" text-anchor="middle" font-size="10" fill="#94a3b8">${esc(label)}</text>`;
    });

    const yTicks = [0, Math.ceil(maxCount / 2), maxCount];
    let yAxis = '';
    yTicks.forEach(t => {
        const y = PAD.top + chartH - (t / maxCount) * chartH;
        yAxis += `<line x1="${PAD.left - 4}" y1="${y.toFixed(1)}" x2="${W - PAD.right}" y2="${y.toFixed(1)}" stroke="#e2e8f0" stroke-width="1"/>`;
        yAxis += `<text x="${(PAD.left - 6).toFixed(1)}" y="${(y + 3).toFixed(1)}" text-anchor="end" font-size="9" fill="#94a3b8">${t}</text>`;
    });

    if (velocity.length === 0) {
        return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block"><text x="${W / 2}" y="${H / 2}" text-anchor="middle" font-size="13" fill="#94a3b8">No completions in last 8 weeks</text></svg>`;
    }

    return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block">${yAxis}${bars}${xLabels}</svg>`;
}

export function svgBurndownChart(burndown) {
    const W = 380, H = 160, PAD = { top: 24, right: 10, bottom: 36, left: 36 };
    const chartW = W - PAD.left - PAD.right;
    const chartH = H - PAD.top - PAD.bottom;
    const n = burndown.length;
    if (n === 0) {
        return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block"><text x="${W / 2}" y="${H / 2}" text-anchor="middle" font-size="13" fill="#94a3b8">No data</text></svg>`;
    }

    const maxVal = Math.max(1, ...burndown.map(p => Math.max(p.remaining, p.completed)));
    const px = (i) => PAD.left + (n > 1 ? (i / (n - 1)) * chartW : chartW / 2);
    const py = (v) => PAD.top + chartH - (v / maxVal) * chartH;

    let remainingPoints = '';
    let completedPoints = '';
    burndown.forEach((p, i) => {
        remainingPoints += `${px(i).toFixed(1)},${py(p.remaining).toFixed(1)} `;
        completedPoints += `${px(i).toFixed(1)},${py(p.completed).toFixed(1)} `;
    });

    let dots = '';
    burndown.forEach((p, i) => {
        dots += `<circle cx="${px(i).toFixed(1)}" cy="${py(p.remaining).toFixed(1)}" r="3" fill="#ef4444"/>`;
        dots += `<circle cx="${px(i).toFixed(1)}" cy="${py(p.completed).toFixed(1)}" r="3" fill="#22c55e"/>`;
    });

    let xLabels = '';
    burndown.forEach((p, i) => {
        if (i % 2 === 0 || i === n - 1) {
            xLabels += `<text x="${px(i).toFixed(1)}" y="${(H - 6).toFixed(1)}" text-anchor="middle" font-size="10" fill="#94a3b8">${esc(p.date.slice(-3))}</text>`;
        }
    });

    const yTicks = [0, Math.ceil(maxVal / 2), maxVal];
    let yAxis = '';
    yTicks.forEach(t => {
        const y = py(t);
        yAxis += `<line x1="${PAD.left}" y1="${y.toFixed(1)}" x2="${W - PAD.right}" y2="${y.toFixed(1)}" stroke="#e2e8f0" stroke-width="1"/>`;
        yAxis += `<text x="${(PAD.left - 4).toFixed(1)}" y="${(y + 3).toFixed(1)}" text-anchor="end" font-size="9" fill="#94a3b8">${t}</text>`;
    });

    const legend = `<rect x="${PAD.left}" y="4" width="10" height="4" fill="#ef4444" rx="1"/><text x="${PAD.left + 14}" y="10" font-size="10" fill="#64748b">Remaining</text><rect x="${PAD.left + 82}" y="4" width="10" height="4" fill="#22c55e" rx="1"/><text x="${PAD.left + 96}" y="10" font-size="10" fill="#64748b">Completed</text>`;

    return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block">${yAxis}<polyline points="${remainingPoints.trim()}" fill="none" stroke="#ef4444" stroke-width="2" stroke-linejoin="round"/><polyline points="${completedPoints.trim()}" fill="none" stroke="#22c55e" stroke-width="2" stroke-linejoin="round"/>${dots}${xLabels}${legend}</svg>`;
}

export function svgLabelChart(labels) {
    if (labels.length === 0) {
        return '<p class="dashboard-empty">No labels in use</p>';
    }
    const BAR_H = 20, GAP = 6, PAD_LEFT = 90, PAD_RIGHT = 40, W = 360;
    const maxCount = Math.max(1, ...labels.map(l => l.count));
    const chartW = W - PAD_LEFT - PAD_RIGHT;
    const H = labels.length * (BAR_H + GAP) + 4;

    let rows = '';
    labels.forEach((l, i) => {
        const y = i * (BAR_H + GAP);
        const barW = Math.max(2, (l.count / maxCount) * chartW);
        const labelText = l.label.length > 12 ? l.label.slice(0, 11) + '…' : l.label;
        rows += `<text x="${PAD_LEFT - 6}" y="${(y + BAR_H / 2 + 4).toFixed(1)}" text-anchor="end" font-size="11" fill="#475569">${esc(labelText)}</text>`;
        rows += `<rect x="${PAD_LEFT}" y="${y}" width="${barW.toFixed(1)}" height="${BAR_H}" fill="#3b82f6" rx="3" opacity="0.85"/>`;
        rows += `<text x="${(PAD_LEFT + barW + 5).toFixed(1)}" y="${(y + BAR_H / 2 + 4).toFixed(1)}" font-size="11" fill="#64748b">${l.count}</text>`;
    });

    return `<svg viewBox="0 0 ${W} ${H}" width="100%" height="${H}" style="display:block">${rows}</svg>`;
}

export function overdueAgeColor(days) {
    if (days >= 8) return '#ef4444';
    if (days >= 4) return '#f97316';
    return '#f59e0b';
}

export async function renderDashboard() {
    const content = document.querySelector('#content');
    content.className = 'content';

    if (!state.currentProject) {
        content.innerHTML = '<div class="empty-state"><p>Select a project</p></div>';
        return;
    }

    content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Loading dashboard...</p></div>';

    const data = await api.getDashboard(state.currentProject.key);

    if (!data) {
        content.innerHTML = '<div class="empty-state"><p>Error loading dashboard</p></div>';
        return;
    }

    const statusCounts = data.status_counts || {};
    const totalTasks = Object.values(statusCounts).reduce((a, b) => a + b, 0);
    const upcomingDue = data.upcoming_tasks || [];
    const recentlyUpdated = data.recently_updated || [];
    const velocity = data.velocity || [];
    const burndown = data.burndown || [];
    const labelDist = data.label_distribution || [];
    const overdueDetail = data.overdue_tasks_detail || [];

    let statusBarsHtml = '';
    for (const s of STATUSES) {
        const count = statusCounts[s.key] || 0;
        const pct = totalTasks > 0 ? ((count / totalTasks) * 100).toFixed(1) : 0;
        statusBarsHtml += `<div class="dashboard-status-row"><div class="dashboard-status-label"><span class="dashboard-status-dot" style="background:${s.color}"></span>${s.label}</div><div class="dashboard-status-bar-track"><div class="dashboard-status-bar-fill" style="width:${pct}%;background:${s.color}"></div></div><span class="dashboard-status-count">${count}</span></div>`;
    }

    let overdueHtml = '';
    if (overdueDetail.length > 0) {
        overdueHtml = overdueDetail.map(t => {
            const color = overdueAgeColor(t.days_overdue);
            const daysLabel = t.days_overdue === 1 ? '1 day' : `${t.days_overdue} days`;
            return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title" style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(t.title)}</span><span style="font-size:11px;font-weight:600;color:${color};white-space:nowrap;padding:2px 6px;border-radius:4px;background:${color}18">${daysLabel} overdue</span></div>`;
        }).join('');
    } else {
        overdueHtml = '<p class="dashboard-empty">No overdue tasks</p>';
    }

    let upcomingHtml = '';
    if (upcomingDue.length > 0) {
        upcomingHtml = upcomingDue.map(t => {
            const overdue = t.status !== 'done' && isOverdue(t.due_date);
            return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title">${esc(t.title)}</span><span class="dashboard-task-due${overdue ? ' overdue' : ''}">${formatDate(t.due_date)}</span></div>`;
        }).join('');
    } else {
        upcomingHtml = '<p class="dashboard-empty">No tasks due in the next 7 days</p>';
    }

    let recentHtml = '';
    if (recentlyUpdated.length > 0) {
        recentHtml = recentlyUpdated.map(t => {
            return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title">${esc(t.title)}</span><span class="status-badge status-${t.status}"><span class="status-badge-dot"></span>${STATUS_LABELS[t.status]}</span>${t.updated_at ? `<span class="dashboard-task-time">${relativeTime(t.updated_at)}</span>` : ''}</div>`;
        }).join('');
    } else {
        recentHtml = '<p class="dashboard-empty">No recently updated tasks</p>';
    }

    content.innerHTML = `<div class="dashboard"><div class="dashboard-grid"><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Velocity — Tasks Completed per Week</h3>${svgVelocityChart(velocity)}</div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Burnup — Remaining vs Completed</h3>${svgBurndownChart(burndown)}</div><div class="dashboard-card"><h3 class="dashboard-card-title">Status Distribution</h3><div class="dashboard-status-bars">${statusBarsHtml}</div><div class="dashboard-total">Total: ${totalTasks} task(s)</div></div><div class="dashboard-card"><h3 class="dashboard-card-title">Labels</h3>${svgLabelChart(labelDist)}</div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Overdue Tasks</h3><div class="dashboard-task-list">${overdueHtml}</div></div><div class="dashboard-card"><h3 class="dashboard-card-title">Upcoming Deadlines (7 days)</h3><div class="dashboard-task-list">${upcomingHtml}</div></div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Recently Updated</h3><div class="dashboard-task-list">${recentHtml}</div></div></div></div>`;

    content.querySelectorAll('.dashboard-task-item').forEach(el => {
        el.addEventListener('click', () => {
            const taskId = parseInt(el.dataset.taskId);
            if (taskId) _openTaskModal(taskId);
        });
    });
}
