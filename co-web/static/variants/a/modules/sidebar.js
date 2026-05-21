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
let _loadMeUniverses = async () => {};
let _renderSidebar = () => {};
let _showToast = () => {};

export function injectSidebarCallbacks(callbacks) {
    _bootAppForUniverse = callbacks.bootAppForUniverse;
    _selectProject = callbacks.selectProject;
    _renderContent = callbacks.renderContent;
    _showTemplateBanner = callbacks.showTemplateBanner;
    _hideTemplateBanner = callbacks.hideTemplateBanner;
    if (callbacks.loadMeUniverses) _loadMeUniverses = callbacks.loadMeUniverses;
    if (callbacks.renderSidebar) _renderSidebar = callbacks.renderSidebar;
    if (callbacks.showToast) _showToast = callbacks.showToast;
}

let _setUniverseSlugInUrl = () => {};
export function injectSetUniverseSlugInUrl(fn) { _setUniverseSlugInUrl = fn; }

// --- Sidebar universe tree helpers ---

function buildChildMap(universes, allUniversesMap = null) {
    const keys = new Set(universes.map(u => u.key));
    const childrenByParent = {};
    const topLevel = [];
    const syntheticByKey = {};
    universes.forEach(u => {
        if (u.parent_key && keys.has(u.parent_key)) {
            (childrenByParent[u.parent_key] = childrenByParent[u.parent_key] || []).push(u);
        } else if (u.parent_key && allUniversesMap && allUniversesMap.has(u.parent_key)) {
            const pk = u.parent_key;
            if (!syntheticByKey[pk]) {
                syntheticByKey[pk] = { ...allUniversesMap.get(pk), _synthetic: true };
                topLevel.push(syntheticByKey[pk]);
            }
            (childrenByParent[pk] = childrenByParent[pk] || []).push(u);
        } else {
            topLevel.push(u);
        }
    });
    return { childrenByParent, topLevel };
}

function renderUniverseItemHtml(u, childrenByParent, depth, showRoleChip) {
    const active = u.key === state.currentUniverseSlug ? ' active' : '';
    const kids = childrenByParent[u.key];
    const hasKids = !!(kids && kids.length);
    const expandKey = `co_universe_tree_${u.key}`;
    const stored = localStorage.getItem(expandKey);
    // Default-expand the tree when EITHER the current universe is this
    // parent (so a user on "tempo" sees its subuniverses below) OR the
    // current universe is one of the descendants. Previously only the
    // descendant case was handled, so navigating to the parent left its
    // children collapsed and looking absent.
    const isSelfActive = u.key === state.currentUniverseSlug;
    const containsActive = hasKids && kids.some(k => k.key === state.currentUniverseSlug);
    const expanded = stored !== null
        ? (stored === '1')
        : (isSelfActive || containsActive);
    const indent = 12 + depth * 16;
    const chevron = hasKids
        ? `<span class="sidebar-universe-chevron" data-toggle="${esc(u.key)}" style="display:inline-block;width:14px;text-align:center;cursor:pointer;user-select:none">${expanded ? '▾' : '▸'}</span>`
        : '<span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>';
    const role = u.role;
    const roleChip = showRoleChip && role && !u._synthetic
        ? `<span class="role-chip">${esc(window.t ? window.t('sidebar.role.' + role) || role : role)}</span>`
        : '';
    const ossChip = u.key === 'co'
        ? `<span class="oss-chip">${esc(window.t ? window.t('sidebar.co_dev_chip') || 'código aberto' : 'código aberto')}</span>`
        : '';
    const subCount = hasKids ? ` (${kids.length})` : '';
    const syntheticClass = u._synthetic ? ' sidebar-universe-synthetic' : '';
    let html = `<div class="sidebar-item sidebar-universe-item${syntheticClass}${active}" data-universe="${esc(u.key)}" style="padding-left:${indent}px">
        ${chevron}<span class="sidebar-item-name">${esc(u.name || u.key)}${subCount}</span>${roleChip}${ossChip}
    </div>`;
    if (hasKids && expanded) {
        for (const k of kids) html += renderUniverseItemHtml(k, childrenByParent, depth + 1, showRoleChip);
    }
    return html;
}

