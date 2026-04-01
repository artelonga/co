(function () {
    'use strict';

    // ===== State =====
    const state = {
        projects: [],
        currentProject: null,
        tasks: [],
        view: 'kanban',
        editingTaskId: null,
        searchQuery: '',
        loading: false,
        showArchived: false,
        // Calendar
        calendarDate: new Date(),
        // Table
        sortColumn: 'key',
        sortDirection: 'asc',
        selectedIds: new Set(),
        collapsedGroups: new Set(),
        openStatusDropdown: null,
        // Timeline
        zoom: 'month',
        timelineStart: null,
        miniCalDate: new Date(),
        collapsedSwimlanes: {},
        unscheduledCollapsed: false,
        // Subtree expand/collapse (shared across views)
        collapsedSubtasks: new Set(),
    };

    const STATUSES = [
        { key: 'todo', label: 'To Do', color: '#94a3b8' },
        { key: 'in_progress', label: 'In Progress', color: '#3b82f6' },
        { key: 'in_review', label: 'In Review', color: '#f59e0b' },
        { key: 'done', label: 'Done', color: '#22c55e' },
    ];

    const PRIORITY_LABELS = { low: 'Low', medium: 'Medium', high: 'High', critical: 'Critical' };
    const PRIORITY_ORDER = { critical: 0, high: 1, medium: 2, low: 3 };
    const STATUS_ORDER = { todo: 0, in_progress: 1, in_review: 2, done: 3 };
    const STATUS_LABELS = { todo: 'To Do', in_progress: 'In Progress', in_review: 'In Review', done: 'Done' };

    const ZOOM_DAYS = { week: 7, month: 30, quarter: 90 };
    const COL_WIDTHS = { week: 50, month: 40, quarter: 60 };

    const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
        'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    const MONTH_NAMES_FULL = ['January', 'February', 'March', 'April', 'May', 'June',
        'July', 'August', 'September', 'October', 'November', 'December'];
    const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
    const DAY_NAMES_MINI = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];

    const ACTIVITY_ACTION_LABELS = {
        task_created: 'created task',
        task_deleted: 'deleted task',
        field_changed: 'updated field in',
        comment_added: 'commented on',
        status_changed: 'changed status of',
        task_archived: 'archived task',
        task_unarchived: 'unarchived task',
    };

    // ===== Toast System =====
    function showToast(message, type) {
        const container = document.getElementById('toast-container');
        if (!container) return;

        const toast = document.createElement('div');
        toast.className = 'toast toast-' + (type || 'success');
        toast.textContent = message;
        container.appendChild(toast);

        setTimeout(() => {
            toast.classList.add('toast-fade-out');
            setTimeout(() => {
                if (toast.parentNode) toast.parentNode.removeChild(toast);
            }, 300);
        }, 3000);
    }

    // ===== Loading Helpers =====
    function showLoading() {
        state.loading = true;
        const content = $('#content');
        if (content) {
            content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Loading...</p></div>';
        }
    }

    function hideLoading() {
        state.loading = false;
    }

    function setSubmitDisabled(disabled) {
        const btn = $('#btn-submit');
        if (btn) btn.disabled = disabled;
    }

    // ===== API =====
    async function apiFetch(url, options) {
        try {
            const r = await fetch(url, options);
            if (!r.ok) {
                let errMsg = 'Request error';
                try {
                    const errData = await r.json();
                    errMsg = errData.message || errData.error || errMsg;
                } catch (_) {
                    // ignore parse error
                }
                showToast(errMsg, 'error');
                return null;
            }
            // DELETE responses may have no body
            if (r.status === 204 || r.headers.get('content-length') === '0') {
                return {};
            }
            return r.json();
        } catch (err) {
            showToast('Connection error: ' + err.message, 'error');
            return null;
        }
    }

    const api = {
        async getProjects() {
            const r = await apiFetch('/api/projects');
            return r || [];
        },
        async getTasks(key, opts) {
            let url = `/api/projects/${key}/tasks`;
            if (opts && typeof opts.archived === 'boolean') {
                url += '?archived=' + (opts.archived ? 'true' : 'false');
            }
            const r = await apiFetch(url);
            return r || [];
        },
        async createTask(key, data) {
            const r = await apiFetch(`/api/projects/${key}/tasks`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async updateTask(key, id, data) {
            const r = await apiFetch(`/api/projects/${key}/tasks/${id}`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async deleteTask(key, id) {
            await apiFetch(`/api/projects/${key}/tasks/${id}`, { method: 'DELETE' });
        },
        async getComments(key, id) {
            const r = await apiFetch(`/api/projects/${key}/tasks/${id}/comments`);
            return r || [];
        },
        async createComment(key, id, data) {
            const r = await apiFetch(`/api/projects/${key}/tasks/${id}/comments`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async getActivity(key, limit) {
            const l = limit || 50;
            const r = await apiFetch(`/api/projects/${key}/activity?limit=${l}`);
            return r || [];
        },
        async getDashboard(key) {
            const r = await apiFetch(`/api/projects/${key}/dashboard`);
            return r;
        },
        async bulkUpdateTasks(key, data) {
            const r = await apiFetch(`/api/projects/${key}/tasks/bulk-update`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async bulkDeleteTasks(key, data) {
            const r = await apiFetch(`/api/projects/${key}/tasks/bulk-delete`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
    };

    // ===== Helpers =====
    function esc(s) {
        const d = document.createElement('div');
        d.textContent = s;
        return d.innerHTML;
    }

    function $(sel) { return document.querySelector(sel); }

    function formatDate(d) {
        if (!d) return '';
        const dt = new Date(d + 'T00:00:00');
        return dt.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
    }

    function isOverdue(d) {
        if (!d) return false;
        return new Date(d + 'T23:59:59') < new Date();
    }

    function getSubtasks(task) {
        return state.tasks.filter(t => t.parent === task.id);
    }

    function getSubtaskProgress(task) {
        const subs = getSubtasks(task);
        if (subs.length === 0) return null;
        const done = subs.filter(t => t.status === 'done').length;
        return { done, total: subs.length };
    }

    // ===== Subtree State (localStorage) =====
    function loadSubtreeState(projectKey) {
        try {
            const raw = localStorage.getItem('co_subtree_' + projectKey);
            state.collapsedSubtasks = new Set(raw ? JSON.parse(raw) : []);
        } catch (e) {
            state.collapsedSubtasks = new Set();
        }
    }

    function saveSubtreeState() {
        if (!state.currentProject) return;
        localStorage.setItem('co_subtree_' + state.currentProject.key, JSON.stringify([...state.collapsedSubtasks]));
    }

    function toggleSubtree(taskId) {
        if (state.collapsedSubtasks.has(taskId)) {
            state.collapsedSubtasks.delete(taskId);
        } else {
            state.collapsedSubtasks.add(taskId);
        }
        saveSubtreeState();
    }

    function filteredTasks() {
        const q = state.searchQuery.toLowerCase();
        if (!q) return state.tasks;
        return state.tasks.filter(t =>
            t.title.toLowerCase().includes(q) ||
            t.key.toLowerCase().includes(q) ||
            t.labels.some(l => l.toLowerCase().includes(q))
        );
    }

    // ===== Relative Time Helper =====
    function relativeTime(dateStr) {
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

    // ===== Timeline helpers =====
    function toDateStr(d) {
        const y = d.getFullYear();
        const m = String(d.getMonth() + 1).padStart(2, '0');
        const day = String(d.getDate()).padStart(2, '0');
        return `${y}-${m}-${day}`;
    }

    function parseDate(s) {
        if (!s) return null;
        return new Date(s + 'T00:00:00');
    }

    function addDays(d, n) {
        const r = new Date(d);
        r.setDate(r.getDate() + n);
        return r;
    }

    function daysBetween(a, b) {
        const msPerDay = 86400000;
        return Math.round((b - a) / msPerDay);
    }

    function isWeekend(d) {
        const day = d.getDay();
        return day === 0 || day === 6;
    }

    function getWeekNumber(d) {
        const start = new Date(d.getFullYear(), 0, 1);
        const diff = d - start;
        return Math.ceil((diff / 86400000 + start.getDay() + 1) / 7);
    }

    function todayDate() {
        const now = new Date();
        return new Date(now.getFullYear(), now.getMonth(), now.getDate());
    }

    function formatDateShort(d) {
        return d.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
    }

    // ===== Table helpers =====
    function sortTasks(tasks) {
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

    function groupTasksByStatus(tasks) {
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

    // ===== Timeline date range =====
    function getTimelineRange() {
        const days = ZOOM_DAYS[state.zoom];
        const start = state.timelineStart || todayDate();
        // For quarter zoom, generate week columns
        if (state.zoom === 'quarter') {
            const columns = [];
            let current = new Date(start);
            // Align to Monday
            const dayOfWeek = current.getDay();
            const offset = dayOfWeek === 0 ? -6 : 1 - dayOfWeek;
            current.setDate(current.getDate() + offset);
            const numWeeks = Math.ceil(days / 7);
            for (let i = 0; i < numWeeks; i++) {
                const weekStart = new Date(current);
                const weekEnd = addDays(weekStart, 6);
                columns.push({ date: weekStart, endDate: weekEnd, type: 'week' });
                current = addDays(current, 7);
            }
            return { columns, startDate: columns[0].date, endDate: columns[columns.length - 1].endDate };
        }
        // Day columns
        const columns = [];
        for (let i = 0; i < days; i++) {
            const date = addDays(start, i);
            columns.push({ date, type: 'day' });
        }
        return { columns, startDate: start, endDate: addDays(start, days - 1) };
    }

    function initTimelineStart() {
        const today = todayDate();
        const days = ZOOM_DAYS[state.zoom];
        // Start a few days before today so today is visible
        const offset = state.zoom === 'week' ? 1 : Math.floor(days * 0.2);
        state.timelineStart = addDays(today, -offset);
    }

    // ===== Render: Sidebar =====
    function renderSidebar() {
        const list = $('#project-list');
        list.innerHTML = state.projects.map(p => {
            const active = state.currentProject?.key === p.key ? ' active' : '';
            return `
                <div class="sidebar-item${active}" data-key="${p.key}">
                    <span class="sidebar-item-key">${esc(p.key)}</span>
                    <span class="sidebar-item-name">${esc(p.name)}</span>
                </div>`;
        }).join('');

        list.querySelectorAll('.sidebar-item').forEach(el => {
            el.addEventListener('click', () => selectProject(el.dataset.key));
        });
    }

    // ===== Render: Header =====
    function renderHeader() {
        const p = state.currentProject;
        $('#project-name').textContent = p ? p.name : 'Select a project';
        $('#project-desc').textContent = p ? (p.description || '') : '';
    }

    // ===== Render: Mini Calendar (sidebar, for timeline) =====
    function renderMiniCalendar() {
        const container = $('#mini-calendar');
        if (!container) return;

        // Show mini calendar only when timeline is active
        if (state.view !== 'timeline') {
            container.classList.add('hidden');
            return;
        }
        container.classList.remove('hidden');

        const d = state.miniCalDate;
        const year = d.getFullYear();
        const month = d.getMonth();
        const firstDay = new Date(year, month, 1);
        const lastDay = new Date(year, month + 1, 0);
        const startDay = firstDay.getDay();
        const daysInMonth = lastDay.getDate();
        const today = todayDate();

        // Build set of dates with tasks
        const taskDates = new Set();
        for (const t of state.tasks) {
            if (t.due_date) taskDates.add(t.due_date);
        }

        const totalCells = Math.ceil((startDay + daysInMonth) / 7) * 7;
        let cells = '';

        for (let i = 0; i < totalCells; i++) {
            const dayNum = i - startDay + 1;
            const isCurrentMonth = dayNum >= 1 && dayNum <= daysInMonth;
            let cellDate = null;
            let displayNum = '';
            let classes = 'mini-cal-day';

            if (isCurrentMonth) {
                const dateObj = new Date(year, month, dayNum);
                cellDate = toDateStr(dateObj);
                displayNum = dayNum;
                if (dateObj.getTime() === today.getTime()) classes += ' today';
                if (taskDates.has(cellDate)) classes += ' has-tasks';
            } else if (dayNum < 1) {
                const prevLast = new Date(year, month, 0).getDate();
                displayNum = prevLast + dayNum;
                classes += ' other-month';
            } else {
                displayNum = dayNum - daysInMonth;
                classes += ' other-month';
            }

            cells += `<div class="${classes}" data-date="${cellDate || ''}">${displayNum}</div>`;
        }

        container.innerHTML = `
            <div class="mini-cal-header">
                <span class="mini-cal-title">${MONTH_NAMES_FULL[month]} ${year}</span>
                <div class="mini-cal-nav">
                    <button class="mini-cal-nav-btn" id="mini-cal-prev">&lsaquo;</button>
                    <button class="mini-cal-nav-btn" id="mini-cal-next">&rsaquo;</button>
                </div>
            </div>
            <div class="mini-cal-grid">
                ${DAY_NAMES_MINI.map(n => `<div class="mini-cal-day-header">${n}</div>`).join('')}
                ${cells}
            </div>`;

        // Events
        $('#mini-cal-prev').addEventListener('click', () => {
            state.miniCalDate = new Date(year, month - 1, 1);
            renderMiniCalendar();
        });
        $('#mini-cal-next').addEventListener('click', () => {
            state.miniCalDate = new Date(year, month + 1, 1);
            renderMiniCalendar();
        });

        container.querySelectorAll('.mini-cal-day').forEach(el => {
            el.addEventListener('click', () => {
                const dateStr = el.dataset.date;
                if (!dateStr) return;
                scrollToDate(parseDate(dateStr));
            });
        });
    }

    // ===== Render: Kanban =====
    function renderKanban() {
        const content = $('#content');
        content.className = 'content';
        const tasks = filteredTasks();
        const taskIds = new Set(tasks.map(t => t.id));
        // Only root-level tasks appear as top-level cards; subtasks render inside parent
        const rootTasks = tasks.filter(t => !t.parent || !taskIds.has(t.parent));

        content.innerHTML = `<div class="kanban">${STATUSES.map(s => {
            const colTasks = rootTasks.filter(t => t.status === s.key);
            return `
                <div class="kanban-column" data-status="${s.key}">
                    <div class="kanban-column-header">
                        <span class="column-dot" style="background:${s.color}"></span>
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

    function renderTaskCard(task) {
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

        return `
            <div class="task-card" draggable="true" data-task-id="${task.id}">
                <div class="task-card-header">
                    <span class="task-key">${esc(task.key)}</span>
                    ${parentKey ? `<span class="task-parent-key">${esc(parentKey)}</span>` : ''}
                </div>
                <div class="task-title">${esc(task.title)}</div>
                <div class="task-meta">
                    <span class="priority-dot ${task.priority}" title="${PRIORITY_LABELS[task.priority]}"></span>
                    ${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}
                    ${task.due_date ? `<span class="due-date-badge${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
                </div>
                ${subtaskHtml}
            </div>`;
    }

    function renderSubtaskKanbanItem(task) {
        const statusInfo = STATUSES.find(s => s.key === task.status);
        const overdue = task.status !== 'done' && isOverdue(task.due_date);
        return `<div class="subtask-item" data-task-id="${task.id}">
            <span class="subtask-item-dot" style="background:${statusInfo ? statusInfo.color : '#94a3b8'}"></span>
            <span class="subtask-item-key">${esc(task.key)}</span>
            <span class="subtask-item-title">${esc(task.title)}</span>
            ${task.due_date ? `<span class="subtask-item-due${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
        </div>`;
    }

    // ===== Render: Calendar =====
    function renderCalendar() {
        const content = $('#content');
        content.className = 'content';
        const d = state.calendarDate;
        const year = d.getFullYear();
        const month = d.getMonth();
        const firstDay = new Date(year, month, 1);
        const lastDay = new Date(year, month + 1, 0);
        const startDay = firstDay.getDay(); // 0=Sun
        const daysInMonth = lastDay.getDate();
        const today = new Date();

        const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

        // Build tasks by date
        const tasksByDate = {};
        const tasks = filteredTasks();
        for (const t of tasks) {
            if (t.due_date) {
                if (!tasksByDate[t.due_date]) tasksByDate[t.due_date] = [];
                tasksByDate[t.due_date].push(t);
            }
        }

        // Build day cells
        const totalCells = Math.ceil((startDay + daysInMonth) / 7) * 7;
        let cells = '';
        for (let i = 0; i < totalCells; i++) {
            const dayNum = i - startDay + 1;
            const isCurrentMonth = dayNum >= 1 && dayNum <= daysInMonth;
            let cellDate = '';
            let displayNum = '';
            let otherClass = '';

            if (isCurrentMonth) {
                cellDate = `${year}-${String(month + 1).padStart(2, '0')}-${String(dayNum).padStart(2, '0')}`;
                displayNum = dayNum;
            } else if (dayNum < 1) {
                const prevLast = new Date(year, month, 0).getDate();
                displayNum = prevLast + dayNum;
                otherClass = ' other-month';
            } else {
                displayNum = dayNum - daysInMonth;
                otherClass = ' other-month';
            }

            const isToday = isCurrentMonth &&
                today.getFullYear() === year &&
                today.getMonth() === month &&
                today.getDate() === dayNum;

            const dayTasks = tasksByDate[cellDate] || [];
            const maxShow = 3;

            cells += `
                <div class="calendar-day${otherClass}${isToday ? ' today' : ''}">
                    <div class="calendar-day-num">${displayNum}</div>
                    ${dayTasks.slice(0, maxShow).map(t => `
                        <div class="calendar-task status-${t.status}" data-task-id="${t.id}" title="${esc(t.key)}: ${esc(t.title)}">
                            ${esc(t.key)} ${esc(t.title)}
                        </div>
                    `).join('')}
                    ${dayTasks.length > maxShow ? `<div class="calendar-more">+${dayTasks.length - maxShow} mais</div>` : ''}
                </div>`;
        }

        content.innerHTML = `
            <div class="calendar">
                <div class="calendar-nav">
                    <div class="calendar-nav-buttons">
                        <button class="calendar-nav-btn" id="cal-prev">&larr;</button>
                        <button class="calendar-nav-btn" id="cal-today">Hoje</button>
                        <button class="calendar-nav-btn" id="cal-next">&rarr;</button>
                    </div>
                    <h2>${MONTH_NAMES_FULL[month]} ${year}</h2>
                </div>
                <div class="calendar-grid">
                    ${dayNames.map(n => `<div class="calendar-day-header">${n}</div>`).join('')}
                    ${cells}
                </div>
            </div>`;

        // Calendar nav events
        $('#cal-prev').addEventListener('click', () => {
            state.calendarDate = new Date(year, month - 1, 1);
            renderCalendar();
        });
        $('#cal-next').addEventListener('click', () => {
            state.calendarDate = new Date(year, month + 1, 1);
            renderCalendar();
        });
        $('#cal-today').addEventListener('click', () => {
            state.calendarDate = new Date();
            renderCalendar();
        });

        // Calendar task clicks
        content.querySelectorAll('.calendar-task').forEach(el => {
            el.addEventListener('click', (e) => {
                e.stopPropagation();
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });
    }

    // Build a depth-ordered list for a task group, respecting collapse state
    function buildGroupHierarchy(groupTasks) {
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

    // ===== Render: Table =====
    function renderTable() {
        const content = $('#content');
        content.className = 'content no-padding';
        const tasks = filteredTasks();
        const sorted = sortTasks(tasks);
        const groups = groupTasksByStatus(sorted);

        const allIds = sorted.map(t => t.id);
        const allSelected = allIds.length > 0 && allIds.every(id => state.selectedIds.has(id));

        let html = '';

        // Bulk actions bar
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

        // Render each status group
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
                <th class="col-labels"><div class="th-inner${sortClass('labels')}" data-sort="labels">Labels${sortArrow('labels')}</div></th>
            </tr></thead>`;
            html += '<tbody>';

            for (const { task, depth, hasGroupChildren } of buildGroupHierarchy(groupTasks)) {
                const selected = state.selectedIds.has(task.id);
                const overdue = task.status !== 'done' && isOverdue(task.due_date);
                const collapsed = state.collapsedSubtasks.has(task.id);
                const indent = depth * 20;
                const connector = depth > 0 ? '<span class="tree-line">└─</span>' : '';
                const toggleBtn = hasGroupChildren
                    ? `<button class="tree-toggle" data-task-id="${task.id}" title="${collapsed ? 'Expand' : 'Collapse'}">${collapsed ? '▶' : '▼'}</button>`
                    : '<span class="tree-toggle-spacer"></span>';
                html += `
                    <tr data-task-id="${task.id}" class="${selected ? 'selected' : ''}${depth > 0 ? ' subtask-row' : ''}">
                        <td>
                            <div class="row-checkbox">
                                <input type="checkbox" class="task-cb" data-task-id="${task.id}" ${selected ? 'checked' : ''}>
                            </div>
                        </td>
                        <td><span class="cell-key">${esc(task.key)}</span></td>
                        <td>
                            <div class="cell-title-tree" style="padding-left:${indent}px">
                                ${connector}${toggleBtn}<span class="cell-title">${esc(task.title)}</span>
                            </div>
                        </td>
                        <td>
                            <span class="status-badge status-${task.status}" data-task-id="${task.id}">
                                <span class="status-badge-dot"></span>
                                ${STATUS_LABELS[task.status]}
                            </span>
                        </td>
                        <td>
                            <span class="cell-priority">
                                <span class="priority-dot ${task.priority}"></span>
                                <span class="priority-label">${PRIORITY_LABELS[task.priority]}</span>
                            </span>
                        </td>
                        <td><span class="cell-due-date${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span></td>
                        <td><span class="cell-labels">${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}</span></td>
                    </tr>`;
            }

            html += '</tbody></table>';
            html += '</div>'; // status-group-body
            html += '</div>'; // status-group
        }

        // Empty state when no tasks match filter
        if (sorted.length === 0) {
            html += '<div class="empty-state"><p>No tasks found</p></div>';
        }

        html += '</div>'; // table-container

        content.innerHTML = html;

        setupTableEvents();
    }

    function sortClass(col) {
        if (state.sortColumn !== col) return '';
        return ` sorted${state.sortDirection === 'desc' ? ' desc' : ''}`;
    }

    function sortArrow(col) {
        if (state.sortColumn !== col) {
            return ' <span class="sort-arrow">&#9650;</span>';
        }
        return state.sortDirection === 'asc'
            ? ' <span class="sort-arrow">&#9650;</span>'
            : ' <span class="sort-arrow">&#9660;</span>';
    }

    // ===== Table Events =====
    function setupTableEvents() {
        // Sort headers
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
                renderTable();
            });
        });

        // Group toggle
        document.querySelectorAll('.status-group-header').forEach(hdr => {
            hdr.addEventListener('click', () => {
                const group = hdr.dataset.group;
                if (state.collapsedGroups.has(group)) {
                    state.collapsedGroups.delete(group);
                } else {
                    state.collapsedGroups.add(group);
                }
                renderTable();
            });
        });

        // Tree toggle (subtask expand/collapse in table)
        document.querySelectorAll('.tree-toggle').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                toggleSubtree(parseInt(btn.dataset.taskId));
                renderTable();
            });
        });

        // Row clicks (open modal)
        document.querySelectorAll('.data-table tbody tr').forEach(row => {
            row.addEventListener('click', (e) => {
                // Don't open modal when clicking checkbox or status badge
                if (e.target.closest('.row-checkbox') || e.target.closest('.status-badge')) return;
                const taskId = parseInt(row.dataset.taskId);
                openTaskModal(taskId);
            });
        });

        // Checkbox: individual
        document.querySelectorAll('.task-cb').forEach(cb => {
            cb.addEventListener('change', (e) => {
                e.stopPropagation();
                const taskId = parseInt(cb.dataset.taskId);
                if (cb.checked) {
                    state.selectedIds.add(taskId);
                } else {
                    state.selectedIds.delete(taskId);
                }
                renderTable();
            });
            cb.addEventListener('click', (e) => e.stopPropagation());
        });

        // Checkbox: select all (per group)
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
                renderTable();
            });
            cb.addEventListener('click', (e) => e.stopPropagation());
        });

        // Bulk bar close
        const bulkClose = document.getElementById('bulk-bar-close');
        if (bulkClose) {
            bulkClose.addEventListener('click', () => {
                state.selectedIds.clear();
                renderTable();
            });
        }

        // Bulk move (status change)
        const bulkMoveBtn = document.getElementById('bulk-move-btn');
        if (bulkMoveBtn) {
            bulkMoveBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                const dd = document.getElementById('bulk-status-dropdown');
                if (dd) dd.classList.toggle('hidden');
            });
        }

        // Bulk status option clicks
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
                    showToast(taskIds.length + ' task(s) updated', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
                const dd = document.getElementById('bulk-status-dropdown');
                if (dd) dd.classList.add('hidden');
            });
        });

        // Bulk archive
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
                    showToast(taskIds.length + ' task(s) archived', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
            });
        }

        // Bulk delete
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
                    showToast(taskIds.length + ' task(s) deleted', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
            });
        }

        // Status badge clicks -> dropdown
        document.querySelectorAll('.status-badge').forEach(badge => {
            badge.addEventListener('click', (e) => {
                e.stopPropagation();
                const taskId = parseInt(badge.dataset.taskId);
                toggleStatusDropdown(badge, taskId);
            });
        });
    }

    // ===== Status Dropdown =====
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
                renderTable();
            });
        });

        // Close on outside click
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

    function closeStatusDropdown() {
        const existing = document.getElementById('active-status-dropdown');
        if (existing) existing.remove();
        state.openStatusDropdown = null;
        document.removeEventListener('click', closeStatusDropdownOnOutside);
    }

    // ===== Render: Timeline =====
    function renderTimeline() {
        const content = $('#content');
        content.className = 'content timeline-mode';
        const tasks = filteredTasks();
        const range = getTimelineRange();
        const colWidth = COL_WIDTHS[state.zoom];
        const today = todayDate();

        // Update CSS variable for column width
        document.documentElement.style.setProperty('--timeline-col-width', colWidth + 'px');

        // Separate scheduled vs unscheduled
        const scheduled = tasks.filter(t => t.due_date || t.created_at);
        const unscheduled = tasks.filter(t => !t.due_date && !t.created_at);

        // Group by status
        const grouped = {};
        for (const s of STATUSES) grouped[s.key] = [];
        for (const t of scheduled) {
            if (grouped[t.status]) grouped[t.status].push(t);
        }

        // Build header
        const headerCols = range.columns.map(col => {
            if (col.type === 'week') {
                const wn = getWeekNumber(col.date);
                const rangeLabel = `${col.date.getDate()}/${col.date.getMonth() + 1} - ${col.endDate.getDate()}/${col.endDate.getMonth() + 1}`;
                return `<div class="timeline-date-col week-col">
                    <span class="timeline-date-week-label">S${wn}</span>
                    <span class="timeline-date-week-range">${rangeLabel}</span>
                </div>`;
            }
            const d = col.date;
            const isToday = d.getTime() === today.getTime();
            const weekend = isWeekend(d);
            let classes = 'timeline-date-col';
            if (isToday) classes += ' today';
            if (weekend) classes += ' weekend';

            const showMonth = d.getDate() === 1 || col === range.columns[0];
            return `<div class="${classes}">
                ${showMonth ? `<span class="timeline-date-month">${MONTH_NAMES[d.getMonth()]}</span>` : '<span class="timeline-date-month">&nbsp;</span>'}
                <span class="timeline-date-day">${d.getDate()}</span>
                <span class="timeline-date-weekday">${DAY_NAMES[d.getDay()]}</span>
            </div>`;
        }).join('');

        // Build swimlanes
        const swimlanesHtml = STATUSES.map(s => {
            const laneTasks = grouped[s.key];
            const collapsed = state.collapsedSwimlanes[s.key] || false;
            const laneIds = new Set(laneTasks.map(t => t.id));
            // Root tasks: no parent in this same swimlane
            const laneRoots = laneTasks.filter(t => !t.parent || !laneIds.has(t.parent));

            const taskRows = laneRoots.map(t => {
                const laneSubtasks = laneTasks.filter(sub => sub.parent === t.id);
                const hasSubtasks = laneSubtasks.length > 0;
                const subtreeCollapsed = state.collapsedSubtasks.has(t.id);

                const gridCells = range.columns.map(col => {
                    let cls = 'timeline-grid-cell';
                    if (col.type === 'week') cls += ' week-col';
                    else if (isWeekend(col.date)) cls += ' weekend';
                    return `<div class="${cls}"></div>`;
                }).join('');

                const subRows = (hasSubtasks && !subtreeCollapsed) ? laneSubtasks.map(sub => {
                    const subGridCells = range.columns.map(col => {
                        let cls = 'timeline-grid-cell';
                        if (col.type === 'week') cls += ' week-col';
                        else if (isWeekend(col.date)) cls += ' weekend';
                        return `<div class="${cls}"></div>`;
                    }).join('');
                    return `<div class="timeline-task-row timeline-subtask-row" data-task-id="${sub.id}">
                        <div class="timeline-task-label timeline-subtask-label" data-task-id="${sub.id}">
                            <span class="timeline-subtask-indent"></span>
                            <span class="task-label-priority ${sub.priority}"></span>
                            <span class="task-label-text">${esc(sub.title)}</span>
                            <span class="task-label-key">${esc(sub.key)}</span>
                        </div>
                        <div class="timeline-task-grid" data-task-id="${sub.id}">
                            ${subGridCells}
                        </div>
                    </div>`;
                }).join('') : '';

                const toggleBtn = hasSubtasks
                    ? `<button class="timeline-subtree-toggle" data-task-id="${t.id}">${subtreeCollapsed ? '▶' : '▼'}</button>`
                    : '';

                return `<div class="timeline-task-row" data-task-id="${t.id}">
                    <div class="timeline-task-label" data-task-id="${t.id}">
                        ${toggleBtn}
                        <span class="task-label-priority ${t.priority}"></span>
                        <span class="task-label-text">${esc(t.title)}</span>
                        <span class="task-label-key">${esc(t.key)}</span>
                    </div>
                    <div class="timeline-task-grid" data-task-id="${t.id}">
                        ${gridCells}
                    </div>
                </div>${subRows}`;
            }).join('');

            const totalRows = laneRoots.reduce((n, t) => {
                const laneSubtasks = laneTasks.filter(sub => sub.parent === t.id);
                return n + 1 + (state.collapsedSubtasks.has(t.id) ? 0 : laneSubtasks.length);
            }, 0);

            return `<div class="timeline-swimlane" data-status="${s.key}">
                <div class="timeline-swimlane-header" data-status="${s.key}">
                    <span class="swimlane-dot" style="background:${s.color}"></span>
                    <span class="swimlane-label">${s.label}</span>
                    <span class="swimlane-count">${laneTasks.length}</span>
                    <span class="swimlane-toggle${collapsed ? ' collapsed' : ''}">&#9660;</span>
                </div>
                <div class="timeline-swimlane-body${collapsed ? ' collapsed' : ''}" style="max-height:${collapsed ? '0' : totalRows * 50 + 'px'}">
                    ${taskRows}
                </div>
            </div>`;
        }).join('');

        // Unscheduled section
        const unschedCollapsed = state.unscheduledCollapsed;
        const unscheduledHtml = unscheduled.length > 0 ? `
            <div class="timeline-unscheduled">
                <div class="timeline-unscheduled-header" id="unscheduled-header">
                    <span class="swimlane-toggle${unschedCollapsed ? ' collapsed' : ''}">&#9660;</span>
                    <span class="unscheduled-label">Sem Data</span>
                    <span class="unscheduled-count">${unscheduled.length}</span>
                </div>
                <div class="timeline-unscheduled-body${unschedCollapsed ? ' collapsed' : ''}" id="unscheduled-body" style="max-height:${unschedCollapsed ? '0' : unscheduled.length * 40 + 'px'}">
                    ${unscheduled.map(t => {
                        const statusInfo = STATUSES.find(s => s.key === t.status);
                        return `<div class="unscheduled-task-row" data-task-id="${t.id}">
                            <span class="unscheduled-status-dot" style="background:${statusInfo ? statusInfo.color : '#94a3b8'}"></span>
                            <span class="unscheduled-task-title">${esc(t.title)}</span>
                            <span class="unscheduled-task-key">${esc(t.key)}</span>
                            <span class="unscheduled-task-priority ${t.priority}"></span>
                        </div>`;
                    }).join('')}
                </div>
            </div>` : '';

        content.innerHTML = `
            <div class="timeline-wrapper">
                <div class="timeline-container" id="timeline-container">
                    <div class="timeline-header">
                        <div class="timeline-header-label">Tasks</div>
                        <div class="timeline-header-dates">${headerCols}</div>
                    </div>
                    <div class="timeline-body" id="timeline-body">
                        ${swimlanesHtml}
                    </div>
                </div>
                ${unscheduledHtml}
            </div>`;

        // Position task bars
        positionTaskBars(range, colWidth, today);

        // Place today marker
        placeTodayMarker(range, colWidth, today);

        // Set up events
        setupTimelineEvents();
        setupSwimlaneToggles();
        setupSubtreeToggles();

        // Scroll to today
        scrollToTodayInitial(range, colWidth, today);
    }

    function positionTaskBars(range, colWidth, today) {
        const tasks = filteredTasks().filter(t => t.due_date || t.created_at);

        for (const task of tasks) {
            const gridEl = document.querySelector(`.timeline-task-grid[data-task-id="${task.id}"]`);
            if (!gridEl) continue;

            const dueDate = parseDate(task.due_date);
            const createdDate = parseDate(task.created_at);

            if (dueDate && createdDate && daysBetween(createdDate, dueDate) > 0) {
                // Bar spanning created_at to due_date
                const startOffset = getColumnOffset(createdDate, range, colWidth);
                const endOffset = getColumnOffset(dueDate, range, colWidth) + colWidth;

                if (startOffset === null && endOffset === null) continue;

                const barLeft = startOffset !== null ? startOffset : 0;
                const barRight = endOffset !== null ? endOffset : range.columns.length * colWidth;
                const barWidth = Math.max(barRight - barLeft, 8);

                const bar = document.createElement('div');
                bar.className = `timeline-task-bar status-${task.status}`;
                bar.style.left = barLeft + 'px';
                bar.style.width = barWidth + 'px';
                bar.dataset.taskId = task.id;

                // Short label on bar
                if (barWidth > 60) {
                    bar.textContent = task.title.length > 20 ? task.title.slice(0, 18) + '...' : task.title;
                }

                // Resize handle
                const handle = document.createElement('div');
                handle.className = 'timeline-bar-handle';
                handle.dataset.taskId = task.id;
                bar.appendChild(handle);

                gridEl.appendChild(bar);

                setupBarDrag(handle, task, range, colWidth);
                setupBarTooltip(bar, task);
                bar.addEventListener('click', (e) => {
                    if (!e.target.classList.contains('timeline-bar-handle')) {
                        openTaskModal(task.id);
                    }
                });
            } else if (dueDate) {
                // Single dot for due_date only
                const offset = getColumnOffset(dueDate, range, colWidth);
                if (offset === null) continue;

                const dot = document.createElement('div');
                dot.className = `timeline-task-dot status-${task.status}`;
                dot.style.left = (offset + colWidth / 2) + 'px';
                dot.dataset.taskId = task.id;
                gridEl.appendChild(dot);

                setupBarTooltip(dot, task);
                dot.addEventListener('click', () => openTaskModal(task.id));
            } else if (createdDate) {
                // Dot at created_at
                const offset = getColumnOffset(createdDate, range, colWidth);
                if (offset === null) continue;

                const dot = document.createElement('div');
                dot.className = `timeline-task-dot status-${task.status}`;
                dot.style.left = (offset + colWidth / 2) + 'px';
                dot.dataset.taskId = task.id;
                gridEl.appendChild(dot);

                setupBarTooltip(dot, task);
                dot.addEventListener('click', () => openTaskModal(task.id));
            }
        }
    }

    function getColumnOffset(date, range, colWidth) {
        if (state.zoom === 'quarter') {
            // Find which week column this date falls in
            for (let i = 0; i < range.columns.length; i++) {
                const col = range.columns[i];
                if (date >= col.date && date <= col.endDate) {
                    // Position within the week
                    const daysIntoWeek = daysBetween(col.date, date);
                    const fraction = daysIntoWeek / 7;
                    return i * colWidth + fraction * colWidth;
                }
            }
            // Out of range
            if (date < range.startDate) return null;
            if (date > range.endDate) return null;
            return null;
        }

        // Day columns
        const dayOffset = daysBetween(range.startDate, date);
        if (dayOffset < 0 || dayOffset >= range.columns.length) return null;
        return dayOffset * colWidth;
    }

    function placeTodayMarker(range, colWidth, today) {
        const container = $('#timeline-container');
        if (!container) return;

        const offset = getColumnOffset(today, range, colWidth);
        if (offset === null) return;

        const labelWidth = 180; // --swimlane-label-width
        const markerX = labelWidth + offset + colWidth / 2;

        // Marker in header
        const marker = document.createElement('div');
        marker.className = 'timeline-today-marker';
        marker.style.left = markerX + 'px';
        container.appendChild(marker);

        // Dashed line through body
        const body = $('#timeline-body');
        if (body) {
            const line = document.createElement('div');
            line.className = 'timeline-today-line';
            line.style.left = markerX + 'px';
            container.appendChild(line);
        }
    }

    function scrollToTodayInitial(range, colWidth, today) {
        const container = $('#timeline-container');
        if (!container) return;

        const offset = getColumnOffset(today, range, colWidth);
        if (offset === null) return;

        // Scroll so today is roughly in view
        const labelWidth = 180;
        const targetScroll = labelWidth + offset - container.clientWidth / 3;
        container.scrollLeft = Math.max(0, targetScroll);
    }

    function scrollToDate(date) {
        // Update timeline start so the date is visible
        const days = ZOOM_DAYS[state.zoom];
        const offset = Math.floor(days * 0.2);
        state.timelineStart = addDays(date, -offset);
        renderContent();
    }

    // ===== Bar Drag (Resize due_date) =====
    function setupBarDrag(handle, task, range, colWidth) {
        let dragging = false;
        let startX = 0;
        let origWidth = 0;
        let barEl = null;

        handle.addEventListener('mousedown', (e) => {
            e.preventDefault();
            e.stopPropagation();
            dragging = true;
            startX = e.clientX;
            barEl = handle.parentElement;
            origWidth = barEl.offsetWidth;
            handle.classList.add('dragging');

            const onMouseMove = (e) => {
                if (!dragging) return;
                const dx = e.clientX - startX;
                const newWidth = Math.max(colWidth, origWidth + dx);
                barEl.style.width = newWidth + 'px';
            };

            const onMouseUp = async (e) => {
                if (!dragging) return;
                dragging = false;
                handle.classList.remove('dragging');
                document.removeEventListener('mousemove', onMouseMove);
                document.removeEventListener('mouseup', onMouseUp);

                // Calculate new due_date from bar width
                const finalWidth = barEl.offsetWidth;
                const barLeft = parseFloat(barEl.style.left);
                const barEnd = barLeft + finalWidth;

                // Find the date at barEnd
                const newDueDate = dateAtOffset(barEnd, range, colWidth);
                if (newDueDate && toDateStr(newDueDate) !== task.due_date) {
                    task.due_date = toDateStr(newDueDate);
                    await api.updateTask(state.currentProject.key, task.id, { due_date: task.due_date });
                    await refreshTasks();
                    renderContent();
                }
            };

            document.addEventListener('mousemove', onMouseMove);
            document.addEventListener('mouseup', onMouseUp);
        });
    }

    function dateAtOffset(offset, range, colWidth) {
        if (state.zoom === 'quarter') {
            const colIndex = Math.floor(offset / colWidth);
            if (colIndex < 0 || colIndex >= range.columns.length) return null;
            const col = range.columns[colIndex];
            const fraction = (offset - colIndex * colWidth) / colWidth;
            const dayOffset = Math.round(fraction * 7);
            return addDays(col.date, Math.min(dayOffset, 6));
        }
        const dayIndex = Math.floor(offset / colWidth);
        if (dayIndex < 0 || dayIndex >= range.columns.length) return null;
        return range.columns[dayIndex].date;
    }

    // ===== Tooltip =====
    let tooltipEl = null;

    function getTooltip() {
        if (!tooltipEl) {
            tooltipEl = document.createElement('div');
            tooltipEl.className = 'timeline-tooltip hidden';
            document.body.appendChild(tooltipEl);
        }
        return tooltipEl;
    }

    function setupBarTooltip(el, task) {
        el.addEventListener('mouseenter', (e) => {
            const tip = getTooltip();
            const statusInfo = STATUSES.find(s => s.key === task.status);

            let dueDateInfo = '';
            if (task.due_date) {
                const dt = parseDate(task.due_date);
                dueDateInfo = `<span class="tooltip-meta-item">
                    <span class="tooltip-meta-label">Prazo:</span>
                    <span class="tooltip-meta-value">${formatDateShort(dt)}</span>
                </span>`;
            }

            tip.innerHTML = `
                <div class="tooltip-title">${esc(task.key)} — ${esc(task.title)}</div>
                <div class="tooltip-meta">
                    <span class="tooltip-meta-item">
                        <span class="tooltip-status-dot" style="background:${statusInfo ? statusInfo.color : '#94a3b8'}"></span>
                        <span class="tooltip-meta-value">${STATUS_LABELS[task.status] || task.status}</span>
                    </span>
                    <span class="tooltip-meta-item">
                        <span class="tooltip-meta-label">Prioridade:</span>
                        <span class="tooltip-meta-value">${PRIORITY_LABELS[task.priority] || task.priority}</span>
                    </span>
                    ${dueDateInfo}
                </div>`;
            tip.classList.remove('hidden');
            positionTooltip(e);
        });

        el.addEventListener('mousemove', positionTooltip);

        el.addEventListener('mouseleave', () => {
            const tip = getTooltip();
            tip.classList.add('hidden');
        });
    }

    function positionTooltip(e) {
        const tip = getTooltip();
        const x = e.clientX + 12;
        const y = e.clientY - 10;
        tip.style.left = x + 'px';
        tip.style.top = y + 'px';

        // Keep in viewport
        const rect = tip.getBoundingClientRect();
        if (rect.right > window.innerWidth) {
            tip.style.left = (e.clientX - rect.width - 12) + 'px';
        }
        if (rect.bottom > window.innerHeight) {
            tip.style.top = (e.clientY - rect.height - 10) + 'px';
        }
    }

    // ===== Timeline Events =====
    function setupTimelineEvents() {
        // Click on task labels
        document.querySelectorAll('.timeline-task-label').forEach(el => {
            el.addEventListener('click', () => {
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });

        // Click on unscheduled rows
        document.querySelectorAll('.unscheduled-task-row').forEach(el => {
            el.addEventListener('click', () => {
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });

        // Unscheduled toggle
        const unschedHeader = document.getElementById('unscheduled-header');
        if (unschedHeader) {
            unschedHeader.addEventListener('click', () => {
                state.unscheduledCollapsed = !state.unscheduledCollapsed;
                const body = document.getElementById('unscheduled-body');
                const toggle = unschedHeader.querySelector('.swimlane-toggle');
                if (body) body.classList.toggle('collapsed', state.unscheduledCollapsed);
                if (toggle) toggle.classList.toggle('collapsed', state.unscheduledCollapsed);
            });
        }
    }

    function setupSwimlaneToggles() {
        document.querySelectorAll('.timeline-swimlane-header').forEach(el => {
            el.addEventListener('click', () => {
                const status = el.dataset.status;
                state.collapsedSwimlanes[status] = !state.collapsedSwimlanes[status];
                const body = el.nextElementSibling;
                const toggle = el.querySelector('.swimlane-toggle');
                if (body) body.classList.toggle('collapsed', state.collapsedSwimlanes[status]);
                if (toggle) toggle.classList.toggle('collapsed', state.collapsedSwimlanes[status]);
            });
        });
    }

    // ===== Drag & Drop (Kanban) - Optimistic =====
    function setupDragDrop() {
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
                    // Optimistic update: save old status, render immediately
                    const oldStatus = task.status;
                    task.status = newStatus;
                    renderKanban();

                    // Call API
                    const result = await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
                    if (!result) {
                        // Revert on failure
                        task.status = oldStatus;
                        renderKanban();
                        showToast('Failed to move task. Reverted.', 'error');
                    }
                }
            });
        });
    }

    // ===== Card Clicks (Kanban) =====
    function setupCardClicks() {
        document.querySelectorAll('.task-card').forEach(card => {
            card.addEventListener('click', () => {
                openTaskModal(parseInt(card.dataset.taskId));
            });
        });
    }

    // ===== Subtree Toggles (shared: kanban, table, timeline) =====
    function setupSubtreeToggles() {
        document.querySelectorAll('.subtask-toggle, .tree-toggle, .timeline-subtree-toggle').forEach(el => {
            el.addEventListener('click', (e) => {
                e.stopPropagation();
                const taskId = parseInt(el.dataset.taskId);
                toggleSubtree(taskId);
                renderContent();
            });
        });
    }

    // ===== Render: Dashboard =====
    async function renderDashboard() {
        const content = $('#content');
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
        const overdueCount = data.overdue_count || 0;
        const upcomingDue = data.upcoming_due || [];
        const recentlyUpdated = data.recently_updated || [];

        // Build status bars
        let statusBarsHtml = '';
        for (const s of STATUSES) {
            const count = statusCounts[s.key] || 0;
            const pct = totalTasks > 0 ? ((count / totalTasks) * 100).toFixed(1) : 0;
            statusBarsHtml += `
                <div class="dashboard-status-row">
                    <div class="dashboard-status-label">
                        <span class="dashboard-status-dot" style="background:${s.color}"></span>
                        ${s.label}
                    </div>
                    <div class="dashboard-status-bar-track">
                        <div class="dashboard-status-bar-fill" style="width:${pct}%;background:${s.color}"></div>
                    </div>
                    <span class="dashboard-status-count">${count}</span>
                </div>`;
        }

        // Build upcoming due list
        let upcomingHtml = '';
        if (upcomingDue.length > 0) {
            upcomingHtml = upcomingDue.map(t => {
                const overdue = t.status !== 'done' && isOverdue(t.due_date);
                return `<div class="dashboard-task-item" data-task-id="${t.id}">
                    <span class="dashboard-task-key">${esc(t.key)}</span>
                    <span class="dashboard-task-title">${esc(t.title)}</span>
                    <span class="dashboard-task-due${overdue ? ' overdue' : ''}">${formatDate(t.due_date)}</span>
                </div>`;
            }).join('');
        } else {
            upcomingHtml = '<p class="dashboard-empty">No tasks due in the next 7 days</p>';
        }

        // Build recently updated list
        let recentHtml = '';
        if (recentlyUpdated.length > 0) {
            recentHtml = recentlyUpdated.map(t => {
                const statusInfo = STATUSES.find(s => s.key === t.status);
                return `<div class="dashboard-task-item" data-task-id="${t.id}">
                    <span class="dashboard-task-key">${esc(t.key)}</span>
                    <span class="dashboard-task-title">${esc(t.title)}</span>
                    <span class="status-badge status-${t.status}">
                        <span class="status-badge-dot"></span>
                        ${STATUS_LABELS[t.status]}
                    </span>
                    ${t.updated_at ? `<span class="dashboard-task-time">${relativeTime(t.updated_at)}</span>` : ''}
                </div>`;
            }).join('');
        } else {
            recentHtml = '<p class="dashboard-empty">No recently updated tasks</p>';
        }

        content.innerHTML = `
            <div class="dashboard">
                <div class="dashboard-grid">
                    <div class="dashboard-card dashboard-card-wide">
                        <h3 class="dashboard-card-title">Status Distribution</h3>
                        <div class="dashboard-status-bars">
                            ${statusBarsHtml}
                        </div>
                        <div class="dashboard-total">Total: ${totalTasks} task(s)</div>
                    </div>
                    <div class="dashboard-card">
                        <h3 class="dashboard-card-title">Overdue</h3>
                        <div class="dashboard-big-number ${overdueCount > 0 ? 'overdue' : ''}">${overdueCount}</div>
                        <p class="dashboard-card-subtitle">task(s) overdue</p>
                    </div>
                    <div class="dashboard-card">
                        <h3 class="dashboard-card-title">Upcoming Deadlines (7 days)</h3>
                        <div class="dashboard-task-list">
                            ${upcomingHtml}
                        </div>
                    </div>
                    <div class="dashboard-card dashboard-card-wide">
                        <h3 class="dashboard-card-title">Recently Updated</h3>
                        <div class="dashboard-task-list">
                            ${recentHtml}
                        </div>
                    </div>
                </div>
            </div>`;

        // Click handlers for dashboard task items
        content.querySelectorAll('.dashboard-task-item').forEach(el => {
            el.addEventListener('click', () => {
                const taskId = parseInt(el.dataset.taskId);
                if (taskId) openTaskModal(taskId);
            });
        });
    }

    // ===== Modal =====
    function openTaskModal(taskId) {
        const overlay = $('#modal-overlay');
        const form = $('#task-form');
        const deleteBtn = $('#btn-delete');
        const archiveBtn = $('#btn-archive');

        // Remove any stale hierarchy info from previous open
        const existingInfo = document.getElementById('task-hierarchy-info');
        if (existingInfo) existingInfo.remove();

        if (taskId) {
            state.editingTaskId = taskId;
            const task = state.tasks.find(t => t.id === taskId);
            if (!task) return;
            $('#modal-title').textContent = task.key;
            $('#task-title').value = task.title;
            $('#task-status').value = task.status;
            $('#task-priority').value = task.priority;
            $('#task-due-date').value = task.due_date || '';
            $('#task-labels').value = task.labels.join(', ');
            $('#task-description').value = task.description || '';
            deleteBtn.classList.remove('hidden');

            // Archive button logic
            if (archiveBtn) {
                archiveBtn.classList.remove('hidden');
                if (task.archived) {
                    archiveBtn.textContent = 'Desarquivar';
                } else {
                    archiveBtn.textContent = 'Archive';
                }
            }

            // Inject parent link + subtask list
            const parentTask = task.parent ? state.tasks.find(t => t.id === task.parent) : null;
            const subtasks = getSubtasks(task);
            if (parentTask || subtasks.length > 0) {
                const info = document.createElement('div');
                info.id = 'task-hierarchy-info';
                info.className = 'task-hierarchy-info';
                let infoHtml = '';
                if (parentTask) {
                    const pKey = `${state.currentProject.key}-${parentTask.id}`;
                    infoHtml += `<div class="hierarchy-parent">
                        <span class="hierarchy-label">Parent:</span>
                        <button class="hierarchy-link" data-task-id="${parentTask.id}">${esc(pKey)} — ${esc(parentTask.title)}</button>
                    </div>`;
                }
                if (subtasks.length > 0) {
                    const sp = getSubtaskProgress(task);
                    infoHtml += `<div class="hierarchy-subtasks">
                        <span class="hierarchy-label">Subtasks</span>
                        ${sp ? `<span class="subtask-badge">${sp.done}/${sp.total}</span>` : ''}
                        <div class="hierarchy-subtask-list">
                            ${subtasks.map(sub => {
                                const si = STATUSES.find(s => s.key === sub.status);
                                return `<div class="hierarchy-subtask-item" data-task-id="${sub.id}">
                                    <span class="subtask-item-dot" style="background:${si ? si.color : '#94a3b8'}"></span>
                                    <span class="hierarchy-subtask-key">${esc(sub.key)}</span>
                                    <span class="hierarchy-subtask-title">${esc(sub.title)}</span>
                                    <span class="hierarchy-subtask-status status-${sub.status}">${STATUS_LABELS[sub.status]}</span>
                                </div>`;
                            }).join('')}
                        </div>
                    </div>`;
                }
                info.innerHTML = infoHtml;
                document.querySelector('.modal-header').insertAdjacentElement('afterend', info);

                info.querySelectorAll('.hierarchy-subtask-item').forEach(el => {
                    el.addEventListener('click', () => openTaskModal(parseInt(el.dataset.taskId)));
                });
                const parentLink = info.querySelector('.hierarchy-link');
                if (parentLink) {
                    parentLink.addEventListener('click', () => openTaskModal(parseInt(parentLink.dataset.taskId)));
                }
            }

            // Load comments
            loadComments(taskId);
        } else {
            state.editingTaskId = null;
            $('#modal-title').textContent = 'New Task';
            form.reset();
            $('#task-status').value = 'todo';
            $('#task-priority').value = 'medium';
            deleteBtn.classList.add('hidden');

            if (archiveBtn) archiveBtn.classList.add('hidden');

            // Clear comments section
            const commentsSection = $('#comments-section');
            if (commentsSection) commentsSection.innerHTML = '';
        }

        // Populate parent select
        const parentSelect = $('#task-parent');
        parentSelect.innerHTML = '<option value="">None</option>';
        state.tasks.forEach(t => {
            if (t.id !== taskId) {
                const opt = document.createElement('option');
                opt.value = t.id;
                opt.textContent = `${t.key} — ${t.title}`;
                parentSelect.appendChild(opt);
            }
        });

        if (taskId) {
            const task = state.tasks.find(t => t.id === taskId);
            if (task?.parent) parentSelect.value = task.parent;
        }

        overlay.classList.remove('hidden');
        setTimeout(() => $('#task-title').focus(), 50);
    }

    // ===== Comments =====
    async function loadComments(taskId) {
        const commentsSection = $('#comments-section');
        if (!commentsSection || !state.currentProject) return;

        commentsSection.innerHTML = '<p class="comments-loading">Loading comments...</p>';

        const comments = await api.getComments(state.currentProject.key, taskId);

        let commentsHtml = '<h4 class="comments-title">Comments</h4>';

        if (comments && comments.length > 0) {
            commentsHtml += '<div class="comments-list">';
            for (const c of comments) {
                commentsHtml += `
                    <div class="comment-item">
                        <div class="comment-header">
                            <span class="comment-author">${esc(c.author || 'Anonymous')}</span>
                            <span class="comment-time">${relativeTime(c.created_at)}</span>
                        </div>
                        <div class="comment-body">${esc(c.body || '')}</div>
                    </div>`;
            }
            commentsHtml += '</div>';
        } else {
            commentsHtml += '<p class="comments-empty">No comments yet</p>';
        }

        commentsHtml += `
            <div class="comment-form">
                <input type="text" class="comment-author-input" id="comment-author" placeholder="Seu nome" />
                <textarea class="comment-body-input" id="comment-body" placeholder="Add a comment..." rows="2"></textarea>
                <button type="button" class="btn btn-primary" id="btn-add-comment">Comment</button>
            </div>`;

        commentsSection.innerHTML = commentsHtml;

        // Add comment button handler
        const addBtn = document.getElementById('btn-add-comment');
        if (addBtn) {
            addBtn.addEventListener('click', async () => {
                const author = document.getElementById('comment-author').value.trim();
                const body = document.getElementById('comment-body').value.trim();
                if (!body) return;

                addBtn.disabled = true;
                const result = await api.createComment(state.currentProject.key, taskId, {
                    author: author || 'Anonymous',
                    body: body,
                });
                addBtn.disabled = false;

                if (result) {
                    showToast('Comment added', 'success');
                    loadComments(taskId);
                }
            });
        }
    }

    // ===== Activity Feed =====
    async function toggleActivityPanel() {
        const panel = $('#activity-panel');
        if (!panel) return;

        if (panel.classList.contains('hidden')) {
            panel.classList.remove('hidden');
            await loadActivity();
        } else {
            panel.classList.add('hidden');
        }
    }

    async function loadActivity() {
        const panel = $('#activity-panel');
        if (!panel || !state.currentProject) return;

        panel.innerHTML = '<div class="activity-loading"><div class="spinner"></div><p>Loading activity...</p></div>';

        const activities = await api.getActivity(state.currentProject.key, 50);

        if (!activities || activities.length === 0) {
            panel.innerHTML = '<div class="activity-header"><h3>Activity</h3><button class="activity-close-btn" id="activity-close">&times;</button></div><p class="activity-empty">No recent activity</p>';
            setupActivityClose();
            return;
        }

        let html = '<div class="activity-header"><h3>Activity</h3><button class="activity-close-btn" id="activity-close">&times;</button></div>';
        html += '<div class="activity-list">';

        for (const entry of activities) {
            const actionLabel = ACTIVITY_ACTION_LABELS[entry.action] || entry.action;
            const timeLabel = relativeTime(entry.created_at || entry.timestamp);
            const actor = entry.actor || entry.user || 'Sistema';
            const target = entry.task_key || entry.target || '';

            let detailHtml = '';
            if (entry.action === 'field_changed' && entry.field) {
                detailHtml = `<span class="activity-detail">${esc(entry.field)}: ${esc(entry.old_value || '?')} &rarr; ${esc(entry.new_value || '?')}</span>`;
            }

            html += `
                <div class="activity-entry">
                    <div class="activity-entry-header">
                        <span class="activity-actor">${esc(actor)}</span>
                        <span class="activity-action">${actionLabel}</span>
                        ${target ? `<span class="activity-target">${esc(target)}</span>` : ''}
                    </div>
                    ${detailHtml}
                    <span class="activity-time">${timeLabel}</span>
                </div>`;
        }

        html += '</div>';
        panel.innerHTML = html;

        setupActivityClose();
    }

    function setupActivityClose() {
        const closeBtn = document.getElementById('activity-close');
        if (closeBtn) {
            closeBtn.addEventListener('click', () => {
                const panel = $('#activity-panel');
                if (panel) panel.classList.add('hidden');
            });
        }
    }

    function closeModal() {
        $('#modal-overlay').classList.add('hidden');
        state.editingTaskId = null;
    }

    async function handleFormSubmit(e) {
        e.preventDefault();
        if (!state.currentProject) return;

        setSubmitDisabled(true);

        const data = {
            title: $('#task-title').value.trim(),
            status: $('#task-status').value,
            priority: $('#task-priority').value,
            description: $('#task-description').value.trim(),
            labels: $('#task-labels').value.split(',').map(l => l.trim()).filter(Boolean),
        };

        const dueDate = $('#task-due-date').value;
        if (dueDate) data.due_date = dueDate;

        const parentVal = $('#task-parent').value;
        if (parentVal) data.parent = parseInt(parentVal);

        const key = state.currentProject.key;

        let result;
        if (state.editingTaskId) {
            result = await api.updateTask(key, state.editingTaskId, data);
        } else {
            result = await api.createTask(key, data);
        }

        setSubmitDisabled(false);

        if (result) {
            showToast(state.editingTaskId ? 'Task updated' : 'Task created', 'success');
            closeModal();
            await refreshTasks();
            render();
        }
    }

    async function handleDelete() {
        if (!state.editingTaskId || !state.currentProject) return;
        if (!confirm('Delete this task?')) return;

        await api.deleteTask(state.currentProject.key, state.editingTaskId);
        showToast('Task deleted', 'success');
        closeModal();
        await refreshTasks();
        render();
    }

    async function handleArchive() {
        if (!state.editingTaskId || !state.currentProject) return;
        const task = state.tasks.find(t => t.id === state.editingTaskId);
        if (!task) return;

        const newArchived = !task.archived;
        const result = await api.updateTask(state.currentProject.key, state.editingTaskId, {
            archived: newArchived,
        });

        if (result) {
            showToast(newArchived ? 'Task archived' : 'Task unarchived', 'success');
            closeModal();
            await refreshTasks();
            render();
        }
    }

    // ===== Navigation =====
    async function selectProject(key) {
        state.currentProject = state.projects.find(p => p.key === key);
        state.selectedIds.clear();
        loadSubtreeState(key);
        showLoading();
        await refreshTasks();
        hideLoading();
        render();
    }

    async function refreshTasks() {
        if (state.currentProject) {
            const opts = {};
            if (!state.showArchived) {
                opts.archived = false;
            }
            state.tasks = await api.getTasks(state.currentProject.key, opts);
        }
    }

    // ===== View Switching =====
    function switchView(view) {
        state.view = view;

        // Update view tabs
        document.querySelectorAll('#view-tabs .view-tab').forEach(tab => {
            tab.classList.toggle('active', tab.dataset.view === view);
        });

        // Show/hide timeline-specific controls
        const zoomTabs = $('#zoom-tabs');
        const timelineNav = $('#timeline-nav');
        if (view === 'timeline') {
            zoomTabs.classList.remove('hidden');
            timelineNav.classList.remove('hidden');
        } else {
            zoomTabs.classList.add('hidden');
            timelineNav.classList.add('hidden');
        }

        // Update mini calendar visibility
        renderMiniCalendar();

        renderContent();
    }

    function switchZoom(zoom) {
        state.zoom = zoom;
        document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab => {
            tab.classList.toggle('active', tab.dataset.zoom === zoom);
        });
        initTimelineStart();
        renderContent();
    }

    function renderContent() {
        if (!state.currentProject) return;
        if (state.loading) return;
        if (state.view === 'kanban') renderKanban();
        else if (state.view === 'calendar') renderCalendar();
        else if (state.view === 'table') renderTable();
        else if (state.view === 'timeline') renderTimeline();
        else if (state.view === 'dashboard') renderDashboard();
    }

    function render() {
        renderSidebar();
        renderHeader();
        renderMiniCalendar();
        renderContent();
    }

    // ===== Mobile Hamburger Menu =====
    function setupHamburgerMenu() {
        const hamburgerBtn = document.getElementById('hamburger-btn');
        const sidebar = document.getElementById('sidebar');
        const overlay = document.getElementById('sidebar-overlay');

        if (hamburgerBtn && sidebar) {
            hamburgerBtn.addEventListener('click', () => {
                sidebar.classList.toggle('open');
                if (overlay) overlay.classList.toggle('visible');
            });
        }

        if (overlay && sidebar) {
            overlay.addEventListener('click', () => {
                sidebar.classList.remove('open');
                overlay.classList.remove('visible');
            });
        }
    }

    // ===== Events =====
    // View tabs
    document.querySelectorAll('#view-tabs .view-tab').forEach(tab => {
        tab.addEventListener('click', () => switchView(tab.dataset.view));
    });

    // Zoom tabs (timeline)
    document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab => {
        tab.addEventListener('click', () => switchZoom(tab.dataset.zoom));
    });

    // Timeline navigation
    $('#btn-prev').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), -shift);
        renderContent();
    });

    $('#btn-today').addEventListener('click', () => {
        initTimelineStart();
        renderContent();
    });

    $('#btn-next').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), shift);
        renderContent();
    });

    // New task
    $('#btn-new-task').addEventListener('click', () => {
        if (state.currentProject) openTaskModal(null);
    });

    // Modal
    $('#modal-close').addEventListener('click', closeModal);
    $('#btn-cancel').addEventListener('click', closeModal);
    $('#task-form').addEventListener('submit', handleFormSubmit);
    $('#btn-delete').addEventListener('click', handleDelete);

    // Archive button
    const archiveBtn = $('#btn-archive');
    if (archiveBtn) {
        archiveBtn.addEventListener('click', handleArchive);
    }

    $('#modal-overlay').addEventListener('click', (e) => {
        if (e.target === e.currentTarget) closeModal();
    });

    // Search
    $('#search-input').addEventListener('input', (e) => {
        state.searchQuery = e.target.value;
        renderContent();
    });

    // Archive toggle
    const showArchivedCb = document.getElementById('show-archived');
    if (showArchivedCb) {
        showArchivedCb.addEventListener('change', async () => {
            state.showArchived = showArchivedCb.checked;
            await refreshTasks();
            render();
        });
    }

    // Activity panel toggle button
    const activityBtn = document.getElementById('btn-activity');
    if (activityBtn) {
        activityBtn.addEventListener('click', toggleActivityPanel);
    }

    // Keyboard shortcuts
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            closeStatusDropdown();
            closeModal();
            // Close activity panel on Escape
            const panel = $('#activity-panel');
            if (panel && !panel.classList.contains('hidden')) {
                panel.classList.add('hidden');
            }
            // Close sidebar on mobile
            const sidebar = document.getElementById('sidebar');
            const sidebarOverlay = document.getElementById('sidebar-overlay');
            if (sidebar) sidebar.classList.remove('open');
            if (sidebarOverlay) sidebarOverlay.classList.remove('visible');
            return;
        }

        // Only handle shortcuts when no input/textarea is focused
        const active = document.activeElement;
        const isEditing = active && (
            active.tagName === 'INPUT' ||
            active.tagName === 'TEXTAREA' ||
            active.tagName === 'SELECT' ||
            active.isContentEditable
        );
        if (isEditing) return;

        switch (e.key) {
            case 'n':
                e.preventDefault();
                if (state.currentProject) openTaskModal(null);
                break;
            case '/':
                e.preventDefault();
                const searchInput = $('#search-input');
                if (searchInput) searchInput.focus();
                break;
            case '1':
                e.preventDefault();
                switchView('kanban');
                break;
            case '2':
                e.preventDefault();
                switchView('table');
                break;
            case '3':
                e.preventDefault();
                switchView('timeline');
                break;
            case '4':
                e.preventDefault();
                switchView('calendar');
                break;
            case '5':
                e.preventDefault();
                switchView('dashboard');
                break;
        }
    });

    // ===== Init =====
    async function init() {
        initTimelineStart();
        setupHamburgerMenu();

        showLoading();
        state.projects = await api.getProjects();
        if (state.projects.length > 0) {
            await selectProject(state.projects[0].key);
        }
        hideLoading();
        render();
    }

    init();
})();
