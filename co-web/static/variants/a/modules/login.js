// ===== Login modal + security modal =====
import { state } from './state.js';
import { api, apiFetch } from './api.js';

let _render = () => {};
let _bootAppForUniverse = async () => {};
let _bootApp = async () => {};
let _renderUserBadge = () => {};
let _setUniverseSlugInUrl = () => {};
let _hideTemplateBanner = () => {};
let _showToast = () => {};

export function injectLoginCallbacks(callbacks) {
    _render = callbacks.render;
    _bootAppForUniverse = callbacks.bootAppForUniverse;
    _bootApp = callbacks.bootApp;
    _renderUserBadge = callbacks.renderUserBadge;
    _setUniverseSlugInUrl = callbacks.setUniverseSlugInUrl;
    _hideTemplateBanner = callbacks.hideTemplateBanner;
    _showToast = callbacks.showToast;
}

export function showLoginModal() {
    const overlay = document.getElementById('login-modal-overlay');
    if (overlay) {
        overlay.classList.remove('hidden');
        window.setLang(window.currentLang);
        const usuarioInput = document.getElementById('login-usuario');
        if (usuarioInput) usuarioInput.focus();
    }
}

export function hideLoginModal() {
    const overlay = document.getElementById('login-modal-overlay');
    if (overlay) overlay.classList.add('hidden');
}

