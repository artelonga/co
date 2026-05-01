// Co — offline module — IndexedDB cache + Background Sync
//
// Schema (co-offline-v1):
//   entries        – vault/entry content keyed by [universe_key, path]
//   pending_writes – queued offline PUT/POST, replayed on reconnect
//
// Exposes window.CoOffline.
// Intercepts PUT/POST to /api/v1/universes/*/entries* and /vault/* so the
// app sees optimistic success while the write is queued for Background Sync.
(function () {
  'use strict';

  var DB_NAME = 'co-offline-v1';
  var DB_VERSION = 1;

  var _db = null;

  function openDb() {
    if (_db) return Promise.resolve(_db);
    return new Promise(function (resolve, reject) {
      var req = indexedDB.open(DB_NAME, DB_VERSION);
      req.onupgradeneeded = function (e) {
        var db = e.target.result;
        if (!db.objectStoreNames.contains('entries')) {
          var store = db.createObjectStore('entries', { keyPath: ['universe_key', 'path'] });
          store.createIndex('by_universe', 'universe_key');
          store.createIndex('by_universe_accessed', ['universe_key', 'last_accessed']);
        }
        if (!db.objectStoreNames.contains('pending_writes')) {
          db.createObjectStore('pending_writes', { keyPath: 'id', autoIncrement: true });
        }
      };
      req.onsuccess = function (e) { _db = e.target.result; resolve(_db); };
      req.onerror = function () { reject(req.error); };
    });
  }

  async function getCachedEntry(universeKey, path) {
    try {
      var db = await openDb();
      return await new Promise(function (resolve, reject) {
        var tx = db.transaction('entries', 'readonly');
        var req = tx.objectStore('entries').get([universeKey, path]);
        req.onsuccess = function () { resolve(req.result || null); };
        req.onerror = function () { reject(req.error); };
      });
    } catch (_) { return null; }
  }

  async function setCachedEntry(universeKey, path, content) {
    try {
      var db = await openDb();
      await new Promise(function (resolve, reject) {
        var tx = db.transaction('entries', 'readwrite');
        tx.objectStore('entries').put({
          universe_key: universeKey,
          path: path,
          content: content,
          cached_at: Date.now(),
          last_accessed: Date.now(),
        });
        tx.oncomplete = function () { resolve(); };
        tx.onerror = function () { reject(tx.error); };
      });
    } catch (_) {}
  }

  async function queueWrite(universeKey, path, method, url, body, contentType, authHeader) {
    try {
      var db = await openDb();
      await new Promise(function (resolve, reject) {
        var tx = db.transaction('pending_writes', 'readwrite');
        tx.objectStore('pending_writes').add({
          universe_key: universeKey,
          path: path,
          method: method,
          url: url,
          body: body,
          content_type: contentType || 'application/json',
          auth_header: authHeader || null,
          queued_at: Date.now(),
          retry_count: 0,
        });
        tx.oncomplete = function () { resolve(); };
        tx.onerror = function () { reject(tx.error); };
      });
    } catch (_) {}
  }

  async function getPendingWrites() {
    try {
      var db = await openDb();
      return await new Promise(function (resolve, reject) {
        var tx = db.transaction('pending_writes', 'readonly');
        var req = tx.objectStore('pending_writes').getAll();
        req.onsuccess = function () { resolve(req.result || []); };
        req.onerror = function () { reject(req.error); };
      });
    } catch (_) { return []; }
  }

  async function clearPendingWrite(id) {
    try {
      var db = await openDb();
      await new Promise(function (resolve, reject) {
        var tx = db.transaction('pending_writes', 'readwrite');
        tx.objectStore('pending_writes').delete(id);
        tx.oncomplete = function () { resolve(); };
        tx.onerror = function () { reject(tx.error); };
      });
    } catch (_) {}
  }

  async function getPendingCount() {
    try {
      var db = await openDb();
      return await new Promise(function (resolve, reject) {
        var tx = db.transaction('pending_writes', 'readonly');
        var req = tx.objectStore('pending_writes').count();
        req.onsuccess = function () { resolve(req.result || 0); };
        req.onerror = function () { reject(req.error); };
      });
    } catch (_) { return 0; }
  }

  // Parse universe_key and path from an entries/vault URL.
  function parseEntryUrl(url) {
    var m = url.match(/\/api\/v1\/universes\/([^/?#]+)\/(entries|vault)(\/([^?#]*))?/);
    if (!m) return null;
    try {
      return {
        universeKey: decodeURIComponent(m[1]),
        path: m[4] ? decodeURIComponent(m[4]) : '',
      };
    } catch (_) {
      return { universeKey: m[1], path: m[4] || '' };
    }
  }

  // Flush pending writes — called on `online` event and manual sync button.
  async function flushPendingWrites() {
    var writes = await getPendingWrites();
    var synced = 0;
    for (var i = 0; i < writes.length; i++) {
      var w = writes[i];
      try {
        var hdrs = { 'Content-Type': w.content_type || 'application/json' };
        if (w.auth_header) hdrs['Authorization'] = w.auth_header;
        var resp = await _realFetch(w.url, {
          method: w.method,
          headers: hdrs,
          credentials: 'same-origin',
          body: w.body,
        });
        if (resp.ok) {
          await clearPendingWrite(w.id);
          synced++;
        }
      } catch (_) {
        break; // still offline
      }
    }
    await updateOfflineBanner();
    return synced;
  }

  // Update the floating offline conflict banner.
  async function updateOfflineBanner() {
    var count = await getPendingCount();
    var banner = document.getElementById('offline-sync-banner');
    if (!banner) return;
    var msg = document.getElementById('offline-sync-msg');
    if (count > 0) {
      if (msg) {
        var lang = (window.currentLang === 'en') ? 'en' : 'pt';
        if (lang === 'en') {
          msg.textContent = count === 1
            ? 'You have 1 offline change — review and sync'
            : 'You have ' + count + ' offline changes — review and sync';
        } else {
          msg.textContent = count === 1
            ? 'Você tem 1 alteração offline — revise e sincronize'
            : 'Você tem ' + count + ' alterações offline — revise e sincronize';
        }
      }
      banner.style.display = 'flex';
    } else {
      banner.style.display = 'none';
    }
  }

  // ===== Install prompt =====

  var _installPrompt = null;

  window.addEventListener('beforeinstallprompt', function (e) {
    e.preventDefault();
    _installPrompt = e;
    var btn = document.getElementById('btn-install-pwa');
    if (btn) btn.style.display = '';
  });

  window.addEventListener('appinstalled', function () {
    _installPrompt = null;
    var btn = document.getElementById('btn-install-pwa');
    if (btn) btn.style.display = 'none';
  });

  async function showInstallPrompt() {
    if (!_installPrompt) return false;
    try {
      var result = await _installPrompt.prompt();
      if (result && result.outcome === 'accepted') {
        _installPrompt = null;
        var btn = document.getElementById('btn-install-pwa');
        if (btn) btn.style.display = 'none';
        return true;
      }
    } catch (_) {}
    return false;
  }

  // ===== Fetch intercept for write-through =====

  var _realFetch = window.fetch.bind(window);

  // Intercept PUT/POST to entries and vault paths.
  window.fetch = async function (input, init) {
    var url = (typeof input === 'string')
      ? input
      : (input && typeof input.url === 'string' ? input.url : String(input));
    var reqMethod = ((init && init.method) || (input && input.method) || 'GET').toUpperCase();
    var isWrite = (reqMethod === 'PUT' || reqMethod === 'POST');
    var isEntryPath = /\/api\/v1\/universes\/[^/?#]+\/(entries|vault)(\/|$|\?)/.test(url);

    if (isWrite && isEntryPath) {
      return _offlineWriteIntercept(url, input, init, reqMethod);
    }
    return _realFetch(input, init);
  };

  async function _offlineWriteIntercept(url, input, init, reqMethod) {
    var parsed = parseEntryUrl(url);
    var initBody = (init && init.body) || (input && input.body) || null;
    var body = (typeof initBody === 'string') ? initBody : JSON.stringify(initBody);
    var initHeaders = (init && init.headers) || {};
    var contentType = initHeaders['Content-Type'] || initHeaders['content-type'] || 'application/json';
    var authHeader = initHeaders['Authorization'] || initHeaders['authorization'] || null;

    // For POST (new entry), try to extract path from body JSON.
    if (reqMethod === 'POST' && parsed && !parsed.path && body) {
      try {
        var obj = JSON.parse(body);
        if (obj && obj.path) parsed.path = obj.path;
      } catch (_) {}
    }

    // Write to IDB immediately — optimistic local cache.
    if (parsed && parsed.path) {
      await setCachedEntry(parsed.universeKey, parsed.path, body);
    }

    try {
      var resp = await _realFetch(input, init);
      // On success, refresh banner (a previously queued write may have been synced).
      if (resp.ok) await updateOfflineBanner();
      return resp;
    } catch (_) {
      // Network failure — queue for Background Sync.
      if (parsed) {
        await queueWrite(parsed.universeKey, parsed.path, reqMethod, url, body, contentType, authHeader);
        if ('serviceWorker' in navigator && 'SyncManager' in window) {
          try {
            var reg = await navigator.serviceWorker.ready;
            await reg.sync.register('co-vault-writes');
          } catch (__) {}
        }
        await updateOfflineBanner();
        document.dispatchEvent(new CustomEvent('co:offline-write', { detail: { url: url } }));
      }
      // Return synthetic success so the app's if(result) check passes.
      return new Response(JSON.stringify({ ok: true, offline: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json', 'X-Co-Offline': 'true' },
      });
    }
  }

  // ===== Connectivity and SW message handlers =====

  // Fallback for browsers without Background Sync API.
  window.addEventListener('online', function () {
    flushPendingWrites().catch(function () {});
  });

  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.addEventListener('message', function (e) {
      if (e.data && e.data.type === 'CO_SYNC_COMPLETE') {
        updateOfflineBanner().catch(function () {});
      }
    });
  }

  // ===== DOM wiring (runs after DOMContentLoaded) =====

  function wireDom() {
    updateOfflineBanner().catch(function () {});

    var syncBtn = document.getElementById('btn-sync-now');
    if (syncBtn) {
      syncBtn.addEventListener('click', async function () {
        syncBtn.disabled = true;
        syncBtn.textContent = '...';
        await flushPendingWrites().catch(function () {});
        syncBtn.disabled = false;
        syncBtn.textContent = window.t ? window.t('offline.sync_now') || 'Sincronizar' : 'Sincronizar';
      });
    }

    var dismissBtn = document.getElementById('btn-sync-dismiss');
    if (dismissBtn) {
      dismissBtn.addEventListener('click', function () {
        var banner = document.getElementById('offline-sync-banner');
        if (banner) banner.style.display = 'none';
      });
    }

    var installBtn = document.getElementById('btn-install-pwa');
    if (installBtn) {
      installBtn.addEventListener('click', function () { showInstallPrompt(); });
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', wireDom);
  } else {
    wireDom();
  }

  // Warm up the DB on load.
  openDb().catch(function () {});

  // ===== Public API =====

  window.CoOffline = {
    openDb: openDb,
    getCachedEntry: getCachedEntry,
    setCachedEntry: setCachedEntry,
    queueWrite: queueWrite,
    getPendingWrites: getPendingWrites,
    clearPendingWrite: clearPendingWrite,
    getPendingCount: getPendingCount,
    flushPendingWrites: flushPendingWrites,
    updateOfflineBanner: updateOfflineBanner,
    showInstallPrompt: showInstallPrompt,
  };
})();
