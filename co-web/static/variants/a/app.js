// ===== CO App — Boot Orchestrator =====
// ES module entry point. Imports all feature modules and wires them together.

import { state, canEditCurrentUniverse } from './modules/state.js';
import { api, apiFetch, injectApiCallbacks } from './modules/api.js';
import { esc, filteredTasks, loadSubtreeState, addDays, todayDate } from './modules/helpers.js';
import { STATUSES, ZOOM_DAYS, rebuildI18nConstants } from './modules/constants.js';
import {
    showToast, showLoading, hideLoading, withTimeout,
    loadThemeCss, applyUniverseConfig, setupSettingsPanel, openSettingsPanel,
    renderSettingsGear, injectSwitchView,
} from './modules/settings.js';
import {
    renderSidebar, renderHeader, renderUsageCount,
    renderHeaderUserArea, renderUserBadge, renderMiniCalendar, setupHamburgerMenu,
    injectSidebarCallbacks, injectSetUniverseSlugInUrl, injectScrollToDate,
    injectShowLoginModal as injectSidebarShowLogin,
} from './modules/sidebar.js';
import { renderKanban, injectKanbanCallbacks } from './modules/views/kanban.js';
import { renderCalendar, renderGantt, renderEventsTimeline, injectCalendarCallbacks } from './modules/views/calendar.js';
import { renderTable, closeStatusDropdown, injectTableCallbacks } from './modules/views/table.js';
import {
    renderTimeline, getTimelineRange, initTimelineStart, scrollToDate,
    injectTimelineCallbacks,
} from './modules/views/timeline.js';
import { renderDashboard, injectDashboardCallbacks } from './modules/views/dashboard.js';
import { renderChangelog, injectChangelogCallbacks } from './modules/views/changelog.js';
import {
    renderConteudo, openZoomModal, injectConteudoCallbacks, injectOpenContentEditor,
} from './modules/views/conteudo.js';
import {
    openTaskModal, closeModal, loadEditorBundle,
    handleFormSubmit, handleDelete, handleArchive,
    showUsageLimitModal, hideUsageLimitModal, setupUsageLimitModal,
    showSubscribePromptModal, hideSubscribePromptModal, setupSubscribePromptModal,
    toggleActivityPanel,
    setupUniverseInfoModal,
    injectModalCallbacks,
} from './modules/modals.js';
import {
    showLoginModal, hideLoginModal, setupLoginModal, setupSecurityModal,
    injectLoginCallbacks,
} from './modules/login.js';
import { setupOnboarding, setupCriarModal, injectOnboardingCallbacks } from './modules/onboarding.js';
import { bootYggdrasil, injectYggdrasilCallbacks } from './modules/yggdrasil.js';
import {
    bootAppForUniverse, renderUniverseHome, bootApp as _bootApp,
    injectBootCallbacks,
} from './modules/boot.js';
import {
    setupInvitationsPage, setupInvitationsPanel,
    injectInvitationsCallbacks, consumePendingInviteToken,
    injectOpenDmWith,
} from './modules/invitations.js';
import { mountChat, destroyChat } from './modules/chat.js';
import { mountDmInbox, destroyDmInbox, openDmWith, updateDmBadge } from './modules/dm.js';
import { openConversas, destroyConversas, injectOpenDmWithForConversas } from './modules/conversas.js';
import { setupNotifications, teardownNotifications, bumpUnreadCount, renderNotificationsPage } from './modules/notifications.js';
import { renderNotificationSettings } from './modules/notification-settings.js';

// ===== CO-191: Bucketed universe loader =====
async function loadMeUniverses() {
    const me = await apiFetch('/api/v1/me/universes', {}, true);
    if (me) {
        state.meUniverses = me;
        // Backward compat: keep state.userUniverses for any code still reading it.
        state.userUniverses = [
            ...(me.owned || []).map(u => u),
            ...(me.member || []).map(u => u),
            ...(me.subscribed || []).map(u => u),
        ];
        return;
    }
    // Anonymous fallback: /me/universes 401'd. Populate state.userUniverses
    // from the public list so the sidebar can render something (without this
    // the sidebar stays empty and the user sees "Carregando..." forever).
    const pub = await apiFetch('/api/v1/universes/public', {}, true);
    if (Array.isArray(pub)) {
        state.userUniverses = pub;
    }
}

