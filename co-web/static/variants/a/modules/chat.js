// ===== CO Chat — sidebar drawer + yggdrasil lobby panel =====
// CO-195: base chat panel (rooms, messages, WS live updates, presence)
// CO-196: message edit + delete moderation affordances
import { esc } from './helpers.js';
import { apiFetch } from './api.js';

const EDIT_WINDOW_MS = 15 * 60 * 1000; // 15 min

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let _state = null; // { universeSlug, mode, container, me, role, rooms, currentRoom, messages, ws, wsRetryMs, presence, destroyed, dmSlug, dmOtherName }

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function mountChat({ universeSlug, mode = 'drawer', container, me, dmSlug, dmOtherName, onRoomSelected }) {
    if (_state && !_state.destroyed) destroyChat();
    _state = {
        universeSlug,
        mode,
        container,
        me,
        role: null,
        rooms: [],
        currentRoom: null,
        messages: [],
        ws: null,
        wsRetryMs: 1000,
        presence: [],
        destroyed: false,
        editingMsgId: null,
        dmSlug: dmSlug || null,
        dmOtherName: dmOtherName || null,
        onRoomSelected: onRoomSelected || null,
    };
    _render();
    // CO-198: in DM mode skip room list and go directly to the DM thread
    if (mode === 'dm' && dmSlug) {
        _switchToDmRoom(dmSlug);
    } else {
        _loadRooms();
    }
}

export function destroyChat() {
    if (!_state) return;
    _state.destroyed = true;
    _closeWs();
    _state = null;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function t(key, vars = {}) {
    const val = (window.t ? window.t(key) : null) || key;
    return Object.entries(vars).reduce(
        (s, [k, v]) => s.replace(`{${k}}`, v),
        val
    );
}

function _render() {
    if (!_state || !_state.container) return;
    const c = _state.container;
    const isDm = _state.mode === 'dm';
    const titleText = isDm && _state.dmOtherName
        ? `📩 ${_state.dmOtherName}`
        : t('chat.title', { universe: _state.universeSlug });
    c.innerHTML = `
<div class="chat-inner">
  <header class="chat-header">
    <span class="chat-title" id="chat-title">${esc(titleText)}</span>
    <button class="chat-close-btn" id="chat-close-btn" aria-label="Fechar chat">&times;</button>
  </header>
  <div class="chat-body">
    <nav class="chat-rail${isDm ? ' hidden' : ''}" id="chat-rail">
      <ul class="chat-room-list" id="chat-room-list"></ul>
      <button class="chat-new-room-btn hidden" id="chat-new-room-btn">${esc(t('chat.new_room'))}</button>
      <div class="chat-presence" id="chat-presence"></div>
    </nav>
    <section class="chat-pane">
      <div class="chat-scrollback" id="chat-scrollback" aria-live="polite" aria-label="Mensagens"></div>
      <div class="chat-conn-banner hidden" id="chat-conn-banner">${esc(t('chat.connection_lost'))}</div>
      <form class="chat-composer" id="chat-composer" novalidate>
        <textarea class="chat-input" id="chat-input" rows="2"
          placeholder="${esc(t('chat.composer_placeholder'))}"
          aria-label="${esc(t('chat.composer_placeholder'))}"></textarea>
        <button type="submit" class="btn btn-primary chat-send-btn" id="chat-send-btn">${esc(t('chat.send'))}</button>
      </form>
    </section>
  </div>
</div>`;

    c.querySelector('#chat-close-btn')?.addEventListener('click', () => {
        destroyChat();
        c.classList.add('hidden');
    });

    c.querySelector('#chat-new-room-btn')?.addEventListener('click', _handleNewRoom);

    const form = c.querySelector('#chat-composer');
    const input = c.querySelector('#chat-input');
    if (form && input) {
        form.addEventListener('submit', e => { e.preventDefault(); _sendMessage(); });
        input.addEventListener('keydown', e => {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); _sendMessage(); }
        });
    }

    document.addEventListener('keydown', _handleEsc);
}

function _handleEsc(e) {
    if (e.key === 'Escape' && _state && !_state.destroyed) {
        if (_state.editingMsgId) {
            _cancelEdit();
        } else {
            destroyChat();
            _state?.container?.classList.add('hidden');
        }
    }
}

