// CO-393: DOM setup — event wiring and boot sequence for variant i.
// Extracted from app.js so that app.js stays ≤ 500 LoC.

import { state } from '../../a/modules/state.js';
import { api, apiFetch } from '../../a/modules/api.js';
import { esc } from '../../a/modules/helpers.js';
import { ZOOM_DAYS, rebuildI18nConstants } from '../../a/modules/constants.js';
import { addDays, todayDate } from '../../a/modules/helpers.js';
import {
    showLoading, hideLoading, setupSettingsPanel, setupLoginModal, setupSecurityModal,
} from '../../a/modules/settings.js';
import {
    renderHeaderUserArea, renderUserBadge, renderUsageCount,
    renderMiniCalendar, setupHamburgerMenu, injectSidebarShowLogin,
    renderNotificationSettings,
} from '../../a/modules/sidebar.js';
import { closeModal, closeStatusDropdown, handleFormSubmit, handleDelete, handleArchive,
    setupUsageLimitModal, setupSubscribePromptModal, toggleActivityPanel,
    setupUniverseInfoModal,
} from '../../a/modules/modals.js';
import { setupLoginModal as _setupLoginModalFn, showLoginModal, hideLoginModal, setupSecurityModal as _setupSecurityModalFn } from '../../a/modules/login.js';
import { setupOnboarding, injectOnboardingCallbacks } from '../../a/modules/onboarding.js';
import { setupInvitationsPage, setupInvitationsPanel } from '../../a/modules/invitations.js';
import { bootAppForUniverse, renderUniverseHome } from '../../a/modules/boot.js';
import { bootYggdrasil } from '../../a/modules/yggdrasil.js';
import { renderNotificationsPage } from '../../a/modules/notifications.js';
import { openConversas, destroyConversas } from '../../a/modules/conversas.js';
import { setupNotifications, bumpUnreadCount } from '../../a/modules/notifications.js';
import { openDmWith, mountDmInbox } from '../../a/modules/dm.js';
import { openZoomModal } from './lenses/document.js';
import { renderForm, collect } from './form/engine.js';
import { renderUniverseForm } from './form/engine.js';

let _notifSetupDone = false;

