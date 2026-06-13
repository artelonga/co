// ===== Settings panel + theme utilities =====
import { state } from './state.js';
import { api } from './api.js';
import { THEME_PALETTE_MAP, THEME_COMPANION, DARK_THEMES } from './constants.js';
// CO-96: danger-zone delete reuses the type-the-key confirmation modal.
import { openDeleteUniverseModal } from './sidebar/universe-actions.js';

// ===== Toast system =====
export function showToast(message, type) {
    const container = document.getElementById('toast-container');
    if (!container) return;

    const toast = document.createElement('div');
    toast.className = 'toast toast-' + (type || 'success');
    toast.textContent = message;
    container.appendChild(toast);

    setTimeout(() => {
        toast.classList.add('toast-fade-out');
        setTimeout(() => {
            if (toast.parentNode) toast.parentNode.removeChild(toast);
        }, 300);
    }, 3000);
}

// ===== Loading helpers =====
export function showLoading() {
    state.loading = true;
    const content = document.querySelector('#content');
    if (content) {
        content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Loading...</p></div>';
    }
}

export function hideLoading() {
    state.loading = false;
}

// ===== Timeout helper =====
export function withTimeout(promise, ms, label) {
    return new Promise((resolve) => {
        let done = false;
        const t = setTimeout(() => {
            if (done) return;
            done = true;
            console.warn(`withTimeout(${label}): timed out after ${ms}ms`);
            resolve(null);
        }, ms);
        promise.then(
            (val) => { if (done) return; done = true; clearTimeout(t); resolve(val); },
            (err) => { if (done) return; done = true; clearTimeout(t); console.warn(`withTimeout(${label}):`, err); resolve(null); }
        );
    });
}

// ===== Theme loading =====
export function loadThemeCss(slug) {
    if (!slug) return;
    // CO-313: theme is global per-user, not per-universe. The user picks once
    // via the header switcher (writes co_user_palette); that pick applies to
    // every universe they visit. Without a pick, default is 'modern'.
    // The universe's own theme_preset is intentionally ignored — per-universe
    // theme switching surprised users who expected one consistent look across
    // their boards.
    let preset = null;
    try {
        preset = localStorage.getItem('co_user_palette');
    } catch (_) {}
    if (!preset) {
        preset = 'modern';
    }
    const v = (window._coBootTs ||= Math.floor(Date.now() / 1000));
    const href = `/api/v1/themes/${encodeURIComponent(preset)}?v=${v}`;
    let link = document.getElementById('co-theme-css');
    if (link) {
        if (link.href !== new URL(href, document.baseURI).href) {
            link.href = href;
        }
    } else {
        link = document.createElement('link');
        link.id = 'co-theme-css';
        link.rel = 'stylesheet';
        link.href = href;
        document.head.appendChild(link);
    }
}

export function loadCustomFonts(config) {
    const fontFamilies = [config.font_headline, config.font_body]
        .filter(Boolean)
        .map(f => encodeURIComponent(f))
        .join('&family=');
    const existingFontLink = document.getElementById('co-universe-fonts');
    if (fontFamilies) {
        const href = `https://fonts.googleapis.com/css2?family=${fontFamilies}&display=swap`;
        if (existingFontLink) {
            existingFontLink.href = href;
        } else {
            if (!document.querySelector('link[href="https://fonts.googleapis.com"]')) {
                const preconn1 = document.createElement('link');
                preconn1.rel = 'preconnect';
                preconn1.href = 'https://fonts.googleapis.com';
                document.head.appendChild(preconn1);
            }
            if (!document.querySelector('link[href="https://fonts.gstatic.com"]')) {
                const preconn2 = document.createElement('link');
                preconn2.rel = 'preconnect';
                preconn2.href = 'https://fonts.gstatic.com';
                preconn2.crossOrigin = 'anonymous';
                document.head.appendChild(preconn2);
            }
            const link = document.createElement('link');
            link.id = 'co-universe-fonts';
            link.rel = 'stylesheet';
            link.href = href;
            document.head.appendChild(link);
        }
    } else if (existingFontLink) {
        existingFontLink.remove();
    }
}

// Callback injected by app.js to call switchView without circular import
let _switchView = () => {};
export function injectSwitchView(fn) { _switchView = fn; }