// ===== Tiny helpers that don't belong in any module =====

function setUniverseSlugInUrl(slug) {
    // Store the universe slug in history state so popstate (browser back/
    // forward) can read it. Without this, back-button changes the URL but
    // no JS reacts and the user stays on the current universe. Bug
    // reported 2026-05-12.
    window.history.pushState(
        { universeSlug: slug },
        '',
        slug === 'template' ? '/' : `/${slug}`,
    );
    if (slug && slug !== 'template') {
        try { localStorage.setItem('co_preferred_universe', slug); } catch (_) {}
    }
}

// Wire browser back/forward to actually load the universe in the URL.
// One-time install at module load; the handler reads from window.state
// (set up by bootApp) and dispatches to the existing bootAppForUniverse.
window.addEventListener('popstate', async (event) => {
    const fromState = event.state && event.state.universeSlug;
    const fromUrl = readUniverseSlugFromUrl();
    const target = fromState || fromUrl;
    if (!target) return;
    if (state.currentUniverseSlug === target) return;
    if (state.switchingUniverse) return;
    try {
        state.switchingUniverse = true;
        state.currentUniverseSlug = target;
        state.isTemplate = target === 'template';
        if (target === 'template') {
            showTemplateBanner();
        } else {
            hideTemplateBanner();
        }
        await bootAppForUniverse(target);
    } finally {
        state.switchingUniverse = false;
    }
});

function showTemplateBanner() {
    const b = document.getElementById('template-banner');
    if (b) b.classList.remove('hidden');
}

function hideTemplateBanner() {
    const b = document.getElementById('template-banner');
    if (b) b.classList.add('hidden');
    const a = document.getElementById('app');
    if (a) a.classList.remove('is-template');
}

function readUniverseSlugFromUrl() {
    const RESERVED = ['', 'admin', 'settings', 'yggdrasil', 'static', 'health', '_app', 'notifications'];
    if (window.location.pathname.match(/^\/yggdrasil\/[a-z0-9-]+/)) return 'yggdrasil';
    const m = window.location.pathname.match(/^\/([a-z0-9-]+)(\/|$)/);
    if (m && !RESERVED.includes(m[1])) return m[1];
    return 'template';
}

function readEntryPathFromUrl(universeSlug) {
    const prefix = `/${universeSlug}/`;
    const p = window.location.pathname;
    if (!p.startsWith(prefix)) return null;
    const sub = p.slice(prefix.length);
    return sub ? sub.replace(/\.md$/i, '') : null;
}

function readGameFromUrl() {
    const m = window.location.pathname.match(/^\/yggdrasil\/([a-z0-9-]+)$/);
    return m ? m[1] : null;
}

async function ensureOwnUniverse() {
    if (state.isTemplate) {
        showLoginModal();
        return false;
    }
    if (!canEditCurrentUniverse()) {
        showSubscribePromptModal();
        return false;
    }
    return true;
}

// ===== CO-73: Manifest view tab injection =====
const MANIFEST_TAB_CLASS = 'manifest-injected-tab';

function removeManifestViewTabs() {
    document.querySelectorAll(`.${MANIFEST_TAB_CLASS}`).forEach(el => el.remove());
}

function injectManifestViewTabs(manifest) {
    removeManifestViewTabs();
    const viewTabs = document.getElementById('view-tabs');
    if (!viewTabs) return;
    for (const v of (manifest.views || []).filter(v => v.type === 'gantt')) {
        const btn = document.createElement('button');
        btn.className = `view-tab ${MANIFEST_TAB_CLASS}`;
        btn.dataset.view = `gantt:${v.name}`;
        btn.innerHTML = `<span class="view-tab-icon material-symbols-outlined">view_timeline</span><span class="view-tab-label">${esc(v.name)}</span>`;
        btn.addEventListener('click', () => switchView(btn.dataset.view));
        viewTabs.appendChild(btn);
    }
}

// ===== Content editor (task description fullscreen) =====
let _contentEditorInstance = null;
let _draftSaveInterval = null;

