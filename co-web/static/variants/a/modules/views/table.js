// ===== Table view =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, filteredTasks, sortTasks, groupTasksByStatus, isOverdue, formatDate, assigneeInitials, toggleSubtree } from '../helpers.js';
import { STATUSES, STATUS_LABELS, PRIORITY_LABELS } from '../constants.js';
import { buildGroupHierarchy } from './kanban.js';

let _openTaskModal = () => {};
let _refreshTasks = async () => {};
let _renderTable = () => {};
let _showToast = () => {};

export function injectTableCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
    _refreshTasks = callbacks.refreshTasks;
    _renderTable = callbacks.renderTable;
    _showToast = callbacks.showToast;
}

export function sortClass(col) {
    if (state.sortColumn !== col) return '';
    return ` sorted${state.sortDirection === 'desc' ? ' desc' : ''}`;
}

export function sortArrow(col) {
    if (state.sortColumn !== col) {
        return ' <span class="sort-arrow">&#9650;</span>';
    }
    return state.sortDirection === 'asc'
        ? ' <span class="sort-arrow">&#9650;</span>'
        : ' <span class="sort-arrow">&#9660;</span>';
}

// CO-358: mobile card labels via the shared i18n table (window.t),
// falling back to pt literals when a key is missing.
function tt(key, fallback) {
    const v = window.t ? window.t(key) : null;
    return v && v !== key ? v : fallback;
}