function renderSectionHtml(label, universes, showRoleChip, tooltip = '', allUniversesMap = null) {
    if (!universes || universes.length === 0) return '';
    const { childrenByParent, topLevel } = buildChildMap(universes, allUniversesMap);
    const titleAttr = tooltip ? ` title="${esc(tooltip)}"` : '';
    return `<div class="sidebar-universe-section">
        <div class="sidebar-section-label"${titleAttr}>${esc(label)}</div>
        ${topLevel.map(u => renderUniverseItemHtml(u, childrenByParent, 0, showRoleChip)).join('')}
    </div>`;
}

function renderInviteRowHtml(inv) {
    const t = window.t || (k => k);
    return `<div class="sidebar-invite-row">
        <div class="sidebar-item sidebar-universe-item" style="padding-left:12px">
            <span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>
            <span class="sidebar-item-name">${esc(inv.universe_name)}</span>
        </div>
        <div class="invite-row-actions">
            <button class="btn btn-sm btn-primary invite-accept-btn" data-key="${esc(inv.universe_key)}">${esc(t('sidebar.invite.accept'))}</button>
            <button class="btn btn-sm btn-ghost invite-decline-btn" data-key="${esc(inv.universe_key)}">${esc(t('sidebar.invite.decline'))}</button>
        </div>
    </div>`;
}

function renderDiscoverableItemHtml(u) {
    const t = window.t || (k => k);
    return `<div class="sidebar-item sidebar-discoverable-item" data-universe="${esc(u.key)}" style="padding-left:12px">
        <span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>
        <span class="sidebar-item-name">${esc(u.name || u.key)}</span>
        <button class="btn btn-sm btn-ghost discover-subscribe-btn" data-key="${esc(u.key)}" style="margin-left:auto;font-size:11px">${esc(t('sidebar.discover.subscribe'))}</button>
    </div>`;
}

