// ===== Task, comment, activity and dashboard API methods =====
import { apiFetch, _u } from './client.js';

export async function getTasks(key, opts) {
    let url = `/api/projects/${key}/tasks`;
    if (opts && typeof opts.archived === 'boolean') {
        url += '?archived=' + (opts.archived ? 'true' : 'false');
    }
    const r = await apiFetch(_u(url), {}, true);
    return r || [];
}

export async function createTask(key, data) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks`), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    return r;
}

export async function updateTask(key, id, data) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks/${id}`), {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    return r;
}

export async function deleteTask(key, id) {
    await apiFetch(_u(`/api/projects/${key}/tasks/${id}`), { method: 'DELETE' });
}

export async function getComments(key, id) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks/${id}/comments`), {}, true);
    return r || [];
}

export async function createComment(key, id, data) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks/${id}/comments`), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    return r;
}

export async function getActivity(key, limit) {
    const l = limit || 50;
    const r = await apiFetch(_u(`/api/projects/${key}/activity?limit=${l}`), {}, true);
    return r || [];
}

export async function getDashboard(key) {
    const r = await apiFetch(_u(`/api/projects/${key}/dashboard`), {}, true);
    return r;
}

export async function bulkUpdateTasks(key, data) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks/bulk-update`), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    return r;
}

export async function bulkDeleteTasks(key, data) {
    const r = await apiFetch(_u(`/api/projects/${key}/tasks/bulk-delete`), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    return r;
}
