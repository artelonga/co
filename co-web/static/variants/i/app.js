// ===== CO App — Variant i: Composable universe UI (CO-393) =====
// Differences from variant a:
//  1. injectManifestViewTabs uses computeDynamicTabs (registry-driven, not gantt-only)
//  2. renderContent handles 'graph' view via graphLens
//  3. Universe create/edit uses form engine (via dom-setup.js setupCriarUniverseModal)
//  4. conteudo.js split behind document.js lens facade

import { state, canEditCurrentUniverse } from '/variants/a/modules/state.js';
import { renderBreadcrumbs } from '/variants/a/modules/breadcrumbs.js';
import { api, apiFetch, injectApiCallbacks } from '/variants/a/modules/api.js';
import { esc, loadSubtreeState, addDays, todayDate } from '/variants/a/modules/helpers.js';
import { ZOOM_DAYS, rebuildI18nConstants } from '/variants/a/modules/constants.js';
import {
    showToast, showLoading, hideLoading, injectSwitchView,
} from '/variants/a/modules/settings.js';
import {
    renderSidebar, renderHeader, renderUsageCount, renderHeaderUserArea,
    renderUserBadge, renderMiniCalendar, injectSidebarCallbacks,
    injectSetUniverseSlugInUrl, injectScrollToDate,
    injectShowLoginModal as injectSidebarShowLogin,
} from '/variants/a/modules/sidebar.js';
import { renderKanban, injectKanbanCallbacks } from '/variants/a/modules/views/kanban.js';
import { renderCalendar, renderEventsTimeline, injectCalendarCallbacks } from '/variants/a/modules/views/calendar.js';
import { renderTable, closeStatusDropdown, injectTableCallbacks } from '/variants/a/modules/views/table.js';
import { renderTimeline, initTimelineStart, scrollToDate, injectTimelineCallbacks } from '/variants/a/modules/views/timeline.js';
import { renderDashboard, injectDashboardCallbacks } from '/variants/a/modules/views/dashboard.js';
import { renderChangelog, injectChangelogCallbacks } from '/variants/a/modules/views/changelog.js';
import { renderWorkspace, injectWorkspaceCallbacks } from '/variants/a/modules/views/workspace.js';
import {
    openTaskModal, closeModal, loadEditorBundle,
    handleFormSubmit, handleDelete, handleArchive,
    showUsageLimitModal, setupUsageLimitModal,
    showSubscribePromptModal, setupSubscribePromptModal,
    toggleActivityPanel, setupUniverseInfoModal, injectModalCallbacks,
} from '/variants/a/modules/modals.js';
import { showLoginModal, hideLoginModal, setupLoginModal, setupSecurityModal, injectLoginCallbacks } from '/variants/a/modules/login.js';
import { setupOnboarding, injectOnboardingCallbacks } from '/variants/a/modules/onboarding.js';
import { bootYggdrasil, injectYggdrasilCallbacks } from '/variants/a/modules/yggdrasil.js';
import { bootAppForUniverse, renderUniverseHome, bootApp as _bootApp, injectBootCallbacks } from '/variants/a/modules/boot.js';
import {
    setupInvitationsPage, setupInvitationsPanel, injectInvitationsCallbacks,
    consumePendingInviteToken, injectOpenDmWith,
} from '/variants/a/modules/invitations.js';
import { openDmWith } from '/variants/a/modules/dm.js';
import { openConversas, destroyConversas, injectOpenDmWithForConversas } from '/variants/a/modules/conversas.js';
import { setupNotifications } from '/variants/a/modules/notifications.js';
import { setupSettingsPanel } from '/variants/a/modules/settings.js';

// CO-393: variant-i lens registry + lens wrappers (self-register on import)
import { computeDynamicTabs, getLensById } from './modules/lenses/registry.js';
import './modules/lenses/kanban.js';
import './modules/lenses/table.js';
import './modules/lenses/timeline.js';
import './modules/lenses/calendar.js';
import './modules/lenses/dashboard.js';
import './modules/lenses/time-grid.js'; // CO-387: <co-time-grid> lens
import './modules/lenses/project-timeline.js'; // CO-396: roadmap/gantt lens
import { graphLens } from './modules/lenses/graph.js';
import { renderConteudo, openZoomModal, injectConteudoCallbacks } from './modules/lenses/document.js';
import { setupDom } from './modules/dom-setup.js';

