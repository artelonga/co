/* CO-201: Web Push notification client module. */

/**
 * Register the push service worker. Call once on app init.
 * Returns the ServiceWorkerRegistration or null if unsupported.
 */
export async function setupPushNotifications() {
    if (!('serviceWorker' in navigator) || !('PushManager' in window)) return null;
    try {
        const reg = await navigator.serviceWorker.register('/push-sw.js');
        return reg;
    } catch (err) {
        console.warn('[push] SW registration failed:', err);
        return null;
    }
}

/**
 * Request notification permission and subscribe to push.
 * POSTs the subscription to the server.
 * Returns the push subscription or null if permission denied / unsupported.
 */
export async function subscribeToPush() {
    if (!('serviceWorker' in navigator) || !('PushManager' in window)) return null;

    const permission = await Notification.requestPermission();
    if (permission !== 'granted') return null;

    const reg = await navigator.serviceWorker.ready;

    const vapidResp = await fetch('/api/v1/notifications/vapid-public-key');
    if (!vapidResp.ok) return null;
    const { key } = await vapidResp.json();

    const sub = await reg.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: urlBase64ToUint8Array(key),
    });

    const keys = sub.toJSON().keys || {};
    await fetch('/api/v1/me/push-subscriptions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({
            endpoint: sub.endpoint,
            p256dh: keys.p256dh || '',
            auth: keys.auth || '',
            user_agent: navigator.userAgent.slice(0, 256),
        }),
    });

    return sub;
}

/**
 * Unsubscribe from push on this device.
 * Removes the subscription from the server if sub_id is known.
 */
export async function unsubscribeFromPush(subId) {
    if ('serviceWorker' in navigator) {
        const reg = await navigator.serviceWorker.ready;
        const sub = await reg.pushManager.getSubscription();
        if (sub) await sub.unsubscribe();
    }
    if (subId) {
        await fetch(`/api/v1/me/push-subscriptions/${subId}`, {
            method: 'DELETE',
            credentials: 'include',
        });
    }
}

/**
 * Check current push permission state without prompting.
 * Returns 'granted' | 'denied' | 'default' | 'unsupported'.
 */
export function pushPermissionState() {
    if (!('Notification' in window)) return 'unsupported';
    return Notification.permission;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function urlBase64ToUint8Array(base64String) {
    const padding = '='.repeat((4 - (base64String.length % 4)) % 4);
    const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/');
    const rawData = atob(base64);
    return Uint8Array.from(rawData, (c) => c.charCodeAt(0));
}
