/* CO-201: Web Push service worker. Handles push events and notification clicks. */

self.addEventListener('push', (event) => {
    const payload = event.data ? event.data.json() : {};
    const title = payload.title || 'CO';
    const options = {
        body: payload.body || 'Nova notificação',
        icon: '/static/icon-192.png',
        badge: '/static/badge-72.png',
        data: { url: payload.url || '/' },
        tag: payload.tag || 'co-notification',
        renotify: false,
    };
    event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    const url = event.notification.data?.url || '/';
    event.waitUntil(
        clients.matchAll({ type: 'window' }).then((clientList) => {
            for (const client of clientList) {
                if (client.url.endsWith(url) && 'focus' in client) {
                    return client.focus();
                }
            }
            if (clients.openWindow) {
                return clients.openWindow(url);
            }
        }),
    );
});
