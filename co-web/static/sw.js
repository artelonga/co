// Co — Service Worker (PWA offline support)
//
// Strategy:
//   - **Network-first** for HTML, JS, and CSS so a deployed change reaches the
//     user the next request, not "next-next request after a stale-while-revalidate
//     cycle." Falls back to cache only when the network is unreachable.
//   - **Cache-first** for icons, fonts, and the manifest, since those rarely
//     change and benefit from offline availability.
//   - **IndexedDB read-through** for vault API reads: IDB first, network second,
//     populate cache on success. Enables offline reading of previously visited content.
//   - **Background Sync** via `co-vault-writes` tag: flushes queued offline
//     writes (from offline.js) when connectivity is restored.
//
// Bump CACHE_NAME on every behaviour change so existing clients purge old
// caches when the new SW activates.
const CACHE_NAME = 'co-v6-offline';
const STATIC_ASSETS = [
  '/manifest.json',
  '/favicon.svg',
  '/offline.html',
];

// ===== IndexedDB helpers (SW context) =====
// Mirrors the schema opened by offline.js on the page side.

const IDB_NAME = 'co-offline-v1';
const IDB_VERSION = 1;

function swOpenDb() {
  return new Promise((resolve, reject) => {
    const req = self.indexedDB.open(IDB_NAME, IDB_VERSION);
    req.onupgradeneeded = (e) => {
      const db = e.target.result;
      if (!db.objectStoreNames.contains('entries')) {
        const store = db.createObjectStore('entries', { keyPath: ['universe_key', 'path'] });
        store.createIndex('by_universe', 'universe_key');
        store.createIndex('by_universe_accessed', ['universe_key', 'last_accessed']);
      }
      if (!db.objectStoreNames.contains('pending_writes')) {
        db.createObjectStore('pending_writes', { keyPath: 'id', autoIncrement: true });
      }
    };
    req.onsuccess = (e) => resolve(e.target.result);
    req.onerror = () => reject(req.error);
  });
}

function swGetCachedEntry(db, universeKey, path) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('entries', 'readonly');
    const req = tx.objectStore('entries').get([universeKey, path]);
    req.onsuccess = () => resolve(req.result || null);
    req.onerror = () => reject(req.error);
  });
}

function swSetCachedEntry(db, universeKey, path, content) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('entries', 'readwrite');
    tx.objectStore('entries').put({
      universe_key: universeKey,
      path,
      content,
      cached_at: Date.now(),
      last_accessed: Date.now(),
    });
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

function swGetAllPendingWrites(db) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('pending_writes', 'readonly');
    const req = tx.objectStore('pending_writes').getAll();
    req.onsuccess = () => resolve(req.result || []);
    req.onerror = () => reject(req.error);
  });
}

function swDeletePendingWrite(db, id) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('pending_writes', 'readwrite');
    tx.objectStore('pending_writes').delete(id);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

function parseVaultUrl(pathname) {
  const m = pathname.match(/^\/api\/v1\/universes\/([^/]+)\/vault\/(.+)$/);
  if (!m) return null;
  try {
    return { universeKey: decodeURIComponent(m[1]), path: decodeURIComponent(m[2]) };
  } catch (_) {
    return { universeKey: m[1], path: m[2] };
  }
}

// GET /api/v1/universes/:slug/vault/:path — IndexedDB read-through.
async function handleVaultGet(req, url) {
  const parsed = parseVaultUrl(url.pathname);

  if (parsed) {
    try {
      const db = await swOpenDb();
      const cached = await swGetCachedEntry(db, parsed.universeKey, parsed.path);
      if (cached) {
        return new Response(cached.content, {
          status: 200,
          headers: {
            'Content-Type': 'text/plain; charset=utf-8',
            'X-Co-Source': 'indexeddb',
          },
        });
      }
    } catch (_) {}
  }

  // IDB miss — try network.
  try {
    const resp = await fetch(req.clone());
    if (resp.ok && parsed) {
      const content = await resp.clone().text();
      try {
        const db = await swOpenDb();
        await swSetCachedEntry(db, parsed.universeKey, parsed.path, content);
      } catch (_) {}
    }
    return resp;
  } catch (_) {
    return new Response(
      JSON.stringify({ error: 'offline', message: 'No connection and no cached data' }),
      { status: 503, headers: { 'Content-Type': 'application/json' } }
    );
  }
}