async function openContentEditor(taskId) {
    const task = state.tasks.find(t => t.id === taskId);
    if (!task) return;
    const content = document.querySelector('#content');
    content.className = 'content content-editor-view';
    content.innerHTML = `
        <div class="content-editor-header">
            <button class="btn btn-secondary content-editor-back" id="content-editor-back">
                <span class="material-symbols-outlined" style="font-size:18px;vertical-align:middle">arrow_back</span>
                ${window.t ? window.t('back') : 'Voltar'}
            </button>
            <div class="content-editor-title-area">
                <span class="task-key" style="margin-right:8px">${esc(task.key)}</span>
                <h2 class="content-editor-title">${esc(task.title)}</h2>
            </div>
            <button class="btn btn-primary" id="content-editor-save">${window.t ? window.t('save') : 'Salvar'}</button>
        </div>
        <div class="content-editor-body" id="content-editor-body"></div>`;

    const draftKey = `co_draft_task_${taskId}`;
    document.getElementById('content-editor-back').addEventListener('click', () => {
        if (_draftSaveInterval) { clearInterval(_draftSaveInterval); _draftSaveInterval = null; }
        if (_contentEditorInstance) { _contentEditorInstance.destroy(); _contentEditorInstance = null; }
        renderConteudo();
    });

    const editorContainer = document.getElementById('content-editor-body');
    editorContainer.innerHTML = `<textarea class="content-editor-textarea" id="content-editor-textarea">${esc(task.description || '')}</textarea>`;
    try {
        await loadEditorBundle();
        if (window.CoEditor) {
            const ta = document.getElementById('content-editor-textarea');
            if (ta) ta.style.display = 'none';
            const cmDiv = document.createElement('div');
            cmDiv.className = 'content-editor-cm';
            editorContainer.appendChild(cmDiv);
            _contentEditorInstance = window.CoEditor.initEditor(cmDiv, {
                content: task.description || '',
                onChange: (val) => { if (ta) ta.value = val; },
                readOnly: false,
            });
            if (_draftSaveInterval) clearInterval(_draftSaveInterval);
            _draftSaveInterval = setInterval(() => {
                try { localStorage.setItem(draftKey, _contentEditorInstance ? _contentEditorInstance.getValue() : ''); } catch (_) {}
            }, 5000);
        }
    } catch (_) {}

    document.getElementById('content-editor-save').addEventListener('click', async () => {
        const saveBtn = document.getElementById('content-editor-save');
        const ta = document.getElementById('content-editor-textarea');
        let newDesc = (_contentEditorInstance && _contentEditorInstance.getContent)
            ? _contentEditorInstance.getContent() : (ta ? ta.value : task.description);
        if (!(await ensureOwnUniverse())) return;
        if (state.isTemplate) {
            showToast(window.t ? window.t('saved') : 'Salvo', 'success'); return;
        }
        if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
        const result = await api.updateTask(state.currentProject.key, taskId, { description: newDesc });
        if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = window.t ? window.t('save') : 'Salvar'; }
        if (result) {
            task.description = newDesc;
            try { localStorage.removeItem(draftKey); } catch (_) {}
            showToast(window.t ? window.t('saved') : 'Salvo', 'success');
        } else { showToast('Erro ao salvar', 'error'); }
    });
}

// ===== Deep URL helpers =====

// CO-232: render a 404 view when a deep-linked entry cannot be resolved.
function showNotFoundView(universeSlug) {
    const t = k => window.t ? window.t(k) : k;
    const existing = document.getElementById('co-not-found-view');
    if (existing) existing.remove();
    const view = document.createElement('div');
    view.id = 'co-not-found-view';
    view.className = 'not-found-view';
    view.innerHTML =
        `<div class="not-found-container">` +
        `<h1 class="not-found-title">${t('not_found.title')}</h1>` +
        `<p class="not-found-subtitle">${t('not_found.subtitle')}</p>` +
        `<div class="not-found-actions">` +
        `<a href="/${esc(universeSlug)}" class="btn btn-secondary">${t('not_found.back_universe')}</a>` +
        `<a href="/" class="btn btn-primary">${t('not_found.back_home')}</a>` +
        `</div></div>`;
    const app = document.getElementById('app');
    if (app) app.appendChild(view);
}

