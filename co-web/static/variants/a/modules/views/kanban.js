// ===== Kanban view =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, filteredTasks, getSubtasks, getSubtaskProgress, isOverdue, formatDate, assigneeInitials, toggleSubtree } from '../helpers.js';
import { STATUSES } from '../constants.js';

// Callbacks injected by app.js
let _openTaskModal = () => {};
let _ensureOwnUniverse = async () => true;
let _renderKanban = () => {};
let _showToast = () => {};
let _renderContent = () => {};

export function injectKanbanCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
    _ensureOwnUniverse = callbacks.ensureOwnUniverse;
    _renderKanban = callbacks.renderKanban;
    _showToast = callbacks.showToast;
    _renderContent = callbacks.renderContent;
}

export function renderKanban() {
    const content = document.querySelector('#content');
    content.className = 'content';
    const tasks = filteredTasks();
    const taskIds = new Set(tasks.map(t => t.id));
    const rootTasks = tasks.filter(t => !t.parent || !taskIds.has(t.parent));

    content.innerHTML = `<div class="kanban">${STATUSES.map(s => {
        const colTasks = rootTasks.filter(t => t.status === s.key);
        return `
            <div class="kanban-column" data-status="${s.key}">
                <div class="kanban-column-header">
                    ${s.label}
                    <span class="column-count">${colTasks.length}</span>
                </div>
                <div class="kanban-cards" data-status="${s.key}">
                    ${colTasks.map(t => renderTaskCard(t)).join('')}
                </div>
            </div>`;
    }).join('')}</div>`;

    setupDragDrop();
    setupCardClicks();
    setupSubtreeToggles();
}

export function renderTaskCard(task) {
    const subtasks = getSubtasks(task);
    const parentTask = task.parent ? state.tasks.find(t => t.id === task.parent) : null;
    const parentKey = parentTask ? `${state.currentProject.key}-${parentTask.id}` : null;
    const overdue = task.status !== 'done' && isOverdue(task.due_date);
    const hasSubtasks = subtasks.length > 0;
    const collapsed = state.collapsedSubtasks.has(task.id);
    const sp = hasSubtasks ? getSubtaskProgress(task) : null;

    const subtaskHtml = hasSubtasks ? `
        <div class="subtask-toggle" data-task-id="${task.id}">
            <span class="subtask-chevron${collapsed ? '' : ' open'}">&#9660;</span>
            <span class="subtask-toggle-label">${subtasks.length} subtask${subtasks.length !== 1 ? 's' : ''}</span>
            ${sp ? `<span class="subtask-badge">${sp.done}/${sp.total}</span>` : ''}
        </div>
        <div class="subtask-list${collapsed ? ' hidden' : ''}">
            ${subtasks.map(sub => renderSubtaskKanbanItem(sub)).join('')}
        </div>` : '';

    const rawPreview = window.CoMarkdown
        ? window.CoMarkdown.extractFirstParagraph(task.description || '')
        : (task.description || '').split('\n').find(l => l.trim() && !l.startsWith('#') && !l.startsWith('```')) || '';
    const descSnippet = rawPreview.length > 100 ? rawPreview.slice(0, 100) + '…' : rawPreview;

    return `
        <div class="task-card" draggable="true" data-task-id="${task.id}">
            <div class="task-card-header">
                <span class="task-key">${esc(task.key)}</span>
                ${parentKey ? `<span class="task-parent-key">${esc(parentKey)}</span>` : ''}
            </div>
            <div class="task-title">${esc(task.title)}</div>
            ${descSnippet ? `<div class="task-desc-preview">${esc(descSnippet)}</div>` : ''}
            <div class="task-meta">
                ${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}
                ${task.due_date ? `<span class="due-date-badge${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
                ${task.assignee ? `<span class="assignee-badge" title="${esc(task.assignee)}">${esc(assigneeInitials(task.assignee))}</span>` : ''}
            </div>
            ${subtaskHtml}
        </div>`;
}

export function renderSubtaskKanbanItem(task) {
    const overdue = task.status !== 'done' && isOverdue(task.due_date);
    return `<div class="subtask-item" data-task-id="${task.id}">
        <span class="subtask-item-key">${esc(task.key)}</span>
        <span class="subtask-item-title">${esc(task.title)}</span>
        ${task.due_date ? `<span class="subtask-item-due${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
    </div>`;
}

export function buildGroupHierarchy(groupTasks) {
    const groupIds = new Set(groupTasks.map(t => t.id));
    const childrenOf = {};
    for (const t of groupTasks) {
        if (t.parent && groupIds.has(t.parent)) {
            if (!childrenOf[t.parent]) childrenOf[t.parent] = [];
            childrenOf[t.parent].push(t);
        }
    }
    const roots = groupTasks.filter(t => !t.parent || !groupIds.has(t.parent));
    const result = [];
    function flatten(task, depth) {
        const children = childrenOf[task.id] || [];
        result.push({ task, depth, hasGroupChildren: children.length > 0 });
        if (!state.collapsedSubtasks.has(task.id)) {
            for (const child of children) flatten(child, depth + 1);
        }
    }
    for (const root of roots) flatten(root, 0);
    return result;
}

export function setupCardClicks() {
    document.querySelectorAll('.subtask-item').forEach(item => {
        item.addEventListener('click', (e) => {
            e.stopPropagation();
            _openTaskModal(parseInt(item.dataset.taskId));
        });
    });

    document.querySelectorAll('.task-card').forEach(card => {
        card.addEventListener('click', () => {
            _openTaskModal(parseInt(card.dataset.taskId));
        });
    });
}

export function setupDragDrop() {
    const cards = document.querySelectorAll('.task-card');
    const zones = document.querySelectorAll('.kanban-cards');

    cards.forEach(card => {
        card.addEventListener('dragstart', (e) => {
            card.classList.add('dragging');
            e.dataTransfer.setData('text/plain', card.dataset.taskId);
            e.dataTransfer.effectAllowed = 'move';
        });
        card.addEventListener('dragend', () => {
            card.classList.remove('dragging');
            zones.forEach(z => z.classList.remove('drag-over'));
        });
    });

    zones.forEach(zone => {
        zone.addEventListener('dragover', (e) => {
            e.preventDefault();
            e.dataTransfer.dropEffect = 'move';
            zone.classList.add('drag-over');
        });
        zone.addEventListener('dragleave', () => {
            zone.classList.remove('drag-over');
        });
        zone.addEventListener('drop', async (e) => {
            e.preventDefault();
            zone.classList.remove('drag-over');
            const taskId = parseInt(e.dataTransfer.getData('text/plain'));
            const newStatus = zone.dataset.status;
            const task = state.tasks.find(t => t.id === taskId);
            if (task && task.status !== newStatus) {
                if (state.isTemplate) {
                    await _ensureOwnUniverse();
                    return;
                }
                const oldStatus = task.status;
                task.status = newStatus;
                _renderKanban();
                const result = await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
                if (!result) {
                    task.status = oldStatus;
                    _renderKanban();
                    _showToast('Failed to move task. Reverted.', 'error');
                }
            }
        });
    });
}

export function setupSubtreeToggles() {
    document.querySelectorAll('.subtask-toggle, .tree-toggle, .timeline-subtree-toggle').forEach(el => {
        el.addEventListener('click', (e) => {
            e.stopPropagation();
            const taskId = parseInt(el.dataset.taskId);
            toggleSubtree(taskId);
            _renderContent();
        });
    });
}
