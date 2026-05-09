// ===== Sidebar, header, usage count =====
import { state } from './state.js';
import { apiFetch } from './api.js';
import { esc, todayDate, toDateStr } from './helpers.js';
import { MONTH_NAMES_FULL, DAY_NAMES_MINI } from './constants.js';

// Callbacks injected by app.js to break circular deps
let _bootAppForUniverse = () => {};
let _selectProject = () => {};
let _renderContent = () => {};
let _showTemplateBanner = () => {};
let _hideTemplateBanner = () => {};

export function injectSidebarCallbacks(callbacks) {
    _bootAppForUniverse = callbacks.bootAppForUniverse;
    _selectProject = callbacks.selectProject;
    _renderContent = callbacks.renderContent;
    _showTemplateBanner = callbacks.showTemplateBanner;
    _hideTemplateBanner = callbacks.hideTemplateBanner;
}

let _setUniverseSlugInUrl = () => {};
export function injectSetUniverseSlugInUrl(fn) { _setUniverseSlugInUrl = fn; }

export function renderSidebar() {
    const list = document.querySelector('#project-list');

    let universeHtml = '';
    if (state.userUniverses && state.userUniverses.length >= 1) {
        const universes = state.userUniverses;
        const seen = new Set(universes.map(u => u.key));
        const childrenByParent = {};
        const topLevel = [];
        universes.forEach(u => {
            if (u.parent_key && seen.has(u.parent_key)) {
                (childrenByParent[u.parent_key] = childrenByParent[u.parent_key] || []).push(u);
            } else {
                topLevel.push(u);
            }
        });

        const renderUniverseItem = (u, depth) => {
            const active = u.key === state.currentUniverseSlug ? ' active' : '';
            const kids = childrenByParent[u.key];
            const hasKids = !!(kids && kids.length);
            const expandKey = `co_universe_tree_${u.key}`;
            const stored = localStorage.getItem(expandKey);
            const containsActive = hasKids
                && kids.some(k => k.key === state.currentUniverseSlug);
            const expanded = stored !== null ? (stored === '1') : containsActive;
            const indent = 12 + depth * 16;
            const chevron = hasKids
                ? `<span class="sidebar-universe-chevron" data-toggle="${u.key}" style="display:inline-block;width:14px;text-align:center;cursor:pointer;user-select:none">${expanded ? '▾' : '▸'}</span>`
                : '<span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>';
            let html = `<div class="sidebar-item sidebar-universe-item${active}" data-universe="${u.key}" style="padding-left:${indent}px">
                ${chevron}<span class="sidebar-item-name">${esc(u.name || u.key)}</span>
            </div>`;
            if (hasKids && expanded) {
                for (const k of kids) html += renderUniverseItem(k, depth + 1);
            }
            return html;
        };

        universeHtml = `<div class="sidebar-universes">
            <div class="sidebar-universe-label">${window.t ? window.t('universes') : 'Universos'}</div>
            ${topLevel.map(u => renderUniverseItem(u, 0)).join('')}
            <hr class="sidebar-divider">
        </div>`;
    }

    list.innerHTML = universeHtml + state.projects.map(p => {
        const active = state.currentProject?.key === p.key ? ' active' : '';
        return `
            <div class="sidebar-item${active}" data-key="${p.key}">
                <span class="sidebar-item-key">${esc(p.key)}</span>
                <span class="sidebar-item-name">${esc(p.name)}</span>
            </div>`;
    }).join('');

    list.querySelectorAll('.sidebar-universe-chevron[data-toggle]').forEach(c => {
        c.addEventListener('click', (e) => {
            e.stopPropagation();
            const k = c.dataset.toggle;
            const expandKey = `co_universe_tree_${k}`;
            const stored = localStorage.getItem(expandKey);
            const wasOpen = stored === null
                ? (state.userUniverses || []).some(u => u.parent_key === k && u.key === state.currentUniverseSlug)
                : stored === '1';
            localStorage.setItem(expandKey, wasOpen ? '0' : '1');
            renderSidebar();
        });
    });

    list.querySelectorAll('.sidebar-universe-item').forEach(el => {
        el.addEventListener('click', async () => {
            const slug = el.dataset.universe;
            if (slug === state.currentUniverseSlug) return;
            if (state.switchingUniverse) return;
            list.querySelectorAll('.sidebar-universe-item').forEach(x => x.classList.remove('active'));
            el.classList.add('active');
            _setUniverseSlugInUrl(slug);
            state.currentUniverseSlug = slug;
            state.isTemplate = (slug === 'template');
            if (state.isTemplate) _showTemplateBanner(); else _hideTemplateBanner();
            await _bootAppForUniverse(slug);
        });
        el.addEventListener('dblclick', (e) => {
            e.stopPropagation();
            const slug = el.dataset.universe;
            const nameSpan = el.querySelector('.sidebar-item-name');
            const oldName = nameSpan.textContent;
            const input = document.createElement('input');
            input.type = 'text';
            input.value = oldName;
            input.className = 'sidebar-rename-input';
            nameSpan.replaceWith(input);
            input.focus();
            input.select();
            const save = async () => {
                const newName = input.value.trim() || oldName;
                const span = document.createElement('span');
                span.className = 'sidebar-item-name';
                span.textContent = newName;
                input.replaceWith(span);
                if (newName !== oldName) {
                    await apiFetch(`/api/v1/universes/${slug}`, {
                        method: 'PUT',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ name: newName }),
                    }, true);
                    const u = state.userUniverses.find(u => u.key === slug);
                    if (u) u.name = newName;
                }
            };
            input.addEventListener('blur', save);
            input.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') input.blur();
                if (e.key === 'Escape') { input.value = oldName; input.blur(); }
            });
        });
    });

    list.querySelectorAll('.sidebar-item:not(.sidebar-universe-item)').forEach(el => {
        if (el.dataset.key) el.addEventListener('click', () => _selectProject(el.dataset.key));
    });
}

