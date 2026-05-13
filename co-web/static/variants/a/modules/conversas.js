// ===== CO-209: Conversas — unified chat surface =====
// Single drawer combining universe chats ("Universos") and DMs ("Mensagens privadas").
// Left rail: two sections + member rail (bottom). Right pane: conversation view.
// Uses existing mountChat / destroyChat from chat.js for the conversation pane.

import { esc } from './helpers.js';
import { apiFetch } from './api.js';
import { mountChat, destroyChat } from './chat.js';
import { openDmWith } from './dm.js';
import { showConversasWelcome } from './conversas-welcome.js';

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

let _state = null;
// _state: {
//   container, me, meUniverses, dms, currentSlug, currentRoomSlug,
//   currentMode,  // 'universe' | 'dm'
//   members, memberFilter, dmSectionCollapsed, destroyed,
//   presenceSet  // Set<user_id> of users online in current room
// }

// Callback injected by app.js so the member DM button can use the openDmWith flow.
let _openDmWith = null;
export function injectOpenDmWithForConversas(fn) { _openDmWith = fn; }

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function mountConversas({ container, me }) {
    if (_state && !_state.destroyed) destroyConversas();

    _state = {
        container,
        me,
        meUniverses: null,
        dms: [],
        currentSlug: null,
        currentRoomSlug: null,
        currentMode: null,
        members: [],
        memberFilter: '',
        dmSectionCollapsed: false,
        destroyed: false,
        presenceSet: new Set(),
    };

    _render();
    _loadData();
}

