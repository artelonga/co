// ===== Calendar and Gantt views =====
import { state } from '../state.js';
import { api } from '../api.js';
import { esc, filteredTasks, addDays, utcToLocalDate } from '../helpers.js';
import { MONTH_NAMES_FULL, MONTH_NAMES } from '../constants.js';

let _openTaskModal = () => {};
let _openZoomModal = () => {};
let _apiFetch = () => {};

export function injectCalendarCallbacks(callbacks) {
    _openTaskModal = callbacks.openTaskModal;
    _openZoomModal = callbacks.openZoomModal;
    _apiFetch = callbacks.apiFetch;
}

export async function renderCalendar() {
    const content = document.querySelector('#content');
    content.className = 'content';
    const d = state.calendarDate;
    const year = d.getFullYear();
    const month = d.getMonth();
    const firstDay = new Date(year, month, 1);
    const lastDay = new Date(year, month + 1, 0);
    const startDay = firstDay.getDay();
    const daysInMonth = lastDay.getDate();
    const today = new Date();

    const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

    const manifest = state.universeManifest;
    let calendarSemantic = null;
    if (manifest) {
        for (const ct of manifest.content_types || []) {
            if (ct.presentation && ct.presentation.calendar && ct.presentation.calendar.date_field) {
                const fieldDef = ct.schema && ct.schema[ct.presentation.calendar.date_field];
                if (fieldDef && fieldDef.semantic) {
                    calendarSemantic = fieldDef.semantic;
                    break;
                }
            }
        }
    }

    const tasksByDate = {};
    if (calendarSemantic) {
        const fromDate = `${year}-${String(month + 1).padStart(2, '0')}-01`;
        const toDate = `${year}-${String(month + 1).padStart(2, '0')}-${String(daysInMonth).padStart(2, '0')}`;
        const slug = state.currentUniverseSlug;
        try {
            const entries = await api.getEntriesByDate(slug, calendarSemantic, fromDate, toDate);
            state.calendarEntries = entries;
            for (const e of entries) {
                const calField = (manifest.content_types || []).find(ct =>
                    ct.presentation && ct.presentation.calendar
                );
                const fieldName = calField && calField.presentation.calendar.date_field;
                const rawDate = fieldName && e.frontmatter && e.frontmatter[fieldName];
                const localDate = rawDate ? utcToLocalDate(rawDate) : '';
                if (localDate) {
                    if (!tasksByDate[localDate]) tasksByDate[localDate] = [];
                    tasksByDate[localDate].push({ ...e, _isEntry: true });
                }
            }
        } catch (err) {
            console.warn('calendar: failed to fetch entries by date', err);
        }
    } else {
        const tasks = filteredTasks();
        for (const t of tasks) {
            if (t.due_date) {
                if (!tasksByDate[t.due_date]) tasksByDate[t.due_date] = [];
                tasksByDate[t.due_date].push(t);
            }
        }
    }

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

        const dayItems = tasksByDate[cellDate] || [];
        const maxShow = 3;

        cells += `
            <div class="calendar-day${otherClass}${isToday ? ' today' : ''}">
                <div class="calendar-day-num">${displayNum}</div>
                ${dayItems.slice(0, maxShow).map(item => item._isEntry
                    ? `<div class="calendar-task" title="${esc(item.title || item.path)}">${esc(item.title || item.path)}</div>`
                    : `<div class="calendar-task status-${item.status}" data-task-id="${item.id}" title="${esc(item.key)}: ${esc(item.title)}">${esc(item.key)} ${esc(item.title)}</div>`
                ).join('')}
                ${dayItems.length > maxShow ? `<div class="calendar-more">+${dayItems.length - maxShow} mais</div>` : ''}
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

    document.querySelector('#cal-prev').addEventListener('click', () => {
        state.calendarDate = new Date(year, month - 1, 1);
        renderCalendar();
    });
    document.querySelector('#cal-next').addEventListener('click', () => {
        state.calendarDate = new Date(year, month + 1, 1);
        renderCalendar();
    });
    document.querySelector('#cal-today').addEventListener('click', () => {
        state.calendarDate = new Date();
        renderCalendar();
    });

    content.querySelectorAll('.calendar-task').forEach(el => {
        el.addEventListener('click', (e) => {
            e.stopPropagation();
            _openTaskModal(parseInt(el.dataset.taskId));
        });
    });
}

export async function renderGantt(viewDef) {
    const content = document.querySelector('#content');
    content.className = 'content';
    content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div></div>';

    const slug = state.currentUniverseSlug;
    const startField = viewDef.date_start;
    const endField = viewDef.date_end;

    let entries = [];
    try {
        entries = await api.getEntriesByDate(slug, startField, null, null);
    } catch (err) {
        console.warn('gantt: failed to fetch entries', err);
    }

    entries.sort((a, b) => {
        const da = (a.frontmatter && a.frontmatter[startField]) || '';
        const db = (b.frontmatter && b.frontmatter[startField]) || '';
        return da.localeCompare(db);
    });

    const today = new Date();
    let minDate = today;
    let maxDate = new Date(today.getFullYear(), today.getMonth() + 2, 1);
    for (const e of entries) {
        const start = e.frontmatter && e.frontmatter[startField];
        const end   = e.frontmatter && e.frontmatter[endField];
        if (start) {
            const d = new Date(start);
            if (d < minDate) minDate = d;
        }
        if (end) {
            const d = new Date(end);
            if (d > maxDate) maxDate = d;
        }
    }
    minDate = addDays(minDate, -7);
    maxDate = addDays(maxDate, 7);

    const totalDays = Math.max(1, Math.round((maxDate - minDate) / 86400000));
    const colW = 28;

    let headerHtml = '';
    let cur = new Date(minDate);
    while (cur <= maxDate) {
        headerHtml += `<div class="gantt-month-label" style="left:${Math.round((cur - minDate) / 86400000) * colW}px">${MONTH_NAMES_FULL[cur.getMonth()]} ${cur.getFullYear()}</div>`;
        cur = new Date(cur.getFullYear(), cur.getMonth() + 1, 1);
    }

    const todayOffset = Math.round((today - minDate) / 86400000) * colW;
    const todayMarker = `<div class="gantt-today-marker" style="left:${todayOffset}px"></div>`;

    const rows = entries.map(e => {
        const title = (e.frontmatter && e.frontmatter.title) || e.path;
        const startStr = e.frontmatter && e.frontmatter[startField];
        const endStr   = e.frontmatter && e.frontmatter[endField];
        const startD = startStr ? new Date(startStr) : null;
        const endD   = endStr   ? new Date(endStr)   : null;

        let barHtml = '';
        if (startD) {
            const barLeft = Math.max(0, Math.round((startD - minDate) / 86400000)) * colW;
            const barRight = endD ? Math.max(0, Math.round((maxDate - endD) / 86400000)) * colW : 0;
            const barWidth = Math.max(colW, totalDays * colW - barLeft - barRight);
            barHtml = `<div class="gantt-bar" style="left:${barLeft}px;width:${barWidth}px" title="${esc(startStr || '')} → ${esc(endStr || '')}"></div>`;
        }

        return `<div class="gantt-row">
            <div class="gantt-row-label" title="${esc(title)}">${esc(title)}</div>
            <div class="gantt-row-track" style="width:${totalDays * colW}px">
                ${barHtml}
            </div>
        </div>`;
    }).join('');

    content.innerHTML = `
        <div class="gantt-view">
            <div class="gantt-header">
                <div class="gantt-header-label">${esc(viewDef.name)}</div>
                <div class="gantt-header-dates" style="width:${totalDays * colW}px;position:relative">
                    ${headerHtml}
                    ${todayMarker}
                </div>
            </div>
            <div class="gantt-body">
                ${rows || '<div class="gantt-empty">Nenhum item com datas declaradas.</div>'}
            </div>
        </div>`;
}

export async function renderEventsTimeline() {
    const content = document.querySelector('#content');
    content.className = 'content events-timeline';
    const manifest = state.universeManifest;
    if (!manifest) {
        content.innerHTML = '<p class="empty-state" style="padding:24px">Sem manifest — adicione `presentation.calendar.date_field` ao tipo do conteúdo.</p>';
        return;
    }

    const ct = (manifest.content_types || []).find(c =>
        c.presentation && c.presentation.calendar && c.presentation.calendar.date_field
    );
    if (!ct) {
        content.innerHTML = '<p class="empty-state" style="padding:24px">Sem campo de data declarado no manifest deste universo.</p>';
        return;
    }
    const dateField = ct.presentation.calendar.date_field;
    const fieldDef = (ct.schema || {})[dateField];
    const semantic = (fieldDef && fieldDef.semantic) || dateField;

    const slug = state.currentUniverseSlug;
    let entries;
    try {
        entries = await api.getEntriesByDate(slug, semantic, null, null);
    } catch (err) {
        content.innerHTML = `<p class="empty-state" style="padding:24px">Erro ao carregar eventos: ${esc(String(err))}</p>`;
        return;
    }
    if (!entries || entries.length === 0) {
        content.innerHTML = '<p class="empty-state" style="padding:24px">Nenhum evento.</p>';
        return;
    }

    const items = entries
        .map(e => {
            const raw = e.frontmatter && e.frontmatter[dateField];
            const dt = raw ? new Date(raw) : null;
            return { e, dt, raw };
        })
        .filter(x => x.dt && !isNaN(x.dt.getTime()))
        .sort((a, b) => a.dt - b.dt);

    const groups = new Map();
    for (const x of items) {
        const key = `${x.dt.getFullYear()}-${String(x.dt.getMonth() + 1).padStart(2, '0')}`;
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(x);
    }

    const monthNames = (typeof MONTH_NAMES !== 'undefined' && MONTH_NAMES.length === 12)
        ? MONTH_NAMES
        : ['Jan','Fev','Mar','Abr','Mai','Jun','Jul','Ago','Set','Out','Nov','Dez'];

    const groupHtml = [...groups.entries()].map(([k, xs]) => {
        const [y, m] = k.split('-').map(Number);
        const monthLabel = `${monthNames[m - 1]} ${y}`;
        const rowsHtml = xs.map(({ e, dt }) => {
            const day = dt.getDate();
            const time = dt.getHours() || dt.getMinutes()
                ? `${String(dt.getHours()).padStart(2, '0')}:${String(dt.getMinutes()).padStart(2, '0')}`
                : '';
            const title = (e.frontmatter && e.frontmatter.title) || e.path || '';
            const path = e.path || '';
            return `
                <div class="event-row" data-entry-path="${esc(path)}">
                    <div class="event-date">
                        <span class="event-day">${day}</span>
                        ${time ? `<span class="event-time">${esc(time)}</span>` : ''}
                    </div>
                    <div class="event-body">
                        <span class="event-title">${esc(title)}</span>
                        <span class="event-path">${esc(path)}</span>
                    </div>
                </div>`;
        }).join('');
        return `
            <section class="events-month">
                <h2 class="events-month-label">${esc(monthLabel)}</h2>
                <div class="events-month-rows">${rowsHtml}</div>
            </section>`;
    }).join('');

    content.innerHTML = `<div class="events-feed" style="padding:16px;max-width:880px;margin:0 auto">${groupHtml}</div>`;

    content.querySelectorAll('.event-row').forEach(row => {
        row.addEventListener('click', async () => {
            const path = row.dataset.entryPath;
            if (!path) return;
            try {
                const entry = await _apiFetch(
                    `/api/v1/universes/${encodeURIComponent(slug)}/entries/${path.split('/').map(encodeURIComponent).join('/')}`
                );
                if (entry && entry.path) _openZoomModal({ ...entry, _universeSlug: slug }, false);
            } catch (_) {}
        });
    });
}