function _renderRooms() {
    if (!_state) return;
    const list = _state.container?.querySelector('#chat-room-list');
    if (!list) return;
    list.innerHTML = _state.rooms
        .map(r => `<li class="chat-room-item${r.id === _state.currentRoom?.id ? ' active' : ''}"
             data-room-id="${esc(r.id)}" data-room-slug="${esc(r.slug)}"
             role="button" tabindex="0">#${esc(r.name)}</li>`)
        .join('');
    list.querySelectorAll('.chat-room-item').forEach(el => {
        el.addEventListener('click', () => _switchRoom(el.dataset.roomSlug));
        el.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') _switchRoom(el.dataset.roomSlug);
        });
    });

    const canManage = _state.role === 'owner' || _state.role === 'admin';
    const newBtn = _state.container?.querySelector('#chat-new-room-btn');
    if (newBtn) newBtn.classList.toggle('hidden', !canManage);
}

function _renderMessages() {
    if (!_state) return;
    const box = _state.container?.querySelector('#chat-scrollback');
    if (!box) return;
    const wasAtBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 60;
    box.innerHTML = _state.messages.length
        ? _state.messages.map(_msgHtml).join('')
        : `<p class="chat-empty">${esc(t('chat.empty_room'))}</p>`;
    _wireMessageActions(box);
    if (wasAtBottom) box.scrollTop = box.scrollHeight;
}

function _msgHtml(msg) {
    if (msg.deleted_at) return _tombstoneHtml(msg);
    const me = _state?.me;
    const isOwnMsg = me && msg.author.user_id === me.user_id;
    const canMod = _state?.role === 'owner' || _state?.role === 'admin';
    const withinWindow = isOwnMsg && (Date.now() - new Date(msg.created_at).getTime()) < EDIT_WINDOW_MS;

    const editBtn = withinWindow
        ? `<button class="chat-edit-btn" data-msg-id="${esc(msg.id)}" title="${esc(t('chat.edit'))}" aria-label="${esc(t('chat.edit'))}">✏️</button>`
        : '';
    const deleteBtn = (isOwnMsg || canMod) && !msg.deleted_at
        ? `<button class="chat-delete-btn" data-msg-id="${esc(msg.id)}" title="${esc(t('chat.delete'))}" aria-label="${esc(t('chat.delete'))}">🗑️</button>`
        : '';
    const editedTag = msg.edited_at
        ? ` <span class="chat-edited-tag">${esc(t('chat.edited_tag'))}</span>`
        : '';
    const ago = _timeAgo(msg.created_at);

    return `<div class="chat-message" data-msg-id="${esc(msg.id)}">
  <div class="chat-message-header">
    <span class="chat-author">${esc(msg.author.display_name)}</span>
    <span class="chat-ts">${esc(ago)}</span>${editedTag}
    <span class="chat-actions">${editBtn}${deleteBtn}</span>
  </div>
  <div class="chat-message-body" id="chat-body-${esc(msg.id)}">${esc(msg.body)}</div>
</div>`;
}

function _tombstoneHtml(msg) {
    return `<div class="chat-message chat-message--deleted" data-msg-id="${esc(msg.id)}">
  <div class="chat-message-header">
    <span class="chat-author">${esc(msg.author?.display_name || '')}</span>
    <span class="chat-ts">${esc(_timeAgo(msg.created_at))}</span>
  </div>
  <div class="chat-message-body chat-tombstone">${esc(t('chat.deleted_message'))}</div>
</div>`;
}

function _renderPresence() {
    if (!_state) return;
    const el = _state.container?.querySelector('#chat-presence');
    if (el) {
        if (!_state.presence.length) { el.innerHTML = ''; }
        else {
            el.innerHTML = `<div class="chat-presence-label">${esc(t('chat.presence', { n: _state.presence.length }))}</div>`
                + _state.presence.map(u => `<div class="chat-presence-item online">${esc(u.display_name)}</div>`).join('');
        }
    }
    // CO-209: notify conversas member rail about presence changes
    if (typeof window.coOnChatPresenceChanged === 'function') {
        window.coOnChatPresenceChanged(_state.presence);
    }
}

function _renderComposer() {
    if (!_state) return;
    const canPost = ['owner', 'admin', 'member'].includes(_state.role);
    const input = _state.container?.querySelector('#chat-input');
    const btn = _state.container?.querySelector('#chat-send-btn');
    if (input) {
        input.disabled = !canPost;
        if (!canPost) input.placeholder = t('chat.cannot_post_viewer');
    }
    if (btn) btn.disabled = !canPost;
}

// ---------------------------------------------------------------------------
// Message actions wiring
// ---------------------------------------------------------------------------

