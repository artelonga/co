// ===== CO-189: Invitations module =====
// CO-198: openDmWith imported lazily to avoid circular deps
let _openDmWithFn = null;
export function injectOpenDmWith(fn) { _openDmWithFn = fn; }

// A. Public /invitations/:token accept page
// B. Settings panel "Convidar pessoas" form + pending list

import { state } from './state.js';

const PENDING_INVITE_KEY = 'co_pending_invite_token';

let _showToast = null;
let _showLoginModal = null;
let _loadMeUniverses = async () => {};

export function injectInvitationsCallbacks({ showToast, showLoginModal, loadMeUniverses }) {
    _showToast = showToast;
    _showLoginModal = showLoginModal;
    if (loadMeUniverses) _loadMeUniverses = loadMeUniverses;
}

// ---------------------------------------------------------------------------
// A. Public invite accept page
// ---------------------------------------------------------------------------

export async function setupInvitationsPage() {
    if (!window.location.pathname.startsWith('/invitations/')) return false;

    const parts = window.location.pathname.split('/');
    const token = parts[2];
    if (!token) return false;

    const overlay = document.getElementById('invite-accept-overlay');
    if (!overlay) return false;
    overlay.classList.remove('hidden');

    const app = document.getElementById('app');
    if (app) app.style.display = 'none';

    await _loadAndRenderInvite(token, overlay);
    return true;
}

async function _loadAndRenderInvite(token, overlay) {
    const card = overlay.querySelector('.login-card');
    if (!card) return;

    card.innerHTML = `<div class="login-header">
        <span class="login-logo">Co</span>
        <p class="login-subtitle">${_t('loading', 'Carregando…')}</p>
    </div>`;

    let inv;
    try {
        const resp = await fetch(`/api/v1/invitations/${encodeURIComponent(token)}`);
        if (resp.status === 410) { _renderState(card, 'expired', null); return; }
        if (!resp.ok) { _renderState(card, 'error', null); return; }
        inv = await resp.json();
    } catch (_) { _renderState(card, 'error', null); return; }

    if (inv.already_consumed) { _renderState(card, 'consumed', inv); return; }

    let me = null;
    try {
        const r = await fetch('/api/v1/auth/me', { credentials: 'include' });
        if (r.ok) me = await r.json();
    } catch (_) {}

    _renderPreview(card, token, inv, me);
}

function _t(key, fallback) {
    return (window.t && window.t(key)) || fallback;
}

function _fmtDate(iso) {
    try {
        return new Date(iso).toLocaleDateString(
            document.documentElement.lang === 'en' ? 'en-US' : 'pt-BR',
            { day: 'numeric', month: 'long', year: 'numeric' }
        );
    } catch (_) { return iso; }
}

function _roleLabel(role) {
    return {
        member: _t('invitations.role_member', 'Membro'),
        admin: _t('invitations.role_admin', 'Administrador'),
        viewer: _t('invitations.role_viewer', 'Apenas leitura'),
    }[role] || role;
}

function _renderState(card, stateKey, inv) {
    const msgs = {
        expired: _t('invitations.expired', 'Convite expirado'),
        consumed: _t('invitations.consumed', 'Convite já usado'),
        error: 'Convite inválido ou não encontrado.',
    };
    const sub = (stateKey === 'consumed' && inv)
        ? `${_esc(inv.invited_by.display_name)} — ${_esc(inv.universe.name)}`
        : '';
    card.innerHTML = `
        <div class="login-header">
            <span class="login-logo">Co</span>
            <h1 class="login-title">${msgs[stateKey]}</h1>
            ${sub ? `<p class="login-subtitle">${sub}</p>` : ''}
        </div>
        <div class="login-footer"><a href="/" class="btn-text-sm">← Co</a></div>`;
}

