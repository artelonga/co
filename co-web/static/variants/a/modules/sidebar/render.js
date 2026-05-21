// ===== Sidebar render + callback injection =====
import { state } from '../state.js';
import { apiFetch } from '../api.js';
import { esc } from '../helpers.js';
import {
    renderSectionHtml, renderUniverseItemHtml, buildChildMap,
    renderInviteRowHtml, renderDiscoverableItemHtml,
} from './sections.js';

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
