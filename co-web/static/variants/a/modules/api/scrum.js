// ===== Scrum API methods (CO-368) =====
// Thin wrappers over the per-universe Scrum endpoints
// (`/api/v1/universes/{key}/scrum/*`). Every call is anonymous-safe: the
// manifest + reads are public, mirroring the calendar-lens config.
import { apiFetch } from './client.js';
import { state } from '../state.js';

function base() {
    const slug = state.currentUniverseSlug || 'template';
    return `/api/v1/universes/${encodeURIComponent(slug)}/scrum`;
}

// Returns the parsed `_scrum.yaml` plus the computed `current_sprint`
// (or `enabled: false` for a universe without a manifest).
export async function getScrumManifest() {
    try {
        return await apiFetch(`${base()}/manifest`, {}, true);
    } catch (_) {
        return { enabled: false, current_sprint: null };
    }
}

export async function getScrumBacklog(status) {
    const q = status ? `?status=${encodeURIComponent(status)}` : '';
    const r = await apiFetch(`${base()}/backlog${q}`, {}, true);
    return (r && r.pbis) || [];
}

export async function getCurrentSprint() {
    try {
        return await apiFetch(`${base()}/sprints/current`, {}, true);
    } catch (_) {
        return null;
    }
}

export async function listSprints() {
    const r = await apiFetch(`${base()}/sprints`, {}, true);
    return (r && r.sprints) || [];
}

export async function createPbi(pbi) {
    return apiFetch(`${base()}/pbi`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(pbi),
    });
}

// Toggle one Definition-of-Done checkbox on a PBI by index.
export async function checkPbiDod(pbiId, index, done) {
    return apiFetch(`${base()}/pbi/${encodeURIComponent(pbiId)}/dod`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ index, done }),
    });
}
