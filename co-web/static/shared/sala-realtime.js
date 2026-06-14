/* CO-353 — Sala realtime client.
 *
 * A thin WebSocket layer for the Sala canvas: presence (live cursors + roster),
 * optimistic node-move broadcast, and reconnect with exponential backoff.
 *
 * Usage (from sala.html):
 *   var rt = SalaRealtime({
 *     universeKey, workspaceSlug,
 *     onNodeMove: function (entryPath, x, y) { ... },
 *     onPresence: function (users) { ... },     // optional
 *     onStatus:   function (connected) { ... }, // optional
 *   });
 *   rt.sendCursor(worldX, worldY);   // throttled to ~20 Hz
 *   rt.sendNodeMove(entryPath, x, y);
 *   rt.cursors  // { user_id: {x, y, name, color, anon} } — remote cursors
 *   rt.users    // [{user_id, name, color, anon}] — current roster
 *   rt.close();
 *
 * The server is the arbiter (last-write-wins); this client applies its own ops
 * optimistically and relies on the server echo for everyone else.
 */
(function (global) {
  'use strict';

  var CURSOR_MIN_INTERVAL_MS = 50; // ~20 Hz outbound (server throttles too)

  function SalaRealtime(opts) {
    var universeKey = opts.universeKey;
    var workspaceSlug = opts.workspaceSlug || 'default';
    var onNodeMove = opts.onNodeMove || function () {};
    var onNodeAdd = opts.onNodeAdd || onNodeMove;
    var onNodeRemove = opts.onNodeRemove || function () {};
    var onPresence = opts.onPresence || function () {};
    var onStatus = opts.onStatus || function () {};
    var onRevert = opts.onRevert || function () {};

    var ws = null;
    var closed = false;
    var backoff = 500; // ms, doubles up to 10s
    var lastCursorSent = 0;
    var pendingCursor = null;
    var cursorTimer = null;

    var api = {
      cursors: {}, // user_id -> {x, y, name, color, anon}
      users: [],
    };

    function wsUrl() {
      var proto = global.location.protocol === 'https:' ? 'wss:' : 'ws:';
      return (
        proto + '//' + global.location.host +
        '/ws/sala/' + encodeURIComponent(universeKey) +
        '/' + encodeURIComponent(workspaceSlug)
      );
    }

    function connect() {
      if (closed) return;
      try {
        ws = new WebSocket(wsUrl());
      } catch (e) {
        scheduleReconnect();
        return;
      }
      ws.onopen = function () {
        backoff = 500;
        onStatus(true);
      };
      ws.onmessage = function (ev) {
        var msg;
        try { msg = JSON.parse(ev.data); } catch (_) { return; }
        handle(msg);
      };
      ws.onclose = function () {
        onStatus(false);
        ws = null;
        scheduleReconnect();
      };
      ws.onerror = function () {
        // onclose fires next; reconnect handled there.
        if (ws) { try { ws.close(); } catch (_) {} }
      };
    }

    function scheduleReconnect() {
      if (closed) return;
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 10000);
    }

    function handle(msg) {
      switch (msg.type) {
        case 'snapshot':
          api.users = msg.users || [];
          // Reset cursors; keep only users still present.
          var present = {};
          api.users.forEach(function (u) { present[u.user_id] = true; });
          Object.keys(api.cursors).forEach(function (id) {
            if (!present[id]) delete api.cursors[id];
          });
          onPresence(api.users);
          break;
        case 'user_join':
          if (msg.user) {
            api.users = api.users.filter(function (u) { return u.user_id !== msg.user.user_id; });
            api.users.push(msg.user);
            onPresence(api.users);
          }
          break;
        case 'user_leave':
          api.users = api.users.filter(function (u) { return u.user_id !== msg.user_id; });
          delete api.cursors[msg.user_id];
          onPresence(api.users);
          break;
        case 'cursor':
          var u = findUser(msg.user_id);
          api.cursors[msg.user_id] = {
            x: msg.x, y: msg.y,
            name: u ? u.name : msg.user_id,
            color: u ? u.color : '#888888',
            anon: u ? u.anon : true,
            t: Date.now(),
          };
          break;
        case 'node_move':
          onNodeMove(msg.entry_path, msg.x, msg.y);
          break;
        case 'node_add':
          onNodeAdd(msg.entry_path, msg.x, msg.y);
          break;
        case 'node_remove':
          onNodeRemove(msg.entry_path);
          break;
        case 'revert':
          onRevert(msg.op_id);
          break;
        default:
          break;
      }
    }

    function findUser(id) {
      for (var i = 0; i < api.users.length; i++) {
        if (api.users[i].user_id === id) return api.users[i];
      }
      return null;
    }

    function rawSend(obj) {
      if (ws && ws.readyState === 1) {
        try { ws.send(JSON.stringify(obj)); return true; } catch (_) {}
      }
      return false;
    }

    // Cursor: throttled to ~20 Hz; coalesces the latest position.
    api.sendCursor = function (x, y) {
      pendingCursor = { type: 'cursor', x: x, y: y };
      var now = Date.now();
      var since = now - lastCursorSent;
      if (since >= CURSOR_MIN_INTERVAL_MS) {
        flushCursor();
      } else if (!cursorTimer) {
        cursorTimer = setTimeout(flushCursor, CURSOR_MIN_INTERVAL_MS - since);
      }
    };

    function flushCursor() {
      cursorTimer = null;
      if (!pendingCursor) return;
      if (rawSend(pendingCursor)) lastCursorSent = Date.now();
      pendingCursor = null;
    }

    api.sendNodeMove = function (entryPath, x, y, opId) {
      rawSend({ type: 'node_move', entry_path: entryPath, x: x, y: y, op_id: opId || '' });
    };
    api.sendNodeAdd = function (entryPath, x, y, opId) {
      rawSend({ type: 'node_add', entry_path: entryPath, x: x, y: y, op_id: opId || '' });
    };
    api.sendNodeRemove = function (entryPath, opId) {
      rawSend({ type: 'node_remove', entry_path: entryPath, op_id: opId || '' });
    };
    api.sendSuggest = function (entryId, status) {
      rawSend({ type: 'suggest', entry_id: entryId, status: status || 'draft' });
    };
    api.sendPublish = function (entryId) {
      rawSend({ type: 'publish', entry_id: entryId });
    };

    api.close = function () {
      closed = true;
      if (cursorTimer) { clearTimeout(cursorTimer); cursorTimer = null; }
      if (ws) { try { ws.close(); } catch (_) {} ws = null; }
    };

    connect();
    return api;
  }

  global.SalaRealtime = SalaRealtime;
})(window);