export function applyUniverseConfig(config) {
    if (!config) return;
    state.universeConfig = config;
    const slug = state.currentUniverseSlug;

    // CO-313: theme is global per-user. Ignore config.theme_preset on switch —
    // the user's chosen palette (or 'modern' by default) applies to every
    // universe. Per-universe theme switching surprised users; this gives a
    // consistent look across all their boards.
    const userPalette = localStorage.getItem('co_user_palette');

    localStorage.removeItem('co_named_palette');
    document.documentElement.removeAttribute('data-palette');

    if (slug) loadThemeCss(slug);

    const effectivePreset = userPalette || 'modern';
    const paletteKey = THEME_PALETTE_MAP[effectivePreset] ?? '';
    if (paletteKey) {
        document.documentElement.setAttribute('data-palette', paletteKey);
    } else {
        document.documentElement.removeAttribute('data-palette');
    }

    loadCustomFonts(config);

    // Content-first as universal entry view. The stored `config.layout`
    // ('board' for everyone created before this change) is no longer
    // used to pick the initial view — Conteúdo is always the entry
    // surface, and Kanban / Tabela / etc. are one click away in the
    // tab strip. Per-universe layout preference can come back later
    // with a real settings UI that distinguishes explicit user choice
    // from the migration default.
    const defaultView = 'conteudo';
    if (state.view !== defaultView) {
        _switchView(defaultView);
    }
}

// ===== Settings gear button =====
// Callback injected by app.js
let _openSettingsPanel = () => {};
export function injectOpenSettingsPanel(fn) { _openSettingsPanel = fn; }

