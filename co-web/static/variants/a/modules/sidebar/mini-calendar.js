// ===== Mini calendar widget =====
import { state } from '../state.js';
import { todayDate, toDateStr } from '../helpers.js';
import { MONTH_NAMES_FULL, DAY_NAMES_MINI } from '../constants.js';

// Callback for timeline scroll-to-date
let _scrollToDate = () => {};
export function injectScrollToDate(fn) { _scrollToDate = fn; }

export function renderMiniCalendar() {
    const container = document.querySelector('#mini-calendar');
    if (!container) return;

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

    document.querySelector('#mini-cal-prev').addEventListener('click', () => {
        state.miniCalDate = new Date(year, month - 1, 1);
        renderMiniCalendar();
    });
    document.querySelector('#mini-cal-next').addEventListener('click', () => {
        state.miniCalDate = new Date(year, month + 1, 1);
        renderMiniCalendar();
    });

    container.querySelectorAll('.mini-cal-day').forEach(el => {
        el.addEventListener('click', () => {
            const dateStr = el.dataset.date;
            if (!dateStr) return;
            _scrollToDate(dateStr);
        });
    });
}