export function setupDom(callbacks) {
    const {
        render, renderContent, selectProject, switchView, switchZoom, refreshTasks,
        bootApp, setUniverseSlugInUrl, loadMeUniverses, openTaskModal, ensureOwnUniverse,
        injectManifestViewTabs, initTimelineStart, showTemplateBanner, hideTemplateBanner,
        readUniverseSlugFromUrl, readEntryPathFromUrl, readGameFromUrl,
        maybeOpenEntryFromUrl, initFooter,
    } = callbacks;

    function _updateChatButton(me) {
        const btn = document.getElementById('btn-open-conversas');
        if (btn) btn.classList.toggle('hidden', !me);
        const notifWrap = document.getElementById('notif-wrap');
        if (notifWrap) notifWrap.classList.toggle('hidden', !me);
        if (me && !_notifSetupDone) { _notifSetupDone = true; setupNotifications(); }
    }

    function bindStaticEvents() {
        document.querySelectorAll('#view-tabs .view-tab').forEach(tab =>
            tab.addEventListener('click', () => switchView(tab.dataset.view)));
        document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab =>
            tab.addEventListener('click', () => switchZoom(tab.dataset.zoom)));

        document.querySelector('#btn-prev')?.addEventListener('click', () => {
            const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
            state.timelineStart = addDays(state.timelineStart || todayDate(), -shift);
            renderContent();
        });
        document.querySelector('#btn-today')?.addEventListener('click', () => { initTimelineStart(); renderContent(); });
        document.querySelector('#btn-next')?.addEventListener('click', () => {
            const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
            state.timelineStart = addDays(state.timelineStart || todayDate(), shift);
            renderContent();
        });
        document.querySelector('#btn-new-task')?.addEventListener('click', async () => {
            if (!(await ensureOwnUniverse())) return;
            if (!state.currentProject) {
                if (!state.projects || state.projects.length === 0) { return; }
                await selectProject(state.projects[0].key);
            }
            if (state.currentProject) openTaskModal(null);
        });
        document.querySelector('#modal-close')?.addEventListener('click', closeModal);
        document.querySelector('#btn-cancel')?.addEventListener('click', closeModal);
        document.querySelector('#task-form')?.addEventListener('submit', handleFormSubmit);
        document.querySelector('#btn-delete')?.addEventListener('click', handleDelete);
        document.querySelector('#btn-archive')?.addEventListener('click', handleArchive);
        document.querySelector('#modal-overlay')?.addEventListener('click', e => { if (e.target === e.currentTarget) closeModal(); });
        document.querySelector('#search-input')?.addEventListener('input', e => { state.searchQuery = e.target.value; renderContent(); });
        document.getElementById('show-archived')?.addEventListener('change', async (e) => {
            state.showArchived = e.target.checked; await refreshTasks(); render();
        });
        document.getElementById('btn-activity')?.addEventListener('click', toggleActivityPanel);
        document.addEventListener('keydown', e => {
            if (e.key === 'Escape') {
                closeStatusDropdown(); closeModal();
                document.querySelector('#activity-panel')?.classList.add('hidden');
                document.getElementById('sidebar')?.classList.remove('open');
                document.getElementById('sidebar-overlay')?.classList.remove('visible');
                return;
            }
            const active = document.activeElement;
            if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.tagName === 'SELECT' || active.isContentEditable)) return;
            switch (e.key) {
                case 'n': e.preventDefault(); if (state.currentProject) openTaskModal(null); break;
                case '/': e.preventDefault(); document.querySelector('#search-input')?.focus(); break;
                case '1': e.preventDefault(); switchView('kanban'); break;
                case '2': e.preventDefault(); switchView('table'); break;
                case '3': e.preventDefault(); switchView('timeline'); break;
                case '4': e.preventDefault(); switchView('calendar'); break;
                case '5': e.preventDefault(); switchView('dashboard'); break;
                case '6': e.preventDefault(); switchView('document'); break;
            }
        });
        document.addEventListener('co:langchange', () => { rebuildI18nConstants(); render(); });
    }

    // Universe create form using the form engine (AC3).
    function setupCriarUniverseModal() {
        const btn = document.getElementById('btn-sidebar-new-universe');
        const overlay = document.getElementById('criar-modal-overlay');
        if (!btn) return;

        btn.addEventListener('click', () => {
            const { form, collect: collectUniverse, validate: validateUniverse } = renderUniverseForm();
            const modal = document.createElement('div');
            modal.className = 'modal-overlay';
            modal.id = 'co-criar-universe-modal';
            modal.innerHTML = `<div class="modal" style="max-width:480px">
                <div class="modal-header">
                    <h2>${window.t ? window.t('sidebar.new_universe') : 'Novo universo'}</h2>
                    <button class="modal-close" id="co-criar-close">&times;</button>
                </div>
                <div id="co-criar-form-body"></div>
                <div class="modal-footer" style="display:flex;gap:8px;justify-content:flex-end;padding:16px">
                    <button class="btn btn-secondary" id="co-criar-cancel">${window.t ? window.t('action.cancel') : 'Cancelar'}</button>
                    <button class="btn btn-primary" id="co-criar-submit">${window.t ? window.t('action.create') : 'Criar'}</button>
                </div>
            </div>`;
            document.body.appendChild(modal);
            modal.querySelector('#co-criar-form-body').appendChild(form);

            const close = () => modal.remove();
            modal.querySelector('#co-criar-close').addEventListener('click', close);
            modal.querySelector('#co-criar-cancel').addEventListener('click', close);
            modal.addEventListener('click', e => { if (e.target === modal) close(); });

            modal.querySelector('#co-criar-submit').addEventListener('click', async () => {
                const errors = validateUniverse();
                if (errors.length > 0) { alert(errors.join('\n')); return; }
                const data = collectUniverse();
                const btn2 = modal.querySelector('#co-criar-submit');
                btn2.disabled = true;
                try {
                    const created = await apiFetch('/api/v1/universes', {
                        method: 'POST',
                        body: JSON.stringify({ key: data.key, name: data.name, description: data.description, visibility: data.visibility || 'private' }),
                        headers: { 'Content-Type': 'application/json' },
                    }, true);
                    if (created) {
                        close();
                        await loadMeUniverses();
                        render();
                    }
                } catch (err) {
                    btn2.disabled = false;
                }
            });
        });
    }

    async function init() {
        if (window.__CO_SUBDOMAIN_UNIVERSE__) document.body.classList.add('co-single-universe-mode');
        window.setLang(window.currentLang);
        document.getElementById('btn-header-lang')?.addEventListener('click', () => { window.setLang(window.currentLang === 'pt' ? 'en' : 'pt'); render(); });
        renderHeaderUserArea(null);
        initTimelineStart();
        callbacks.wireModules();
        setupHamburgerMenu();
        setupLoginModal();
        setupSecurityModal();
        document.getElementById('btn-security')?.addEventListener('click', () => {
            import('../../a/modules/notification-settings.js').then(({ renderNotificationSettings: rns }) => {
                rns(document.getElementById('notif-settings-content'));
            }).catch(() => {});
        });
        setupCriarUniverseModal();
        setupUsageLimitModal();
        setupSubscribePromptModal();
        setupSettingsPanel();
        setupInvitationsPanel();
        initFooter();
        bindStaticEvents();

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

        if (state.isTemplate) {
            const requestedEntry = readEntryPathFromUrl('template');
            const requestedPage = new URLSearchParams(window.location.search).get('page');
            const stayOnTemplate = !!(requestedEntry || requestedPage);
            const me = await api.me();
            if (me) {
                hideLoginModal(); renderUserBadge(me); state.me = me; _updateChatButton(me);
                await loadMeUniverses();
                const mine = state.userUniverses.filter(u => !u.is_template);
                if (mine.length > 0 && !stayOnTemplate) {
                    const preferred = (() => { try { return localStorage.getItem('co_preferred_universe'); } catch (_) { return null; } })();
                    const target = (preferred && mine.find(u => u.key === preferred)) ? preferred : mine[0].key;
                    state.currentUniverseSlug = target; state.isTemplate = false;
                    hideTemplateBanner();
                    await bootAppForUniverse(target); return;
                }
            }
            if (!me) { showTemplateBanner(); setupOnboarding(); }
            await bootAppForUniverse('template');
            await maybeOpenEntryFromUrl('template');
            return;
        }

        const me = await api.me();
        if (me) {
            hideLoginModal(); renderUserBadge(me); state.me = me; _updateChatButton(me);
            await loadMeUniverses();
            await bootAppForUniverse(slug);
            await maybeOpenEntryFromUrl(slug);
            return;
        }

        const info = await api.getUniverseInfo(slug);
        if (info && info.is_anonymous) {
            state.universeInfo = info; renderUsageCount();
            state.projects = await api.getUniverseProjects(slug);
            if (state.projects.length > 0) await selectProject(state.projects[0].key);
            render(); return;
        }
        if (info && info.visibility === 'public-subscribable') {
            await bootAppForUniverse(slug); await maybeOpenEntryFromUrl(slug); return;
        }

        state.currentUniverseSlug = 'template'; state.isTemplate = true;
        setUniverseSlugInUrl('template');
        showTemplateBanner(); setupOnboarding();
        await bootAppForUniverse('template');
    }

    document.addEventListener('DOMContentLoaded', init);
}
