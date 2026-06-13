// ===== CO-96: Universe CRUD actions — context menu, delete, archive, trash =====
//
// Right-click a universe in the sidebar to open a context menu (Open, Rename,
// Change visibility, Duplicate, Settings, Archive, Delete). Delete shows a
// type-the-key confirmation and soft-deletes; the trash view restores deleted
// or archived universes within the 30-day window.

import { state } from '../state.js';
import { api } from '../api.js';
import { openCriarModal } from '../onboarding.js';
import { openSettingsPanel } from '../settings.js';

// Callbacks injected by app.js to break circular deps.
let _loadMeUniverses = async () => {};
let _renderSidebar = () => {};
let _bootAppForUniverse = async () => {};
let _setUniverseSlugInUrl = () => {};
let _showToast = () => {};

export function injectUniverseActionsCallbacks(cb) {
    if (cb.loadMeUniverses) _loadMeUniverses = cb.loadMeUniverses;
    if (cb.renderSidebar) _renderSidebar = cb.renderSidebar;
    if (cb.bootAppForUniverse) _bootAppForUniverse = cb.bootAppForUniverse;
    if (cb.setUniverseSlugInUrl) _setUniverseSlugInUrl = cb.setUniverseSlugInUrl;
    if (cb.showToast) _showToast = cb.showToast;
}

// --- Helpers ---------------------------------------------------------------

function findUniverse(slug) {
    const me = state.meUniverses;
    if (me) {
        for (const bucket of ['owned', 'member', 'subscribed', 'discoverable']) {
            const u = (me[bucket] || []).find(x => x.key === slug);
            if (u) return u;
        }
    }
    return (state.userUniverses || []).find(u => u.key === slug) || { key: slug, name: slug };
}

async function refreshAfterChange() {
    await _loadMeUniverses();
    _renderSidebar();
}

// --- Context menu ----------------------------------------------------------

function closeContextMenu() {
    const menu = document.getElementById('universe-context-menu');
    if (menu) {
        menu.classList.add('hidden');
        menu.replaceChildren();
    }
}

const VISIBILITIES = [
    { value: 'private', label: 'Privado' },
    { value: 'public', label: 'Público' },
    { value: 'unlisted', label: 'Não listado' },
];

export function showUniverseContextMenu(slug, x, y) {
    const menu = document.getElementById('universe-context-menu');
    if (!menu) return;
    const u = findUniverse(slug);
    const isTemplate = slug === 'template';

    // CO-96: built with DOM nodes + textContent (not innerHTML) so labels can
    // never inject markup and the security scanner stays clean.
    menu.replaceChildren();
    const item = (id, label, extraClass = '') => {
        const b = document.createElement('button');
        b.className = `context-menu-item ${extraClass}`.trim();
        b.dataset.action = id;
        b.setAttribute('role', 'menuitem');
        b.textContent = label;
        return b;
    };
    const sep = () => {
        const d = document.createElement('div');
        d.className = 'context-menu-sep';
        return d;
    };
    const groupLabel = (text) => {
        const d = document.createElement('div');
        d.className = 'context-menu-group-label';
        d.textContent = text;
        return d;
    };

    menu.append(item('open', 'Abrir'), sep(), item('rename', 'Renomear…'), groupLabel('Visibilidade'));
    for (const v of VISIBILITIES) {
        const b = item('visibility', v.label, 'context-menu-sub');
        b.dataset.value = v.value;
        menu.appendChild(b);
    }
    menu.append(item('duplicate', 'Duplicar…'), sep(), item('settings', 'Configurações…'), sep());
    if (!isTemplate) {
        menu.append(item('archive', 'Arquivar'), item('delete', 'Excluir…', 'context-menu-danger'));
    }

    // Position, keeping the menu on-screen.
    menu.classList.remove('hidden');
    const rect = menu.getBoundingClientRect();
    const left = Math.min(x, window.innerWidth - rect.width - 8);
    const top = Math.min(y, window.innerHeight - rect.height - 8);
    menu.style.left = `${Math.max(4, left)}px`;
    menu.style.top = `${Math.max(4, top)}px`;

    menu.querySelectorAll('.context-menu-item').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const action = btn.dataset.action;
            closeContextMenu();
            await handleAction(action, slug, u, btn.dataset.value);
        });
    });
}

async function handleAction(action, slug, u, value) {
    switch (action) {
        case 'open':
            _setUniverseSlugInUrl(slug);
            state.currentUniverseSlug = slug;
            await _bootAppForUniverse(slug);
            break;
        case 'rename': {
            const next = window.prompt('Novo nome do universo:', u.name || slug);
            if (next == null) return;
            const name = next.trim();
            if (!name || name === u.name) return;
            const r = await api.updateUniverse(slug, { name });
            if (r) {
                _showToast('Universo renomeado', 'success');
                await refreshAfterChange();
            }
            break;
        }
        case 'visibility': {
            const r = await api.updateUniverse(slug, { visibility: value });
            if (r) {
                _showToast('Visibilidade atualizada', 'success');
                await refreshAfterChange();
            }
            break;
        }
        case 'duplicate':
            openCriarModal({ copyFrom: slug });
            break;
        case 'settings':
            // Switch to the universe first so the settings panel edits the right one.
            if (slug !== state.currentUniverseSlug) {
                _setUniverseSlugInUrl(slug);
                state.currentUniverseSlug = slug;
                await _bootAppForUniverse(slug);
            }
            openSettingsPanel();
            break;
        case 'archive': {
            const r = await api.archiveUniverse(slug);
            if (r) {
                _showToast('Universo arquivado', 'success');
                await refreshAfterChange();
            }
            break;
        }
        case 'delete':
            openDeleteUniverseModal(slug, u.name || slug);
            break;
    }
}

