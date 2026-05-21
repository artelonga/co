// ===== Header render — project name, usage count, user area =====
import { state } from '../state.js';
import { apiFetch } from '../api.js';
import { esc } from '../helpers.js';

// Callback for login modal (injected to break circular dep)
let _showLoginModal = () => {};
export function injectShowLoginModal(fn) { _showLoginModal = fn; }

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