function _renderPreview(card, token, inv, me) {
    const invBy = _t('invitations.preview_invited_by', '{name} convidou você para')
        .replace('{name}', _esc(inv.invited_by.display_name || '?'));
    const roleStr = _t('invitations.preview_role', 'Permissão: {role}')
        .replace('{role}', _roleLabel(inv.role || 'member'));
    const expiresStr = _t('invitations.preview_expires', 'Expira em: {date}')
        .replace('{date}', _fmtDate(inv.expires_at));

    card.innerHTML = `
        <div class="login-header">
            <span class="login-logo">Co</span>
            <h1 class="login-title">${_t('invitations.accept', 'Aceitar convite')}</h1>
            <p class="login-subtitle">${invBy} <strong>${_esc(inv.universe.name)}</strong></p>
        </div>
        <div class="invite-preview-meta">
            <p>${roleStr}</p>
            <p class="form-hint">${expiresStr}</p>
        </div>
        <p id="invite-page-msg" class="form-hint invite-page-msg"></p>
        <div class="form-actions" style="flex-direction:column;gap:0.5rem;margin-top:1rem">
            <button class="btn btn-primary btn-full" id="btn-invite-accept">
                ${_t('invitations.accept', 'Aceitar convite')}
            </button>
        </div>
        <div class="login-footer"><a href="/" class="btn-text-sm">← Co</a></div>`;

    document.getElementById('btn-invite-accept').addEventListener('click', async () => {
        if (!me) {
            // Store token so after login we redirect back
            try { sessionStorage.setItem(PENDING_INVITE_KEY, token); } catch (_) {}
            // Show login modal (overlay stays visible, app is hidden)
            // Restore app so login modal is accessible
            const app = document.getElementById('app');
            if (app) app.style.removeProperty('display');
            const invOverlay = document.getElementById('invite-accept-overlay');
            if (invOverlay) invOverlay.classList.add('hidden');
            if (_showLoginModal) _showLoginModal();
            return;
        }
        await _doAccept(token, inv);
    });
}

async function _doAccept(token, inv) {
    const btn = document.getElementById('btn-invite-accept');
    const msg = document.getElementById('invite-page-msg');
    if (btn) { btn.disabled = true; btn.textContent = '…'; }

    try {
        const resp = await fetch(`/api/v1/invitations/${encodeURIComponent(token)}/accept`, {
            method: 'POST',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({}),
        });

        if (resp.ok) {
            const data = await resp.json();
            const loadMsg = _t('invitations.accepted_loading', 'Convite aceito! Carregando {universe}…')
                .replace('{universe}', inv.universe.name);
            if (msg) { msg.style.color = 'var(--color-success,#16a34a)'; msg.textContent = loadMsg; }
            await _loadMeUniverses();
            setTimeout(() => { window.location.href = `/${data.universe_key}`; }, 1200);
        } else if (resp.status === 410) {
            const body = await resp.json().catch(() => ({}));
            const isConsumed = (body.message || '').toLowerCase().includes('already');
            if (msg) msg.textContent = isConsumed
                ? _t('invitations.consumed', 'Convite já usado')
                : _t('invitations.expired', 'Convite expirado');
            if (btn) btn.disabled = true;
        } else if (resp.status === 403) {
            if (msg) msg.textContent = _t('invitations.identity_mismatch',
                'Esse convite é para outro email/usuário. Saia e entre com a conta certa.');
            if (btn) { btn.disabled = false; btn.textContent = _t('invitations.accept', 'Aceitar convite'); }
        } else {
            if (msg) msg.textContent = 'Erro ao aceitar convite. Tente novamente.';
            if (btn) { btn.disabled = false; btn.textContent = _t('invitations.accept', 'Aceitar convite'); }
        }
    } catch (_) {
        if (msg) msg.textContent = 'Erro ao aceitar convite. Tente novamente.';
        if (btn) { btn.disabled = false; btn.textContent = _t('invitations.accept', 'Aceitar convite'); }
    }
}

// Called from app.js after successful login — redirect back to invite page if pending.
export function consumePendingInviteToken() {
    try {
        const t = sessionStorage.getItem(PENDING_INVITE_KEY);
        if (t) { sessionStorage.removeItem(PENDING_INVITE_KEY); }
        return t || null;
    } catch (_) { return null; }
}

// ---------------------------------------------------------------------------
// B. Settings panel invite form
// ---------------------------------------------------------------------------

export function setupInvitationsPanel() {
    const sendBtn = document.getElementById('btn-send-invite');
    if (sendBtn) {
        sendBtn.addEventListener('click', _handleSendInvite);
    }

    // Watch settings overlay for open/close transitions
    const overlay = document.getElementById('settings-modal-overlay');
    if (overlay) {
        new MutationObserver(() => {
            if (!overlay.classList.contains('hidden')) {
                _updateInviteSectionVisibility();
                _refreshPendingList();
                _refreshMembersList();
            }
        }).observe(overlay, { attributes: true, attributeFilter: ['class'] });
    }
}

