// ===== Entry, manifest and oplog API methods =====
import { apiFetch } from './client.js';
import { state } from '../state.js';

export async function getUniverseEntries(slug, type) {
    let url = type
        ? `/api/v1/universes/${slug}/entries?type=${encodeURIComponent(type)}`
        : `/api/v1/universes/${slug}/entries`;
    const pin = state.subscriptionPin && state.subscriptionPin[slug];
    if (pin) {
        url += (url.includes('?') ? '&' : '?') + 'as_of=' + encodeURIComponent(pin);
    }
    const r = await apiFetch(url, {}, true);
    return (r && r.entries) || [];
}

export async function getEntriesByDate(slug, semantic, from, to) {
    let url = `/api/v1/universes/${slug}/entries?date_semantic=${encodeURIComponent(semantic)}`;
    if (from) url += `&from=${encodeURIComponent(from)}`;
    if (to)   url += `&to=${encodeURIComponent(to)}`;
    const r = await apiFetch(url, {}, true);
    return (r && r.entries) || [];
}

export async function getUniverseManifest(slug) {
    return apiFetch(`/api/v1/universes/${slug}/manifest`, {}, true);
}

// CO-264: list entries whose path starts with `prefix` (e.g. `public/`).
export async function getEntriesByPathPrefix(slug, prefix) {
    const url = `/api/v1/universes/${encodeURIComponent(slug)}/entries?path_prefix=${encodeURIComponent(prefix)}`;
    const r = await apiFetch(url, {}, true);
    return (r && r.entries) || [];
}

// CO-272: fetch entries-as-tasks from work/ for the kanban dev-board view.
export async function getDevTasks(slug, opts) {
    let url = `/api/v1/universes/${encodeURIComponent(slug)}/dev-tasks`;
    const params = [];
    if (opts?.status) params.push(`status=${encodeURIComponent(opts.status)}`);
    if (opts?.limit) params.push(`limit=${opts.limit}`);
    if (params.length) url += '?' + params.join('&');
    const r = await apiFetch(url, {}, true);
    return (r && r.tasks) || [];
}
