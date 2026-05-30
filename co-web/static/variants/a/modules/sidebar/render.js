// ===== Sidebar render + callback injection =====
import { state } from '../state.js';
import { apiFetch } from '../api.js';
import { esc } from '../helpers.js';
import {
    renderSectionHtml, renderUniverseItemHtml, buildChildMap,
    renderInviteRowHtml, renderDiscoverableItemHtml,
} from './sections.js';
// CO-311: Platforms section removed (was hardcoded cross-deployment links,
// not actual universes — confused users). Tools section remains.
import { renderTools } from './tools.js';

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
    // CO-311: two-section sidebar (This universe / Tools). The old Platforms
    // section was a hardcoded list of cross-deployment links — removed because
    // users expected it to show universes, not external sites.
    renderTools();

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
        // CO-320: pass showUnsubscribe=true so subscribed rows render a × button.
        universeHtml += renderSectionHtml(t('sidebar.section.subscribed'), me.subscribed, true, t('sidebar.section.subscribed.tooltip'), allUniversesMap, true);

        if (me.invited && me.invited.length > 0) {
            universeHtml += `<div class="sidebar-universe-section sidebar-invited-section">
                <div class="sidebar-section-label">🎁 ${esc(t('sidebar.section.invited'))} (${me.invited.length})</div>
                ${me.invited.map(renderInviteRowHtml).join('')}
            </div>`;
        }

        // CO-320: Discoverable section removed from default sidebar — was
        // confusing per user feedback ("we don't want to show all universes
        // available unless user seeks"). Replaced with a compact "+ Buscar
        // universos" button that opens a search prompt; users only see the
        // discoverable list when they actively look for one to subscribe to.
        if (me.discoverable && me.discoverable.length > 0) {
            universeHtml += `<div class="sidebar-universe-section sidebar-discover-cta-section">
                <button class="btn btn-sm btn-ghost sidebar-discover-search-btn" style="width:100%;justify-content:flex-start;font-size:12px;padding:6px 12px">
                    + ${esc(t('sidebar.discover.find_button') === 'sidebar.discover.find_button' ? 'Buscar universos públicos' : t('sidebar.discover.find_button'))}
                </button>
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

    // Discoverable subscribe buttons (still rendered when search opens)
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

    // CO-320: "Buscar universos públicos" search button — prompts for a key
    // and POSTs subscribe. Quick MVP; replace with a modal+autocomplete in a
    // follow-up when there are enough public universes to warrant browsing.
    list.querySelectorAll('.sidebar-discover-search-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const me = state.meUniverses;
            const opts = (me?.discoverable || []).map(u => `${u.key} — ${u.name || u.key}`).join('\n');
            // CO-321: instruction text avoids the substring "universo" so e2e
            // assertions on the listed keys aren't false-matched by prompt copy
            // (the old phrase "do universo" matched the universo public-static key).
            const promptText = `Universos públicos disponíveis:\n\n${opts}\n\nDigite a chave para inscrever-se:`;
            const key = window.prompt(promptText);
            if (!key) return;
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(key.trim())}/subscribe`, { method: 'POST' }, true);
                await _loadMeUniverses();
                renderSidebar();
                if (_showToast) _showToast(`Inscrito em ${key.trim()}`, 'success');
            } catch (err) {
                if (_showToast) _showToast(`Não foi possível inscrever em ${key.trim()}`, 'error');
            }
        });
    });

    // CO-320: unsubscribe × button on subscribed-bucket rows.
    list.querySelectorAll('.sidebar-unsubscribe-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const key = btn.dataset.key;
            if (!key) return;
            if (!window.confirm(`Cancelar inscrição em ${key}?`)) return;
            btn.disabled = true;
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(key)}/subscribe`, { method: 'DELETE' }, true);
                await _loadMeUniverses();
                renderSidebar();
                if (_showToast) _showToast(`Inscrição cancelada: ${key}`, 'success');
            } catch (_) {
                btn.disabled = false;
                if (_showToast) _showToast('Falha ao cancelar inscrição', 'error');
            }
        });
    });

    list.querySelectorAll('.sidebar-item:not(.sidebar-universe-item):not(.sidebar-discoverable-item)').forEach(el => {
        if (el.dataset.key) el.addEventListener('click', () => _selectProject(el.dataset.key));
    });
}
