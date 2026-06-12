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
let _loadMeUniverses = async () => {};

// CO-303: last dev_code returned by onboard-with-email (non-prod only).
let _lastDevCode = null;

export function injectLoginCallbacks(callbacks) {
    _render = callbacks.render;
    _bootAppForUniverse = callbacks.bootAppForUniverse;
    _bootApp = callbacks.bootApp;
    _renderUserBadge = callbacks.renderUserBadge;
    _setUniverseSlugInUrl = callbacks.setUniverseSlugInUrl;
    _hideTemplateBanner = callbacks.hideTemplateBanner;
    _showToast = callbacks.showToast;
    if (callbacks.loadMeUniverses) _loadMeUniverses = callbacks.loadMeUniverses;
}

export function showLoginModal() {
    const overlay = document.getElementById('login-modal-overlay');
    if (overlay) {
        overlay.classList.remove('hidden');
        window.setLang(window.currentLang);
        const emailInput = document.getElementById('onboard-email');
        if (emailInput) emailInput.focus();
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

            await _loadMeUniverses();
            const mine = state.userUniverses.filter(u => !u.is_template);

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
                await _loadMeUniverses();
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

    // CO-190: Passwordless onboarding via email
    const btnOnboardContinue = document.getElementById('btn-onboard-continue');
    const btnOnboardVerify = document.getElementById('btn-onboard-verify');
    const btnOnboardResend = document.getElementById('btn-onboard-resend');
    const btnOnboardEditEmail = document.getElementById('btn-onboard-edit-email');
    const btnShowClassicLogin = document.getElementById('btn-show-classic-login');
    const btnBackToEmail = document.getElementById('btn-back-to-email');

    let _onboardEmail = '';
    let _resendCooldownTimer = null;

    function startResendCooldown() {
        if (!btnOnboardResend) return;
        btnOnboardResend.disabled = true;
        let secs = 60;
        btnOnboardResend.textContent = `${window.t('onboard_resend')} (${secs}s)`;
        _resendCooldownTimer = setInterval(() => {
            secs -= 1;
            if (secs <= 0) {
                clearInterval(_resendCooldownTimer);
                btnOnboardResend.disabled = false;
                btnOnboardResend.textContent = window.t('onboard_resend');
            } else {
                btnOnboardResend.textContent = `${window.t('onboard_resend')} (${secs}s)`;
            }
        }, 1000);
    }

    async function sendOnboardCode(email) {
        const errEl = document.getElementById('onboard-email-error');
        if (errEl) errEl.classList.add('hidden');
        if (!email || !email.includes('@')) {
            if (errEl) { errEl.textContent = window.t('onboard_invalid_email'); errEl.classList.remove('hidden'); }
            return false;
        }
        if (btnOnboardContinue) { btnOnboardContinue.disabled = true; btnOnboardContinue.textContent = window.t('onboard_sending'); }
        try {
            const resp = await fetch('/api/v1/auth/onboard-with-email', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email }),
            });
            if (!resp.ok) {
                let msg = window.t('onboard_error');
                try { const j = await resp.json(); msg = j.message || msg; } catch (_) {}
                if (errEl) { errEl.textContent = msg; errEl.classList.remove('hidden'); }
                return false;
            }
            // CO-303: capture dev_code if server is in non-prod mode.
            _lastDevCode = null;
            try {
                const data = await resp.json();
                if (data && data.dev_code) _lastDevCode = data.dev_code;
            } catch (_) {}
            return true;
        } catch (_) {
            if (errEl) { errEl.textContent = window.t('onboard_error'); errEl.classList.remove('hidden'); }
            return false;
        } finally {
            if (btnOnboardContinue) { btnOnboardContinue.disabled = false; btnOnboardContinue.textContent = window.t('onboard_continue'); }
        }
    }

    // CO-303: show or hide the dev-code banner and auto-fill the code input.
    function applyDevCode() {
        const banner = document.getElementById('dev-code-banner');
        const codeInput = document.getElementById('onboard-code');
        const codeValue = document.getElementById('dev-code-value');
        if (_lastDevCode && banner && codeInput && codeValue) {
            codeValue.textContent = _lastDevCode;
            banner.classList.remove('hidden');
            codeInput.value = _lastDevCode;
        } else if (banner) {
            banner.classList.add('hidden');
        }
    }

    if (btnOnboardContinue) {
        btnOnboardContinue.addEventListener('click', async () => {
            const email = (document.getElementById('onboard-email')?.value || '').trim().toLowerCase();
            const ok = await sendOnboardCode(email);
            if (ok) {
                _onboardEmail = email;
                const sentToEl = document.getElementById('onboard-sent-to');
                if (sentToEl) sentToEl.textContent = window.t('onboard_sent_to').replace('{email}', email);
                showLoginStep('login-step-code');
                // CO-303: show inline code in non-prod envs.
                applyDevCode();
                document.getElementById('onboard-code')?.focus();
                startResendCooldown();
            }
        });
    }

    document.getElementById('onboard-email')?.addEventListener('keydown', e => {
        if (e.key === 'Enter' && btnOnboardContinue) btnOnboardContinue.click();
    });

    if (btnOnboardVerify) {
        btnOnboardVerify.addEventListener('click', async () => {
            const code = (document.getElementById('onboard-code')?.value || '').trim();
            const errEl = document.getElementById('onboard-code-error');
            if (errEl) errEl.classList.add('hidden');
            if (!code) return;
            btnOnboardVerify.disabled = true;
            btnOnboardVerify.textContent = window.t('onboard_verifying');
            try {
                const resp = await fetch('/api/v1/auth/onboard-with-email/verify', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ email: _onboardEmail, code }),
                });
                if (resp.ok) {
                    const data = await resp.json();
                    hideLoginModal();
                    if (data.return_to) {
                        window.location.href = data.return_to;
                    } else {
                        window.location.reload();
                    }
                } else {
                    let msg = window.t('onboard_code_error');
                    try { const j = await resp.json(); msg = j.message || msg; } catch (_) {}
                    if (resp.status === 410) {
                        msg = window.t('onboard_code_locked');
                    }
                    if (errEl) { errEl.textContent = msg; errEl.classList.remove('hidden'); }
                }
            } catch (_) {
                if (errEl) { errEl.textContent = window.t('onboard_code_error'); errEl.classList.remove('hidden'); }
            } finally {
                btnOnboardVerify.disabled = false;
                btnOnboardVerify.textContent = window.t('onboard_verify');
            }
        });
    }

    document.getElementById('onboard-code')?.addEventListener('keydown', e => {
        if (e.key === 'Enter' && btnOnboardVerify) btnOnboardVerify.click();
    });

    if (btnOnboardResend) {
        btnOnboardResend.addEventListener('click', async () => {
            const ok = await sendOnboardCode(_onboardEmail);
            if (ok) {
                // CO-303: update banner with the newly generated code.
                applyDevCode();
                startResendCooldown();
            }
        });
    }

    if (btnOnboardEditEmail) {
        btnOnboardEditEmail.addEventListener('click', () => {
            showLoginStep('login-step-email');
            document.getElementById('onboard-email')?.focus();
        });
    }

    if (btnShowClassicLogin) {
        btnShowClassicLogin.addEventListener('click', () => {
            showLoginStep('login-step-password');
            document.getElementById('login-usuario')?.focus();
        });
    }

    if (btnBackToEmail) {
        btnBackToEmail.addEventListener('click', () => {
            showLoginStep('login-step-email');
            document.getElementById('onboard-email')?.focus();
        });
    }

    // CO-177: probe whether Google OAuth is configured on this deploy.
    // If yes, reveal both the login + signup OAuth blocks. Stays hidden
    // (default) when GOOGLE_CLIENT_ID isn't set so users don't click a
    // button that 503s. Also forwards `?return_to=` from the current URL
    // so cross-deployment bounces (quilombo SvelteKit, future ArteLonga)
    // round-trip back to the originating site after auth.
    (async () => {
        try {
            const r = await fetch('/api/v1/auth/google/status');
            if (!r.ok) return;
            const { configured } = await r.json();
            if (!configured) return;
            const here = new URLSearchParams(window.location.search);
            const returnTo = here.get('return_to');
            const startHref = returnTo
                ? `/api/v1/auth/google/start?return_to=${encodeURIComponent(returnTo)}`
                : '/api/v1/auth/google/start';
            document.querySelectorAll('.oauth-btn-google').forEach(a => { a.href = startHref; });
            document.getElementById('oauth-providers')?.classList.remove('hidden');
            document.getElementById('oauth-providers-signup')?.classList.remove('hidden');
        } catch (_) { /* leave hidden on any error */ }
    })();

    // CO-415: probe whether GitHub OAuth is configured on this deploy. Same
    // gating posture as Google above — the GitHub buttons stay hidden when
    // GITHUB_OAUTH_CLIENT_ID isn't set so users never hit a 503. Forwards
    // `?return_to=` for cross-deployment round-trips.
    (async () => {
        try {
            const r = await fetch('/api/v1/auth/github/status');
            if (!r.ok) return;
            const { configured } = await r.json();
            if (!configured) return;
            const here = new URLSearchParams(window.location.search);
            const returnTo = here.get('return_to');
            const startHref = returnTo
                ? `/api/v1/auth/github/start?return_to=${encodeURIComponent(returnTo)}`
                : '/api/v1/auth/github/start';
            document.querySelectorAll('.oauth-btn-github').forEach(a => {
                a.href = startHref;
                a.classList.remove('hidden');
            });
            document.getElementById('oauth-providers')?.classList.remove('hidden');
            document.getElementById('oauth-providers-signup')?.classList.remove('hidden');
        } catch (_) { /* leave hidden on any error */ }
    })();

    // CO-303: probe login-options and reveal the admin sign-in tab in non-prod envs.
    (async () => {
        try {
            const r = await fetch('/api/v1/auth/login-options');
            if (!r.ok) return;
            const opts = await r.json();
            if (opts.password) {
                document.getElementById('admin-signin-row')?.classList.remove('hidden');
            }
        } catch (_) { /* leave hidden on any error */ }
    })();

    const btnAdminSignin = document.getElementById('btn-admin-signin');
    if (btnAdminSignin) {
        btnAdminSignin.addEventListener('click', () => {
            showLoginStep('login-step-password');
            document.getElementById('login-usuario')?.focus();
        });
    }

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
        ['login-step-email', 'login-step-code', 'login-step-password', 'login-step-signup', 'login-step-forgot', 'login-step-reset'].forEach(id => {
            const el = document.getElementById(id);
            if (el) el.classList.add('hidden');
        });
        const target = document.getElementById(step);
        if (target) target.classList.remove('hidden');
        if (btnBackLogin) btnBackLogin.style.display = (step === 'login-step-email' || step === 'login-step-password') ? 'none' : '';
    }

    if (btnForgot) {
        btnForgot.addEventListener('click', () => {
            showLoginStep('login-step-forgot');
            const el = document.getElementById('forgot-identifier');
            if (el) el.focus();
        });
    }

    // CO-175 (G3): public signup wiring.
    const btnShowSignup = document.getElementById('btn-show-signup');
    const btnBackFromSignup = document.getElementById('btn-back-to-login-from-signup');
    const btnSignupSubmit = document.getElementById('btn-signup-submit');
    if (btnShowSignup) {
        btnShowSignup.addEventListener('click', () => {
            showLoginStep('login-step-signup');
            document.getElementById('signup-usuario')?.focus();
        });
    }

    // Default: show the email-first step on open
    showLoginStep('login-step-email');
    if (btnBackFromSignup) {
        btnBackFromSignup.addEventListener('click', () => {
            showLoginStep('login-step-password');
        });
    }
    if (btnSignupSubmit) {
        btnSignupSubmit.addEventListener('click', async () => {
            const usuario = (document.getElementById('signup-usuario')?.value || '').trim();
            const senha = document.getElementById('signup-senha')?.value || '';
            const email = (document.getElementById('signup-email')?.value || '').trim();
            const errEl = document.getElementById('signup-error');
            errEl.classList.add('hidden');

            // Mirror backend validation client-side for fast feedback.
            if (usuario.length < 3 || usuario.length > 30) {
                errEl.textContent = window.t('signup_error_usuario_len');
                errEl.classList.remove('hidden');
                return;
            }
            if (senha.length < 8) {
                errEl.textContent = window.t('signup_error_senha_len');
                errEl.classList.remove('hidden');
                return;
            }

            btnSignupSubmit.disabled = true;
            btnSignupSubmit.textContent = window.t('signup_submitting');
            try {
                const resp = await fetch('/api/v1/auth/signup', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ usuario, password: senha, email: email || undefined }),
                });
                if (!resp.ok) {
                    let serverMsg = '';
                    try {
                        const j = await resp.json();
                        serverMsg = j.message || j.error || '';
                    } catch (_) {}
                    errEl.textContent = serverMsg || window.t('signup_error_generic');
                    errEl.classList.remove('hidden');
                    return;
                }
                // Server set the session cookie; reload triggers `init()` to
                // read /me and route the freshly-logged-in user to their hub.
                window.location.reload();
            } catch (e) {
                errEl.textContent = window.t('signup_error_generic');
                errEl.classList.remove('hidden');
            } finally {
                btnSignupSubmit.disabled = false;
                btnSignupSubmit.textContent = window.t('signup_submit');
            }
        });
    }

    if (btnBackLogin) {
        btnBackLogin.addEventListener('click', () => {
            showLoginStep('login-step-password');
        });
    }

    let _forgotIdentifier = '';
    let _forgotEmail = '';

    if (btnForgotSend) {
        btnForgotSend.addEventListener('click', async () => {
            const usuario = (document.getElementById('forgot-identifier')?.value || '').trim();
            const email = (document.getElementById('forgot-email')?.value || '').trim();
            const errEl = document.getElementById('forgot-error');
            errEl.classList.add('hidden');
            // CO-176: both fields are required and must match the same account.
            if (!usuario) {
                errEl.textContent = window.t('forgot_password_username_required');
                errEl.classList.remove('hidden');
                return;
            }
            if (!email || !email.includes('@')) {
                errEl.textContent = window.t('forgot_password_email_required');
                errEl.classList.remove('hidden');
                return;
            }
            btnForgotSend.disabled = true;
            btnForgotSend.textContent = window.t('forgot_password_sending');
            // Track both for the verify step (server checks the pair again).
            _forgotIdentifier = usuario;
            _forgotEmail = email;
            await fetch('/api/v1/auth/forgot-password', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    username_or_channel_value: usuario,
                    email,
                }),
            });
            btnForgotSend.disabled = false;
            btnForgotSend.textContent = window.t('forgot_password_send');
            showLoginStep('login-step-reset');
        });
    }

    // CO-172: validate a return_to URL against the artelonga safelist.
    // CO-282: also allow localhost / 127.0.0.1 for `co serve` local distribution.
    function isAllowedReturnTo(url) {
        try {
            const { hostname } = new URL(url);
            return hostname === 'localhost'
                || hostname === '127.0.0.1'
                || hostname === 'quilomboaraucaria.com.br'
                || hostname === 'artelonga.com.br'
                || hostname.endsWith('.artelonga.com.br');
        } catch (_) { return false; }
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
                body: JSON.stringify({
                    username_or_channel_value: _forgotIdentifier,
                    email: _forgotEmail,
                    code,
                }),
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
                // CO-172: redirect back to origin if /recover was loaded with a safelisted return_to
                // CO-185: when return_to ends in /auth/co-handover, append the short-lived
                // co_token from the response so the receiving deployment can mint its own
                // session cookie cross-apex (cookie set here only valid on co.artelonga.com.br).
                const urlParams = new URLSearchParams(window.location.search);
                const returnTo = urlParams.get('return_to');
                if (returnTo && isAllowedReturnTo(returnTo)) {
                    let final_url = returnTo;
                    try {
                        const body = await resetResp.clone().json();
                        if (body && body.co_token && returnTo.includes('/auth/co-handover')) {
                            const sep = returnTo.includes('?') ? '&' : '?';
                            final_url = `${returnTo}${sep}co_token=${encodeURIComponent(body.co_token)}`;
                        }
                    } catch (_) { /* fall through with bare returnTo */ }
                    window.location.href = final_url;
                    return;
                }
                hideLoginModal();
                showLoginStep('login-step-password');
                window.location.reload();
            } else {
                errEl.textContent = window.t('forgot_password_error');
                errEl.classList.remove('hidden');
            }
        });
    }

    // CO-172: when loaded at /recover, pre-fill identifier and show forgot-password step.
    if (window.location.pathname === '/recover') {
        const urlParams = new URLSearchParams(window.location.search);
        const prefilledId = urlParams.get('identifier');
        if (prefilledId) {
            const el = document.getElementById('forgot-identifier');
            if (el) el.value = prefilledId;
        }

        // CO-176: title + subtitle for the /recover page. Drop the marketing
        // copy — just say what's happening.
        const titleEl = document.getElementById('login-modal-title');
        const subtitleEl = document.querySelector('#login-modal-overlay .login-subtitle');
        if (titleEl) titleEl.textContent = window.t('recover_title') || 'Recuperar senha';
        if (subtitleEl) {
            subtitleEl.textContent = window.t('recover_subtitle') || 'Recupere o acesso à sua conta.';
        }
        // The forgot-step subtitle ("Digite seu usuário ou email") becomes
        // redundant when the modal title already says "Recuperar senha".
        const forgotSubtitle = document.querySelector('#login-step-forgot .login-subtitle');
        if (forgotSubtitle) forgotSubtitle.style.display = 'none';

        showLoginStep('login-step-forgot');
        showLoginModal();
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

    /** Channel-type → display affordances (icon + verbose-type label). */
    function channelDisplay(channelType) {
        switch (channelType) {
            case 'email':    return { icon: '✉', label: 'email' };
            case 'whatsapp': return { icon: '💬', label: 'WhatsApp' };
            case 'sms':      return { icon: '📱', label: 'SMS' };
            default:         return { icon: '•', label: channelType };
        }
    }

    async function loadRecoveryChannels() {
        const listEl = document.getElementById('recovery-channels-list');
        if (!listEl) return;
        try {
            const resp = await apiFetch('/api/v1/auth/recovery/channels', {}, true);
            if (!resp || !Array.isArray(resp) || resp.length === 0) {
                listEl.textContent = '';
                return;
            }
            listEl.innerHTML = resp.map(ch => {
                const d = channelDisplay(ch.channel_type);
                const verified = !!ch.verified_at;
                const statusKey = verified ? 'recovery_verified' : 'recovery_pending';
                return `
                <div class="recovery-channel-row${verified ? '' : ' unverified'}">
                    <div class="recovery-channel-meta">
                        <span class="recovery-channel-icon" aria-hidden="true">${d.icon}</span>
                        <span class="recovery-channel-value">${ch.masked_value}</span>
                    </div>
                    <span class="recovery-channel-status">${window.t(statusKey)}</span>
                    <button class="btn-text-sm" type="button" data-channel-id="${ch.id}" onclick="window._removeChannel('${ch.id}')">${window.t('recovery_remove')}</button>
                </div>`;
            }).join('');
        } catch (_) {}
    }

    /** Update the value-input affordances when channel type changes. */
    function syncChannelInputForType() {
        const typeEl = document.getElementById('recovery-channel-type');
        const valEl = document.getElementById('recovery-channel-value');
        const hintEl = document.getElementById('recovery-channel-value-hint');
        if (!typeEl || !valEl) return;
        const t = typeEl.value;
        if (t === 'email') {
            valEl.type = 'email';
            valEl.inputMode = 'email';
            valEl.autocomplete = 'email';
            valEl.placeholder = 'email@exemplo.com';
            if (hintEl) hintEl.textContent = window.t('recovery_channel_email_hint');
        } else {
            // whatsapp / sms — international phone number
            valEl.type = 'tel';
            valEl.inputMode = 'tel';
            valEl.autocomplete = 'tel';
            valEl.placeholder = '+55 41 99999-9999';
            if (hintEl) {
                hintEl.textContent = t === 'whatsapp'
                    ? window.t('recovery_channel_whatsapp_hint')
                    : window.t('recovery_channel_sms_hint');
            }
        }
    }
    document.getElementById('recovery-channel-type')?.addEventListener('change', syncChannelInputForType);
    syncChannelInputForType();

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
            const emailInput = document.getElementById('security-attach-email');
            const email = emailInput ? (emailInput.value || '').trim() : '';
            const msgEl = document.getElementById('security-pw-msg');
            if (!current || !next) return;
            const body = { current_password: current, new_password: next };
            if (email) body.email = email;
            const resp = await apiFetch('/api/v1/auth/change-password', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            }, true);
            if (msgEl) {
                if (resp && resp.ok) {
                    msgEl.textContent = email
                        ? window.t('change_password_success_with_email')
                        : window.t('change_password_success');
                } else if (resp && resp.error === 'conflict') {
                    msgEl.textContent = window.t('change_password_email_conflict');
                } else {
                    msgEl.textContent = window.t('change_password_error');
                }
            }
            if (resp && resp.ok) {
                document.getElementById('security-current-password').value = '';
                document.getElementById('security-new-password').value = '';
                if (emailInput) emailInput.value = '';
            }
        });
    }

    // CO-198: DM policy save
    const btnSaveDmPolicy = document.getElementById('btn-save-dm-policy');
    if (btnSaveDmPolicy) {
        // Load current policy when modal opens
        function loadDmPolicy() {
            apiFetch('/api/v1/auth/me', {}, true).then(me => {
                if (!me || !me.dm_policy) return;
                const radio = document.querySelector(`input[name="dm_policy"][value="${me.dm_policy}"]`);
                if (radio) radio.checked = true;
            }).catch(() => {});
        }

        const origOpen = document.getElementById('btn-security')?.onclick;
        const btnSec = document.getElementById('btn-security');
        if (btnSec) {
            const _oldClick = btnSec.onclick;
            btnSec.addEventListener('click', loadDmPolicy);
        }

        btnSaveDmPolicy.addEventListener('click', async () => {
            const selected = document.querySelector('input[name="dm_policy"]:checked');
            if (!selected) return;
            const msgEl = document.getElementById('dm-policy-msg');
            const resp = await fetch('/api/v1/me/dm-policy', {
                method: 'PUT',
                credentials: 'include',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ policy: selected.value }),
            }).catch(() => null);
            if (msgEl) {
                msgEl.textContent = resp && resp.ok
                    ? (window.t ? window.t('saved') : 'Salvo!')
                    : (window.t ? window.t('chat.send_error') : 'Erro');
                setTimeout(() => { if (msgEl) msgEl.textContent = ''; }, 2000);
            }
        });
    }

    // CO-198: Block list
    function loadBlockedList() {
        const listEl = document.getElementById('dm-blocked-list');
        if (!listEl) return;
        apiFetch('/api/v1/me/dms', {}, true).then(() => {
            // Block list is not directly available via me/dms; show placeholder
            // In a full implementation we'd have a GET /api/v1/me/blocks endpoint
            listEl.innerHTML = `<p class="form-hint" data-i18n="dm.blocked.empty">${window.t ? window.t('dm.blocked.empty') : 'Você não bloqueou ninguém.'}</p>`;
        }).catch(() => {});
    }
}
