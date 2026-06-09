// ===== CO App — Boot Orchestrator =====
// ES module entry point. Imports all feature modules and wires them together.

import { state, canEditCurrentUniverse } from './modules/state.js';
import { renderBreadcrumbs } from './modules/breadcrumbs.js';
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
    // CO-323: in subdomain mode the browser is already on the correct host
    // (yuri.artelonga.com.br); don't push a new slug to the path.
    if (window.__CO_SUBDOMAIN_UNIVERSE__) {
        if (slug && slug !== 'template') {
            try { localStorage.setItem('co_preferred_universe', slug); } catch (_) {}
        }
        return;
    }
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
    hideSourceBusBanner();
}

// CO-383: Read-only banner for event-bus-backed universes (e.g. yggdrasil notes).
function showSourceBusBanner(info) {
    const b = document.getElementById('source-bus-banner');
    if (!b) return;
    if (info && info.source_kind === 'event-bus') {
        const msg = document.getElementById('source-bus-msg');
        if (msg) msg.textContent = window.t ? window.t('universe.source_bus_readonly') : 'Somente-leitura — publicado via Yggdrasil';
        const link = document.getElementById('source-bus-link');
        if (link && info.source_url) {
            // Convert wss:// bus URL to an https:// editor URL for the link.
            const editorUrl = info.source_url.replace(/^wss?:\/\/([^/]+).*/, 'https://$1');
            link.href = editorUrl;
            link.style.display = '';
        } else if (link) {
            link.style.display = 'none';
        }
        b.style.display = 'flex';
    } else {
        b.style.display = 'none';
    }
}

function hideSourceBusBanner() {
    const b = document.getElementById('source-bus-banner');
    if (b) b.style.display = 'none';
}

function readUniverseSlugFromUrl() {
    // CO-323: subdomain single-universe mode overrides URL-based detection.
    // window.__CO_SUBDOMAIN_UNIVERSE__ is injected by the server when the
    // request arrives via a *.artelonga.com.br subdomain.
    if (window.__CO_SUBDOMAIN_UNIVERSE__) return window.__CO_SUBDOMAIN_UNIVERSE__;
    const RESERVED = ['', 'admin', 'settings', 'yggdrasil', 'static', 'health', '_app', 'notifications', 'graph-views'];
    if (window.location.pathname.match(/^\/yggdrasil\/[a-z0-9-]+/)) return 'yggdrasil';
    const m = window.location.pathname.match(/^\/([a-z0-9-]+)(\/|$)/);
    if (m && !RESERVED.includes(m[1])) return m[1];
    return 'template';
}