export function renderTable() {
    const content = document.querySelector('#content');
    content.className = 'content no-padding';
    const tasks = filteredTasks();
    const sorted = sortTasks(tasks);
    const groups = groupTasksByStatus(sorted, STATUSES);

    const allIds = sorted.map(t => t.id);
    const allSelected = allIds.length > 0 && allIds.every(id => state.selectedIds.has(id));

    let html = '';

    if (state.selectedIds.size > 0) {
        html += `
            <div class="bulk-bar" id="bulk-bar">
                <span><span class="bulk-bar-count">${state.selectedIds.size}</span> selecionada(s)</span>
                <div class="bulk-bar-actions">
                    <div class="bulk-status-wrapper">
                        <button class="btn btn-bulk" id="bulk-move-btn">Mover para...</button>
                        <div class="bulk-status-dropdown hidden" id="bulk-status-dropdown">
                            ${STATUSES.map(s => `
                                <button class="bulk-status-option" data-status="${s.key}">
                                    <span class="status-dd-dot" style="background:${s.color}"></span>
                                    ${s.label}
                                </button>
                            `).join('')}
                        </div>
                    </div>
                    <button class="btn btn-bulk" id="bulk-archive-btn">Archive</button>
                    <button class="btn btn-bulk btn-danger" id="bulk-delete-btn">Delete</button>
                </div>
                <button class="btn" id="bulk-bar-close">&times; Limpar</button>
            </div>`;
    }

    html += '<div class="table-container">';

    for (const status of STATUSES) {
        const groupTasks = groups[status.key];
        if (groupTasks.length === 0) continue;

        const collapsed = state.collapsedGroups.has(status.key);
        const chevronSvg = `<svg class="group-chevron" viewBox="0 0 16 16" fill="none"><path d="M5 3l5 5-5 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

        html += `<div class="status-group${collapsed ? ' collapsed' : ''}" data-group="${status.key}">`;
        html += `
            <div class="status-group-header" data-group="${status.key}">
                ${chevronSvg}
                <span class="group-dot" style="background:${status.color}"></span>
                <span class="status-group-label">${status.label}</span>
                <span class="status-group-count">${groupTasks.length}</span>
            </div>`;

        html += '<div class="status-group-body">';
        html += '<table class="data-table">';
        html += `<thead><tr>
            <th class="col-checkbox">
                <div class="row-checkbox">
                    <input type="checkbox" class="select-all-cb" data-group="${status.key}" ${allSelected ? 'checked' : ''}>
                </div>
            </th>
            <th class="col-key"><div class="th-inner${sortClass('key')}" data-sort="key">Key${sortArrow('key')}</div></th>
            <th class="col-title"><div class="th-inner${sortClass('title')}" data-sort="title">Title${sortArrow('title')}</div></th>
            <th class="col-status"><div class="th-inner${sortClass('status')}" data-sort="status">Status${sortArrow('status')}</div></th>
            <th class="col-priority"><div class="th-inner${sortClass('priority')}" data-sort="priority">Prioridade${sortArrow('priority')}</div></th>
            <th class="col-due-date"><div class="th-inner${sortClass('due_date')}" data-sort="due_date">Data Limite${sortArrow('due_date')}</div></th>
            <th class="col-assignee"><div class="th-inner">Responsável</div></th>
            <th class="col-labels"><div class="th-inner${sortClass('labels')}" data-sort="labels">Labels${sortArrow('labels')}</div></th>
        </tr></thead>`;
        html += '<tbody>';

        for (const { task, depth, hasGroupChildren } of buildGroupHierarchy(groupTasks)) {
            const selected = state.selectedIds.has(task.id);
            const overdue = task.status !== 'done' && isOverdue(task.due_date);
            const isCollapsed = state.collapsedSubtasks.has(task.id);
            const indent = depth * 20;
            const connector = depth > 0 ? '<span class="tree-line">└─</span>' : '';
            const toggleBtn = hasGroupChildren
                ? `<button class="tree-toggle" data-task-id="${task.id}" title="${isCollapsed ? 'Expand' : 'Collapse'}">${isCollapsed ? '▶' : '▼'}</button>`
                : '<span class="tree-toggle-spacer"></span>';
            html += `
                <tr data-task-id="${task.id}" class="${selected ? 'selected' : ''}${depth > 0 ? ' subtask-row' : ''}">
                    <td class="col-checkbox">
                        <div class="row-checkbox">
                            <input type="checkbox" class="task-cb" data-task-id="${task.id}" ${selected ? 'checked' : ''}>
                        </div>
                    </td>
                    <td data-label="Key"><span class="cell-key">${esc(task.key)}</span></td>
                    <td data-label="${tt('title', 'Título')}">
                        <div class="cell-title-tree" style="padding-left:${indent}px">
                            ${connector}${toggleBtn}<span class="cell-title">${esc(task.title)}</span>
                        </div>
                    </td>
                    <td data-label="${tt('status', 'Status')}">
                        <span class="status-badge status-${task.status}" data-task-id="${task.id}">
                            <span class="status-badge-dot"></span>
                            ${STATUS_LABELS[task.status]}
                        </span>
                    </td>
                    <td data-label="${tt('priority', 'Prioridade')}">
                        <span class="cell-priority">
                            <span class="priority-dot ${task.priority}"></span>
                            <span class="priority-label">${PRIORITY_LABELS[task.priority]}</span>
                        </span>
                    </td>
                    <td data-label="${tt('due_date', 'Data Limite')}"><span class="cell-due-date${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span></td>
                    <td data-label="${tt('assignee', 'Responsável')}">${task.assignee ? `<span class="assignee-badge" title="${esc(task.assignee)}">${esc(assigneeInitials(task.assignee))}</span>` : ''}</td>
                    <td data-label="Labels"><span class="cell-labels">${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}</span></td>
                </tr>`;
        }

        html += '</tbody></table>';
        html += '</div>';
        html += '</div>';
    }

    if (sorted.length === 0) {
        html += '<div class="empty-state"><p>No tasks found</p></div>';
    }

    html += '</div>';
    content.innerHTML = html;

    setupTableEvents();
}

export function setupTableEvents() {
    document.querySelectorAll('.th-inner[data-sort]').forEach(th => {
        th.addEventListener('click', (e) => {
            e.stopPropagation();
            const col = th.dataset.sort;
            if (state.sortColumn === col) {
                state.sortDirection = state.sortDirection === 'asc' ? 'desc' : 'asc';
            } else {
                state.sortColumn = col;
                state.sortDirection = 'asc';
            }
            _renderTable();
        });
    });

    document.querySelectorAll('.status-group-header').forEach(hdr => {
        hdr.addEventListener('click', () => {
            const group = hdr.dataset.group;
            if (state.collapsedGroups.has(group)) {
                state.collapsedGroups.delete(group);
            } else {
                state.collapsedGroups.add(group);
            }
            _renderTable();
        });
    });

    document.querySelectorAll('.tree-toggle').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            toggleSubtree(parseInt(btn.dataset.taskId));
            _renderTable();
        });
    });

    document.querySelectorAll('.data-table tbody tr').forEach(row => {
        row.addEventListener('click', (e) => {
            if (e.target.closest('.row-checkbox') || e.target.closest('.status-badge')) return;
            const taskId = parseInt(row.dataset.taskId);
            _openTaskModal(taskId);
        });
    });

    document.querySelectorAll('.task-cb').forEach(cb => {
        cb.addEventListener('change', (e) => {
            e.stopPropagation();
            const taskId = parseInt(cb.dataset.taskId);
            if (cb.checked) {
                state.selectedIds.add(taskId);
            } else {
                state.selectedIds.delete(taskId);
            }
            _renderTable();
        });
        cb.addEventListener('click', (e) => e.stopPropagation());
    });

    document.querySelectorAll('.select-all-cb').forEach(cb => {
        cb.addEventListener('change', (e) => {
            e.stopPropagation();
            const groupKey = cb.dataset.group;
            const groupTasks = filteredTasks().filter(t => t.status === groupKey);
            if (cb.checked) {
                groupTasks.forEach(t => state.selectedIds.add(t.id));
            } else {
                groupTasks.forEach(t => state.selectedIds.delete(t.id));
            }
            _renderTable();
        });
        cb.addEventListener('click', (e) => e.stopPropagation());
    });

    const bulkClose = document.getElementById('bulk-bar-close');
    if (bulkClose) {
        bulkClose.addEventListener('click', () => {
            state.selectedIds.clear();
            _renderTable();
        });
    }

    const bulkMoveBtn = document.getElementById('bulk-move-btn');
    if (bulkMoveBtn) {
        bulkMoveBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            const dd = document.getElementById('bulk-status-dropdown');
            if (dd) dd.classList.toggle('hidden');
        });
    }

    document.querySelectorAll('.bulk-status-option').forEach(opt => {
        opt.addEventListener('click', async (e) => {
            e.stopPropagation();
            const newStatus = opt.dataset.status;
            const taskIds = Array.from(state.selectedIds);
            if (taskIds.length === 0) return;
            const result = await api.bulkUpdateTasks(state.currentProject.key, {
                task_ids: taskIds,
                status: newStatus,
            });
            if (result) {
                _showToast(taskIds.length + ' task(s) updated', 'success');
                state.selectedIds.clear();
                await _refreshTasks();
                _renderTable();
            }
            const dd = document.getElementById('bulk-status-dropdown');
            if (dd) dd.classList.add('hidden');
        });
    });

    const bulkArchiveBtn = document.getElementById('bulk-archive-btn');
    if (bulkArchiveBtn) {
        bulkArchiveBtn.addEventListener('click', async () => {
            const taskIds = Array.from(state.selectedIds);
            if (taskIds.length === 0) return;
            const result = await api.bulkUpdateTasks(state.currentProject.key, {
                task_ids: taskIds,
                archived: true,
            });
            if (result) {
                _showToast(taskIds.length + ' task(s) archived', 'success');
                state.selectedIds.clear();
                await _refreshTasks();
                _renderTable();
            }
        });
    }

    const bulkDeleteBtn = document.getElementById('bulk-delete-btn');
    if (bulkDeleteBtn) {
        bulkDeleteBtn.addEventListener('click', async () => {
            const taskIds = Array.from(state.selectedIds);
            if (taskIds.length === 0) return;
            if (!confirm('Are you sure you want to delete ' + taskIds.length + ' task(s)? This action cannot be undone.')) return;
            const result = await api.bulkDeleteTasks(state.currentProject.key, {
                task_ids: taskIds,
            });
            if (result) {
                _showToast(taskIds.length + ' task(s) deleted', 'success');
                state.selectedIds.clear();
                await _refreshTasks();
                _renderTable();
            }
        });
    }

    document.querySelectorAll('.status-badge').forEach(badge => {
        badge.addEventListener('click', (e) => {
            e.stopPropagation();
            const taskId = parseInt(badge.dataset.taskId);
            toggleStatusDropdown(badge, taskId);
        });
    });
}

function toggleStatusDropdown(badge, taskId) {
    closeStatusDropdown();

    const task = state.tasks.find(t => t.id === taskId);
    if (!task) return;

    const dropdown = document.createElement('div');
    dropdown.className = 'status-dropdown';
    dropdown.id = 'active-status-dropdown';

    dropdown.innerHTML = STATUSES.map(s => `
        <button class="status-dropdown-item${s.key === task.status ? ' active' : ''}" data-status="${s.key}">
            <span class="status-dd-dot" style="background:${s.color}"></span>
            ${s.label}
        </button>
    `).join('');

    badge.style.position = 'relative';
    badge.appendChild(dropdown);

    state.openStatusDropdown = taskId;

    dropdown.querySelectorAll('.status-dropdown-item').forEach(item => {
        item.addEventListener('click', async (e) => {
            e.stopPropagation();
            const newStatus = item.dataset.status;
            if (newStatus !== task.status) {
                task.status = newStatus;
                await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
            }
            closeStatusDropdown();
            _renderTable();
        });
    });

    setTimeout(() => {
        document.addEventListener('click', closeStatusDropdownOnOutside);
    }, 0);
}

function closeStatusDropdownOnOutside(e) {
    const dd = document.getElementById('active-status-dropdown');
    if (dd && !dd.contains(e.target)) {
        closeStatusDropdown();
    }
}

export function closeStatusDropdown() {
    const existing = document.getElementById('active-status-dropdown');
    if (existing) existing.remove();
    state.openStatusDropdown = null;
    document.removeEventListener('click', closeStatusDropdownOnOutside);
}
