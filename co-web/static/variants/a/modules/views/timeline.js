// ===== Timeline view =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, filteredTasks, parseDate, toDateStr, todayDate, isWeekend, getWeekNumber, addDays, daysBetween, formatDateShort } from '../helpers.js';
import { STATUSES, STATUS_LABELS, PRIORITY_LABELS, ZOOM_DAYS, COL_WIDTHS, MONTH_NAMES, DAY_NAMES } from '../constants.js';

let _openTaskModal = () => {};
let _refreshTasks = async () => {};
let _renderContent = () => {};

export function injectTimelineCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
    _refreshTasks = callbacks.refreshTasks;
    _renderContent = callbacks.renderContent;
}

export function getTimelineRange() {
    const days = ZOOM_DAYS[state.zoom];
    const start = state.timelineStart || todayDate();
    if (state.zoom === 'quarter') {
        const columns = [];
        let current = new Date(start);
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
    const columns = [];
    for (let i = 0; i < days; i++) {
        const date = addDays(start, i);
        columns.push({ date, type: 'day' });
    }
    return { columns, startDate: start, endDate: addDays(start, days - 1) };
}

export function initTimelineStart() {
    const today = todayDate();
    const days = ZOOM_DAYS[state.zoom];
    const offset = state.zoom === 'week' ? 1 : Math.floor(days * 0.2);
    state.timelineStart = addDays(today, -offset);
}

export function getColumnOffset(date, range, colWidth) {
    if (state.zoom === 'quarter') {
        for (let i = 0; i < range.columns.length; i++) {
            const col = range.columns[i];
            if (date >= col.date && date <= col.endDate) {
                const daysIntoWeek = daysBetween(col.date, date);
                const fraction = daysIntoWeek / 7;
                return i * colWidth + fraction * colWidth;
            }
        }
        if (date < range.startDate) return null;
        if (date > range.endDate) return null;
        return null;
    }
    const dayOffset = daysBetween(range.startDate, date);
    if (dayOffset < 0 || dayOffset >= range.columns.length) return null;
    return dayOffset * colWidth;
}

export function dateAtOffset(offset, range, colWidth) {
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

export function renderTimeline() {
    const content = document.querySelector('#content');
    content.className = 'content timeline-mode';
    const tasks = filteredTasks();
    const range = getTimelineRange();
    const colWidth = COL_WIDTHS[state.zoom];
    const today = todayDate();

    document.documentElement.style.setProperty('--timeline-col-width', colWidth + 'px');

    const scheduled = tasks.filter(t => t.due_date || t.created_at);
    const unscheduled = tasks.filter(t => !t.due_date && !t.created_at);

    const grouped = {};
    for (const s of STATUSES) grouped[s.key] = [];
    for (const t of scheduled) {
        if (grouped[t.status]) grouped[t.status].push(t);
    }

    const headerCols = range.columns.map((col, idx) => {
        if (col.type === 'week') {
            const isFirst = idx === 0;
            const showMonth = isFirst || col.date.getDate() <= 7;
            const monthLabel = showMonth ? MONTH_NAMES[col.date.getMonth()] : '';
            const wn = getWeekNumber(col.date);
            return `<div class="timeline-date-col week-col">
                <span class="timeline-date-month">${monthLabel || '&nbsp;'}</span>
                <span class="timeline-date-week-label">W${wn}</span>
            </div>`;
        }
        const d = col.date;
        const isToday = d.getTime() === today.getTime();
        const weekend = isWeekend(d);
        let classes = 'timeline-date-col';
        if (isToday) classes += ' today';
        if (weekend) classes += ' weekend';

        let topLabel;
        if (state.zoom === 'month') {
            const isMonday = d.getDay() === 1;
            const isFirst = idx === 0;
            topLabel = (isMonday || isFirst)
                ? `<span class="timeline-date-week-label">W${getWeekNumber(d)}</span>`
                : '<span class="timeline-date-week-label">&nbsp;</span>';
        } else {
            const showMonth = d.getDate() === 1 || idx === 0;
            topLabel = showMonth
                ? `<span class="timeline-date-month">${MONTH_NAMES[d.getMonth()]}</span>`
                : '<span class="timeline-date-month">&nbsp;</span>';
        }

        return `<div class="${classes}">
            ${topLabel}
            <span class="timeline-date-day">${d.getDate()}</span>
            <span class="timeline-date-weekday">${DAY_NAMES[d.getDay()]}</span>
        </div>`;
    }).join('');

    const swimlanesHtml = STATUSES.map(s => {
        const laneTasks = grouped[s.key];
        const collapsed = state.collapsedSwimlanes[s.key] || false;
        const laneIds = new Set(laneTasks.map(t => t.id));
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

    positionTaskBars(range, colWidth, today);
    renderDependencyArrows();
    placeTodayMarker(range, colWidth, today);
    setupTimelineEvents();
    setupSwimlaneToggles();
    // setupSubtreeToggles is in kanban.js — caller (app.js renderContent) sets it up
    scrollToTodayInitial(range, colWidth, today);
}

export function positionTaskBars(range, colWidth, today) {
    const tasks = filteredTasks().filter(t => t.due_date || t.created_at);

    for (const task of tasks) {
        const gridEl = document.querySelector(`.timeline-task-grid[data-task-id="${task.id}"]`);
        if (!gridEl) continue;

        const dueDate = parseDate(task.due_date);
        const createdDate = parseDate(task.created_at);

        if (dueDate && createdDate && daysBetween(createdDate, dueDate) > 0) {
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

            if (barWidth > 60) {
                bar.textContent = task.title.length > 20 ? task.title.slice(0, 18) + '...' : task.title;
            }

            const handle = document.createElement('div');
            handle.className = 'timeline-bar-handle';
            handle.dataset.taskId = task.id;
            bar.appendChild(handle);

            gridEl.appendChild(bar);

            setupBarDrag(handle, task, range, colWidth);
            setupBarTooltip(bar, task);
            bar.addEventListener('click', (e) => {
                if (!e.target.classList.contains('timeline-bar-handle')) {
                    _openTaskModal(task.id);
                }
            });
        } else if (dueDate) {
            const offset = getColumnOffset(dueDate, range, colWidth);
            if (offset === null) continue;

            const dot = document.createElement('div');
            dot.className = `timeline-task-dot status-${task.status}`;
            dot.style.left = (offset + colWidth / 2) + 'px';
            dot.dataset.taskId = task.id;
            gridEl.appendChild(dot);

            setupBarTooltip(dot, task);
            dot.addEventListener('click', () => _openTaskModal(task.id));
        } else if (createdDate) {
            const offset = getColumnOffset(createdDate, range, colWidth);
            if (offset === null) continue;

            const dot = document.createElement('div');
            dot.className = `timeline-task-dot status-${task.status}`;
            dot.style.left = (offset + colWidth / 2) + 'px';
            dot.dataset.taskId = task.id;
            gridEl.appendChild(dot);

            setupBarTooltip(dot, task);
            dot.addEventListener('click', () => _openTaskModal(task.id));
        }
    }
}

export function renderDependencyArrows() {
    const container = document.getElementById('timeline-container');
    if (!container) return;

    const existing = container.querySelector('.dep-arrows-svg');
    if (existing) existing.remove();

    const tasks = filteredTasks();
    const taskMap = new Map(tasks.map(t => [t.id, t]));
    const containerRect = container.getBoundingClientRect();
    const scrollLeft = container.scrollLeft;
    const scrollTop = container.scrollTop;

    const arrows = [];
    for (const task of tasks) {
        if (!task.parent) continue;
        const parentTask = taskMap.get(task.parent);
        if (!parentTask) continue;

        const parentBar = container.querySelector(`.timeline-task-bar[data-task-id="${parentTask.id}"]`);
        const childBar = container.querySelector(`.timeline-task-bar[data-task-id="${task.id}"]`);
        if (!parentBar || !childBar) continue;

        const pRect = parentBar.getBoundingClientRect();
        const cRect = childBar.getBoundingClientRect();

        const x1 = pRect.right - containerRect.left + scrollLeft;
        const y1 = pRect.top + pRect.height / 2 - containerRect.top + scrollTop;
        const x2 = cRect.left - containerRect.left + scrollLeft;
        const y2 = cRect.top + cRect.height / 2 - containerRect.top + scrollTop;

        arrows.push({ x1, y1, x2, y2 });
    }

    if (arrows.length === 0) return;

    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.classList.add('dep-arrows-svg');
    svg.setAttribute('width', container.scrollWidth);
    svg.setAttribute('height', container.scrollHeight);

    const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
    const marker = document.createElementNS('http://www.w3.org/2000/svg', 'marker');
    marker.setAttribute('id', 'dep-arrowhead');
    marker.setAttribute('markerWidth', '8');
    marker.setAttribute('markerHeight', '8');
    marker.setAttribute('refX', '7');
    marker.setAttribute('refY', '3');
    marker.setAttribute('orient', 'auto');
    const arrowTip = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    arrowTip.setAttribute('d', 'M0,0 L0,6 L8,3 z');
    arrowTip.setAttribute('fill', '#94a3b8');
    marker.appendChild(arrowTip);
    defs.appendChild(marker);
    svg.appendChild(defs);

    for (const { x1, y1, x2, y2 } of arrows) {
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        const mx = x1 + (x2 - x1) * 0.5;
        path.setAttribute('d', `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`);
        path.setAttribute('stroke', '#94a3b8');
        path.setAttribute('stroke-width', '1.5');
        path.setAttribute('stroke-dasharray', '4,3');
        path.setAttribute('fill', 'none');
        path.setAttribute('marker-end', 'url(#dep-arrowhead)');
        svg.appendChild(path);
    }

    container.appendChild(svg);
}

function placeTodayMarker(range, colWidth, today) {
    const container = document.querySelector('#timeline-container');
    if (!container) return;

    const offset = getColumnOffset(today, range, colWidth);
    if (offset === null) return;

    const labelWidth = 180;
    const markerX = labelWidth + offset + colWidth / 2;

    const marker = document.createElement('div');
    marker.className = 'timeline-today-marker';
    marker.style.left = markerX + 'px';
    container.appendChild(marker);

    const body = document.querySelector('#timeline-body');
    if (body) {
        const line = document.createElement('div');
        line.className = 'timeline-today-line';
        line.style.left = markerX + 'px';
        container.appendChild(line);
    }
}

function scrollToTodayInitial(range, colWidth, today) {
    const container = document.querySelector('#timeline-container');
    if (!container) return;

    const offset = getColumnOffset(today, range, colWidth);
    if (offset === null) return;

    const labelWidth = 180;
    const targetScroll = labelWidth + offset - container.clientWidth / 3;
    container.scrollLeft = Math.max(0, targetScroll);
}

export function scrollToDate(dateStr) {
    const date = parseDate(dateStr);
    if (!date) return;
    const days = ZOOM_DAYS[state.zoom];
    const offset = Math.floor(days * 0.2);
    state.timelineStart = addDays(date, -offset);
    _renderContent();
}

export function setupBarDrag(handle, task, range, colWidth) {
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

            const finalWidth = barEl.offsetWidth;
            const barLeft = parseFloat(barEl.style.left);
            const barEnd = barLeft + finalWidth;

            const newDueDate = dateAtOffset(barEnd, range, colWidth);
            if (newDueDate && toDateStr(newDueDate) !== task.due_date) {
                task.due_date = toDateStr(newDueDate);
                await api.updateTask(state.currentProject.key, task.id, { due_date: task.due_date });
                await _refreshTasks();
                _renderContent();
            }
        };

        document.addEventListener('mousemove', onMouseMove);
        document.addEventListener('mouseup', onMouseUp);
    });
}

let tooltipEl = null;

function getTooltip() {
    if (!tooltipEl) {
        tooltipEl = document.createElement('div');
        tooltipEl.className = 'timeline-tooltip hidden';
        document.body.appendChild(tooltipEl);
    }
    return tooltipEl;
}

export function setupBarTooltip(el, task) {
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

    const rect = tip.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
        tip.style.left = (e.clientX - rect.width - 12) + 'px';
    }
    if (rect.bottom > window.innerHeight) {
        tip.style.top = (e.clientY - rect.height - 10) + 'px';
    }
}

export function setupTimelineEvents() {
    document.querySelectorAll('.timeline-task-label').forEach(el => {
        el.addEventListener('click', () => {
            _openTaskModal(parseInt(el.dataset.taskId));
        });
    });

    document.querySelectorAll('.unscheduled-task-row').forEach(el => {
        el.addEventListener('click', () => {
            _openTaskModal(parseInt(el.dataset.taskId));
        });
    });

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

export function setupSwimlaneToggles() {
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