async function _refreshMembersList() {
    const listEl = document.getElementById('universe-members-list');
    if (!listEl) return;
    const slug = state.currentUniverseSlug;
    if (!slug || slug === 'template') { listEl.innerHTML = ''; return; }

    try {
        const resp = await fetch(`/api/v1/universes/${encodeURIComponent(slug)}/members`, {
            credentials: 'include',
        });
        if (!resp.ok) { listEl.innerHTML = ''; return; }
        const members = await resp.json();
        if (!Array.isArray(members) || !members.length) { listEl.innerHTML = ''; return; }

        listEl.innerHTML = `<p class="form-hint" style="margin-top:0.5rem">${members.length} membro(s)</p>` +
            members.map(m => {
                const name = _esc(m.display_name || m.usuario || m.user_id || '?');
                return `<div class="invite-pending-row" style="display:flex;align-items:center;gap:0.5rem">
                    <span style="flex:1">${name}</span>
                    <span class="invite-role-badge">${_esc(m.role || '')}</span>
                    <button class="btn-text-sm dm-member-btn" data-user-id="${_esc(m.user_id)}" title="${window.t ? window.t('dm.start_dm') : 'Enviar mensagem'}">📩</button>
                </div>`;
            }).join('');

        listEl.querySelectorAll('.dm-member-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                if (_openDmWithFn) {
                    document.getElementById('settings-modal-overlay')?.classList.add('hidden');
                    _openDmWithFn(btn.dataset.userId, document.getElementById('chat-drawer'));
                }
            });
        });
    } catch (_) {
        listEl.innerHTML = '';
    }
}

function _updateInviteSectionVisibility() {
    const section = document.getElementById('invite-section');
    if (!section) return;
    const slug = state.currentUniverseSlug;
    if (!slug || slug === 'template') {
        section.classList.add('hidden');
    } else {
        section.classList.remove('hidden');
    }
}

async function _handleSendInvite() {
    const recipientEl = document.getElementById('invite-recipient');
    const roleEl = document.getElementById('invite-role');
    const msgEl = document.getElementById('invite-msg');
    const sendBtn = document.getElementById('btn-send-invite');
    if (!recipientEl || !roleEl) return;

    const recipient = recipientEl.value.trim();
    if (!recipient) return;

    const slug = state.currentUniverseSlug;
    if (!slug || slug === 'template') return;

    const body = recipient.includes('@')
        ? { email: recipient, role: roleEl.value || 'member' }
        : { usuario: recipient, role: roleEl.value || 'member' };

    if (sendBtn) sendBtn.disabled = true;
    if (msgEl) { msgEl.textContent = ''; msgEl.style.color = ''; }

    try {
        const resp = await fetch(`/api/v1/universes/${encodeURIComponent(slug)}/invitations`, {
            method: 'POST',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        });

        if (resp.status === 201) {
            const data = await resp.json();
            const msg = _t('invitations.sent', 'Convite enviado para {recipient}.')
                .replace('{recipient}', data.sent_to);
            if (_showToast) _showToast(msg, 'success');
            if (msgEl) { msgEl.textContent = msg; msgEl.style.color = ''; }
            recipientEl.value = '';
            await _refreshPendingList();
        } else if (resp.status === 409) {
            const m = _t('invitations.already_member', 'Esse usuário já é membro.');
            if (msgEl) { msgEl.textContent = m; msgEl.style.color = 'var(--color-error,#dc2626)'; }
        } else if (resp.status === 404) {
            const m = _t('invitations.user_not_found', 'Usuário não encontrado.');
            if (msgEl) { msgEl.textContent = m; msgEl.style.color = 'var(--color-error,#dc2626)'; }
        } else {
            if (msgEl) { msgEl.textContent = 'Erro ao enviar convite.'; msgEl.style.color = 'var(--color-error,#dc2626)'; }
        }
    } catch (_) {
        if (msgEl) { msgEl.textContent = 'Erro ao enviar convite.'; msgEl.style.color = 'var(--color-error,#dc2626)'; }
    } finally {
        if (sendBtn) sendBtn.disabled = false;
    }
}

async function _refreshPendingList() {
    const listEl = document.getElementById('invite-pending-list');
    if (!listEl) return;
    const slug = state.currentUniverseSlug;
    if (!slug || slug === 'template') { listEl.innerHTML = ''; return; }

    try {
        const resp = await fetch(`/api/v1/universes/${encodeURIComponent(slug)}/invitations`, {
            credentials: 'include',
        });
        if (!resp.ok) { listEl.innerHTML = ''; return; }
        const items = await resp.json();

        if (!items || !items.length) {
            listEl.innerHTML = `<p class="form-hint">${_t('invitations.empty_pending', 'Nenhum convite pendente.')}</p>`;
            return;
        }

        listEl.innerHTML = items.map(item => `
            <div class="invite-pending-row">
                <span class="invite-recipient">${_esc(item.recipient)}</span>
                <span class="invite-role-badge">${_esc(_roleLabel(item.role))}</span>
                ${item.consumed_at
                    ? `<span class="invite-status-badge invite-status-done">Aceito</span>`
                    : `<span class="invite-status-badge invite-status-pending">Pendente</span>`}
                <button class="btn-text-sm invite-cancel-btn" disabled title="Revogar (em breve)">Cancelar</button>
            </div>`).join('');
    } catch (_) {
        listEl.innerHTML = '';
    }
}

function _esc(str) {
    return String(str || '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}
