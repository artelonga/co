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
