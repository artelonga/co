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
    overlay.innerHTML = `
        <div class="modal post-register-modal" role="dialog" aria-modal="true">
            <h2>${title}</h2>
            <p>${blurb}</p>
            <button type="button" id="post-register-cta" class="btn btn-primary">${ctaLabel}</button>
            <button type="button" id="post-register-later" class="btn btn-link">${laterLabel}</button>
        </div>
    `;
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
