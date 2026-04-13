// Co — Service Worker (PWA offline support)
const CACHE_NAME = 'co-v1';
const STATIC_ASSETS = [
  '/',
  '/style.css',
  '/app.js',
  '/shared/i18n.js',
  '/shared/markdown.js',
  '/shared/experiment.js',
  '/shared/manifest.json',
];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE_NAME).then(cache => cache.addAll(STATIC_ASSETS))
  );
  self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys().then(keys =>
      Promise.all(keys.filter(k => k !== CACHE_NAME).map(k => caches.delete(k)))
    )
  );
  self.clients.claim();
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);

  // API calls: network-first
  if (url.pathname.startsWith('/api/')) {
    e.respondWith(
      fetch(e.request).catch(() => caches.match(e.request))
    );
    return;
  }

  // Static assets: cache-first, fallback to network
  e.respondWith(
    caches.match(e.request).then(cached => {
      if (cached) {
        // Refresh cache in background
        fetch(e.request).then(resp => {
          if (resp.ok) {
            caches.open(CACHE_NAME).then(cache => cache.put(e.request, resp));
          }
        }).catch(() => {});
        return cached;
      }
      return fetch(e.request);
    })
  );
});