export function renderSettingsGear(universeInfo) {
    const existing = document.getElementById('btn-settings-gear');
    if (existing) existing.remove();

    if (!universeInfo || universeInfo.is_template) return;

    const headerRight = document.querySelector('.header-right');
    if (!headerRight) return;

    const gearBtn = document.createElement('button');
    gearBtn.id = 'btn-settings-gear';
    gearBtn.className = 'btn-icon';
    gearBtn.title = 'Configurações do universo';
    gearBtn.setAttribute('aria-label', 'Configurações');
    gearBtn.innerHTML = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3"/>
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
    </svg>`;

    const btnNewTask = document.getElementById('btn-new-task');
    if (btnNewTask) {
        headerRight.insertBefore(gearBtn, btnNewTask);
    } else {
        headerRight.appendChild(gearBtn);
    }

    gearBtn.addEventListener('click', openSettingsPanel);
}

export function openSettingsPanel() {
    const overlay = document.getElementById('settings-modal-overlay');
    if (!overlay) return;

    const config = state.universeConfig || {};

    // CO-96: editable universe identity (name / description / visibility).
    const info = state.universeInfo || {};
    const nameInput = document.getElementById('settings-name');
    const descInput = document.getElementById('settings-description');
    const visSelect = document.getElementById('settings-visibility');
    if (nameInput) nameInput.value = info.name || '';
    if (descInput) descInput.value = info.description || '';
    if (visSelect) {
        // Map canonical stored visibility to the three user-facing options.
        const v = info.visibility === 'public-subscribable' ? 'public' : (info.visibility || 'private');
        visSelect.value = (v === 'public' || v === 'unlisted') ? v : 'private';
    }

    const themeSelect = document.getElementById('settings-theme');
    const layoutSelect = document.getElementById('settings-layout');
    const fontHeadlineInput = document.getElementById('settings-font-headline');
    const fontBodyInput = document.getElementById('settings-font-body');
    const customTokensInput = document.getElementById('settings-custom-tokens');

    const preset = config.theme_preset === 'scholarly-light' ? 'scholarly' : (config.theme_preset || 'scholarly');
    if (themeSelect) themeSelect.value = preset;
    if (layoutSelect) layoutSelect.value = config.layout || 'board';
    if (fontHeadlineInput) fontHeadlineInput.value = config.font_headline || '';
    if (fontBodyInput) fontBodyInput.value = config.font_body || '';
    if (customTokensInput) {
        customTokensInput.value = config.custom_tokens
            ? JSON.stringify(config.custom_tokens, null, 2)
            : '';
    }

    updateDarkToggleIcon(preset);
    overlay.classList.remove('hidden');
}

function updateDarkToggleIcon(preset) {
    const btn = document.getElementById('settings-dark-toggle');
    if (!btn) return;
    const isDark = DARK_THEMES.has(preset);
    if (isDark) {
        btn.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>`;
        btn.title = 'Modo claro';
    } else {
        btn.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>`;
        btn.title = 'Modo escuro';
    }
}

export function setupSettingsPanel() {
    const overlay = document.getElementById('settings-modal-overlay');
    if (!overlay) return;

    const closeBtn = document.getElementById('settings-modal-close');
    const cancelBtn = document.getElementById('settings-cancel');
    const form = document.getElementById('settings-form');

    function close() { overlay.classList.add('hidden'); }

    closeBtn && closeBtn.addEventListener('click', close);
    cancelBtn && cancelBtn.addEventListener('click', close);
    overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

    // CO-96: danger zone — archive / delete.
    const archiveBtn = document.getElementById('settings-archive');
    if (archiveBtn) {
        archiveBtn.addEventListener('click', async () => {
            const slug = state.currentUniverseSlug;
            if (!slug || slug === 'template') return;
            const r = await api.archiveUniverse(slug);
            if (r) {
                showToast('Universo arquivado', 'success');
                close();
            }
        });
    }
    const deleteBtn = document.getElementById('settings-delete');
    if (deleteBtn) {
        deleteBtn.addEventListener('click', () => {
            const slug = state.currentUniverseSlug;
            if (!slug || slug === 'template') return;
            close();
            openDeleteUniverseModal(slug, (state.universeInfo && state.universeInfo.name) || slug);
        });
    }

    const darkToggleBtn = document.getElementById('settings-dark-toggle');
    if (darkToggleBtn) {
        darkToggleBtn.addEventListener('click', () => {
            const themeSelect = document.getElementById('settings-theme');
            if (!themeSelect) return;
            const current = themeSelect.value;
            const companion = THEME_COMPANION[current] || 'scholarly';
            themeSelect.value = companion;
            updateDarkToggleIcon(companion);
        });
    }

    const themeSelectEl = document.getElementById('settings-theme');
    if (themeSelectEl) {
        themeSelectEl.addEventListener('change', () => {
            updateDarkToggleIcon(themeSelectEl.value);
        });
    }

    form && form.addEventListener('submit', async e => {
        e.preventDefault();
        const slug = state.currentUniverseSlug;

        const themeSelect = document.getElementById('settings-theme');
        const layoutSelect = document.getElementById('settings-layout');
        const fontHeadlineInput = document.getElementById('settings-font-headline');
        const fontBodyInput = document.getElementById('settings-font-body');
        const customTokensInput = document.getElementById('settings-custom-tokens');

        let customTokens = undefined;
        if (customTokensInput) {
            const raw = customTokensInput.value.trim();
            if (raw) {
                try {
                    customTokens = JSON.parse(raw);
                } catch {
                    showToast('JSON inválido nos tokens CSS', 'error');
                    return;
                }
            } else {
                customTokens = null;
            }
        }

        const update = {
            theme_preset: themeSelect ? themeSelect.value : undefined,
            layout: layoutSelect ? layoutSelect.value : undefined,
            font_headline: fontHeadlineInput ? fontHeadlineInput.value : undefined,
            font_body: fontBodyInput ? fontBodyInput.value : undefined,
            custom_tokens: customTokens,
        };

        const saveBtn = document.getElementById('settings-save');
        if (saveBtn) saveBtn.disabled = true;

        // CO-96: persist editable identity (name / description / visibility) via
        // PUT, alongside the presentation config.
        const nameInput = document.getElementById('settings-name');
        const descInput = document.getElementById('settings-description');
        const visSelect = document.getElementById('settings-visibility');
        const info = state.universeInfo || {};
        const patch = {};
        if (nameInput && nameInput.value.trim() && nameInput.value.trim() !== (info.name || '')) {
            patch.name = nameInput.value.trim();
        }
        if (descInput && descInput.value !== (info.description || '')) {
            patch.description = descInput.value;
        }
        if (visSelect) {
            const currentVis = info.visibility === 'public-subscribable' ? 'public' : (info.visibility || 'private');
            if (visSelect.value !== currentVis) patch.visibility = visSelect.value;
        }
        if (Object.keys(patch).length && slug) {
            const updated = await api.updateUniverse(slug, patch);
            if (updated) {
                state.universeInfo = { ...info, name: updated.name, description: updated.description, visibility: updated.visibility };
            }
        }

        const result = await api.updateUniverseConfig(slug, update);
        if (saveBtn) saveBtn.disabled = false;

        if (result) {
            const themeLink = document.getElementById('co-theme-css');
            if (themeLink && slug) {
                themeLink.href = `/api/v1/universes/${slug}/theme.css?_=${Date.now()}`;
            }
            applyUniverseConfig(result);
            showToast('Configurações salvas', 'success');
            close();
        }
    });
}