function _wireMessageActions(box) {
    box.querySelectorAll('.chat-edit-btn').forEach(btn => {
        btn.addEventListener('click', () => _startEdit(btn.dataset.msgId));
    });
    box.querySelectorAll('.chat-delete-btn').forEach(btn => {
        btn.addEventListener('click', () => _startDelete(btn.dataset.msgId));
    });
}

// --- Edit flow ---

function _startEdit(msgId) {
    if (!_state) return;
    const msg = _state.messages.find(m => m.id === msgId);
    if (!msg || msg.deleted_at) return;
    _state.editingMsgId = msgId;

    const bodyEl = _state.container?.querySelector(`#chat-body-${msgId}`);
    if (!bodyEl) return;

    const originalBody = msg.body;
    bodyEl.innerHTML = `<textarea class="chat-edit-textarea" id="chat-edit-textarea">${esc(originalBody)}</textarea>
<div class="chat-edit-actions">
  <button class="btn btn-primary btn-sm" id="chat-edit-save">${esc(t('chat.edit_save'))}</button>
  <button class="btn btn-secondary btn-sm" id="chat-edit-cancel">${esc(t('chat.edit_cancel'))}</button>
</div>`;

    const textarea = bodyEl.querySelector('#chat-edit-textarea');
    textarea?.focus();
    textarea?.setSelectionRange(textarea.value.length, textarea.value.length);

    textarea?.addEventListener('keydown', e => {
        if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); _submitEdit(msgId, textarea.value); }
        if (e.key === 'Escape') _cancelEdit();
    });

    bodyEl.querySelector('#chat-edit-save')?.addEventListener('click', () => {
        _submitEdit(msgId, textarea?.value || '');
    });
    bodyEl.querySelector('#chat-edit-cancel')?.addEventListener('click', _cancelEdit);
}

function _cancelEdit() {
    if (!_state || !_state.editingMsgId) return;
    _state.editingMsgId = null;
    _renderMessages();
}

async function _submitEdit(msgId, newBody) {
    if (!_state || !_state.currentRoom) return;
    const trimmed = newBody.trim();
    if (!trimmed) return;

    const universeSlug = _state.mode === 'dm' ? 'dm' : _state.universeSlug;
    const { currentRoom } = _state;
    const resp = await fetch(
        `/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(currentRoom.slug)}/messages/${encodeURIComponent(msgId)}`,
        {
            method: 'PATCH',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ body: trimmed }),
        }
    ).catch(() => null);

    if (!resp || !resp.ok) {
        if (resp?.status === 403) {
            const d = await resp.json().catch(() => ({}));
            if (d.message === 'edit_window_expired') {
                _showToast(t('chat.edit_window_expired'), 'error');
            } else {
                _showToast(t('chat.edit_failed'), 'error');
            }
        } else {
            _showToast(t('chat.edit_failed'), 'error');
        }
        _cancelEdit();
        return;
    }

    const updated = await resp.json().catch(() => null);
    if (updated) {
        const idx = _state.messages.findIndex(m => m.id === msgId);
        if (idx !== -1) _state.messages[idx] = updated;
    }
    _state.editingMsgId = null;
    _renderMessages();
}

// --- Delete flow ---

function _startDelete(msgId) {
    if (!_state) return;

    const msgEl = _state.container?.querySelector(`.chat-message[data-msg-id="${msgId}"]`);
    if (!msgEl) return;

    // Remove any existing popover
    msgEl.querySelector('.chat-delete-popover')?.remove();

    const popover = document.createElement('div');
    popover.className = 'chat-delete-popover';
    popover.innerHTML = `<span>${esc(t('chat.delete_confirm'))}</span>
<button class="btn btn-danger btn-sm" id="chat-delete-confirm-yes">${esc(t('chat.delete_yes'))}</button>
<button class="btn btn-secondary btn-sm" id="chat-delete-confirm-no">${esc(t('chat.edit_cancel'))}</button>`;
    msgEl.appendChild(popover);

    popover.querySelector('#chat-delete-confirm-yes')?.addEventListener('click', async () => {
        popover.remove();
        await _submitDelete(msgId);
    });
    popover.querySelector('#chat-delete-confirm-no')?.addEventListener('click', () => popover.remove());
}