function maybeOpenPageFromUrl(universeSlug) {
    const params = new URLSearchParams(window.location.search);
    const page = params.get('page');
    if (!page) return;
    if (page === 'dados-rastreados' || page === 'privacidade') {
        try { localStorage.setItem('co_cookie_consent', '1'); } catch (_) {}
    }
    openZoomModal({ path: `content/${page}.md`, body: undefined, _universeSlug: 'template' }, false);
    const clean = new URL(window.location.href);
    clean.searchParams.delete('page');
    window.history.replaceState({}, '', clean.toString());
}

async function maybeOpenEntryFromUrl(universeSlug) {
    let entryPath = readEntryPathFromUrl(universeSlug);
    if (!entryPath) { maybeOpenPageFromUrl(universeSlug); return; }
    if (entryPath.startsWith('entries/')) entryPath = entryPath.slice('entries/'.length);
    // Try the literal entry path, then the canonical seed-page location
    // `content/<slug>.md` — most public seed pages (seguranca, licensa,
    // infra, etc.) live under `content/` so a bare `/template/seguranca`
    // URL needs to fall through to `content/seguranca.md`.
    const candidates = [
        entryPath + '.md',
        entryPath,
        `content/${entryPath}.md`,
        `content/${entryPath}`,
    ];
    for (const p of candidates) {
        try {
            const encodedPath = p.split('/').map(encodeURIComponent).join('/');
            const entry = await apiFetch(`/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${encodedPath}`);
            if (entry && entry.path) {
                window.history.replaceState({}, '', `/${universeSlug}`);
                openZoomModal({ ...entry, _universeSlug: universeSlug }, false);
                return;
            }
        } catch (_) {}
    }
    const stem = entryPath.split('/').pop().replace(/\.md$/i, '');
    try {
        const res = await apiFetch(`/api/v1/universes/${encodeURIComponent(universeSlug)}/entries?q=${encodeURIComponent(stem)}&limit=5`);
        if (res && res.entries && res.entries.length > 0) {
            const exact = res.entries.find(e => (e.path || '').split('/').pop().replace(/\.md$/i, '').toLowerCase() === stem.toLowerCase()) || res.entries[0];
            window.history.replaceState({}, '', `/${universeSlug}`);
            openZoomModal({ ...exact, _universeSlug: universeSlug }, false);
            return;
        }
    } catch (_) {}
    // CO-232: entry not found — show 404 view instead of silently landing on universe home.
    showNotFoundView(universeSlug);
}

// ===== View switching =====
function switchView(view) {
    state.view = view;
    document.querySelectorAll('#view-tabs .view-tab').forEach(tab => {
        tab.classList.toggle('active', tab.dataset.view === view);
    });
    const zoomTabs = document.querySelector('#zoom-tabs');
    const timelineNav = document.querySelector('#timeline-nav');
    if (view === 'timeline') {
        zoomTabs.classList.remove('hidden');
        timelineNav.classList.remove('hidden');
    } else {
        zoomTabs.classList.add('hidden');
        timelineNav.classList.add('hidden');
    }
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
    if (state.loading) return;
    if (state.view === 'conteudo') { renderConteudo(); return; }
    if (state.view.startsWith('gantt:')) {
        const viewName = state.view.slice(6);
        const viewDef = state.universeManifest && (state.universeManifest.views || []).find(v => v.type === 'gantt' && v.name === viewName);
        if (viewDef) { renderGantt(viewDef); return; }
    }
    const manifestHasCalendar = (state.universeManifest?.content_types || []).some(ct => ct.presentation?.calendar?.date_field);
    if (state.view === 'calendar' && manifestHasCalendar) { renderCalendar(); return; }
    if (state.view === 'timeline' && manifestHasCalendar) { renderEventsTimeline(); return; }
    if (!state.currentProject) { renderUniverseHome(); return; }
    if (state.view === 'kanban') renderKanban();
    else if (state.view === 'calendar') renderCalendar();
    else if (state.view === 'table') renderTable();
    else if (state.view === 'timeline') renderTimeline();
    else if (state.view === 'dashboard') renderDashboard();
    else if (state.view === 'changelog') renderChangelog();
}

