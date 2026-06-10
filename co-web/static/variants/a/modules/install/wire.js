// CO — PWA install prompt (CO-357)
//
// Handles beforeinstallprompt (Android Chrome) and the iOS Safari manual tip.
// Dismissal stored in localStorage; re-prompts after 30 days.

(function () {
  'use strict';

  const DISMISSED_KEY = 'co.installPromptDismissed';
  const THIRTY_DAYS = 30 * 24 * 60 * 60 * 1000;

  function wasDismissedRecently() {
    const ts = localStorage.getItem(DISMISSED_KEY);
    if (!ts) return false;
    return Date.now() - Number(ts) < THIRTY_DAYS;
  }

  function dismiss() {
    localStorage.setItem(DISMISSED_KEY, String(Date.now()));
    if (typeof window.coTrack === 'function') {
      window.coTrack('pwa_install_prompt_dismissed');
    }
  }

  function isIosSafari() {
    const ua = navigator.userAgent;
    return /iP(hone|ad|od)/.test(ua) && /WebKit/.test(ua) && !/CriOS|FxiOS/.test(ua);
  }

  function isInStandaloneMode() {
    return window.matchMedia('(display-mode: standalone)').matches ||
      window.navigator.standalone === true;
  }

  // ===== Android Chrome / Chromium-based =====

  let deferredPrompt = null;

  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    deferredPrompt = e;

    if (wasDismissedRecently()) return;

    const wrap = document.getElementById('pwa-install-wrap');
    if (wrap) {
      wrap.style.display = '';
      if (typeof window.coTrack === 'function') {
        window.coTrack('pwa_install_prompt_shown', { method: 'beforeinstallprompt' });
      }
    }
  });

  // Show tip on install button click.
  document.addEventListener('click', (e) => {
    if (e.target.closest('#btn-install-pwa')) {
      const tip = document.getElementById('pwa-install-tip');
      if (tip) tip.style.display = tip.style.display === 'none' ? 'block' : 'none';
    }

    // Confirm install.
    if (e.target.closest('#pwa-install-confirm') && deferredPrompt) {
      deferredPrompt.prompt();
      deferredPrompt.userChoice.then((choice) => {
        if (choice.outcome === 'dismissed') {
          dismiss();
        }
        deferredPrompt = null;
        const wrap = document.getElementById('pwa-install-wrap');
        if (wrap) wrap.style.display = 'none';
      });
    }

    // Dismiss when clicking outside the prompt.
    if (!e.target.closest('#pwa-install-wrap')) {
      const tip = document.getElementById('pwa-install-tip');
      if (tip) tip.style.display = 'none';
    }
  });

  window.addEventListener('appinstalled', () => {
    const wrap = document.getElementById('pwa-install-wrap');
    if (wrap) wrap.style.display = 'none';
    if (typeof window.coTrack === 'function') {
      window.coTrack('pwa_installed');
    }
  });

  // ===== iOS Safari one-shot tip =====

  if (isIosSafari() && !isInStandaloneMode() && !wasDismissedRecently()) {
    const IOS_TIP_KEY = 'co.iosTipShown';
    if (!localStorage.getItem(IOS_TIP_KEY)) {
      localStorage.setItem(IOS_TIP_KEY, '1');
      const tip = document.createElement('div');
      tip.id = 'ios-install-tip';
      tip.setAttribute('role', 'tooltip');
      tip.style.cssText = [
        'position:fixed', 'bottom:72px', 'left:50%', 'transform:translateX(-50%)',
        'background:#1e293b', 'color:#fff', 'border-radius:12px',
        'padding:12px 16px', 'font-size:13px', 'line-height:1.5',
        'max-width:280px', 'text-align:center', 'z-index:9999',
        'box-shadow:0 8px 24px rgba(0,0,0,.3)',
      ].join(';');
      tip.innerHTML = [
        '<strong style="display:block;margin-bottom:4px">Instalar CO</strong>',
        'Toque em <strong>Compartilhar</strong> ↑ e depois em',
        '<strong>Adicionar à Tela de Início</strong>.',
        '<button id="ios-tip-close" style="display:block;margin:10px auto 0;',
        'background:rgba(255,255,255,.15);border:none;color:#fff;',
        'border-radius:6px;padding:6px 20px;cursor:pointer;font-size:12px">OK</button>',
      ].join(' ');
      document.body.appendChild(tip);

      if (typeof window.coTrack === 'function') {
        window.coTrack('pwa_install_prompt_shown', { method: 'ios_tip' });
      }

      document.getElementById('ios-tip-close').addEventListener('click', () => {
        tip.remove();
        dismiss();
      });

      setTimeout(() => tip.remove(), 12000);
    }
  }
})();