export function renderSidebar() {
    const list = document.querySelector('#project-list');
    const t = window.t || (k => k);

    let universeHtml = '';
    const me = state.meUniverses;

    if (me) {
        // CO-238: sidebar section clarity — tooltips, owner chip, sub-universe counts, cross-bucket parents
        const allUniversesMap = new Map();
        [...(me.owned || []), ...(me.member || []), ...(me.subscribed || []), ...(me.discoverable || [])]
            .forEach(u => allUniversesMap.set(u.key, u));
        universeHtml += renderSectionHtml(t('sidebar.section.owned'), me.owned, true, t('sidebar.section.owned.tooltip'), allUniversesMap);
        universeHtml += renderSectionHtml(t('sidebar.section.member'), me.member, true, t('sidebar.section.member.tooltip'), allUniversesMap);
        universeHtml += renderSectionHtml(t('sidebar.section.subscribed'), me.subscribed, true, t('sidebar.section.subscribed.tooltip'), allUniversesMap);

        if (me.invited && me.invited.length > 0) {
            universeHtml += `<div class="sidebar-universe-section sidebar-invited-section">
                <div class="sidebar-section-label">🎁 ${esc(t('sidebar.section.invited'))} (${me.invited.length})</div>
                ${me.invited.map(renderInviteRowHtml).join('')}
            </div>`;
        }

        if (me.discoverable && me.discoverable.length > 0) {
            const open = localStorage.getItem('co_sidebar_discover') === '1';
            universeHtml += `<div class="sidebar-universe-section sidebar-discoverable-section">
                <div class="sidebar-section-label sidebar-discover-toggle" data-toggle="discover" style="cursor:pointer">
                    ${open ? '▾' : '▸'} ${esc(t('sidebar.section.discoverable'))}
                </div>
                ${open ? me.discoverable.map(u => renderDiscoverableItemHtml(u)).join('') : ''}
            </div>`;
        }

        if (universeHtml) {
            universeHtml += '<hr class="sidebar-divider">';
        }
    } else if (state.userUniverses && state.userUniverses.length >= 1) {
        // Fallback: flat list for anonymous users or before meUniverses loads.
        // CO-252: pin the 'co' dev universe to the top of the list.
        const sorted = [...state.userUniverses].sort((a, b) => {
            if (a.key === 'co') return -1;
            if (b.key === 'co') return 1;
            return 0;
        });
        const { childrenByParent, topLevel } = buildChildMap(sorted);
        universeHtml = `<div class="sidebar-universes">
            <div class="sidebar-universe-label">${t('universes')}</div>
            ${topLevel.map(u => renderUniverseItemHtml(u, childrenByParent, 0, false)).join('')}
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

    // Universe chevron toggles (tree expand/collapse)
    list.querySelectorAll('.sidebar-universe-chevron[data-toggle]').forEach(c => {
        c.addEventListener('click', (e) => {
            e.stopPropagation();
            const k = c.dataset.toggle;
            const expandKey = `co_universe_tree_${k}`;
            const stored = localStorage.getItem(expandKey);
            const allUniverses = state.userUniverses || [];
            const wasOpen = stored === null
                ? allUniverses.some(u => u.parent_key === k && u.key === state.currentUniverseSlug)
                : stored === '1';
            localStorage.setItem(expandKey, wasOpen ? '0' : '1');
            renderSidebar();
        });
    });

    // Discoverable section toggle
    list.querySelectorAll('.sidebar-discover-toggle').forEach(el => {
        el.addEventListener('click', () => {
            const open = localStorage.getItem('co_sidebar_discover') === '1';
            localStorage.setItem('co_sidebar_discover', open ? '0' : '1');
            renderSidebar();
        });
    });

    // Universe item click (navigate to universe)
    list.querySelectorAll('.sidebar-universe-item').forEach(el => {
        el.addEventListener('click', async () => {
            const slug = el.dataset.universe;
            if (!slug) return;
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
            if (!nameSpan) return;
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
                    if (state.meUniverses) {
                        for (const bucket of ['owned', 'member', 'subscribed']) {
                            const entry = (state.meUniverses[bucket] || []).find(u => u.key === slug);
                            if (entry) entry.name = newName;
                        }
                    }
                }
            };
            input.addEventListener('blur', save);
            input.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') input.blur();
                if (e.key === 'Escape') { input.value = oldName; input.blur(); }
            });
        });
    });

    // Invite accept/decline buttons
    list.querySelectorAll('.invite-accept-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const key = btn.dataset.key;
            if (!key) return;
            // Optimistic: remove the row immediately
            const row = btn.closest('.sidebar-invite-row');
            if (row) row.remove();
            try {
                const resp = await fetch('/api/v1/me/invitations/accept', {
                    method: 'POST',
                    credentials: 'include',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ universe_key: key }),
                });
                await _loadMeUniverses();
                renderSidebar();
                if (resp.ok && _showToast) _showToast(window.t ? window.t('sidebar.invite.accepted') : 'Convite aceito!', 'success');
            } catch (_) {
                await _loadMeUniverses();
                renderSidebar();
            }
        });
    });

    list.querySelectorAll('.invite-decline-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const key = btn.dataset.key;
            if (!key) return;
            const row = btn.closest('.sidebar-invite-row');
            if (row) row.remove();
            // Note: no decline endpoint yet — just refresh to keep state consistent
            await _loadMeUniverses();
            renderSidebar();
        });
    });

    // Discoverable subscribe buttons
    list.querySelectorAll('.discover-subscribe-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const key = btn.dataset.key;
            if (!key) return;
            btn.disabled = true;
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(key)}/subscribe`, { method: 'POST' }, true);
                await _loadMeUniverses();
                renderSidebar();
            } catch (_) {
                btn.disabled = false;
            }
        });
    });

    list.querySelectorAll('.sidebar-item:not(.sidebar-universe-item):not(.sidebar-discoverable-item)').forEach(el => {
        if (el.dataset.key) el.addEventListener('click', () => _selectProject(el.dataset.key));
    });
}

export function renderHeader() {
    const p = state.currentProject;
    // Header precedence: project name (if a project is selected) →
    // universe name (we're in a universe but no project yet) →
    // generic "select project" placeholder (no universe context at all).
    // Surfaced when "Comunicação" / "RFQ" universes loaded a board view
    // without a project pinned, showing the wrong "Selecione um projeto"
    // string in the H1 instead of the universe name.
    const t = window.t || (k => k);
    const u = state.universeInfo;
    let title;
    if (p) {
        title = p.name;
    } else if (u && u.name) {
        title = u.name;
    } else {
        title = t('select_project');
    }
    document.querySelector('#project-name').textContent = title;
    document.querySelector('#project-desc').textContent = p
        ? (p.description || '')
        : (u && u.description) || '';
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
            <a class="btn btn-ghost btn-storage" href="/storage" title="${window.t ? window.t('storage') : 'Armazenamento'}">
                <span class="material-symbols-outlined" style="font-size:18px">database</span>
            </a>
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
        // Defer the dereference: `renderHeaderUserArea(null)` runs before
        // `wireModules()` injects the real `showLoginModal`, so binding the
        // listener directly to the variable would capture the initial noop.
        // The arrow wrapper reads `_showLoginModal` at click time instead.
        if (btn) btn.addEventListener('click', () => _showLoginModal());
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