function render() {
    renderSidebar();
    renderHeader();
    renderMiniCalendar();
    renderContent();
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
        state.tasks = await api.getTasks(state.currentProject.key, { archived: state.showArchived });
    }
}

async function bootApp() {
    await _bootApp(null, selectProject);
}

// ===== Footer =====
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
    } catch {}
}

// ===== Wire callbacks =====
function wireModules() {
    injectApiCallbacks(showLoginModal, showUsageLimitModal, showToast);
    injectSwitchView(switchView);
    injectBootCallbacks({ showLoading, hideLoading, render, selectProject, removeManifestViewTabs, injectManifestViewTabs, switchView });
    injectSidebarCallbacks({
        bootAppForUniverse: async (slug) => {
            destroyConversas();
            document.getElementById('conversas-drawer')?.classList.add('hidden');
            return bootAppForUniverse(slug);
        },
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
    injectConteudoCallbacks({ openZoomModal, showLoginModal, showToast, loadEditorBundle, showSubscribePromptModal });
    injectOpenContentEditor(openContentEditor);
    injectModalCallbacks({ showToast, showLoginModal, refreshTasks, render, renderContent, ensureOwnUniverse, loadMeUniverses, renderSidebar });
    setupUniverseInfoModal({});
    injectInvitationsCallbacks({ showToast, showLoginModal, loadMeUniverses });
    injectOpenDmWith((userId, drawerContainer) => openDmWith(userId, drawerContainer));
    injectLoginCallbacks({
        render,
        bootAppForUniverse: async (slug) => {
            const pendingToken = consumePendingInviteToken();
            if (pendingToken) {
                window.location.href = `/invitations/${pendingToken}`;
                return;
            }
            return bootAppForUniverse(slug);
        },
        bootApp,
        renderUserBadge,
        setUniverseSlugInUrl,
        hideTemplateBanner,
        showToast,
        loadMeUniverses,
    });
    injectOnboardingCallbacks({ render, showLoginModal, showToast, setUniverseSlugInUrl, bootAppForUniverse, hideTemplateBanner, renderUsageCount, loadMeUniverses });
    injectYggdrasilCallbacks({ hideLoading, hideLoginModal, renderUserBadge });
}

// ===== CO-209: Conversas unified panel =====
function _setupConversasTrigger() {
    const drawer = document.getElementById('conversas-drawer');
    if (!drawer) return;

    // Inject single 💬 Conversas button into sidebar footer
    const sidebarFooter = document.querySelector('.sidebar-footer');
    if (sidebarFooter) {
        const conversasBtn = document.createElement('button');
        conversasBtn.id = 'btn-open-conversas';
        conversasBtn.className = 'chat-sidebar-toggle hidden';
        conversasBtn.textContent = `💬 ${window.t ? window.t('conversas.title') : 'Conversas'}`;
        sidebarFooter.insertBefore(conversasBtn, sidebarFooter.firstChild);

        conversasBtn.addEventListener('click', () => {
            openConversas(drawer, state.me || null);
        });
    }

    // Wire openDmWith into conversas so member rail DM links work
    injectOpenDmWithForConversas((userId, pane) => openDmWith(userId, pane || drawer));
}

// Show/hide the Conversas button based on login state
function _updateChatButton(me) {
    const btn = document.getElementById('btn-open-conversas');
    if (btn) btn.classList.toggle('hidden', !me);

    // CO-202: show/hide notification bell
    const notifWrap = document.getElementById('notif-wrap');
    if (notifWrap) notifWrap.classList.toggle('hidden', !me);
    if (me && !_notifSetupDone) {
        _notifSetupDone = true;
        setupNotifications();
    }
}

let _notifSetupDone = false;

// ---------------------------------------------------------------------------

// ===== Static DOM event bindings =====
function bindStaticEvents() {
    document.querySelectorAll('#view-tabs .view-tab').forEach(tab => tab.addEventListener('click', () => switchView(tab.dataset.view)));
    document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab => tab.addEventListener('click', () => switchZoom(tab.dataset.zoom)));

    document.querySelector('#btn-prev').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), -shift);
        renderContent();
    });
    document.querySelector('#btn-today').addEventListener('click', () => { initTimelineStart(); renderContent(); });
    document.querySelector('#btn-next').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), shift);
        renderContent();
    });

    document.querySelector('#btn-new-task').addEventListener('click', async () => {
        if (!(await ensureOwnUniverse())) return;
        // 2.7.29: auto-select the first project on the universe when no
        // current project is set. Previously the click silently no-op'd
        // on the Conteudo home view because state.currentProject was null
        // — bootAppForUniverse only auto-selects on initial load, not
        // after the user navigates between views.
        if (!state.currentProject) {
            if (!state.projects || state.projects.length === 0) {
                showToast(window.t ? window.t('no_projects') : 'Crie um projeto antes', 'warning');
                return;
            }
            await selectProject(state.projects[0].key);
        }
        if (state.currentProject) openTaskModal(null);
    });

    document.querySelector('#modal-close').addEventListener('click', closeModal);
    document.querySelector('#btn-cancel').addEventListener('click', closeModal);
    document.querySelector('#task-form').addEventListener('submit', handleFormSubmit);
    document.querySelector('#btn-delete').addEventListener('click', handleDelete);
    const archiveBtn = document.querySelector('#btn-archive');
    if (archiveBtn) archiveBtn.addEventListener('click', handleArchive);
    document.querySelector('#modal-overlay').addEventListener('click', e => { if (e.target === e.currentTarget) closeModal(); });

    document.querySelector('#search-input').addEventListener('input', e => { state.searchQuery = e.target.value; renderContent(); });

    const showArchivedCb = document.getElementById('show-archived');
    if (showArchivedCb) showArchivedCb.addEventListener('change', async () => { state.showArchived = showArchivedCb.checked; await refreshTasks(); render(); });

    const activityBtn = document.getElementById('btn-activity');
    if (activityBtn) activityBtn.addEventListener('click', toggleActivityPanel);

    document.addEventListener('keydown', e => {
        if (e.key === 'Escape') {
            closeStatusDropdown(); closeModal();
            const panel = document.querySelector('#activity-panel');
            if (panel && !panel.classList.contains('hidden')) panel.classList.add('hidden');
            const sidebar = document.getElementById('sidebar');
            const sidebarOverlay = document.getElementById('sidebar-overlay');
            if (sidebar) sidebar.classList.remove('open');
            if (sidebarOverlay) sidebarOverlay.classList.remove('visible');
            return;
        }
        const active = document.activeElement;
        const isEditing = active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.tagName === 'SELECT' || active.isContentEditable);
        if (isEditing) return;
        switch (e.key) {
            case 'n': e.preventDefault(); if (state.currentProject) openTaskModal(null); break;
            case '/': e.preventDefault(); { const si = document.querySelector('#search-input'); if (si) si.focus(); } break;
            case '1': e.preventDefault(); switchView('kanban'); break;
            case '2': e.preventDefault(); switchView('table'); break;
            case '3': e.preventDefault(); switchView('timeline'); break;
            case '4': e.preventDefault(); switchView('calendar'); break;
            case '5': e.preventDefault(); switchView('dashboard'); break;
            case '6': e.preventDefault(); switchView('conteudo'); break;
        }
    });

    document.addEventListener('co:langchange', () => { rebuildI18nConstants(); render(); });
}

