// CO-202: In-app notification center — bell badge + dropdown + full-page view
import { apiFetch } from './api.js';

let _unreadCount = 0;
let _pollTimer = null;
let _dropdownOpen = false;

function t(key) {
    return (window.t ? window.t(key) : null) || key;
}

export async function setupNotifications() {
    await refreshUnreadCount();
    _pollTimer = setInterval(refreshUnreadCount, 30000);

    const btn = document.getElementById('btn-notifications');
    if (btn) {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            if (_dropdownOpen) closeDropdown();
            else openDropdown();
        });
    }

    document.addEventListener('click', (e) => {
        if (!_dropdownOpen) return;
        const dropdown = document.getElementById('notifications-dropdown');
        const bellBtn = document.getElementById('btn-notifications');
        if (dropdown && !dropdown.contains(e.target) && e.target !== bellBtn && !bellBtn?.contains(e.target)) {
            closeDropdown();
        }
    });
}

export function teardownNotifications() {
    if (_pollTimer) { clearInterval(_pollTimer); _pollTimer = null; }
}

async function refreshUnreadCount() {
    const resp = await apiFetch('/api/v1/me/notifications?limit=1', {}, true);
    if (resp) _unreadCount = resp.unread_count || 0;
    renderBellBadge();
}

export function bumpUnreadCount() {
    _unreadCount++;
    renderBellBadge();
}

function renderBellBadge() {
    const badge = document.getElementById('notif-badge');
    if (!badge) return;
    if (_unreadCount > 0) {
        badge.textContent = _unreadCount > 99 ? '99+' : String(_unreadCount);
        badge.classList.remove('hidden');
    } else {
        badge.classList.add('hidden');
    }
}

function relativeTime(isoStr) {
    if (!isoStr) return '';
    const diff = Math.max(0, Date.now() - new Date(isoStr).getTime());
    const mins = Math.floor(diff / 60000);
    const hours = Math.floor(diff / 3600000);
    const days = Math.floor(diff / 86400000);
    const lang = window.currentLang || 'pt';
    if (mins < 1) return lang === 'pt' ? 'agora' : 'just now';
    if (mins < 60) return lang === 'pt' ? `há ${mins} min` : `${mins} min ago`;
    if (hours < 24) return lang === 'pt' ? `há ${hours} h` : `${hours} h ago`;
    return lang === 'pt' ? `há ${days} d` : `${days} d ago`;
}

function eventIcon(eventType) {
    switch (eventType) {
        case 'chat.dm': return '📩';
        case 'chat.message': return '💬';
        case 'universe.invitation': return '🎁';
        case 'chat.mention': return '@';
        default: return '🔔';
    }
}

function notifUrl(notif) {
    const ctx = notif.context || {};
    switch (notif.event_type) {
        case 'chat.message':
        case 'chat.mention':
            return ctx.universe_key
                ? `/${ctx.universe_key}#chat-${ctx.room_id || ''}-${ctx.object_id || ''}`
                : null;
        case 'chat.dm':
            return ctx.room_id ? `/?dm=${ctx.room_id}#msg-${ctx.object_id || ''}` : null;
        case 'universe.invitation':
            return ctx.object_id ? `/invitations/${ctx.object_id}` : null;
        default:
            return null;
    }
}

function renderSummary(notif) {
    const s = notif.summary;
    if (!s) return '';
    const raw = t(s.key) || s.key;
    const params = s.params || {};
    return Object.entries(params).reduce((str, [k, v]) => str.replace(`{${k}}`, String(v)), raw);
}

function _notifRowHtml(n) {
    const unreadClass = n.read_at ? '' : ' unread';
    const url = notifUrl(n) || '';
    return `<div class="notif-row${unreadClass}" data-notif-id="${n.id}" data-notif-url="${url}">
        <span class="icon">${eventIcon(n.event_type)}</span>
        <div class="body">
            <div class="summary">${renderSummary(n)}</div>
            <div class="time">${relativeTime(n.created_at)}</div>
        </div>
    </div>`;
}

function renderDropdown(notifications) {
    const dropdown = document.getElementById('notifications-dropdown');
    if (!dropdown) return;

    const rows = (notifications && notifications.length > 0)
        ? notifications.map(_notifRowHtml).join('')
        : `<div class="notif-empty">${t('notif.dropdown.empty')}</div>`;

    dropdown.innerHTML = `
        <div class="notif-dropdown-header">
            <span>${t('notif.bell.tooltip')}</span>
            <button class="btn-text-sm" id="btn-mark-all-read">${t('notif.dropdown.mark_all')}</button>
        </div>
        ${rows}
        <div class="notif-dropdown-footer">
            <a href="/notifications" class="btn-text-sm">${t('notif.dropdown.see_all')}</a>
        </div>`;

    dropdown.querySelectorAll('.notif-row').forEach(row => {
        row.addEventListener('click', () => {
            onNotifClick(row.dataset.notifId, row.dataset.notifUrl || null);
        });
    });

    dropdown.querySelector('#btn-mark-all-read')?.addEventListener('click', (e) => {
        e.stopPropagation();
        markAllRead();
    });
}