function readEntryPathFromUrl(universeSlug) {
    // CO-323: in subdomain mode the URL path is relative to the universe root.
    // e.g. yuri.artelonga.com.br/2026-05-31 → path '2026-05-31' in yuri universe.
    if (window.__CO_SUBDOMAIN_UNIVERSE__ && window.__CO_SUBDOMAIN_UNIVERSE__ === universeSlug) {
        const p = window.location.pathname;
        if (!p || p === '/' || p === `/${universeSlug}`) return null;
        // Also handle /yuri/... in case history.pushState added the slug prefix.
        const withSlug = `/${universeSlug}/`;
        const stripped = p.startsWith(withSlug)
            ? p.slice(withSlug.length)
            : p.slice(1); // strip leading /
        return stripped ? stripped.replace(/\.md$/i, '') : null;
    }
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

    const lower = entryPath.replace(/\/$/, '').toLowerCase();

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

    // CO-264: folder-level index.md for trailing-slash paths (e.g. `/co/public/`).
    if (entryPath.endsWith('/')) {
        candidates.unshift(`${entryPath}index.md`);
    }

    // CO-264: well-known file aliases — map short URL segment to canonical filename.
    if (lower === 'changelog') {
        candidates.push('CHANGELOG.md', 'changelog.md');
    } else if (lower === 'readme') {
        candidates.push('README.md', 'readme.md');
    } else if (lower === 'license') {
        candidates.push('LICENSE.md', 'LICENSE');
    }

    for (const p of candidates) {
        try {
            const encodedPath = p.split('/').map(encodeURIComponent).join('/');
            const entry = await apiFetch(`/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${encodedPath}`);
            if (entry && entry.path) {
                window.history.replaceState({}, '', `/${universeSlug}`);
                await openZoomModal({ ...entry, _universeSlug: universeSlug }, false);
                injectFeedbackBadge(universeSlug, entry.path);
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
            injectFeedbackBadge(universeSlug, exact.path);
            return;
        }
    } catch (_) {}

    // CO-264: show a helpful empty state for the /changelog path when no CHANGELOG.md exists.
    if (lower === 'changelog') {
        showChangelogNotFoundView(universeSlug);
        return;
    }

    // CO-333: /<universe>/feedback → show feedback mural.
    if (lower === 'feedback') {
        window.history.replaceState({}, '', `/${universeSlug}`);
        showFeedbackMural(universeSlug);
        return;
    }

    // CO-232: entry not found — show 404 view instead of silently landing on universe home.
    showNotFoundView(universeSlug);
}

// CO-264: render an empty state when /<universe>/changelog has no CHANGELOG.md entry.
function showChangelogNotFoundView(universeSlug) {
    const existing = document.getElementById('co-not-found-view');
    if (existing) existing.remove();
    const view = document.createElement('div');
    view.id = 'co-not-found-view';
    view.className = 'not-found-view';
    view.innerHTML =
        `<div class="not-found-container">` +
        `<h2 style="margin-bottom:8px">CHANGELOG não encontrado</h2>` +
        `<p style="color:var(--text-muted,#888);margin-bottom:16px">` +
        `Este universo ainda não tem um CHANGELOG. ` +
        `Crie um arquivo <code>CHANGELOG.md</code> no nível raiz.</p>` +
        `<a href="/${esc(universeSlug)}" class="btn btn-secondary">← Voltar ao universo</a>` +
        `</div>`;
    const app = document.getElementById('app');
    if (app) app.appendChild(view);
}

// CO-333: inject unread-feedback badge into the zoom modal toolbar for the owner.
function injectFeedbackBadge(universeSlug, entryPath) {
    if (!state.me || !state.universeInfo) return;
    if (state.me.id !== state.universeInfo.owner_id) return;
    const actionsDiv = document.querySelector('#co-zoom-overlay .co-zoom-actions');
    if (!actionsDiv || !entryPath) return;
    import('./modules/feedback-panel.js').then(({ mountFeedbackBadge }) => {
        mountFeedbackBadge(actionsDiv, universeSlug, entryPath);
    }).catch(() => {});
}

// CO-333: visitor-facing feedback widget. Bottom-left floating button visible
// to all users (anon + authenticated) on every page. The widget self-initializes
// on module load (mounts the floating button + attaches to window.CoFeedbackWidget),
// so importing it is sufficient. Owners ALSO see the badge above on individual
// entries (for in-locus review via feedback-panel.js).
import('./modules/feedback-widget.js').catch((e) =>
    console.warn('feedback widget load failed:', e),
);

// CO-333: render the feedback mural for a universe.
async function showFeedbackMural(universeSlug) {
    const existing = document.getElementById('co-feedback-mural');
    if (existing) existing.remove();
    const mural = document.createElement('div');
    mural.id = 'co-feedback-mural';
    mural.style.cssText = 'max-width:680px;margin:32px auto;padding:0 16px';
    mural.innerHTML = `
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:20px">
            <button onclick="this.closest('#co-feedback-mural').remove()" style="
                background:none;border:1px solid var(--border,#d1d5db);border-radius:6px;
                padding:5px 12px;cursor:pointer;font-size:13px">← Voltar</button>
            <h2 style="margin:0;font-size:20px">Feedback — ${esc(universeSlug)}</h2>
        </div>
        <div id="co-feedback-mural-body" style="color:var(--text-muted,#888)">Carregando…</div>`;
    const app = document.getElementById('app');
    if (app) app.appendChild(mural);

    try {
        const data = await apiFetch(`/api/v1/feedback/${encodeURIComponent(universeSlug)}`, {}, true);
        const body = mural.querySelector('#co-feedback-mural-body');
        if (!data || !data.items || data.items.length === 0) {
            body.innerHTML = '<p>Nenhum feedback público disponível.</p>';
            return;
        }
        const kindLabel = { feedback: 'Comentário', duvida: 'Dúvida', sugestao: 'Sugestão' };
        body.innerHTML = data.items.map(item => {
            const ts = new Date(item.created_at * 1000).toLocaleString('pt-BR');
            const from = item.name ? esc(item.name) : 'Anônimo';
            const epLink = item.entry_path
                ? `<a href="/${esc(universeSlug)}/${item.entry_path.split('/').map(encodeURIComponent).join('/')}" style="font-size:12px;color:var(--primary,#3b82f6)">${esc(item.entry_path)}</a>`
                : '';
            return `
<div style="border:1px solid var(--border,#e5e7eb);border-radius:8px;padding:14px 16px;margin-bottom:12px">
  <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px;flex-wrap:wrap">
    <span style="font-size:11px;padding:2px 6px;border-radius:4px;background:var(--tag-bg,#f3f4f6)">${esc(kindLabel[item.kind] || item.kind)}</span>
    <span style="font-size:12px;opacity:.65">${esc(from)}</span>
    ${epLink}
    <span style="font-size:11px;opacity:.4;margin-left:auto">${esc(ts)}</span>
  </div>
  <p style="margin:0;font-size:14px;line-height:1.5">${esc(item.message)}</p>
</div>`;
        }).join('');
    } catch (err) {
        const body = mural.querySelector('#co-feedback-mural-body');
        if (body) body.innerHTML = `<p style="color:var(--error,#ef4444)">Erro: ${esc(err.message)}</p>`;
    }
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
    renderBreadcrumbs();
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

// CO-272: map a dev-task (entry-as-task) to the legacy task shape expected by
// the kanban and modal, using a djb2-derived stable integer id.
function _devTaskId(path) {
    let h = 5381;
    for (let i = 0; i < path.length; i++) h = (((h << 5) + h) ^ path.charCodeAt(i)) >>> 0;
    return (h % 9_000_000) + 1_000_000;
}

function _mapDevTask(dt) {
    return {
        id: _devTaskId(dt.path),
        key: dt.key,
        title: dt.title,
        status: dt.status,
        priority: dt.priority,
        description: dt.description,
        labels: [],
        due_date: null,
        assignee: null,
        parent: null,
        archived: false,
        created_at: dt.created_at,
        updated_at: dt.updated_at,
    };
}

async function refreshTasks() {
    if (state.currentProject) {
        state.tasks = await api.getTasks(state.currentProject.key, { archived: state.showArchived });
    }
    // CO-272: merge dev-tasks for public-subscribable universes so the kanban
    // shows actual work/ entries (CO-N, AL-N, QB-N, ...) alongside legacy tasks.
    const slug = state.currentUniverseSlug;
    if (state.universeInfo?.visibility === 'public-subscribable' && slug) {
        const devTasks = await api.getDevTasks(slug);
        if (devTasks.length > 0) {
            state.tasks = [...state.tasks, ...devTasks.map(_mapDevTask)];
        }
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
    injectBootCallbacks({ showLoading, hideLoading, render, selectProject, removeManifestViewTabs, injectManifestViewTabs, switchView, onUniverseInfoLoaded: showSourceBusBanner });
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
    // CO-323: apply single-universe mode when the page is served from a
    // *.artelonga.com.br subdomain (window.__CO_SUBDOMAIN_UNIVERSE__ is injected
    // by the server). Hides the multi-universe sidebar via CSS class.
    if (window.__CO_SUBDOMAIN_UNIVERSE__) {
        document.body.classList.add('co-single-universe-mode');
    }

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