async function _submitDelete(msgId) {
    if (!_state || !_state.currentRoom) return;
    const universeSlug = _state.mode === 'dm' ? 'dm' : _state.universeSlug;
    const { currentRoom } = _state;

    // Optimistic: flip to tombstone immediately
    const idx = _state.messages.findIndex(m => m.id === msgId);
    const backup = idx !== -1 ? { ..._state.messages[idx] } : null;
    if (idx !== -1) {
        _state.messages[idx] = {
            ..._state.messages[idx],
            deleted_at: new Date().toISOString(),
        };
        _renderMessages();
    }

    const resp = await fetch(
        `/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(currentRoom.slug)}/messages/${encodeURIComponent(msgId)}`,
        { method: 'DELETE', credentials: 'include' }
    ).catch(() => null);

    if (!resp || !resp.ok) {
        // Rollback
        if (backup && idx !== -1) _state.messages[idx] = backup;
        _renderMessages();
        _showToast(t('chat.delete_failed'), 'error');
    }
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

async function _loadRooms() {
    if (!_state) return;
    const data = await apiFetch(
        `/api/v1/universes/${encodeURIComponent(_state.universeSlug)}/chat/rooms`,
        {},
        true
    ).catch(() => null);

    if (!_state || _state.destroyed) return;
    if (!data || !data.rooms) return;

    _state.rooms = data.rooms;
    _renderRooms();

    const defaultRoom = data.rooms.find(r => r.is_default) || data.rooms[0];
    if (defaultRoom) await _switchRoom(defaultRoom.slug);
}

async function _switchRoom(slug) {
    if (!_state) return;
    const room = _state.rooms.find(r => r.slug === slug);
    if (!room) return;
    if (_state.currentRoom?.id === room.id) return;

    _closeWs();
    _state.currentRoom = room;
    _state.messages = [];
    _state.presence = [];
    _state.editingMsgId = null;
    _renderRooms();
    _renderMessages();

    // Update title
    const titleEl = _state.container?.querySelector('#chat-title');
    if (titleEl) titleEl.textContent = t('chat.title', { universe: _state.universeSlug }) + ` — #${room.name}`;

    await _loadMessages();
    _openWs(room.slug);
    if (_state?.onRoomSelected) _state.onRoomSelected(room);
}

// CO-198: switch into a DM room directly (bypassing room list)
async function _switchToDmRoom(dmSlug) {
    if (!_state) return;
    _closeWs();
    // Build a synthetic room object for the DM
    _state.currentRoom = { id: null, slug: dmSlug, name: _state.dmOtherName || dmSlug };
    _state.messages = [];
    _state.presence = [];
    _state.editingMsgId = null;
    _renderMessages();
    _renderComposer();
    await _loadMessages();
    _openWs(dmSlug);
}

async function _loadMessages() {
    if (!_state || !_state.currentRoom) return;
    const universeSlug = _state.mode === 'dm' ? 'dm' : _state.universeSlug;
    const { currentRoom } = _state;
    const data = await apiFetch(
        `/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(currentRoom.slug)}/messages?limit=50`,
        {},
        true
    ).catch(() => null);

    if (!_state || _state.destroyed) return;
    if (!data || !data.messages) return;

    _state.messages = [...data.messages].reverse(); // REST returns newest-first; reverse for display
    _renderMessages();
    // scroll to bottom on initial load
    const box = _state.container?.querySelector('#chat-scrollback');
    if (box) box.scrollTop = box.scrollHeight;
}

async function _sendMessage() {
    if (!_state || !_state.currentRoom) return;
    const input = _state.container?.querySelector('#chat-input');
    if (!input) return;
    const body = input.value.trim();
    if (!body) return;

    input.value = '';
    input.focus();

    const universeSlug = _state.mode === 'dm' ? 'dm' : _state.universeSlug;
    const { currentRoom } = _state;
    const resp = await fetch(
        `/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(currentRoom.slug)}/messages`,
        {
            method: 'POST',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ body }),
        }
    ).catch(() => null);

    if (!resp || !resp.ok) {
        _showToast(t('chat.send_error'), 'error');
    }
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

function _isAuthenticated() {
    // CO-234: rely on the `me` state passed to mountChat(), not document.cookie.
    // The session cookie is HttpOnly (set at co-web/src/auth.rs:271), so it's
    // invisible to JavaScript by design. Previously we used a document.cookie
    // regex which returned false for every logged-in user, surfacing the
    // "Entre para participar do chat" hint even when the session was valid.
    return !!_state?.me;
}

