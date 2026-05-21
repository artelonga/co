// ===== API client — apiFetch + URL builder =====
import { state } from '../state.js';

// Callbacks injected by app.js at startup to break circular deps.
let _showLoginModal = () => {};
let _showUsageLimitModal = () => {};
let _showToast = () => {};

export function injectApiCallbacks(showLoginModal, showUsageLimitModal, showToast) {
    _showLoginModal = showLoginModal;
    _showUsageLimitModal = showUsageLimitModal;
    _showToast = showToast;
}

export function _u(url) {
    const slug = state.currentUniverseSlug;
    if (!slug) return url;
    return url + (url.includes('?') ? '&' : '?') + `u=${slug}`;
}

export async function apiFetch(url, options, silent401 = false) {
    try {
        const r = await fetch(url, options);
        if (!r.ok) {
            if (r.status === 401) {
                if (!silent401) _showLoginModal();
                return null;
            }
            let errData = null;
            try { errData = await r.json(); } catch (_) {}
            if (r.status === 402 && errData && errData.error === 'usage_limit') {
                _showUsageLimitModal(errData);
                return null;
            }
            const errMsg = (errData && (errData.message || errData.error)) || 'Request error';
            _showToast(errMsg, 'error');
            return null;
        }
        if (r.status === 204 || r.headers.get('content-length') === '0') {
            return {};
        }
        return r.json();
    } catch (err) {
        _showToast('Connection error: ' + err.message, 'error');
        return null;
    }
}
