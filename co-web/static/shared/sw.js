// Co — Service Worker (PWA offline support)
//
// Strategy:
//   - **Network-first** for HTML, JS, and CSS so a deployed change reaches the
//     user the next request, not "next-next request after a stale-while-revalidate
//     cycle." Falls back to cache only when the network is unreachable.
//   - **Cache-first** for icons, fonts, and the manifest, since those rarely
//     change and benefit from offline availability.
//   - **Network-first** for /api/* (no offline fallback — APIs need fresh data).
//
// Bump CACHE_NAME on every behaviour change so existing clients purge old
// caches when the new SW activates.
const CACHE_NAME = 'co-v3-network-first';
const STATIC_ASSETS = [
  '/shared/manifest.json',
  '/favicon.svg',
];

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

self.addEventListener('fetch', (e) => {
  const req = e.request;
  const url = new URL(req.url);

  // Only handle GETs.
  if (req.method !== 'GET') return;

  // API: network-first, no cache fallback (don't serve stale data quietly).
  if (url.pathname.startsWith('/api/')) {
    e.respondWith(
      fetch(req).catch(() =>
        new Response(
          JSON.stringify({ error: 'offline', message: 'No connection' }),
          { status: 503, headers: { 'Content-Type': 'application/json' } }
        )
      )
    );
    return;
  }

  // HTML / JS / CSS: network-first so deploys are picked up on next request.
  // Falls back to cache only if the network errors (offline).
  const isAppShell =
    url.pathname === '/' ||
    url.pathname.endsWith('.html') ||
    url.pathname.endsWith('.js') ||
    url.pathname.endsWith('.css');
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
        .catch(() => caches.match(req))
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

// Allow the page to ask the SW to skip waiting (used after a deploy).
self.addEventListener('message', (e) => {
  if (e.data && e.data.type === 'SKIP_WAITING') {
    self.skipWaiting();
  }
});