export function renderHeader() {
    const p = state.currentProject;
    document.querySelector('#project-name').textContent = p ? p.name : (window.t ? window.t('select_project') : 'Selecione um projeto');
    document.querySelector('#project-desc').textContent = p ? (p.description || '') : '';
    renderUsageCount();
}

export function renderUsageCount() {
    const el = document.getElementById('usage-count');
    if (!el) return;
    const info = state.universeInfo;
    if (!info || state.isTemplate) {
        el.classList.add('hidden');
        return;
    }
    el.classList.remove('hidden');
    if (info.is_anonymous) {
        el.textContent = `${info.content_count} / 100 entradas`;
    } else {
        el.textContent = `${info.content_count} entradas`;
    }
}

export function incrementLocalUsageCount() {
    if (state.universeInfo) {
        state.universeInfo.content_count += 1;
        renderUsageCount();
    }
}

export function renderHeaderUserArea(me) {
    const area = document.getElementById('header-user-area');
    if (!area) return;
    if (me) {
        const name = me.display_name || me.usuario || me.email || '';
        area.innerHTML = `
            <span class="header-user-badge" title="${esc(name)}">${esc(name)}</span>
            <button class="btn btn-ghost btn-signout" id="btn-signout" title="${window.t ? window.t('sign_out') : 'Sair'}">
                <span class="material-symbols-outlined" style="font-size:18px">logout</span>
            </button>`;
        document.getElementById('btn-signout').addEventListener('click', async () => {
            await apiFetch('/api/v1/auth/logout', { method: 'POST' }, true);
            state.userUniverses = [];
            state.currentUniverseSlug = 'template';
            state.isTemplate = true;
            localStorage.removeItem('co_local_universe');
            window.location.href = '/';
        });
    } else {
        area.innerHTML = `<button class="btn btn-ghost header-entrar" id="btn-header-entrar" data-i18n="action.login">${window.t ? window.t('action.login') : 'Entrar'}</button>`;
        const btn = document.getElementById('btn-header-entrar');
        if (btn) btn.addEventListener('click', _showLoginModal);
    }
}

// Callback for login modal (injected to break circular dep)
let _showLoginModal = () => {};
export function injectShowLoginModal(fn) { _showLoginModal = fn; }

export function renderUserBadge(me) {
    const sidebarUser = document.getElementById('sidebar-user');
    const nameEl = document.getElementById('user-display-name');
    if (sidebarUser) sidebarUser.classList.remove('hidden');
    if (nameEl) nameEl.textContent = me.display_name || me.email;
    renderHeaderUserArea(me);
}

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

// Callback for timeline scroll-to-date
let _scrollToDate = () => {};
export function injectScrollToDate(fn) { _scrollToDate = fn; }

export function setupHamburgerMenu() {
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

    const toggleProjects = document.getElementById('btn-toggle-projects');
    const projectList = document.getElementById('project-list');
    if (toggleProjects && projectList) {
        toggleProjects.addEventListener('click', () => {
            const expanded = toggleProjects.getAttribute('aria-expanded') === 'true';
            toggleProjects.setAttribute('aria-expanded', String(!expanded));
            projectList.classList.toggle('collapsed', expanded);
        });
    }
}