export function destroyConversas() {
    if (!_state) return;
    _state.destroyed = true;
    destroyChat();
    window.coOnChatPresenceChanged = null;
    _state = null;
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

async function _loadData() {
    if (!_state) return;
    const [meUniverses, inboxData] = await Promise.all([
        apiFetch('/api/v1/me/universes', {}, true).catch(() => null),
        apiFetch('/api/v1/me/dms', {}, true).catch(() => null),
    ]);
    if (!_state || _state.destroyed) return;
    _state.meUniverses = meUniverses || {};
    _state.dms = inboxData?.threads || [];
    _renderLeftRail();
}

async function _loadMembers(universeSlug, roomSlug) {
    if (!_state) return;
    const data = await apiFetch(
        `/api/v1/universes/${encodeURIComponent(universeSlug)}/chat/rooms/${encodeURIComponent(roomSlug)}/members`,
        {},
        true
    ).catch(() => null);
    if (!_state || _state.destroyed) return;
    _state.members = data || [];
    _renderMemberRail();
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

function t(key, vars = {}) {
    const val = (window.t ? window.t(key) : null) || key;
    return Object.entries(vars).reduce((s, [k, v]) => s.replace(`{${k}}`, v), val);
}

function _render() {
    if (!_state || !_state.container) return;
    const c = _state.container;
    c.innerHTML = `
<div class="conversas-inner">
  <header class="chat-header">
    <span class="chat-title">💬 ${esc(t('conversas.title'))}</span>
    <button class="chat-close-btn" id="conversas-close-btn" aria-label="Fechar">&times;</button>
  </header>
  <div class="conversas-body">
    <nav class="conversas-rail" id="conversas-rail">
      <div id="conversas-rail-content">
        <div class="conversas-loading">…</div>
      </div>
      <div class="conversas-member-rail" id="conversas-member-rail"></div>
    </nav>
    <section class="conversas-pane" id="conversas-pane">
      <div class="conversas-empty" id="conversas-empty">
        <p>${esc(t('conversas.empty'))}</p>
      </div>
    </section>
  </div>
</div>`;

    c.querySelector('#conversas-close-btn').addEventListener('click', () => {
        destroyConversas();
        c.classList.add('hidden');
    });

    document.addEventListener('keydown', _handleEsc);
}

function _handleEsc(e) {
    if (e.key === 'Escape' && _state && !_state.destroyed) {
        destroyConversas();
        _state?.container?.classList.add('hidden');
    }
}

function _renderLeftRail() {
    if (!_state) return;
    const content = _state.container?.querySelector('#conversas-rail-content');
    if (!content) return;

    const me = _state.meUniverses || {};
    const allUniverses = [
        ...(me.owned || []),
        ...(me.member || []),
        ...(me.subscribed || []),
    ];

    const dms = _state.dms;

    content.innerHTML = `
<div class="conversas-section">
  <div class="conversas-section-label">${esc(t('conversas.section.universos'))}</div>
  <ul class="conversas-universe-list" id="conversas-universe-list">
    ${allUniverses.length
        ? allUniverses.map(u => `
      <li class="conversas-universe-item${_state.currentSlug === u.key && _state.currentMode === 'universe' ? ' active' : ''}"
          data-slug="${esc(u.key)}" role="button" tabindex="0">
        <span class="conversas-universe-name">${esc(u.name || u.key)}</span>
      </li>`).join('')
        : `<li class="conversas-empty-hint">${esc(t('conversas.empty'))}</li>`
    }
  </ul>
</div>
<div class="conversas-section">
  <div class="conversas-section-label conversas-dm-toggle" id="conversas-dm-toggle" role="button" tabindex="0">
    ${_state.dmSectionCollapsed ? '▸' : '▾'} ${esc(t('conversas.section.mensagens_privadas'))}
  </div>
  ${_state.dmSectionCollapsed ? '' : `
  <ul class="conversas-dm-list" id="conversas-dm-list">
    ${dms.length
        ? dms.map(th => {
            const name = esc(th.other_user.display_name || th.other_user.user_id);
            const unread = th.unread_count > 0
                ? `<span class="dm-unread-badge">${th.unread_count}</span>` : '';
            const active = _state.currentSlug === th.slug && _state.currentMode === 'dm' ? ' active' : '';
            return `<li class="conversas-dm-item${active}"
                data-slug="${esc(th.slug)}"
                data-other-user-id="${esc(th.other_user.user_id)}"
                data-other-name="${name}"
                role="button" tabindex="0">
              📩 ${name}${unread}
            </li>`;
        }).join('')
        : `<li class="conversas-empty-hint">${esc(t('dm.empty'))}</li>`
    }
  </ul>`}
</div>`;

    // Wire up universe clicks
    content.querySelectorAll('.conversas-universe-item').forEach(el => {
        el.addEventListener('click', () => _selectUniverseChat(el.dataset.slug));
        el.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') _selectUniverseChat(el.dataset.slug);
        });
    });

    // Wire up DM clicks
    content.querySelectorAll('.conversas-dm-item').forEach(el => {
        el.addEventListener('click', () => _selectDm(
            el.dataset.slug,
            el.dataset.otherUserId,
            el.dataset.otherName,
        ));
        el.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') _selectDm(
                el.dataset.slug, el.dataset.otherUserId, el.dataset.otherName,
            );
        });
    });

    // DM section toggle
    const dmToggle = content.querySelector('#conversas-dm-toggle');
    if (dmToggle) {
        dmToggle.addEventListener('click', () => {
            _state.dmSectionCollapsed = !_state.dmSectionCollapsed;
            _renderLeftRail();
        });
        dmToggle.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') {
                _state.dmSectionCollapsed = !_state.dmSectionCollapsed;
                _renderLeftRail();
            }
        });
    }
}

async function _selectUniverseChat(universeSlug) {
    if (!_state || _state.destroyed) return;

    _state.currentSlug = universeSlug;
    _state.currentMode = 'universe';
    _state.currentRoomSlug = null;
    _state.members = [];
    _state.presenceSet = new Set();
    _renderLeftRail();
    _clearConversationPane();

    const pane = _state.container?.querySelector('#conversas-pane');
    if (!pane) return;

    // Mount the existing chat module into the pane
    destroyChat();
    mountChat({
        universeSlug,
        mode: 'drawer',
        container: pane,
        me: _state.me,
        onRoomSelected: (room) => {
            if (!_state || _state.destroyed) return;
            _state.currentRoomSlug = room.slug;
            _loadMembers(universeSlug, room.slug);
        },
    });

    // Set up presence hook
    window.coOnChatPresenceChanged = (presence) => {
        if (!_state || _state.destroyed) return;
        _state.presenceSet = new Set((presence || []).map(u => u.user_id));
        _renderMemberRail();
    };
}