// ===== CO-191: Bucketed universe loader =====
async function loadMeUniverses() {
    const me = await apiFetch('/api/v1/me/universes', {}, true);
    if (me) {
        state.meUniverses = me;
        state.userUniverses = [
            ...(me.owned || []), ...(me.member || []), ...(me.subscribed || []),
        ];
        return;
    }
    const pub = await apiFetch('/api/v1/universes/public', {}, true);
    if (Array.isArray(pub)) state.userUniverses = pub;
}

function setUniverseSlugInUrl(slug) {
    if (window.__CO_SUBDOMAIN_UNIVERSE__) {
        if (slug && slug !== 'template') { try { localStorage.setItem('co_preferred_universe', slug); } catch (_) {} }
        return;
    }
    window.history.pushState({ universeSlug: slug }, '', slug === 'template' ? '/' : `/${slug}`);
    if (slug && slug !== 'template') { try { localStorage.setItem('co_preferred_universe', slug); } catch (_) {} }
}

window.addEventListener('popstate', async (event) => {
    const target = (event.state && event.state.universeSlug) || readUniverseSlugFromUrl();
    if (!target || state.currentUniverseSlug === target || state.switchingUniverse) return;
    try {
        state.switchingUniverse = true;
        state.currentUniverseSlug = target;
        state.isTemplate = target === 'template';
        if (target === 'template') showTemplateBanner(); else hideTemplateBanner();
        await bootAppForUniverse(target);
    } finally { state.switchingUniverse = false; }
});

function showTemplateBanner() {
    document.getElementById('template-banner')?.classList.remove('hidden');
}
function hideTemplateBanner() {
    document.getElementById('template-banner')?.classList.add('hidden');
    document.getElementById('app')?.classList.remove('is-template');
    document.getElementById('source-bus-banner') && (document.getElementById('source-bus-banner').style.display = 'none');
}
function showSourceBusBanner(info) {
    const b = document.getElementById('source-bus-banner');
    if (!b) return;
    if (info && info.source_kind === 'event-bus') {
        const msg = document.getElementById('source-bus-msg');
        if (msg) msg.textContent = window.t ? window.t('universe.source_bus_readonly') : 'Somente-leitura — publicado via Yggdrasil';
        b.style.display = 'flex';
    } else { b.style.display = 'none'; }
}

function readUniverseSlugFromUrl() {
    if (window.__CO_SUBDOMAIN_UNIVERSE__) return window.__CO_SUBDOMAIN_UNIVERSE__;
    const RESERVED = ['', 'admin', 'settings', 'yggdrasil', 'static', 'health', '_app', 'notifications', 'graph-views'];
    if (window.location.pathname.match(/^\/yggdrasil\/[a-z0-9-]+/)) return 'yggdrasil';
    const m = window.location.pathname.match(/^\/([a-z0-9-]+)(\/|$)/);
    if (m && !RESERVED.includes(m[1])) return m[1];
    return 'template';
}

function readEntryPathFromUrl(universeSlug) {
    if (window.__CO_SUBDOMAIN_UNIVERSE__ && window.__CO_SUBDOMAIN_UNIVERSE__ === universeSlug) {
        const p = window.location.pathname;
        if (!p || p === '/' || p === `/${universeSlug}`) return null;
        const withSlug = `/${universeSlug}/`;
        return (p.startsWith(withSlug) ? p.slice(withSlug.length) : p.slice(1)).replace(/\.md$/i, '') || null;
    }
    const prefix = `/${universeSlug}/`;
    const p = window.location.pathname;
    if (!p.startsWith(prefix)) return null;
    return p.slice(prefix.length).replace(/\.md$/i, '') || null;
}

function readGameFromUrl() {
    const m = window.location.pathname.match(/^\/yggdrasil\/([a-z0-9-]+)$/);
    return m ? m[1] : null;
}

async function ensureOwnUniverse() {
    if (state.isTemplate) { showLoginModal(); return false; }
    if (!canEditCurrentUniverse()) { showSubscribePromptModal(); return false; }
    return true;
}

// ===== CO-393: Registry-driven manifest tab injection (replaces gantt-only check) =====
const MANIFEST_TAB_CLASS = 'manifest-injected-tab';

function removeManifestViewTabs() {
    document.querySelectorAll(`.${MANIFEST_TAB_CLASS}`).forEach(el => el.remove());
}

function injectManifestViewTabs(manifest) {
    removeManifestViewTabs();
    const viewTabs = document.getElementById('view-tabs');
    if (!viewTabs) return;
    const tabs = computeDynamicTabs(manifest);
    for (const tab of tabs) {
        const btn = document.createElement('button');
        btn.className = `view-tab ${MANIFEST_TAB_CLASS}`;
        btn.dataset.view = tab.viewKey || tab.id;
        btn.innerHTML = `<span class="view-tab-icon material-symbols-outlined">${esc(tab.icon || 'view_module')}</span><span class="view-tab-label">${esc(tab.label)}</span>`;
        btn.addEventListener('click', () => switchView(btn.dataset.view));
        viewTabs.appendChild(btn);
    }
}