function _openWs(roomSlug) {
    if (!_state || _state.destroyed) return;
    if (!_isAuthenticated()) {
        // Anonymous users can read the room history but can't hold an
        // authed WebSocket. Hide the connection banner so they don't see
        // a perpetual "Conexão perdida. Reconectando…" loop and surface
        // a one-time hint inviting them to log in.
        _hideBanner();
        _showAuthHint();
        return;
    }
    const universeSlug = _state.mode === 'dm' ? 'dm' : _state.universeSlug;
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${proto}//${location.host}/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(roomSlug)}/ws`;

    const ws = new WebSocket(url);
    _state.ws = ws;

    ws.onopen = () => {
        _state.wsRetryMs = 1000;
        _hideBanner();
    };

    ws.onmessage = e => {
        if (!_state || _state.destroyed) return;
        let evt;
        try { evt = JSON.parse(e.data); } catch (_) { return; }
        _handleWsEvent(evt);
    };

    ws.onerror = () => {};
    ws.onclose = () => {
        if (!_state || _state.destroyed) return;
        _showBanner();
        const delay = Math.min(_state.wsRetryMs, 30000);
        _state.wsRetryMs = Math.min(delay * 2, 30000);
        setTimeout(() => {
            if (_state && !_state.destroyed && _state.currentRoom) {
                _openWs(_state.currentRoom.slug);
            }
        }, delay);
    };
}

function _closeWs() {
    if (!_state || !_state.ws) return;
    const ws = _state.ws;
    _state.ws = null;
    try { ws.close(); } catch (_) {}
}

function _handleWsEvent(evt) {
    if (!_state) return;
    switch (evt.type) {
        case 'ready':
            _state.role = evt.your_role;
            _state.presence = evt.presence || [];
            _renderPresence();
            _renderComposer();
            break;
        case 'message.created':
            _onMessageCreated(evt.message);
            break;
        case 'message.edited':
            _onMessageEdited(evt.message);
            break;
        case 'message.deleted':
            _onMessageDeleted(evt.message_id, evt.deleted_at);
            break;
        case 'presence.join':
            if (!_state.presence.find(u => u.user_id === evt.user.user_id)) {
                _state.presence.push(evt.user);
                _renderPresence();
            }
            break;
        case 'presence.leave':
            _state.presence = _state.presence.filter(u => u.user_id !== evt.user_id);
            _renderPresence();
            break;
        default:
            break;
    }
}

function _onMessageCreated(msg) {
    if (!_state) return;
    if (_state.messages.find(m => m.id === msg.id)) return; // dedupe
    // CO-202: notify bell badge via hook
    if (typeof window.coOnChatMessageArrived === 'function') {
        window.coOnChatMessageArrived(msg, _state.currentRoom?.id);
    }
    const box = _state.container?.querySelector('#chat-scrollback');
    const wasAtBottom = box ? (box.scrollHeight - box.scrollTop - box.clientHeight < 60) : false;
    _state.messages.push(msg);
    _renderMessages();
    if (wasAtBottom && box) box.scrollTop = box.scrollHeight;
}

function _onMessageEdited(msg) {
    if (!_state) return;
    const idx = _state.messages.findIndex(m => m.id === msg.id);
    if (idx !== -1) {
        _state.messages[idx] = msg;
        _renderMessages();
    }
}

function _onMessageDeleted(msgId, deletedAt) {
    if (!_state) return;
    const idx = _state.messages.findIndex(m => m.id === msgId);
    if (idx !== -1) {
        _state.messages[idx] = { ..._state.messages[idx], deleted_at: deletedAt };
        _renderMessages();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function _handleNewRoom() {
    if (!_state) return;
    const name = prompt(t('chat.new_room_prompt_name'));
    if (!name || !name.trim()) return;
    const data = await apiFetch(
        `/api/v1/universes/${encodeURIComponent(_state.universeSlug)}/chat/rooms`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: name.trim() }),
        },
        true
    ).catch(() => null);
    if (data && data.slug) {
        _state.rooms.push(data);
        _renderRooms();
        await _switchRoom(data.slug);
    }
}

function _showBanner() {
    _state?.container?.querySelector('#chat-conn-banner')?.classList.remove('hidden');
}

function _hideBanner() {
    _state?.container?.querySelector('#chat-conn-banner')?.classList.add('hidden');
}

function _showAuthHint() {
    const banner = _state?.container?.querySelector('#chat-conn-banner');
    if (!banner) return;
    banner.classList.remove('hidden');
    banner.textContent = 'Entre para participar do chat.';
}

function _showToast(msg, type = 'info') {
    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.textContent = msg;
    document.getElementById('toast-container')?.appendChild(toast);
    setTimeout(() => toast.remove(), 4000);
}

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
