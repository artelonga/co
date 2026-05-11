// ===== CO-198: Private DM inbox =====
// Manages the DM inbox drawer, thread list, unread counts, and DM policy settings.
import { esc } from './helpers.js';
import { apiFetch } from './api.js';
import { mountChat } from './chat.js';

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let _inbox = { threads: [], total_unread: 0 };
let _container = null;
let _me = null;
let _pollTimer = null;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function mountDmInbox({ container, me }) {
    _container = container;
    _me = me;
    _render();
    _loadInbox();
    _startPoll();
}

export function destroyDmInbox() {
    _stopPoll();
    _container = null;
    _me = null;
}

/** Open a DM thread with another user by their user_id. */
export async function openDmWith(userId, drawerContainer) {
    const t = k => window.t ? window.t(k) : k;
    const resp = await fetch(`/api/v1/dms/with/${encodeURIComponent(userId)}`, {
        method: 'POST',
        credentials: 'include',
    }).catch(() => null);

    if (!resp) return;

    if (resp.status === 400) {
        const d = await resp.json().catch(() => ({}));
        const key = d.message || '';
        alert(t(key.includes('self_dm') ? 'dm.error.self_dm' : key));
        return;
    }
    if (resp.status === 403) {
        const d = await resp.json().catch(() => ({}));
        const key = d.message || '';
        alert(t(key.includes('blocked') ? 'dm.error.blocked' : 'dm.error.policy_denied'));
        return;
    }
    if (!resp.ok) return;

    const dm = await resp.json().catch(() => null);
    if (!dm) return;

    _openDmThread(dm, drawerContainer);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function t(key, vars = {}) {
    const val = window.t ? window.t(key) : key;
    return Object.entries(vars).reduce((s, [k, v]) => s.replace(`{${k}}`, v), val || key);
}

function _render() {
    if (!_container) return;
    _container.innerHTML = `
<div class="dm-inbox-inner">
  <header class="chat-header">
    <span class="chat-title">📩 ${esc(t('dm.title'))}${_inbox.total_unread > 0 ? ` <span class="dm-badge">${_inbox.total_unread}</span>` : ''}</span>
    <button class="chat-close-btn" id="dm-close-btn" aria-label="Fechar">&times;</button>
  </header>
  <div class="dm-thread-list" id="dm-thread-list">
    ${_renderThreads()}
  </div>
</div>`;
    _container.querySelector('#dm-close-btn')?.addEventListener('click', () => {
        destroyDmInbox();
        _container?.classList.add('hidden');
    });
    _wireThreadClicks();
}

function _renderThreads() {
    if (!_inbox.threads || _inbox.threads.length === 0) {
        return `<p class="dm-empty">${esc(t('dm.empty'))}</p>`;
    }
    return _inbox.threads.map(thread => {
        const unread = thread.unread_count > 0
            ? `<span class="dm-unread-badge">${thread.unread_count}</span>` : '';
        const preview = thread.last_message
            ? esc(thread.last_message.body_preview)
            : '';
        const ago = thread.last_message ? _timeAgo(thread.last_message.created_at) : '';
        const name = esc(thread.other_user.display_name || thread.other_user.user_id);
        return `<div class="dm-thread-row" data-room-id="${esc(thread.room_id)}" data-slug="${esc(thread.slug)}" data-other-user-id="${esc(thread.other_user.user_id)}" data-other-name="${name}" role="button" tabindex="0">
  <div class="dm-thread-header">
    <span class="dm-thread-name">${name}</span>${unread}
  </div>
  <div class="dm-thread-preview">${preview}</div>
  <div class="dm-thread-ago">${esc(ago)}</div>
</div>`;
    }).join('');
}

function _wireThreadClicks() {
    const list = _container?.querySelector('#dm-thread-list');
    if (!list) return;
    list.querySelectorAll('.dm-thread-row').forEach(el => {
        el.addEventListener('click', () => _handleThreadClick(el));
        el.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') _handleThreadClick(el);
        });
    });
}

function _handleThreadClick(el) {
    const roomId = el.dataset.roomId;
    const slug = el.dataset.slug;
    const otherUserId = el.dataset.otherUserId;
    const otherName = el.dataset.otherName;
    _openDmThread({ room_id: roomId, slug, other_user: { user_id: otherUserId, display_name: otherName } }, null);
    // Mark as read
    fetch(`/api/v1/dms/${encodeURIComponent(roomId)}/read`, {
        method: 'POST',
        credentials: 'include',
    }).catch(() => {});
    // Update local unread count optimistically
    const thread = _inbox.threads.find(th => th.room_id === roomId);
    if (thread) {
        _inbox.total_unread = Math.max(0, _inbox.total_unread - thread.unread_count);
        thread.unread_count = 0;
    }
    _updateSidebarBadge();
}

function _openDmThread(dm, drawerContainer) {
    // Hide inbox, open chat drawer in DM mode
    const drawer = drawerContainer || document.getElementById('chat-drawer');
    if (!drawer) return;

    destroyDmInbox();
    document.getElementById('dm-drawer')?.classList.add('hidden');
    drawer.classList.remove('hidden');

    mountChat({
        universeSlug: 'dm',
        mode: 'dm',
        container: drawer,
        me: _me,
        dmSlug: dm.slug,
        dmOtherName: dm.other_user?.display_name || dm.other_user?.user_id || '',
    });
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

async function _loadInbox() {
    const data = await apiFetch('/api/v1/me/dms', {}, true).catch(() => null);
    if (!data) return;
    _inbox = data;
    _render();
    _updateSidebarBadge();
}

function _startPoll() {
    _stopPoll();
    _pollTimer = setInterval(_loadInbox, 30000);
}

function _stopPoll() {
    if (_pollTimer) { clearInterval(_pollTimer); _pollTimer = null; }
}

// ---------------------------------------------------------------------------
// Sidebar badge
// ---------------------------------------------------------------------------

export function updateDmBadge(totalUnread) {
    const badge = document.getElementById('dm-sidebar-badge');
    if (!badge) return;
    if (totalUnread > 0) {
        badge.textContent = totalUnread;
        badge.classList.remove('hidden');
    } else {
        badge.classList.add('hidden');
    }
    // Also update button text
    const btn = document.getElementById('btn-open-dm');
    if (btn) {
        btn.setAttribute('aria-label', totalUnread > 0 ? `${t('dm.title')} (${totalUnread})` : t('dm.title'));
    }
}

function _updateSidebarBadge() {
    updateDmBadge(_inbox.total_unread || 0);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function _timeAgo(isoStr) {
    if (!isoStr) return '';
    const diffMs = Date.now() - new Date(isoStr).getTime();
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return window.t ? window.t('chat.just_now') || 'agora' : 'agora';
    if (diffMin < 60) return `há ${diffMin} min`;
    const diffH = Math.floor(diffMin / 60);
    if (diffH < 24) return `há ${diffH}h`;
    return `há ${Math.floor(diffH / 24)}d`;
}
