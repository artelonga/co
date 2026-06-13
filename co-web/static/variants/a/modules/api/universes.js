// ===== Universe and project API methods =====
import { apiFetch, _u } from './client.js';
import { state } from '../state.js';

export async function getProjects() {
    // 2.13.2 hotfix: the legacy /api/projects?u=<slug> endpoint requires
    // auth and 401s for anonymous visitors, leaving the board "Carregando…"
    // forever. The v1 endpoint /api/v1/universes/<slug>/projects works
    // anonymously, so try it first and fall back to the legacy path only
    // if v1 doesn't return.
    const slug = state.currentUniverseSlug;
    if (slug) {
        try {
            const r = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/projects`, {}, true);
            if (Array.isArray(r)) return r;
        } catch (_) {}
    }
    const r = await apiFetch(_u('/api/projects'), {}, true);
    return r || [];
}

export async function getUniverses() {
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
}

export async function listUniverses() {
    const r = await apiFetch('/api/v1/universes', {}, true);
    return r || [];
}

export async function getUniverseInfo(slug) {
    return apiFetch(`/api/v1/universes/${slug}`, {}, true);
}

export async function getUniverseProjects(slug) {
    const r = await apiFetch(`/api/v1/universes/${slug}/projects`, {}, true);
    return r || [];
}

export async function cloneUniverse(sourceSlug, body) {
    return apiFetch(`/api/v1/universes/${sourceSlug}/clone`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
    });
}

// CO-96: owner-controlled duplication (copies all source entries into a new
// universe owned by the caller). Distinct from clone — works for any universe
// the caller can read, not just public/template.
export async function duplicateUniverse(sourceSlug, body) {
    return apiFetch(`/api/v1/universes/${encodeURIComponent(sourceSlug)}/duplicate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
    });
}

// CO-96: rename / change visibility / change description via PUT.
export async function updateUniverse(slug, patch) {
    return apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(patch),
    }, true);
}

// CO-96: soft-delete (recoverable via trash).
export async function deleteUniverse(slug) {
    return apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}`, { method: 'DELETE' }, true);
}

// CO-96: archive (soft-hide) a universe.
export async function archiveUniverse(slug) {
    return apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/archive`, { method: 'POST' }, true);
}

// CO-96: restore a trashed or archived universe.
export async function restoreUniverse(slug) {
    return apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/restore`, { method: 'POST' }, true);
}

// CO-96: list the caller's trashed + archived universes.
export async function getTrash() {
    const r = await apiFetch('/api/v1/universes/trash', {}, true);
    return r || [];
}

// CO-96: check whether a universe key is available (404 = free, 200 = taken).
// Returns true if available.
export async function isUniverseKeyAvailable(key) {
    try {
        const r = await fetch(`/api/v1/universes/${encodeURIComponent(key)}`, {
            headers: { 'Accept': 'application/json' },
        });
        return r.status === 404;
    } catch (_) {
        // Network hiccup — don't block submission on the check.
        return true;
    }
}

export async function claimUniverse(slug) {
    return apiFetch(`/api/v1/universes/${slug}/claim`, { method: 'POST' }, true);
}

export async function getUniverseConfig(slug) {
    return apiFetch(`/api/v1/universes/${slug}/config`, {}, true);
}

export async function updateUniverseConfig(slug, config) {
    return apiFetch(`/api/v1/universes/${slug}/config`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
    });
}

export async function getPublicacoes() {
    const r = await apiFetch('/api/v1/quilombo/publicacoes', {}, true);
    return r || [];
}

export async function getEventos() {
    const r = await apiFetch('/api/v1/quilombo/eventos', {}, true);
    return r || [];
}

export async function getMissoes() {
    const r = await apiFetch('/api/v1/quilombo/missoes', {}, true);
    return r || [];
}

export async function getOplog(slug, afterSeq, limit) {
    let url = `/api/v1/universes/${encodeURIComponent(slug)}/oplog`;
    const params = [];
    if (afterSeq != null) params.push(`after_seq=${afterSeq}`);
    if (limit != null) params.push(`limit=${limit}`);
    if (params.length) url += '?' + params.join('&');
    return apiFetch(url, {}, false);
}

export async function getOpDiff(slug, fromOp, toOp) {
    return apiFetch(
        `/api/v1/universes/${encodeURIComponent(slug)}/op-diff?from_op=${fromOp}&to_op=${toOp}`,
        {}, false,
    );
}

export async function revertToOp(slug, toOp) {
    return apiFetch(
        `/api/v1/universes/${encodeURIComponent(slug)}/revert?to=${toOp}`,
        { method: 'POST' }, false,
    );
}
