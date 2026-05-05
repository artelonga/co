/**
 * CO Telemetry — CO-46
 *
 * Privacy-respecting client-side event tracking.
 * - Respects navigator.doNotTrack === '1'
 * - Respects cross-product opt-out via the apex cookie al_optout=1 on
 *   .artelonga.com.br (set by artelonga.com.br's analytics client; honored here
 *   so a single opt-out covers both surfaces).
 * - Requires cookie consent (co_cookie_consent in localStorage)
 * - No PII: no email, no passwords, no entry content
 * - Session ID: random, stored in sessionStorage (expires on tab close)
 */
(function () {
  'use strict';

  // --- Guards ---

  function readCookie(name) {
    var esc = name.replace(/[^a-zA-Z0-9_-]/g, '\\$&');
    var m = (document.cookie || '').match(new RegExp('(?:^|; )' + esc + '=([^;]*)'));
    if (!m) return null;
    try { return decodeURIComponent(m[1]); } catch (_) { return null; }
  }

  function isTrackingAllowed() {
    // Do Not Track: honour the browser signal
    if (navigator.doNotTrack === '1' || navigator.doNotTrack === 'yes') return false;
    // Cross-product opt-out: respect the apex-domain cookie set by artelonga.com.br
    if (readCookie('al_optout') === '1') return false;
    // Cookie consent must be accepted
    if (!localStorage.getItem('co_cookie_consent')) return false;
    return true;
  }

  // --- Session ID (random, per tab, expires on close) ---

  function getSessionId() {
    let sid = sessionStorage.getItem('co_session_id');
    if (!sid) {
      sid = Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
      sessionStorage.setItem('co_session_id', sid);
    }
    return sid;
  }

  // --- Screen size bucket ---

  function screenBucket() {
    const w = window.innerWidth || screen.width;
    if (w < 768) return 'small';
    if (w < 1280) return 'medium';
    return 'large';
  }

  // --- Core track function ---

  /**
   * Track an event. Safe to call before consent — will be silently dropped.
   *
   * @param {string} eventName  dot.separated name, e.g. 'task.create'
   * @param {Object} [props]    event-specific properties (no PII)
   */
  window.coTrack = function track(eventName, props) {
    if (!isTrackingAllowed()) return;

    const body = {
      event_name: eventName,
      event_type: eventTypeFrom(eventName),
      path: location.pathname,
      session_id: getSessionId(),
      properties: props || null,
    };

    // universe slug from /{slug}/...
    const RESERVED = ['admin', 'settings', 'yggdrasil', 'static', 'health', '_app', 'api'];
    const m = location.pathname.match(/^\/([^/]+)/);
    if (m && !RESERVED.includes(m[1])) body.universe_key = m[1];

    // sendBeacon with a string body sends Content-Type: text/plain (or none),
    // which the server's Json extractor rejects with 415. Use a Blob with
    // explicit application/json type so the beacon carries the right CT.
    navigator.sendBeacon
      ? navigator.sendBeacon(
          '/api/v1/telemetry/event',
          new Blob([JSON.stringify(body)], { type: 'application/json' })
        )
      : fetch('/api/v1/telemetry/event', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          keepalive: true,
        }).catch(function () {});
  };

  function eventTypeFrom(name) {
    if (name.startsWith('page.')) return 'pageview';
    if (name.startsWith('error.') || name === 'js.error') return 'error';
    if (name.startsWith('perf.') || name.startsWith('performance.')) return 'performance';
    return 'interaction';
  }

  // --- Auto-track: page views ---

  function trackPageView() {
    if (!isTrackingAllowed()) return;
    var t = performance && performance.timing;
    var loadMs = t && t.loadEventEnd > 0
      ? t.loadEventEnd - t.navigationStart
      : null;
    var ttiMs = t && t.domInteractive > 0
      ? t.domInteractive - t.navigationStart
      : null;

    window.coTrack('page.view', {
      referrer: document.referrer || null,
      theme: localStorage.getItem('co_theme') || null,
      lang: localStorage.getItem('co_lang') || document.documentElement.lang || null,
      screen: screenBucket(),
      load_ms: loadMs,
      tti_ms: ttiMs,
    });
  }

  // --- Auto-track: JavaScript errors ---

  window.addEventListener('error', function (e) {
    window.coTrack('js.error', {
      message: e.message ? e.message.slice(0, 120) : null,
      source: e.filename ? e.filename.replace(location.origin, '') : null,
      line: e.lineno || null,
    });
  });

  // --- Auto-track: performance (Web Vitals via PerformanceObserver) ---

  if (typeof PerformanceObserver !== 'undefined') {
    // Largest Contentful Paint
    try {
      new PerformanceObserver(function (list) {
        var entries = list.getEntries();
        var last = entries[entries.length - 1];
        if (last) {
          window.coTrack('perf.lcp', {
            lcp_ms: Math.round(last.startTime),
          });
        }
      }).observe({ type: 'largest-contentful-paint', buffered: true });
    } catch (_) {}

    // First Input Delay
    try {
      new PerformanceObserver(function (list) {
        list.getEntries().forEach(function (entry) {
          window.coTrack('perf.fid', {
            fid_ms: Math.round(entry.processingStart - entry.startTime),
          });
        });
      }).observe({ type: 'first-input', buffered: true });
    } catch (_) {}
  }

  // --- Auto-track: page view on DOMContentLoaded ---

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () {
      // Wait for load to capture timing
      window.addEventListener('load', trackPageView, { once: true });
    });
  } else {
    // Already loaded (script deferred / at end of body)
    if (document.readyState === 'complete') {
      trackPageView();
    } else {
      window.addEventListener('load', trackPageView, { once: true });
    }
  }

  // --- Re-enable tracking when consent is given ---
  // If the user accepts cookies after the script loads, start tracking.
  var _origAccept = null;
  (function patchConsentButton() {
    var btn = document.getElementById('cookie-accept');
    if (!btn) return;
    btn.addEventListener('click', function () {
      // Small delay to let localStorage be written first
      setTimeout(trackPageView, 100);
    }, { once: true });
  })();

  void _origAccept; // suppress unused warning
})();
