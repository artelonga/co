// ===== Helper / utility functions =====
import { state } from './state.js';
import { STATUS_ORDER, PRIORITY_ORDER } from './constants.js';

// ===== DOM helpers =====
export function esc(s) {
    const d = document.createElement('div');
    d.textContent = s;
    return d.innerHTML;
}

export function $(sel) { return document.querySelector(sel); }

// ===== Date helpers =====
export function formatDate(d) {
    if (!d) return '';
    const dt = new Date(d + 'T00:00:00');
    return dt.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
}

export function isOverdue(d) {
    if (!d) return false;
    return new Date(d + 'T23:59:59') < new Date();
}

export function toDateStr(d) {
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
}

export function parseDate(s) {
    if (!s) return null;
    return new Date(s + 'T00:00:00');
}

export function addDays(d, n) {
    const r = new Date(d);
    r.setDate(r.getDate() + n);
    return r;
}

export function daysBetween(a, b) {
    const msPerDay = 86400000;
    return Math.round((b - a) / msPerDay);
}

export function isWeekend(d) {
    const day = d.getDay();
    return day === 0 || day === 6;
}

export function getWeekNumber(d) {
    const start = new Date(d.getFullYear(), 0, 1);
    const diff = d - start;
    return Math.ceil((diff / 86400000 + start.getDay() + 1) / 7);
}

export function todayDate() {
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), now.getDate());
}

export function formatDateShort(d) {
    return d.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
}

// ===== Relative time helpers =====
export function relativeTime(dateStr) {
    if (!dateStr) return '';
    const now = new Date();
    const past = new Date(dateStr);
    const diffMs = now - past;
    if (diffMs < 0) return 'agora';

    const seconds = Math.floor(diffMs / 1000);
    if (seconds < 60) return 'agora';

    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return minutes + ' min ago';

    const hours = Math.floor(minutes / 60);
    if (hours < 24) return hours + ' h ago';

    const days = Math.floor(hours / 24);
    if (days < 30) return days + (days === 1 ? ' day ago' : ' days ago');

    const months = Math.floor(days / 30);
    if (months < 12) return months + (months === 1 ? ' month ago' : ' months ago');

    const years = Math.floor(months / 12);
    return years + (years === 1 ? ' year ago' : ' years ago');
}

export function relativeDate(iso) {
    if (!iso) return '';
    const then = new Date(iso).getTime();
    if (isNaN(then)) return '';
    const diff = Math.floor((Date.now() - then) / 1000);
    if (diff < 60) return 'agora';
    if (diff < 3600) return `${Math.floor(diff / 60)}min atrás`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h atrás`;
    const days = Math.floor(diff / 86400);
    if (days === 1) return '1 dia atrás';
    if (days < 30) return `${days} dias atrás`;
    const months = Math.floor(days / 30);
    if (months === 1) return '1 mês atrás';
    if (months < 12) return `${months} meses atrás`;
    const years = Math.floor(days / 365);
    return years === 1 ? '1 ano atrás' : `${years} anos atrás`;
}

// ===== CO-73: Timezone-aware date helpers =====
export function userTimezone() {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

export function utcToLocalDate(utcStr) {
    if (!utcStr) return '';
    try {
        const d = new Date(utcStr);
        return d.toLocaleDateString('en-CA', { timeZone: userTimezone() });
    } catch (_) {
        return utcStr.slice(0, 10);
    }
}

// ===== Person helpers =====
export function assigneeInitials(name) {
    if (!name) return '';
    const parts = name.trim().split(/[\s@.]+/).filter(Boolean);
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
    return parts[0].slice(0, 2).toUpperCase();
}

// ===== Subtask helpers =====
export function getSubtasks(task) {
    return state.tasks.filter(t => t.parent === task.id);
}

export function getSubtaskProgress(task) {
    const subs = getSubtasks(task);
    if (subs.length === 0) return null;
    const done = subs.filter(t => t.status === 'done').length;
    return { done, total: subs.length };
}

// ===== Subtree state (localStorage) =====
export function loadSubtreeState(projectKey) {
    try {
        const raw = localStorage.getItem('co_subtree_' + projectKey);
        state.collapsedSubtasks = new Set(raw ? JSON.parse(raw) : []);
    } catch (e) {
        state.collapsedSubtasks = new Set();
    }
}

export function saveSubtreeState() {
    if (!state.currentProject) return;
    localStorage.setItem('co_subtree_' + state.currentProject.key, JSON.stringify([...state.collapsedSubtasks]));
}

export function toggleSubtree(taskId) {
    if (state.collapsedSubtasks.has(taskId)) {
        state.collapsedSubtasks.delete(taskId);
    } else {
        state.collapsedSubtasks.add(taskId);
    }
    saveSubtreeState();
}

// ===== Filtering / sorting =====
export function filteredTasks() {
    const q = state.searchQuery.toLowerCase();
    if (!q) return state.tasks;
    return state.tasks.filter(t =>
        t.title.toLowerCase().includes(q) ||
        t.key.toLowerCase().includes(q) ||
        t.labels.some(l => l.toLowerCase().includes(q))
    );
}

export function sortTasks(tasks) {
    const col = state.sortColumn;
    const dir = state.sortDirection === 'asc' ? 1 : -1;

    return [...tasks].sort((a, b) => {
        let va, vb;
        switch (col) {
            case 'key':
                va = a.id;
                vb = b.id;
                break;
            case 'title':
                va = a.title.toLowerCase();
                vb = b.title.toLowerCase();
                break;
            case 'status':
                va = STATUS_ORDER[a.status] || 0;
                vb = STATUS_ORDER[b.status] || 0;
                break;
            case 'priority':
                va = PRIORITY_ORDER[a.priority] || 99;
                vb = PRIORITY_ORDER[b.priority] || 99;
                break;
            case 'due_date':
                va = a.due_date || '9999-12-31';
                vb = b.due_date || '9999-12-31';
                break;
            case 'labels':
                va = a.labels.join(',').toLowerCase();
                vb = b.labels.join(',').toLowerCase();
                break;
            default:
                va = a.id;
                vb = b.id;
        }

        if (va < vb) return -1 * dir;
        if (va > vb) return 1 * dir;
        return 0;
    });
}

export function groupTasksByStatus(tasks, STATUSES) {
    const groups = {};
    for (const s of STATUSES) {
        groups[s.key] = [];
    }
    for (const t of tasks) {
        if (groups[t.status]) {
            groups[t.status].push(t);
        }
    }
    return groups;
}

// ===== Yggdrasil helpers =====
export function formatTimeAgo(ts) {
    if (!ts) return '?';
    const diffSecs = Math.floor(Date.now() / 1000) - ts;
    if (diffSecs < 60) return `${diffSecs}s`;
    if (diffSecs < 3600) return `${Math.floor(diffSecs / 60)}m`;
    if (diffSecs < 86400) return `${Math.floor(diffSecs / 3600)}h`;
    return `${Math.floor(diffSecs / 86400)}d`;
}

export function capitalize(s) {
    return s ? s.charAt(0).toUpperCase() + s.slice(1) : s;
}
