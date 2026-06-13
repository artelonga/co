// ===== CO-366: Post-register conversion CTA =====
//
// Shown right after a successful registration. A single call to action opens
// the billing checkout flow (Hostinger / manual invoice / future Pix+Stripe,
// chosen server-side behind the BillingProvider trait) so the BaaS
// `t_register → t_payment` conversion KPI exists end to end.
//
// Usage (from the post-signup success handler):
//   import { showPostRegisterModal } from './auth/post-register.js';
//   showPostRegisterModal({ plan: 'starter', priceLabel: 'R$9/mês' });

import { apiFetch } from '../api/client.js';

const OVERLAY_ID = 'post-register-modal-overlay';

/**
 * Start a billing checkout and redirect to the provider's URL.
 * @param {string} plan one of 'starter' | 'pro' | 'enterprise'
 * @returns {Promise<boolean>} true if a redirect was initiated
 */
export async function startCheckout(plan) {
    const resp = await apiFetch('/api/v1/me/billing/checkout', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ plan }),
    });
    if (resp && resp.url) {
        window.location.assign(resp.url);
        return true;
    }
    return false;
}

export function hidePostRegisterModal() {
    const overlay = document.getElementById(OVERLAY_ID);
    if (overlay) overlay.remove();
}

/**
 * Render the register-success modal with the activation CTA.
 * @param {{ plan?: string, priceLabel?: string }} opts
 */
export function showPostRegisterModal(opts = {}) {
    const plan = opts.plan || 'starter';
    const priceLabel = opts.priceLabel || 'R$9/mês';
    const isPt = (window.currentLang || 'pt') === 'pt';

    const title = isPt ? 'Conta criada! 🎉' : 'Account created! 🎉';
    const blurb = isPt
        ? 'Ative seu cérebro: domínio próprio, armazenamento ilimitado e sincronização prioritária.'
        : 'Activate your brain: own domain, unlimited storage, priority sync.';
    const ctaLabel = isPt
        ? `Ativar seu cérebro — ${priceLabel}`
        : `Activate your brain — ${priceLabel}`;
    const laterLabel = isPt ? 'Agora não' : 'Not now';

    // Tear down any previous instance so re-showing is idempotent.
    hidePostRegisterModal();

    const overlay = document.createElement('div');
    overlay.id = OVERLAY_ID;
    overlay.className = 'modal-overlay';
    // Built via DOM + textContent (not innerHTML): title/blurb/labels are i18n
    // strings, but textContent escapes them regardless — XSS-safe by construction
    // (CWE-79), and avoids the innerHTML-assignment scanner flag.
    const modal = document.createElement('div');
    modal.className = 'modal post-register-modal';
    modal.setAttribute('role', 'dialog');
    modal.setAttribute('aria-modal', 'true');
    const h2 = document.createElement('h2');
    h2.textContent = title;
    const p = document.createElement('p');
    p.textContent = blurb;
    const ctaBtn = document.createElement('button');
    ctaBtn.type = 'button';
    ctaBtn.id = 'post-register-cta';
    ctaBtn.className = 'btn btn-primary';
    ctaBtn.textContent = ctaLabel;
    const laterBtn = document.createElement('button');
    laterBtn.type = 'button';
    laterBtn.id = 'post-register-later';
    laterBtn.className = 'btn btn-link';
    laterBtn.textContent = laterLabel;
    modal.append(h2, p, ctaBtn, laterBtn);
    overlay.append(modal);
    document.body.appendChild(overlay);

    const cta = overlay.querySelector('#post-register-cta');
    cta.addEventListener('click', async () => {
        cta.disabled = true;
        const ok = await startCheckout(plan);
        if (!ok) cta.disabled = false; // apiFetch already surfaced the error
    });
    overlay
        .querySelector('#post-register-later')
        .addEventListener('click', hidePostRegisterModal);

    return overlay;
}