// ===== Deep URL helpers =====
function showNotFoundView(universeSlug) {
    document.getElementById('co-not-found-view')?.remove();
    const view = document.createElement('div');
    view.id = 'co-not-found-view'; view.className = 'not-found-view';
    const t = k => window.t ? window.t(k) : k;
    view.innerHTML = `<div class="not-found-container">
        <h1 class="not-found-title">${t('not_found.title')}</h1>
        <p class="not-found-subtitle">${t('not_found.subtitle')}</p>
        <div class="not-found-actions">
            <a href="/${esc(universeSlug)}" class="btn btn-secondary">${t('not_found.back_universe')}</a>
            <a href="/" class="btn btn-primary">${t('not_found.back_home')}</a>
        </div></div>`;
    document.getElementById('app')?.appendChild(view);
}

function maybeOpenPageFromUrl(universeSlug) {
    const page = new URLSearchParams(window.location.search).get('page');
    if (!page) return;
    openZoomModal({ path: `content/${page}.md`, body: undefined, _universeSlug: 'template' }, false);
    const clean = new URL(window.location.href);
    clean.searchParams.delete('page');
    window.history.replaceState({}, '', clean.toString());
}

async function maybeOpenEntryFromUrl(universeSlug) {
    let entryPath = readEntryPathFromUrl(universeSlug);
    if (!entryPath) { maybeOpenPageFromUrl(universeSlug); return; }
    if (entryPath.startsWith('entries/')) entryPath = entryPath.slice('entries/'.length);
    const lower = entryPath.replace(/\/$/, '').toLowerCase();
    const candidates = [entryPath + '.md', entryPath, `content/${entryPath}.md`, `content/${entryPath}`];
    if (entryPath.endsWith('/')) candidates.unshift(`${entryPath}index.md`);
    if (lower === 'changelog') candidates.push('CHANGELOG.md', 'changelog.md');
    for (const p of candidates) {
        try {
            const entry = await apiFetch(`/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${p.split('/').map(encodeURIComponent).join('/')}`);
            if (entry && entry.path) {
                window.history.replaceState({}, '', `/${universeSlug}`);
                await openZoomModal({ ...entry, _universeSlug: universeSlug }, false);
                return;
            }
        } catch (_) {}
    }
    const stem = entryPath.replace(/\/$/, '').split('/').pop().replace(/\.md$/i, '');
    try {
        const res = await apiFetch(`/api/v1/universes/${encodeURIComponent(universeSlug)}/entries?q=${encodeURIComponent(stem)}&limit=5`);
        if (res && res.entries && res.entries.length > 0) {
            const exact = res.entries.find(e => (e.path || '').split('/').pop().replace(/\.md$/i, '').toLowerCase() === stem.toLowerCase()) || res.entries[0];
            window.history.replaceState({}, '', `/${universeSlug}`);
            await openZoomModal({ ...exact, _universeSlug: universeSlug }, false);
            return;
        }
    } catch (_) {}
    showNotFoundView(universeSlug);
}

async function openContentEditor(taskId) {
    const task = state.tasks.find(t => t.id === taskId);
    if (!task) return;
    const content = document.querySelector('#content');
    content.className = 'content content-editor-view';
    content.innerHTML = `<div class="content-editor-header">
        <button class="btn btn-secondary" id="content-editor-back">← ${window.t ? window.t('back') : 'Voltar'}</button>
        <h2>${esc(task.title)}</h2>
        <button class="btn btn-primary" id="content-editor-save">${window.t ? window.t('save') : 'Salvar'}</button>
    </div><div class="content-editor-body" id="content-editor-body">
        <textarea id="content-editor-textarea" rows="20" style="width:100%;padding:16px">${esc(task.description || '')}</textarea>
    </div>`;
    document.getElementById('content-editor-back').addEventListener('click', () => renderConteudo());
    document.getElementById('content-editor-save').addEventListener('click', async () => {
        const ta = document.getElementById('content-editor-textarea');
        const newDesc = ta ? ta.value : task.description;
        if (!(await ensureOwnUniverse())) return;
        const btn = document.getElementById('content-editor-save');
        if (btn) { btn.disabled = true; btn.textContent = '...'; }
        const result = await api.updateTask(state.currentProject.key, taskId, { description: newDesc });
        if (btn) { btn.disabled = false; btn.textContent = window.t ? window.t('save') : 'Salvar'; }
        if (result) { task.description = newDesc; showToast(window.t ? window.t('saved') : 'Salvo', 'success'); }
        else showToast('Erro ao salvar', 'error');
    });
    try {
        await loadEditorBundle();
        if (window.CoEditor) {
            const ta = document.getElementById('content-editor-textarea');
            if (ta) { ta.style.display = 'none'; }
            const cmDiv = document.createElement('div');
            cmDiv.className = 'content-editor-cm';
            document.getElementById('content-editor-body').appendChild(cmDiv);
            window.CoEditor.initEditor(cmDiv, {
                content: task.description || '',
                readOnly: false,
                // Keep the hidden textarea in sync — the save handler reads
                // from it; without this every save persists the stale text.
                onChange: (val) => { if (ta) ta.value = val; },
            });
        }
    } catch (_) {}
}