export function setupLoginModal() {
    const btnEntrar = document.getElementById('btn-entrar');
    const btnLogout = document.getElementById('btn-logout');
    const btnLang = document.getElementById('btn-lang-toggle');

    if (btnLang) {
        btnLang.addEventListener('click', () => {
            window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
            _render();
        });
    }

    async function attemptLogin() {
        const usuario = document.getElementById('login-usuario').value.trim();
        const senha = document.getElementById('login-senha').value;
        if (!usuario || !senha) return;

        const errEl = document.getElementById('login-error');
        errEl.classList.add('hidden');
        btnEntrar.disabled = true;
        btnEntrar.textContent = window.t('signing_in');

        const r = await api.loginWithPassword(usuario, senha);

        btnEntrar.disabled = false;
        btnEntrar.textContent = window.t('sign_in');

        if (r && r.usuario) {
            hideLoginModal();
            localStorage.removeItem('co_local_universe');

            const me = await api.me();
            if (me) _renderUserBadge(me);

            const owned = await api.listUniverses();
            const mine = (owned || []).filter(u => !u.is_template);
            state.userUniverses = mine;

            let targetSlug;
            const userId = me?.user_id || '';
            const personal = mine.find(u => u.owner_id === userId);
            if (personal) {
                targetSlug = personal.key;
            } else {
                const displayName = r.display_name || r.usuario || me?.display_name || 'Minha comunidade';
                const slugVal = displayName.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 40) || 'minha-comunidade';

                let result = await apiFetch('/api/v1/universes', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ key: slugVal, name: displayName, description: '' }),
                }, true);

                if (!result) {
                    const fallbackSlug = slugVal + '-' + Math.random().toString(36).slice(2, 6);
                    result = await api.cloneUniverse('template', {
                        name: displayName,
                        key: fallbackSlug,
                        description: '',
                    });
                }

                if (result) {
                    targetSlug = result.key;
                    state.userUniverses.push(result);
                }
            }

            if (targetSlug) {
                _setUniverseSlugInUrl(targetSlug);
                state.currentUniverseSlug = targetSlug;
                state.isTemplate = false;
                _hideTemplateBanner();
                const refreshed = await api.listUniverses();
                state.userUniverses = (refreshed || []).filter(u => !u.is_template);
                await _bootAppForUniverse(targetSlug);
                return;
            }
            await _bootApp();
        } else if (r && r.error === 'unauthorized') {
            errEl.textContent = window.t('invalid_credentials');
            errEl.classList.remove('hidden');
            document.getElementById('login-senha').value = '';
            document.getElementById('login-senha').focus();
        } else {
            errEl.textContent = window.t('login_error');
            errEl.classList.remove('hidden');
        }
    }

    if (btnEntrar) btnEntrar.addEventListener('click', attemptLogin);

    ['login-usuario', 'login-senha'].forEach(id => {
        const el = document.getElementById(id);
        if (el) el.addEventListener('keydown', e => { if (e.key === 'Enter') attemptLogin(); });
    });

    if (btnLogout) {
        btnLogout.addEventListener('click', async () => {
            await api.logout();
            document.getElementById('sidebar-user').classList.add('hidden');
            document.getElementById('login-usuario').value = '';
            document.getElementById('login-senha').value = '';
            showLoginModal();
        });
    }

    // CO-165: Forgot password flow
    const btnForgot = document.getElementById('btn-forgot-password');
    const btnBackLogin = document.getElementById('btn-back-to-login');
    const btnForgotSend = document.getElementById('btn-forgot-send');
    const btnResetSubmit = document.getElementById('btn-reset-submit');

    function showLoginStep(step) {
        ['login-step-password', 'login-step-forgot', 'login-step-reset'].forEach(id => {
            const el = document.getElementById(id);
            if (el) el.classList.add('hidden');
        });
        const target = document.getElementById(step);
        if (target) target.classList.remove('hidden');
        if (btnBackLogin) btnBackLogin.style.display = (step === 'login-step-password') ? 'none' : '';
    }

    if (btnForgot) {
        btnForgot.addEventListener('click', () => {
            showLoginStep('login-step-forgot');
            const el = document.getElementById('forgot-identifier');
            if (el) el.focus();
        });
    }

    if (btnBackLogin) {
        btnBackLogin.addEventListener('click', () => {
            showLoginStep('login-step-password');
        });
    }

    let _forgotIdentifier = '';

    if (btnForgotSend) {
        btnForgotSend.addEventListener('click', async () => {
            const identifier = (document.getElementById('forgot-identifier')?.value || '').trim();
            const errEl = document.getElementById('forgot-error');
            errEl.classList.add('hidden');
            if (!identifier) return;
            btnForgotSend.disabled = true;
            btnForgotSend.textContent = window.t('forgot_password_sending');
            _forgotIdentifier = identifier;
            await fetch('/api/v1/auth/forgot-password', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ username_or_channel_value: identifier }),
            });
            btnForgotSend.disabled = false;
            btnForgotSend.textContent = window.t('forgot_password_send');
            showLoginStep('login-step-reset');
        });
    }

    if (btnResetSubmit) {
        btnResetSubmit.addEventListener('click', async () => {
            const code = (document.getElementById('reset-code')?.value || '').trim();
            const newPassword = document.getElementById('reset-new-password')?.value || '';
            const errEl = document.getElementById('reset-error');
            errEl.classList.add('hidden');
            if (!code || !newPassword) return;
            btnResetSubmit.disabled = true;

            const verifyResp = await fetch('/api/v1/auth/forgot-password/verify', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ username_or_channel_value: _forgotIdentifier, code }),
            });
            if (!verifyResp.ok) {
                errEl.textContent = window.t('forgot_password_error');
                errEl.classList.remove('hidden');
                btnResetSubmit.disabled = false;
                return;
            }
            const { reset_token } = await verifyResp.json();

            const resetResp = await fetch('/api/v1/auth/reset-password', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ reset_token, new_password: newPassword }),
            });
            btnResetSubmit.disabled = false;
            if (resetResp.ok) {
                hideLoginModal();
                showLoginStep('login-step-password');
                window.location.reload();
            } else {
                errEl.textContent = window.t('forgot_password_error');
                errEl.classList.remove('hidden');
            }
        });
    }

    const forgotIdentifierEl = document.getElementById('forgot-identifier');
    if (forgotIdentifierEl) forgotIdentifierEl.addEventListener('keydown', e => { if (e.key === 'Enter' && btnForgotSend) btnForgotSend.click(); });
    const resetCodeEl = document.getElementById('reset-code');
    if (resetCodeEl) resetCodeEl.addEventListener('keydown', e => { if (e.key === 'Enter' && btnResetSubmit) btnResetSubmit.click(); });
}

