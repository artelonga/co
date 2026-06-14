/**
 * CO-210 — Docs viewer client.
 *
 * Responsibilities (all CSP-safe; this is the page's only script):
 *  - Reader telemetry opt-out: source of truth is
 *    localStorage["co_viewer_telemetry"] === "0". Mirrored to a
 *    `co_viewer_telemetry=0` cookie so the server gates its own page_view /
 *    404_route emission too. A footer checkbox toggles it.
 *  - page_render: fires a client-measured load duration to
 *    /api/v1/telemetry/event (the slow-page signal), respecting opt-out.
 *  - Navigation: mobile dropdown <select> + hamburger sidebar toggle.
 */
(function () {
  'use strict';

  var OPTOUT_KEY = 'co_viewer_telemetry';

  function isOptedOut() {
    try {
      return localStorage.getItem(OPTOUT_KEY) === '0';
    } catch (_) {
      return false;
    }
  }

  // Mirror the localStorage opt-out into a cookie the server can read.
  function syncOptOutCookie() {
    var optedOut = isOptedOut();
    if (optedOut) {
      document.cookie = OPTOUT_KEY + '=0; path=/; max-age=31536000; SameSite=Lax';
    } else {
      // Expire any prior opt-out cookie.
      document.cookie = OPTOUT_KEY + '=; path=/; max-age=0; SameSite=Lax';
    }
  }

  function setOptOut(optedOut) {
    try {
      if (optedOut) {
        localStorage.setItem(OPTOUT_KEY, '0');
      } else {
        localStorage.removeItem(OPTOUT_KEY);
      }
    } catch (_) {}
    syncOptOutCookie();
  }

  function meta() {
    return document.getElementById('docs-meta');
  }

  function track(eventName, eventType, props, durationMs) {
    if (isOptedOut()) return;
    var m = meta();
    var path = m ? m.getAttribute('data-doc-path') : location.pathname;
    var body = {
      event_name: eventName,
      event_type: eventType,
      path: path,
      duration_ms: typeof durationMs === 'number' ? durationMs : null,
      properties: props || null,
    };
    try {
      if (navigator.sendBeacon) {
        navigator.sendBeacon(
          '/api/v1/telemetry/event',
          new Blob([JSON.stringify(body)], { type: 'application/json' })
        );
      } else {
        fetch('/api/v1/telemetry/event', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          keepalive: true,
        }).catch(function () {});
      }
    } catch (_) {}
  }

  function trackPageRender() {
    if (isOptedOut()) return;
    var durationMs = null;
    try {
      var nav = performance.getEntriesByType('navigation')[0];
      if (nav && nav.duration > 0) {
        durationMs = Math.round(nav.duration);
      } else if (performance && performance.now) {
        durationMs = Math.round(performance.now());
      }
    } catch (_) {}
    var referrer = (document.referrer || '').split('?')[0].split('#')[0] || null;
    track('page_render', 'performance', { referrer: referrer }, durationMs);
  }

  function wireNav() {
    var select = document.querySelector('.docs-nav-select');
    if (select) {
      select.addEventListener('change', function () {
        if (select.value) location.href = select.value;
      });
    }
    var toggle = document.querySelector('.docs-nav-toggle');
    var shell = document.querySelector('.docs-shell');
    if (toggle && shell) {
      toggle.addEventListener('click', function () {
        var open = shell.classList.toggle('nav-open');
        toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
    }
  }

  function wireOptOutToggle() {
    var box = document.getElementById('docs-telemetry-optout');
    if (!box) return;
    box.checked = isOptedOut();
    box.addEventListener('change', function () {
      setOptOut(box.checked);
    });
  }

  function init() {
    syncOptOutCookie();
    wireNav();
    wireOptOutToggle();
    trackPageRender();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