async function _selectDm(dmSlug, otherUserId, otherName) {
    if (!_state || _state.destroyed) return;

    _state.currentSlug = dmSlug;
    _state.currentMode = 'dm';
    _state.currentRoomSlug = dmSlug;
    _state.members = [];
    _state.presenceSet = new Set();
    _renderLeftRail();
    _clearConversationPane();

    const pane = _state.container?.querySelector('#conversas-pane');
    if (!pane) return;

    destroyChat();
    mountChat({
        universeSlug: 'dm',
        mode: 'dm',
        container: pane,
        me: _state.me,
        dmSlug,
        dmOtherName: otherName,
    });

    // Mark DM as read
    const thread = _state.dms.find(th => th.slug === dmSlug);
    if (thread && thread.room_id) {
        fetch(`/api/v1/dms/${encodeURIComponent(thread.room_id)}/read`, {
            method: 'POST', credentials: 'include',
        }).catch(() => {});
        _loadMembers('dm', dmSlug);
    }

    window.coOnChatPresenceChanged = (presence) => {
        if (!_state || _state.destroyed) return;
        _state.presenceSet = new Set((presence || []).map(u => u.user_id));
        _renderMemberRail();
    };
}

function _clearConversationPane() {
    const pane = _state?.container?.querySelector('#conversas-pane');
    if (!pane) return;
    pane.innerHTML = `<div class="conversas-empty" id="conversas-empty"><p>${esc(t('conversas.empty'))}</p></div>`;
}

function _renderMemberRail() {
    if (!_state) return;
    const rail = _state.container?.querySelector('#conversas-member-rail');
    if (!rail) return;

    const members = _state.members || [];
    if (!members.length) { rail.innerHTML = ''; return; }

    const filtered = _state.memberFilter
        ? members.filter(m => m.display_name.toLowerCase().includes(_state.memberFilter.toLowerCase()))
        : members;

    const filterInput = members.length > 10
        ? `<input class="conversas-member-filter" id="conversas-member-filter"
             placeholder="${esc(t('conversas.filter_members'))}"
             value="${esc(_state.memberFilter)}" aria-label="${esc(t('conversas.filter_members'))}">`
        : '';

    const myUserId = _state.me?.user_id || '';

    rail.innerHTML = `
<div class="conversas-members-header">
  👥 ${esc(t('conversas.members_header', { n: members.length }))}
</div>
${filterInput}
<ul class="conversas-member-list">
  ${filtered.map(m => {
        const online = _state.presenceSet.has(m.user_id);
        const isMe = m.user_id === myUserId;
        const displayName = isMe ? `${esc(m.display_name)} (você)` : esc(m.display_name);
        return `<li class="conversas-member-item${online ? ' online' : ''}"
             data-user-id="${esc(m.user_id)}"
             role="button" tabindex="0"
             title="${esc(m.display_name)}">
          <span class="conversas-presence-dot">${online ? '●' : '○'}</span>
          ${displayName}
        </li>`;
    }).join('')}
</ul>`;

    // Wire up member filter
    const filterEl = rail.querySelector('#conversas-member-filter');
    if (filterEl) {
        filterEl.addEventListener('input', (e) => {
            _state.memberFilter = e.target.value;
            _renderMemberRail();
        });
    }

    // Click member → open DM
    rail.querySelectorAll('.conversas-member-item').forEach(el => {
        el.addEventListener('click', () => _openDmWithMember(el.dataset.userId));
        el.addEventListener('keydown', e => {
            if (e.key === 'Enter' || e.key === ' ') _openDmWithMember(el.dataset.userId);
        });
    });
}

async function _openDmWithMember(userId) {
    if (!_state || !userId || userId === _state.me?.user_id) return;
    if (_openDmWith) {
        const pane = _state.container?.querySelector('#conversas-pane');
        await _openDmWith(userId, pane || _state.container);
    }
}

/**
 * Open Conversas drawer. Shows welcome modal on first use.
 * Called by app.js when the Conversas button is clicked.
 */
export function openConversas(container, me) {
    container.classList.remove('hidden');
    if (_state && !_state.destroyed && _state.container === container) return;
    mountConversas({ container, me });
    showConversasWelcome(container, () => {
        // After welcome dismissed, focus the composer if a conversation is open
        const composer = container.querySelector('#chat-input');
        if (composer) composer.focus();
    });
}