export function setupSecurityModal() {
    const overlay = document.getElementById('security-modal-overlay');
    const closeBtn = document.getElementById('security-modal-close');
    const btnSecurity = document.getElementById('btn-security');

    function openSecurityModal() {
        if (overlay) overlay.classList.remove('hidden');
        loadRecoveryChannels();
    }

    function closeSecurityModal() {
        if (overlay) overlay.classList.add('hidden');
    }

    if (btnSecurity) btnSecurity.addEventListener('click', openSecurityModal);
    if (closeBtn) closeBtn.addEventListener('click', closeSecurityModal);
    if (overlay) overlay.addEventListener('click', e => { if (e.target === overlay) closeSecurityModal(); });

    async function loadRecoveryChannels() {
        const listEl = document.getElementById('recovery-channels-list');
        if (!listEl) return;
        try {
            const resp = await apiFetch('/api/v1/auth/recovery/channels', {}, true);
            if (!resp || !Array.isArray(resp)) { listEl.textContent = ''; return; }
            if (resp.length === 0) {
                listEl.textContent = '';
                return;
            }
            listEl.innerHTML = resp.map(ch => `
                <div class="recovery-channel-row" style="display:flex;align-items:center;gap:0.5rem;margin-bottom:0.4rem">
                    <span class="badge" style="font-size:0.75rem">${ch.channel_type}</span>
                    <span>${ch.masked_value}</span>
                    <span style="font-size:0.75rem;opacity:0.6">${ch.verified_at ? window.t('recovery_verified') : window.t('recovery_pending')}</span>
                    <button class="btn-text-sm" data-channel-id="${ch.id}" onclick="window._removeChannel('${ch.id}')">${window.t('recovery_remove')}</button>
                </div>`).join('');
        } catch (_) {}
    }

    window._removeChannel = async (channelId) => {
        const pwd = prompt(window.t('change_password_current'));
        if (!pwd) return;
        await apiFetch(`/api/v1/auth/recovery/channels/${channelId}`, {
            method: 'DELETE',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ current_password: pwd }),
        }, true);
        loadRecoveryChannels();
    };

    let _pendingChannelId = null;

    const btnAdd = document.getElementById('btn-recovery-add');
    if (btnAdd) {
        btnAdd.addEventListener('click', async () => {
            const type = document.getElementById('recovery-channel-type')?.value;
            const value = document.getElementById('recovery-channel-value')?.value?.trim();
            if (!type || !value) return;
            const resp = await apiFetch('/api/v1/auth/recovery/channels', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ channel_type: type, value }),
            }, true);
            if (resp && resp.channel_id) {
                _pendingChannelId = resp.channel_id;
                const verifySection = document.getElementById('recovery-verify-section');
                if (verifySection) verifySection.classList.remove('hidden');
                const msg = document.getElementById('recovery-channel-msg');
                if (msg) msg.textContent = window.t('recovery_code_sent');
            }
        });
    }

    const btnVerify = document.getElementById('btn-recovery-verify');
    if (btnVerify) {
        btnVerify.addEventListener('click', async () => {
            const code = document.getElementById('recovery-verify-code')?.value?.trim();
            if (!code || !_pendingChannelId) return;
            const resp = await apiFetch('/api/v1/auth/recovery/channels/verify', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ channel_id: _pendingChannelId, code }),
            }, true);
            const msg = document.getElementById('recovery-channel-msg');
            if (resp && resp.verified) {
                if (msg) msg.textContent = window.t('recovery_channel_verified');
                _pendingChannelId = null;
                document.getElementById('recovery-verify-section')?.classList.add('hidden');
                document.getElementById('recovery-channel-value').value = '';
                loadRecoveryChannels();
            } else {
                if (msg) msg.textContent = window.t('forgot_password_error');
            }
        });
    }

    const btnChangePw = document.getElementById('btn-change-password');
    if (btnChangePw) {
        btnChangePw.addEventListener('click', async () => {
            const current = document.getElementById('security-current-password')?.value || '';
            const next = document.getElementById('security-new-password')?.value || '';
            const msgEl = document.getElementById('security-pw-msg');
            if (!current || !next) return;
            const resp = await apiFetch('/api/v1/auth/change-password', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ current_password: current, new_password: next }),
            }, true);
            if (msgEl) {
                msgEl.textContent = resp && resp.ok
                    ? window.t('change_password_success')
                    : window.t('change_password_error');
            }
            if (resp && resp.ok) {
                document.getElementById('security-current-password').value = '';
                document.getElementById('security-new-password').value = '';
            }
        });
    }
}