async function openDropdown() {
    _dropdownOpen = true;
    const dropdown = document.getElementById('notifications-dropdown');
    if (dropdown) dropdown.classList.remove('hidden');
    const resp = await apiFetch('/api/v1/me/notifications?limit=15', {}, true);
    if (resp) renderDropdown(resp.notifications || []);
}

function closeDropdown() {
    _dropdownOpen = false;
    const dropdown = document.getElementById('notifications-dropdown');
    if (dropdown) dropdown.classList.add('hidden');
    refreshUnreadCount();
}

async function onNotifClick(notifId, url) {
    if (notifId) {
        await apiFetch(`/api/v1/me/notifications/${notifId}/read`, { method: 'POST', credentials: 'include' }, true);
        _unreadCount = Math.max(0, _unreadCount - 1);
        renderBellBadge();
    }
    closeDropdown();
    if (url) window.location.href = url;
}

async function markAllRead() {
    await apiFetch('/api/v1/me/notifications/read-all', { method: 'POST', credentials: 'include' }, true);
    _unreadCount = 0;
    renderBellBadge();
    const resp = await apiFetch('/api/v1/me/notifications?limit=15', {}, true);
    if (resp) renderDropdown(resp.notifications || []);
}

// ---------------------------------------------------------------------------
// Full notifications page
// ---------------------------------------------------------------------------

export async function renderNotificationsPage(container) {
    if (!container) return;
    let filter = 'all';
    let since = null;

    async function load(reset) {
        if (reset) {
            since = null;
            const list = container.querySelector('#notif-page-list');
            if (list) list.innerHTML = '';
        }
        let url = '/api/v1/me/notifications?limit=50';
        if (since) url += `&since=${encodeURIComponent(since)}`;
        if (filter === 'unread') url += '&unread_only=true';
        else if (filter !== 'all') url += `&event_type=${encodeURIComponent(filter)}`;

        const resp = await apiFetch(url, {}, true);
        if (!resp) return;
        const notifs = resp.notifications || [];
        _appendPageRows(container, notifs, reset);
        if (notifs.length > 0) since = notifs[notifs.length - 1].created_at;
        const moreBtn = container.querySelector('#notif-load-more');
        if (moreBtn) moreBtn.classList.toggle('hidden', !resp.has_more);
    }

    container.innerHTML = `
        <div class="notif-page">
            <div class="notif-page-header">
                <h2>${t('notif.page.title')}</h2>
                <button class="btn btn-secondary btn-sm" id="notif-page-mark-all">${t('notif.dropdown.mark_all')}</button>
            </div>
            <div class="notif-page-filters">
                <button class="notif-filter-btn active" data-filter="all">${t('notif.page.filter_all')}</button>
                <button class="notif-filter-btn" data-filter="unread">${t('notif.page.filter_unread')}</button>
                <button class="notif-filter-btn" data-filter="chat.message">${t('notif.event.chat_message')}</button>
                <button class="notif-filter-btn" data-filter="chat.dm">${t('notif.event.chat_dm')}</button>
                <button class="notif-filter-btn" data-filter="universe.invitation">${t('notif.event.invitation')}</button>
                <button class="notif-filter-btn" data-filter="chat.mention">${t('notif.event.mention')}</button>
            </div>
            <div class="notif-page-list" id="notif-page-list"></div>
            <div id="notif-load-more" class="notif-load-more hidden">
                <button class="btn btn-secondary btn-sm" id="btn-load-more-notifs">Carregar mais</button>
            </div>
        </div>`;

    container.querySelectorAll('.notif-filter-btn').forEach(btn => {
        btn.addEventListener('click', async () => {
            container.querySelectorAll('.notif-filter-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            filter = btn.dataset.filter;
            await load(true);
        });
    });

    container.querySelector('#notif-page-mark-all')?.addEventListener('click', async () => {
        await apiFetch('/api/v1/me/notifications/read-all', { method: 'POST', credentials: 'include' }, true);
        _unreadCount = 0;
        renderBellBadge();
        await load(true);
    });

    container.querySelector('#btn-load-more-notifs')?.addEventListener('click', () => load(false));

    await load(true);
}

function _appendPageRows(container, notifs, reset) {
    const list = container.querySelector('#notif-page-list');
    if (!list) return;
    if (reset && notifs.length === 0) {
        list.innerHTML = `<div class="notif-empty">${t('notif.dropdown.empty')}</div>`;
        return;
    }
    notifs.forEach(n => {
        const row = document.createElement('div');
        row.className = 'notif-row' + (n.read_at ? '' : ' unread');
        const url = notifUrl(n) || '';
        row.innerHTML = `
            <span class="icon">${eventIcon(n.event_type)}</span>
            <div class="body">
                <div class="summary">${renderSummary(n)}</div>
                <div class="time">${relativeTime(n.created_at)}</div>
            </div>`;
        row.addEventListener('click', async () => {
            await apiFetch(`/api/v1/me/notifications/${n.id}/read`, { method: 'POST', credentials: 'include' }, true);
            _unreadCount = Math.max(0, _unreadCount - 1);
            renderBellBadge();
            row.classList.remove('unread');
            if (url) window.location.href = url;
        });
        list.appendChild(row);
    });
}
