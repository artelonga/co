// ===== Auth API methods =====
import { apiFetch } from './client.js';

export async function me() {
    return apiFetch('/api/v1/auth/me', {}, true);
}

export async function logout() {
    await apiFetch('/api/v1/auth/logout', { method: 'POST' }, true);
}

export async function loginWithPassword(usuario, senha) {
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
}