// CO-202: WS chat hook — called by chat.js when a message arrives in any room.
// Bumps the bell badge if the chat drawer is not currently open.
window.coOnChatMessageArrived = (_msg, _roomId) => {
    const conversasDrawer = document.getElementById('conversas-drawer');
    if (!conversasDrawer || conversasDrawer.classList.contains('hidden')) {
        bumpUnreadCount();
    }
};

// ===== Entry point =====
async function init() {
    window.setLang(window.currentLang);

    const btnHeaderLang = document.getElementById('btn-header-lang');
    if (btnHeaderLang) btnHeaderLang.addEventListener('click', () => { window.setLang(window.currentLang === 'pt' ? 'en' : 'pt'); render(); });

    renderHeaderUserArea(null);
    initTimelineStart();
    wireModules();
    setupHamburgerMenu();
    setupLoginModal();
    setupSecurityModal();
    // CO-202: render notification settings when security modal opens
    document.getElementById('btn-security')?.addEventListener('click', () => {
        renderNotificationSettings(document.getElementById('notif-settings-content'));
    });
    setupCriarModal();
    setupUsageLimitModal();
    setupSubscribePromptModal();
    setupSettingsPanel();
    setupInvitationsPanel();
    _setupConversasTrigger();
    initFooter();
    bindStaticEvents();

    // CO-189: Handle /invitations/:token SPA route before normal boot
    const isInvitePage = await setupInvitationsPage();
    if (isInvitePage) return;

    const slug = readUniverseSlugFromUrl();
    state.currentUniverseSlug = slug;
    state.isTemplate = slug === 'template';
    state.isYggdrasil = slug === 'yggdrasil';

    if (state.isYggdrasil) {
        state.gameView = readGameFromUrl();
        showLoading();
        await bootYggdrasil();
        return;
    }

    // CO-202: /notifications full-page SPA route
    if (slug === 'notifications') {
        const me = await api.me();
        if (me) {
            hideLoginModal();
            renderUserBadge(me);
            state.me = me;
            _updateChatButton(me);
            await loadMeUniverses();
            renderSidebar();
            renderHeader();
            const content = document.querySelector('#content');
            if (content) {
                content.className = 'content';
                renderNotificationsPage(content);
            }
        } else {
            showLoginModal();
            state.currentUniverseSlug = 'template';
            state.isTemplate = true;
            setUniverseSlugInUrl('template');
            showTemplateBanner();
            setupOnboarding();
            await bootAppForUniverse('template');
        }
        return;
    }

    if (state.isTemplate) {
        // If the URL points at a specific entry within the template
        // (`/template/content/seguranca`, `?page=seguranca`, etc.), the
        // visitor came for that entry — keep them on template and open
        // it. Without this guard, logged-in users get auto-redirected
        // to their own universe and the targeted entry never opens.
        const requestedEntry = readEntryPathFromUrl('template');
        const requestedPage = new URLSearchParams(window.location.search).get('page');
        const stayOnTemplate = !!(requestedEntry || requestedPage);

        const me = await api.me();
        if (me) {
            hideLoginModal();
            renderUserBadge(me);
            state.me = me;
            _updateChatButton(me);
            await loadMeUniverses();
            const mine = state.userUniverses.filter(u => !u.is_template);
            if (mine.length > 0 && !stayOnTemplate) {
                const preferred = (() => { try { return localStorage.getItem('co_preferred_universe'); } catch (_) { return null; } })();
                const target = (preferred && mine.find(u => u.key === preferred)) ? preferred : mine[0].key;
                state.currentUniverseSlug = target;
                state.isTemplate = false;
                hideTemplateBanner();
                await bootAppForUniverse(target);
                return;
            }
        }
        if (!me) {
            showTemplateBanner();
            setupOnboarding();
        }
        await bootAppForUniverse('template');
        // Resolve URL-path-based entries (`/template/seguranca`) AND
        // query-param fallback (`?page=seguranca`). The Entry resolver
        // delegates to Page resolver when no path is present, so this
        // covers both shapes.
        await maybeOpenEntryFromUrl('template');
        return;
    }

    const me = await api.me();
    if (me) {
        hideLoginModal();
        renderUserBadge(me);
        state.me = me;
        _updateChatButton(me);
        await loadMeUniverses();
        await bootAppForUniverse(slug);
        await maybeOpenEntryFromUrl(slug);
        return;
    }

    const info = await api.getUniverseInfo(slug);
    if (info && info.is_anonymous) {
        state.universeInfo = info;
        renderUsageCount();
        state.projects = await api.getUniverseProjects(slug);
        if (state.projects.length > 0) await selectProject(state.projects[0].key);
        render();
        return;
    }

    // CO-253: public universe → read-only for anonymous visitors (no redirect to template).
    if (info && info.visibility === 'public-subscribable') {
        await bootAppForUniverse(slug);
        await maybeOpenEntryFromUrl(slug);
        return;
    }

    state.currentUniverseSlug = 'template';
    state.isTemplate = true;
    setUniverseSlugInUrl('template');
    showTemplateBanner();
    setupOnboarding();
    await bootAppForUniverse('template');
}

init();