// ===== View switching =====
function switchView(view) {
    state.view = view;
    document.querySelectorAll('#view-tabs .view-tab').forEach(tab =>
        tab.classList.toggle('active', tab.dataset.view === view));
    const zoomTabs = document.querySelector('#zoom-tabs');
    const timelineNav = document.querySelector('#timeline-nav');
    if (view === 'timeline') {
        zoomTabs?.classList.remove('hidden');
        timelineNav?.classList.remove('hidden');
    } else {
        zoomTabs?.classList.add('hidden');
        timelineNav?.classList.add('hidden');
    }
    renderMiniCalendar();
    renderContent();
}

function switchZoom(zoom) {
    state.zoom = zoom;
    document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab =>
        tab.classList.toggle('active', tab.dataset.zoom === zoom));
    initTimelineStart();
    renderContent();
}

// ===== CO-393: renderContent — adds graph lens (AC5) =====
function renderContent() {
    if (state.loading) return;
    if (state.view === 'conteudo' || state.view === 'document') { renderConteudo(); return; }
    if (state.view === 'graph') { graphLens.render(); return; }
    // CO-387: generalized named manifest views — any lens with namedViews
    // renders via `<type>:<name>` keys (gantt:<name>, time-grid:<name>, …).
    if (state.view.includes(':')) {
        const sep = state.view.indexOf(':');
        const lensId = state.view.slice(0, sep);
        const viewName = state.view.slice(sep + 1);
        const lens = getLensById(lensId);
        const viewDef = state.universeManifest && (state.universeManifest.views || [])
            .find(v => (v.type || v.id) === lensId && (v.name || 'default') === viewName);
        if (lens && viewDef && typeof lens.render === 'function') { lens.render(viewDef); return; }
    }
    const hasCalendar = (state.universeManifest?.content_types || []).some(ct => ct.presentation?.calendar?.date_field);
    if (state.view === 'calendar' && hasCalendar) { renderCalendar(); return; }
    if (state.view === 'timeline' && hasCalendar) { renderEventsTimeline(); return; }
    if (!state.currentProject) { renderUniverseHome(); return; }
    if (state.view === 'kanban') renderKanban();
    else if (state.view === 'calendar') renderCalendar();
    else if (state.view === 'table') renderTable();
    else if (state.view === 'timeline') renderTimeline();
    else if (state.view === 'dashboard') renderDashboard();
    else if (state.view === 'changelog') renderChangelog();
    else if (state.view === 'workspace') renderWorkspace();
    else {
        // Registry dispatch: any lens registered via registerLens() renders
        // without touching this chain (CO-387/CO-396 extension point).
        const lens = getLensById(state.view);
        if (lens && typeof lens.render === 'function') lens.render();
    }
}

function render() {
    renderSidebar(); renderHeader(); renderBreadcrumbs(); renderMiniCalendar(); renderContent();
}

async function selectProject(key) {
    state.currentProject = state.projects.find(p => p.key === key);
    state.selectedIds.clear();
    loadSubtreeState(key);
    showLoading();
    await refreshTasks();
    hideLoading();
    render();
}

function _devTaskId(path) {
    let h = 5381;
    for (let i = 0; i < path.length; i++) h = (((h << 5) + h) ^ path.charCodeAt(i)) >>> 0;
    return (h % 9_000_000) + 1_000_000;
}

function _mapDevTask(dt) {
    return { id: _devTaskId(dt.path), key: dt.key, title: dt.title, status: dt.status, priority: dt.priority, description: dt.description, labels: [], due_date: null, assignee: null, parent: null, archived: false, created_at: dt.created_at, updated_at: dt.updated_at };
}