// Background Sync: replay pending writes with exponential back-off handled
// by the browser's sync manager. Stop on first network failure so we don't
// overwhelm the server on mass-reconnect.
async function syncPendingWrites() {
  let db;
  try { db = await swOpenDb(); } catch (_) { return; }

  let writes;
  try { writes = await swGetAllPendingWrites(db); } catch (_) { return; }

  for (const w of writes) {
    const hdrs = { 'Content-Type': w.content_type || 'application/json' };
    if (w.auth_header) hdrs['Authorization'] = w.auth_header;
    try {
      const resp = await fetch(w.url, {
        method: w.method,
        headers: hdrs,
        credentials: 'same-origin',
        body: w.body,
      });
      if (resp.ok) {
        await swDeletePendingWrite(db, w.id);
      }
    } catch (_) {
      break; // still offline — let the browser retry via sync manager
    }
  }

  // Notify all open windows so they can refresh the conflict banner.
  const clients = await self.clients.matchAll({ type: 'window' });
  clients.forEach(c => c.postMessage({ type: 'CO_SYNC_COMPLETE' }));
}

// ===== Lifecycle =====

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE_NAME).then(cache => cache.addAll(STATIC_ASSETS))
  );
  self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then(keys =>
        Promise.all(keys.filter(k => k !== CACHE_NAME).map(k => caches.delete(k)))
      )
      .then(() => self.clients.claim())
  );
});

// ===== Fetch =====

self.addEventListener('fetch', (e) => {
  const req = e.request;
  const url = new URL(req.url);

  if (req.method !== 'GET') return;

  // Vault reads: IndexedDB read-through (check IDB first, populate on success).
  if (/^\/api\/v1\/universes\/[^/]+\/vault\//.test(url.pathname)) {
    e.respondWith(handleVaultGet(req, url));
    return;
  }

  // API: network-first with cache fallback (stale read shown when offline).
  if (url.pathname.startsWith('/api/')) {
    e.respondWith(
      fetch(req)
        .then(resp => {
          if (resp.ok) {
            const clone = resp.clone();
            caches.open(CACHE_NAME).then(cache => cache.put(req, clone)).catch(() => {});
          }
          return resp;
        })
        .catch(() => caches.match(req).then(cached => cached || new Response(
          JSON.stringify({ error: 'offline', message: 'No connection' }),
          { status: 503, headers: { 'Content-Type': 'application/json' } }
        )))
    );
    return;
  }

  // HTML / JS / CSS + SPA routes: network-first so deploys are picked up on
  // next request. Falls back to cache only if the network errors (offline).
  // After the v1.43 URL refactor, every non-/api path is a SPA navigation
  // route — they all return index.html and must be network-first so stale
  // HTML doesn't block post-deploy updates.
  const isAppShell =
    !url.pathname.endsWith('.png') &&
    !url.pathname.endsWith('.jpg') &&
    !url.pathname.endsWith('.jpeg') &&
    !url.pathname.endsWith('.webp') &&
    !url.pathname.endsWith('.gif') &&
    !url.pathname.endsWith('.svg') &&
    !url.pathname.endsWith('.pdf');
  if (isAppShell) {
    e.respondWith(
      fetch(req)
        .then(resp => {
          if (resp.ok) {
            const clone = resp.clone();
            caches.open(CACHE_NAME).then(cache => cache.put(req, clone)).catch(() => {});
          }
          return resp;
        })
        .catch(() =>
          caches.match(req).then(cached => {
            if (cached) return cached;
            if (req.mode === 'navigate') {
              return caches.match('/offline.html').then(
                offline => offline || new Response('Offline', { status: 503 })
              );
            }
            return new Response('Offline', { status: 503 });
          })
        )
    );
    return;
  }

  // Everything else (images, fonts, manifest, icons): cache-first.
  e.respondWith(
    caches.match(req).then(cached => {
      if (cached) return cached;
      return fetch(req).then(resp => {
        if (resp.ok && resp.type === 'basic') {
          const clone = resp.clone();
          caches.open(CACHE_NAME).then(cache => cache.put(req, clone)).catch(() => {});
        }
        return resp;
      });
    })
  );
});

// ===== Background Sync =====

self.addEventListener('sync', (e) => {
  if (e.tag === 'co-vault-writes') {
    e.waitUntil(syncPendingWrites());
  }
});

// ===== Messages =====

// Allow the page to ask the SW to skip waiting (used after a deploy).
self.addEventListener('message', (e) => {
  if (e.data && e.data.type === 'SKIP_WAITING') {
    self.skipWaiting();
  }
});
