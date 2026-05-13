// ===== CO-209: Conversas — first-time welcome modal =====
// Shown once per browser profile when the user opens Conversas for the first time.
// Tracks dismissal via localStorage["co_chat_welcome_seen"] = "1".

import { esc } from './helpers.js';

const STORAGE_KEY = 'co_chat_welcome_seen';

export function hasSeenWelcome() {
    try { return localStorage.getItem(STORAGE_KEY) === '1'; } catch (_) { return true; }
}

function markSeen() {
    try { localStorage.setItem(STORAGE_KEY, '1'); } catch (_) {}
}

function t(key, vars = {}) {
    const val = (window.t ? window.t(key) : null) || key;
    return Object.entries(vars).reduce((s, [k, v]) => s.replace(`{${k}}`, v), val);
}

/**
 * Show the welcome modal over the given container.
 * @param {HTMLElement} container — the conversas drawer element
 * @param {Function} onDismiss — called after the modal is dismissed
 */
export function showConversasWelcome(container, onDismiss) {
    if (hasSeenWelcome()) { onDismiss(); return; }

    const overlay = document.createElement('div');
    overlay.className = 'conversas-welcome-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-label', esc(t('conversas.title')));

    overlay.innerHTML = `
<div class="conversas-welcome-card">
  <button class="conversas-welcome-close" id="conversas-welcome-close" aria-label="Fechar">&times;</button>
  <h2 class="conversas-welcome-title">${esc(t('conversas.title'))}</h2>
  <p class="conversas-welcome-headline">${esc(t('conversas.welcome.headline'))}</p>
  <p class="conversas-welcome-body">${esc(t('conversas.welcome.body'))}</p>
  <p class="conversas-welcome-hint">${esc(t('conversas.welcome.members_hint'))}</p>
  <div class="conversas-welcome-dismiss-hint" aria-hidden="true">⏎ &nbsp;◻ &nbsp;&times;</div>
</div>`;

    container.appendChild(overlay);

    function dismiss() {
        markSeen();
        overlay.remove();
        document.removeEventListener('keydown', onKey);
        onDismiss();
    }

    function onKey(e) {
        if (e.key === 'Enter') { e.preventDefault(); dismiss(); }
    }

    document.addEventListener('keydown', onKey);

    overlay.querySelector('#conversas-welcome-close').addEventListener('click', dismiss);

    // Click outside the card
    overlay.addEventListener('click', (e) => {
        if (e.target === overlay) dismiss();
    });
}
