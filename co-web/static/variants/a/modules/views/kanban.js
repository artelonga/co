// ===== Kanban view =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, filteredTasks, getSubtasks, getSubtaskProgress, isOverdue, formatDate, assigneeInitials, toggleSubtree } from '../helpers.js';
import { STATUSES } from '../constants.js';
import { attachPointerDrag } from '../../lib/pointer-drag.js';

// Callbacks injected by app.js
let _openTaskModal = () => {};
let _ensureOwnUniverse = async () => true;
let _renderKanban = () => {};
let _showToast = () => {};
let _renderContent = () => {};

// Cleanup function for the currently active pointer-drag listener.
// Called before each re-render to prevent duplicate listeners.
let _dragCleanup = null;

export function injectKanbanCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
    _ensureOwnUniverse = callbacks.ensureOwnUniverse;
    _renderKanban = callbacks.renderKanban;
    _showToast = callbacks.showToast;
    _renderContent = callbacks.renderContent;
}

const MOBILE_COL_KEY = 'co.board.mobileActiveColumn';

function getMobileActiveColumn() {
    return localStorage.getItem(MOBILE_COL_KEY) || (STATUSES[0]?.key ?? '');
}

function setMobileActiveColumn(key) {
    localStorage.setItem(MOBILE_COL_KEY, key);
}

// CO-556: refetch (default true) gates the fire-and-forget agent-session footer
// network load, so a pure language re-render (refetch:false) makes no fetch.
export function renderKanban({ refetch = true } = {}) {
    // Detach previous drag listener before rebuilding the DOM.
    if (_dragCleanup) { _dragCleanup(); _dragCleanup = null; }

    const content = document.querySelector('#content');
    content.className = 'content';
    const tasks = filteredTasks();
    const taskIds = new Set(tasks.map(t => t.id));
    const rootTasks = tasks.filter(t => !t.parent || !taskIds.has(t.parent));

    const activeCol = getMobileActiveColumn();

    // CO-358: segmented control for mobile single-column view
    const segmentedControl = `
        <div class="kanban-segment-control" id="kanban-segment-control">
            ${STATUSES.map(s => `
                <button class="kanban-segment-btn${s.key === activeCol ? ' active' : ''}"
                        data-status="${s.key}"
                        aria-label="${s.label}">
                    ${s.label}
                    <span class="column-count">${rootTasks.filter(t => t.status === s.key).length}</span>
                </button>`).join('')}
        </div>`;

    content.innerHTML = segmentedControl + `<div class="kanban">${STATUSES.map(s => {
        const colTasks = rootTasks.filter(t => t.status === s.key);
        const isMobileActive = s.key === activeCol;
        return `
            <div class="kanban-column${isMobileActive ? ' mobile-active' : ''}" data-status="${s.key}">
                <div class="kanban-column-header">
                    ${s.label}
                    <span class="column-count">${colTasks.length}</span>
                </div>
                <div class="kanban-cards" data-status="${s.key}">
                    ${colTasks.map(t => renderTaskCard(t)).join('')}
                </div>
            </div>`;
    }).join('')}</div>`;

    setupSegmentedControl();
    setupDragDrop();
    setupCardClicks();
    setupSubtreeToggles();
    // CO-275: lazy-load agent session footers; fire-and-forget.
    // CO-556: skipped on a pure language re-render so the toggle stays network-free.
    if (refetch) loadAgentSessionFooters();
}

function setupSegmentedControl() {
    const ctrl = document.getElementById('kanban-segment-control');
    if (!ctrl) return;
    ctrl.addEventListener('click', (e) => {
        const btn = e.target.closest('.kanban-segment-btn');
        if (!btn) return;
        const status = btn.dataset.status;
        setMobileActiveColumn(status);
        // Update active button
        ctrl.querySelectorAll('.kanban-segment-btn').forEach(b => {
            b.classList.toggle('active', b.dataset.status === status);
        });
        // Show/hide columns
        document.querySelectorAll('.kanban-column').forEach(col => {
            col.classList.toggle('mobile-active', col.dataset.status === status);
        });
    });
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
        <div class="task-card" data-task-id="${task.id}" data-task-key="${esc(task.key)}">
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
            <div class="agent-session-footer" data-loaded="false"></div>
        </div>`;
}

/** Format milliseconds as "Xm" or "Xs" for the session footer. */
function fmtDuration(ms) {
    const s = Math.round(ms / 1000);
    return s >= 60 ? `${Math.round(s / 60)}m` : `${s}s`;
}

/** Format a number with a K suffix if ≥ 1000. */
function fmtNum(n) {
    return n >= 1000 ? `${Math.round(n / 1000)}k` : String(n);
}

/** Render the footer line from a session object returned by the API. */
function renderSessionFooter(session) {
    if (!session) return '';
    const dur = fmtDuration(session.duration_ms || 0);
    const tokens = (session.tokens_in || session.tokens_out)
        ? `· ${fmtNum((session.tokens_in || 0) + (session.tokens_out || 0))} tok`
        : '';
    const tools = session.tool_calls
        ? (() => {
            try {
                const tc = typeof session.tool_calls === 'string'
                    ? JSON.parse(session.tool_calls)
                    : session.tool_calls;
                const parts = Object.entries(tc)
                    .map(([k, v]) => `${v}${k[0]}`)
                    .join('/');
                return parts ? `· ${parts}` : '';
            } catch { return ''; }
        })()
        : '';
    const sha = session.final_commit_sha
        ? `· <span class="session-sha">${esc(session.final_commit_sha)}</span>`
        : '';
    const pr = session.pr_number
        ? `· <span class="session-pr">#${session.pr_number}</span>`
        : '';
    return `<div class="agent-session-footer loaded" title="Last agent run">
        <span class="session-run-icon">⚙</span>
        ${dur} ${tokens} ${tools} ${sha} ${pr}
    </div>`;
}