async function refreshTasks() {
    if (state.currentProject) state.tasks = await api.getTasks(state.currentProject.key, { archived: state.showArchived });
    const slug = state.currentUniverseSlug;
    if (state.universeInfo?.visibility === 'public-subscribable' && slug) {
        const devTasks = await api.getDevTasks(slug);
        if (devTasks.length > 0) state.tasks = [...state.tasks, ...devTasks.map(_mapDevTask)];
    }
}

async function bootApp() { await _bootApp(null, selectProject); }

async function initFooter() {
    const versionEl = document.getElementById('footer-version');
    if (!versionEl) return;
    try {
        const r = await fetch('/api/health');
        if (r.ok) {
            const data = await r.json();
            const tagline = window.t ? window.t('footer.tagline') : 'CO — código aberto';
            versionEl.textContent = `CO v${data.version} — ${tagline.replace('CO — ', '')}`;
        }
    } catch (_) {}
}

function wireModules() {
    injectApiCallbacks(showLoginModal, showUsageLimitModal, showToast);
    injectSwitchView(switchView);
    injectBootCallbacks({ showLoading, hideLoading, render, selectProject, removeManifestViewTabs, injectManifestViewTabs, switchView, onUniverseInfoLoaded: showSourceBusBanner });
    injectSidebarCallbacks({
        bootAppForUniverse: async (slug) => { destroyConversas(); document.getElementById('conversas-drawer')?.classList.add('hidden'); return bootAppForUniverse(slug); },
        selectProject, renderContent, showTemplateBanner, hideTemplateBanner, loadMeUniverses, renderSidebar, showToast,
    });
    injectSetUniverseSlugInUrl(setUniverseSlugInUrl);
    injectScrollToDate((dateStr) => scrollToDate(dateStr));
    injectSidebarShowLogin(showLoginModal);
    injectKanbanCallbacks({ openTaskModal, ensureOwnUniverse, renderKanban, showToast, renderContent });
    injectCalendarCallbacks({ openTaskModal, openZoomModal, apiFetch });
    injectTableCallbacks({ openTaskModal, refreshTasks, renderTable, showToast });
    injectTimelineCallbacks({ openTaskModal, refreshTasks, renderContent });
    injectDashboardCallbacks({ openTaskModal });
    injectChangelogCallbacks({ showToast, showLoginModal });
    injectWorkspaceCallbacks({ showToast });
    injectConteudoCallbacks({ openZoomModal, showLoginModal, showToast, loadEditorBundle, showSubscribePromptModal });
    // injectOpenContentEditor is from a's conteudo.js — variant i delegates openContentEditor
    import('/variants/a/modules/views/conteudo.js').then(({ injectOpenContentEditor }) => injectOpenContentEditor(openContentEditor)).catch(() => {});
    injectModalCallbacks({ showToast, showLoginModal, refreshTasks, render, renderContent, ensureOwnUniverse, loadMeUniverses, renderSidebar });
    setupUniverseInfoModal({});
    injectInvitationsCallbacks({ showToast, showLoginModal, loadMeUniverses });
    injectOpenDmWith((userId, drawerContainer) => openDmWith(userId, drawerContainer));
    injectLoginCallbacks({
        render, bootAppForUniverse: async (slug) => {
            const pendingToken = consumePendingInviteToken();
            if (pendingToken) { window.location.href = `/invitations/${pendingToken}`; return; }
            return bootAppForUniverse(slug);
        }, bootApp, renderUserBadge, setUniverseSlugInUrl, hideTemplateBanner, showToast, loadMeUniverses,
    });
    injectOnboardingCallbacks({ render, showLoginModal, showToast, setUniverseSlugInUrl, bootAppForUniverse, hideTemplateBanner, renderUsageCount, loadMeUniverses });
    injectYggdrasilCallbacks({ hideLoading, hideLoginModal, renderUserBadge });
    injectOpenDmWithForConversas((userId, pane) => openDmWith(userId, pane));
}

// ===== Boot =====
setupDom({
    render, renderContent, selectProject, switchView, switchZoom, refreshTasks,
    bootApp, setUniverseSlugInUrl, loadMeUniverses, openTaskModal, ensureOwnUniverse,
    injectManifestViewTabs, initTimelineStart, showTemplateBanner, hideTemplateBanner,
    readUniverseSlugFromUrl, readEntryPathFromUrl, readGameFromUrl, maybeOpenEntryFromUrl,
    initFooter, wireModules,
});