// --- Delete confirmation modal ---------------------------------------------

export function openDeleteUniverseModal(slug, name) {
    const overlay = document.getElementById('delete-universe-overlay');
    if (!overlay) return;
    const nameEl = document.getElementById('delete-universe-name');
    const keyHint = document.getElementById('delete-universe-keyhint');
    const input = document.getElementById('delete-universe-input');
    const confirmBtn = document.getElementById('delete-universe-confirm');
    if (nameEl) nameEl.textContent = name || slug;
    if (keyHint) keyHint.textContent = slug;
    if (input) input.value = '';
    if (confirmBtn) {
        confirmBtn.disabled = true;
        confirmBtn.dataset.slug = slug;
    }
    overlay.classList.remove('hidden');
    if (input) input.focus();
}

function closeDeleteModal() {
    const overlay = document.getElementById('delete-universe-overlay');
    if (overlay) overlay.classList.add('hidden');
}

// --- Trash / recovery modal -------------------------------------------------

async function openTrashModal() {
    const overlay = document.getElementById('trash-overlay');
    if (!overlay) return;
    const list = document.getElementById('trash-list');
    const empty = document.getElementById('trash-empty');
    if (list) {
        const loading = document.createElement('p');
        loading.className = 'form-hint';
        loading.textContent = 'Carregando…';
        list.replaceChildren(loading);
    }
    overlay.classList.remove('hidden');

    const items = await api.getTrash();
    if (!list) return;
    if (!items.length) {
        list.replaceChildren();
        if (empty) empty.classList.remove('hidden');
        return;
    }
    if (empty) empty.classList.add('hidden');
    // CO-96: DOM + textContent (not innerHTML) — universe names are user data,
    // so building with textContent escapes them by construction.
    list.replaceChildren();
    for (const it of items) {
        const row = document.createElement('div');
        row.className = 'trash-row';
        row.dataset.slug = it.key;

        const info = document.createElement('div');
        info.className = 'trash-row-info';
        const name = document.createElement('span');
        name.className = 'trash-row-name';
        name.textContent = it.name || it.key;
        const badge = document.createElement('span');
        badge.className = 'trash-row-badge';
        badge.textContent = it.state === 'deleted' ? 'Excluído' : 'Arquivado';
        info.append(name, badge);

        const restore = document.createElement('button');
        restore.className = 'btn btn-sm btn-secondary trash-restore-btn';
        restore.dataset.slug = it.key;
        restore.textContent = 'Restaurar';

        row.append(info, restore);
        list.appendChild(row);
    }

    list.querySelectorAll('.trash-restore-btn').forEach(btn => {
        btn.addEventListener('click', async () => {
            const slug = btn.dataset.slug;
            btn.disabled = true;
            const r = await api.restoreUniverse(slug);
            if (r) {
                _showToast('Universo restaurado', 'success');
                const row = btn.closest('.trash-row');
                if (row) row.remove();
                await refreshAfterChange();
                if (!list.querySelector('.trash-row') && empty) empty.classList.remove('hidden');
            } else {
                btn.disabled = false;
            }
        });
    });
}

function closeTrashModal() {
    const overlay = document.getElementById('trash-overlay');
    if (overlay) overlay.classList.add('hidden');
}

// --- One-time wiring --------------------------------------------------------

export function setupUniverseActions() {
    // Dismiss the context menu on any outside click / escape / scroll.
    document.addEventListener('click', (e) => {
        const menu = document.getElementById('universe-context-menu');
        if (menu && !menu.classList.contains('hidden') && !menu.contains(e.target)) {
            closeContextMenu();
        }
    });
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') closeContextMenu();
    });

    // Delete modal wiring.
    const delOverlay = document.getElementById('delete-universe-overlay');
    const delInput = document.getElementById('delete-universe-input');
    const delConfirm = document.getElementById('delete-universe-confirm');
    const delCancel = document.getElementById('delete-universe-cancel');
    const delClose = document.getElementById('delete-universe-close');
    if (delInput && delConfirm) {
        delInput.addEventListener('input', () => {
            delConfirm.disabled = delInput.value.trim() !== delConfirm.dataset.slug;
        });
        delInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter' && !delConfirm.disabled) delConfirm.click();
        });
    }
    if (delConfirm) {
        delConfirm.addEventListener('click', async () => {
            const slug = delConfirm.dataset.slug;
            if (!slug || delInput.value.trim() !== slug) return;
            delConfirm.disabled = true;
            const r = await api.deleteUniverse(slug);
            closeDeleteModal();
            if (r) {
                _showToast('Universo movido para a lixeira', 'success');
                // If we deleted the universe we're viewing, fall back to template.
                if (slug === state.currentUniverseSlug) {
                    _setUniverseSlugInUrl('template');
                    state.currentUniverseSlug = 'template';
                    state.isTemplate = true;
                    await _bootAppForUniverse('template');
                }
                await refreshAfterChange();
            }
        });
    }
    delCancel && delCancel.addEventListener('click', closeDeleteModal);
    delClose && delClose.addEventListener('click', closeDeleteModal);
    delOverlay && delOverlay.addEventListener('click', e => { if (e.target === delOverlay) closeDeleteModal(); });

    // Trash modal wiring.
    const trashBtn = document.getElementById('btn-sidebar-trash');
    trashBtn && trashBtn.addEventListener('click', openTrashModal);
    const trashOverlay = document.getElementById('trash-overlay');
    const trashDone = document.getElementById('trash-done');
    const trashClose = document.getElementById('trash-close');
    trashDone && trashDone.addEventListener('click', closeTrashModal);
    trashClose && trashClose.addEventListener('click', closeTrashModal);
    trashOverlay && trashOverlay.addEventListener('click', e => { if (e.target === trashOverlay) closeTrashModal(); });
}
