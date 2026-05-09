// ===== API layer =====
// apiFetch + api object. Calls showLoginModal / showUsageLimitModal / showToast
// via callbacks injected at boot time to avoid circular imports.
import { state } from './state.js';

// Callbacks injected by app.js at startup to break circular deps.
let _showLoginModal = () => {};
let _showUsageLimitModal = () => {};
let _showToast = () => {};

export function injectApiCallbacks(showLoginModal, showUsageLimitModal, showToast) {
    _showLoginModal = showLoginModal;
    _showUsageLimitModal = showUsageLimitModal;
    _showToast = showToast;
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

export const api = {
    _u(url) {
        const slug = state.currentUniverseSlug;
        if (!slug) return url;
        return url + (url.includes('?') ? '&' : '?') + `u=${slug}`;
    },
    async getProjects() {
        const r = await apiFetch(this._u('/api/projects'), {}, true);
        return r || [];
    },
    async getTasks(key, opts) {
        let url = `/api/projects/${key}/tasks`;
        if (opts && typeof opts.archived === 'boolean') {
            url += '?archived=' + (opts.archived ? 'true' : 'false');
        }
        const r = await apiFetch(this._u(url), {}, true);
        return r || [];
    },
    async createTask(key, data) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks`), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        return r;
    },
    async updateTask(key, id, data) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}`), {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        return r;
    },
    async deleteTask(key, id) {
        await apiFetch(this._u(`/api/projects/${key}/tasks/${id}`), { method: 'DELETE' });
    },
    async getComments(key, id) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}/comments`), {}, true);
        return r || [];
    },
    async createComment(key, id, data) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}/comments`), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        return r;
    },
    async getActivity(key, limit) {
        const l = limit || 50;
        const r = await apiFetch(this._u(`/api/projects/${key}/activity?limit=${l}`), {}, true);
        return r || [];
    },
    async getDashboard(key) {
        const r = await apiFetch(this._u(`/api/projects/${key}/dashboard`), {}, true);
        return r;
    },
    async bulkUpdateTasks(key, data) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks/bulk-update`), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        return r;
    },
    async bulkDeleteTasks(key, data) {
        const r = await apiFetch(this._u(`/api/projects/${key}/tasks/bulk-delete`), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data),
        });
        return r;
    },
    async me() {
        return apiFetch('/api/v1/auth/me', {}, true);
    },
    async logout() {
        await apiFetch('/api/v1/auth/logout', { method: 'POST' }, true);
    },
    async loginWithPassword(usuario, senha) {
        if (usuario.includes('@')) {
            const resp = await apiFetch('/api/v1/auth/password-login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email: usuario, password: senha }),
            }, true);
            if (resp && resp.user_id) {
                return { usuario: resp.display_name || resp.email, ...resp };
            }
        }
        return apiFetch('/api/v1/quilombo/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ usuario, senha }),
        }, true);
    },
    async getUniverses() {
        try {
            const r = await apiFetch('/api/v1/universes', {}, true);
            return r || [];
        } catch (err) {
            if (err && (err.status === 401 || /401|unauthorized/i.test(String(err.message || '')))) {
                try {
                    const r = await apiFetch('/api/v1/universes/public', {}, true);
                    return r || [];
                } catch (_) {
                    return [];
                }
            }
            return [];
        }
    },
    async getPublicacoes() {
        const r = await apiFetch('/api/v1/quilombo/publicacoes', {}, true);
        return r || [];
    },
    async getEventos() {
        const r = await apiFetch('/api/v1/quilombo/eventos', {}, true);
        return r || [];
    },
    async getMissoes() {
        const r = await apiFetch('/api/v1/quilombo/missoes', {}, true);
        return r || [];
    },
    async getUniverseProjects(slug) {
        const r = await apiFetch(`/api/v1/universes/${slug}/projects`, {}, true);
        return r || [];
    },
    async cloneUniverse(sourceSlug, body) {
        return apiFetch(`/api/v1/universes/${sourceSlug}/clone`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        });
    },
    async getUniverseInfo(slug) {
        return apiFetch(`/api/v1/universes/${slug}`, {}, true);
    },
    async listUniverses() {
        const r = await apiFetch('/api/v1/universes', {}, true);
        return r || [];
    },
    async claimUniverse(slug) {
        return apiFetch(`/api/v1/universes/${slug}/claim`, { method: 'POST' }, true);
    },
    async getUniverseConfig(slug) {
        return apiFetch(`/api/v1/universes/${slug}/config`, {}, true);
    },
    async updateUniverseConfig(slug, config) {
        return apiFetch(`/api/v1/universes/${slug}/config`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(config),
        });
    },
    async getUniverseEntries(slug, type) {
        let url = type
            ? `/api/v1/universes/${slug}/entries?type=${encodeURIComponent(type)}`
            : `/api/v1/universes/${slug}/entries`;
        const pin = state.subscriptionPin && state.subscriptionPin[slug];
        if (pin) {
            url += (url.includes('?') ? '&' : '?') + 'as_of=' + encodeURIComponent(pin);
        }
        const r = await apiFetch(url, {}, true);
        return (r && r.entries) || [];
    },
    async getUniverseManifest(slug) {
        return apiFetch(`/api/v1/universes/${slug}/manifest`, {}, true);
    },
    async getEntriesByDate(slug, semantic, from, to) {
        let url = `/api/v1/universes/${slug}/entries?date_semantic=${encodeURIComponent(semantic)}`;
        if (from) url += `&from=${encodeURIComponent(from)}`;
        if (to)   url += `&to=${encodeURIComponent(to)}`;
        const r = await apiFetch(url, {}, true);
        return (r && r.entries) || [];
    },
};