/** Lazy-load agent-session footers for all visible cards after render. */
export async function loadAgentSessionFooters() {
    const footers = document.querySelectorAll('.agent-session-footer[data-loaded="false"]');
    if (!footers.length) return;
    const fetches = Array.from(footers).map(async (el) => {
        const card = el.closest('[data-task-key]');
        if (!card) return;
        const taskKey = card.dataset.taskKey;
        if (!taskKey) return;
        el.dataset.loaded = 'pending';
        try {
            const universeKey = state.currentUniverse || 'co';
            const resp = await fetch(
                `/api/v1/agent/sessions/latest?task_id=${encodeURIComponent(taskKey)}`
            );
            if (!resp.ok) return;
            const session = await resp.json();
            if (session) {
                el.outerHTML = renderSessionFooter(session);
            } else {
                el.remove();
            }
        } catch {
            el.remove();
        }
    });
    await Promise.allSettled(fetches);
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
    const kanban = document.querySelector('.kanban');
    if (!kanban) return;

    let activeCard = null;
    let ghost = null;
    let offsetX = 0;
    let offsetY = 0;

    function findDropZone(x, y) {
        for (const el of document.elementsFromPoint(x, y)) {
            // Skip the ghost and its children to see through it during hit-test.
            if (el === ghost || ghost?.contains(el)) continue;
            // CO-358: in mobile single-column view the other columns are
            // hidden, so the segmented-control buttons act as drop targets
            // for cross-column moves (they carry data-status).
            const seg = el.closest('.kanban-segment-btn');
            if (seg) return seg;
            if (el.classList.contains('kanban-cards')) return el;
            const col = el.closest('.kanban-column');
            if (col) return col.querySelector('.kanban-cards');
        }
        return null;
    }

    function clearHighlights() {
        document.querySelectorAll('.kanban-cards.drag-over, .kanban-segment-btn.drag-over')
            .forEach(z => z.classList.remove('drag-over'));
    }

    function cleanupDrag() {
        if (ghost) { ghost.remove(); ghost = null; }
        if (activeCard) { activeCard.style.opacity = ''; activeCard = null; }
        clearHighlights();
    }

    _dragCleanup = attachPointerDrag(kanban, '.task-card', {
        onDragStart(card, _pointerId, x, y) {
            activeCard = card;
            card.style.opacity = '0.4';

            const rect = card.getBoundingClientRect();
            offsetX = x - rect.left;
            offsetY = y - rect.top;

            ghost = card.cloneNode(true);
            ghost.classList.add('dragging-ghost');
            Object.assign(ghost.style, {
                position: 'fixed',
                left: `${rect.left}px`,
                top: `${rect.top}px`,
                width: `${rect.width}px`,
                pointerEvents: 'none',
                opacity: '0.9',
                zIndex: '9999',
                transform: 'rotate(2deg)',
                boxShadow: '0 8px 24px rgba(0,0,0,.25)',
                transition: 'none',
                margin: '0',
            });
            document.body.appendChild(ghost);
        },

        onDragMove(x, y) {
            if (ghost) {
                ghost.style.left = `${x - offsetX}px`;
                ghost.style.top = `${y - offsetY}px`;
            }
            clearHighlights();
            const zone = findDropZone(x, y);
            if (zone) zone.classList.add('drag-over');
        },

        async onDragEnd(x, y) {
            const card = activeCard;
            const zone = findDropZone(x, y);
            cleanupDrag();

            if (!card || !zone) return;

            const taskId = parseInt(card.dataset.taskId);
            const newStatus = zone.dataset.status;
            const task = state.tasks.find(t => t.id === taskId);
            if (task && task.status !== newStatus) {
                if (!(await _ensureOwnUniverse())) return;
                const oldStatus = task.status;
                task.status = newStatus;
                // Dropped on a segment button → follow the card to its new
                // column so it stays visible in mobile single-column view.
                if (zone.classList.contains('kanban-segment-btn')) {
                    setMobileActiveColumn(newStatus);
                }
                _renderKanban();
                const result = await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
                if (!result) {
                    task.status = oldStatus;
                    _renderKanban();
                    _showToast('Failed to move task. Reverted.', 'error');
                }
            }
        },

        onDragCancel() {
            cleanupDrag();
        },
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
