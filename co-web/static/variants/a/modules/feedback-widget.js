// ===== CO-333: Feedback widget — visitor-facing floating button + modal =====
// Detects universe + entry_path from the current URL or window globals.
// Bottom-left floating button (avoids CO-332 chat button, which is bottom-right).
//
// Public API:
//   CoFeedbackWidget.init()               — auto-called on load
//   CoFeedbackWidget.setEntryPath(path)   — called by SPA when entry changes
//   CoFeedbackWidget.setUniverse(key)     — override universe

const CoFeedbackWidget = (() => {
    let _universe = null;
    let _entryPath = null;
    let _modal = null;

    // -----------------------------------------------------------------------
    // Universe / entry detection
    // -----------------------------------------------------------------------

    function detectUniverse() {
        if (window.__CO_SUBDOMAIN_UNIVERSE__) return window.__CO_SUBDOMAIN_UNIVERSE__;
        const m = window.location.pathname.match(/^\/([a-z0-9-]+)/);
        const RESERVED = ['admin', 'settings', 'yggdrasil', 'static', 'health', '_app', 'notifications', 'graph-views'];
        if (m && !RESERVED.includes(m[1])) return m[1];
        return 'template';
    }

    function detectEntryPath() {
        if (window.__CO_SUBDOMAIN_UNIVERSE__) {
            const p = window.location.pathname;
            if (!p || p === '/') return null;
            return p.slice(1) || null;
        }
        const uni = detectUniverse();
        const prefix = `/${uni}/`;
        const p = window.location.pathname;
        if (!p.startsWith(prefix)) return null;
        return p.slice(prefix.length) || null;
    }

    // -----------------------------------------------------------------------
    // Submit
    // -----------------------------------------------------------------------

    async function submitFeedback(kind, message, name, email, anonymous) {
        const universe = _universe || detectUniverse();
        const entryPath = _entryPath || detectEntryPath();

        const body = { kind, message };
        if (!anonymous) {
            if (name) body.name = name;
            if (email) body.email = email;
        }

        const url = entryPath
            ? `/api/v1/feedback/${encodeURIComponent(universe)}/${entryPath.split('/').map(encodeURIComponent).join('/')}`
            : `/api/v1/feedback`;

        if (!entryPath) body.universe = universe;

        const resp = await fetch(url, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
            credentials: 'include',
        });
        if (!resp.ok) {
            const err = await resp.json().catch(() => ({}));
            throw new Error(err.message || `HTTP ${resp.status}`);
        }
        return resp.json();
    }

    // -----------------------------------------------------------------------
    // Modal
    // -----------------------------------------------------------------------

    function buildModal() {
        const overlay = document.createElement('div');
        overlay.id = 'co-feedback-overlay';
        overlay.style.cssText = [
            'position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:9998',
            'display:flex;align-items:center;justify-content:center',
        ].join(';');

        overlay.innerHTML = `
<div id="co-feedback-modal" style="
    background:var(--card-bg,#fff);color:var(--text,#1a1a1a);
    border-radius:10px;padding:24px;width:min(440px,92vw);
    box-shadow:0 8px 32px rgba(0,0,0,.18);position:relative;z-index:9999">
  <button id="co-fb-close" aria-label="Fechar" style="
      position:absolute;top:12px;right:14px;background:none;border:none;
      font-size:20px;cursor:pointer;color:var(--text-muted,#888)">✕</button>
  <h3 style="margin:0 0 16px;font-size:17px">Deixar feedback</h3>

  <div style="display:flex;gap:8px;margin-bottom:14px" id="co-fb-kind-group">
    <button class="co-fb-kind-btn" data-kind="feedback" style="flex:1">Comentário</button>
    <button class="co-fb-kind-btn" data-kind="duvida"   style="flex:1">Dúvida</button>
    <button class="co-fb-kind-btn" data-kind="sugestao" style="flex:1">Sugestão</button>
  </div>

  <textarea id="co-fb-message" placeholder="Sua mensagem…" rows="4" style="
      width:100%;box-sizing:border-box;resize:vertical;
      border:1px solid var(--border,#d1d5db);border-radius:6px;
      padding:8px 10px;font-size:14px;font-family:inherit;
      background:var(--input-bg,#fff);color:var(--text,#1a1a1a)"></textarea>

  <label style="display:flex;align-items:center;gap:6px;margin:10px 0;font-size:13px;cursor:pointer">
    <input type="checkbox" id="co-fb-anon" style="cursor:pointer">
    Enviar anonimamente
  </label>

  <div id="co-fb-identity" style="display:flex;flex-direction:column;gap:8px;margin-bottom:14px">
    <input id="co-fb-name" type="text" placeholder="Seu nome (opcional)" style="
        border:1px solid var(--border,#d1d5db);border-radius:6px;
        padding:7px 10px;font-size:13px;font-family:inherit;
        background:var(--input-bg,#fff);color:var(--text,#1a1a1a)">
    <input id="co-fb-email" type="email" placeholder="Seu e-mail (opcional)" style="
        border:1px solid var(--border,#d1d5db);border-radius:6px;
        padding:7px 10px;font-size:13px;font-family:inherit;
        background:var(--input-bg,#fff);color:var(--text,#1a1a1a)">
  </div>

  <div style="display:flex;justify-content:flex-end;gap:8px">
    <button id="co-fb-cancel" style="
        padding:8px 16px;border:1px solid var(--border,#d1d5db);border-radius:6px;
        background:none;cursor:pointer;font-size:14px">Cancelar</button>
    <button id="co-fb-submit" style="
        padding:8px 16px;border:none;border-radius:6px;
        background:var(--primary,#3b82f6);color:#fff;
        cursor:pointer;font-size:14px;font-weight:500">Enviar</button>
  </div>
  <p id="co-fb-status" style="margin:10px 0 0;font-size:13px;min-height:18px"></p>
</div>`;

        document.body.appendChild(overlay);

        // Kind selector
        let selectedKind = 'feedback';
        const kindBtns = overlay.querySelectorAll('.co-fb-kind-btn');
        const kindStyle = (btn) => {
            btn.style.cssText = [
                'padding:6px 0;border:1px solid var(--border,#d1d5db);border-radius:6px',
                'cursor:pointer;font-size:13px;background:none',
                btn.dataset.kind === selectedKind
                    ? 'background:var(--primary,#3b82f6);color:#fff;border-color:var(--primary,#3b82f6)'
                    : 'background:none;color:var(--text,#1a1a1a)',
            ].join(';');
        };
        kindBtns.forEach(btn => {
            kindStyle(btn);
            btn.addEventListener('click', () => {
                selectedKind = btn.dataset.kind;
                kindBtns.forEach(kindStyle);
            });
        });

        // Anonymous toggle
        const anonChk = overlay.querySelector('#co-fb-anon');
        const identDiv = overlay.querySelector('#co-fb-identity');
        anonChk.addEventListener('change', () => {
            identDiv.style.display = anonChk.checked ? 'none' : 'flex';
        });

        // Close
        const close = () => { overlay.remove(); _modal = null; };
        overlay.querySelector('#co-fb-close').addEventListener('click', close);
        overlay.querySelector('#co-fb-cancel').addEventListener('click', close);
        overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });

        // Submit
        overlay.querySelector('#co-fb-submit').addEventListener('click', async () => {
            const message = overlay.querySelector('#co-fb-message').value.trim();
            const status = overlay.querySelector('#co-fb-status');
            if (!message) { status.style.color = 'var(--error,#ef4444)'; status.textContent = 'Escreva uma mensagem.'; return; }
            status.style.color = 'var(--text-muted,#888)';
            status.textContent = 'Enviando…';
            overlay.querySelector('#co-fb-submit').disabled = true;
            try {
                await submitFeedback(
                    selectedKind, message,
                    overlay.querySelector('#co-fb-name').value.trim(),
                    overlay.querySelector('#co-fb-email').value.trim(),
                    anonChk.checked,
                );
                status.style.color = 'var(--success,#22c55e)';
                status.textContent = 'Obrigado! Feedback enviado.';
                setTimeout(close, 1800);
            } catch (err) {
                status.style.color = 'var(--error,#ef4444)';
                status.textContent = `Erro: ${err.message}`;
                overlay.querySelector('#co-fb-submit').disabled = false;
            }
        });

        _modal = overlay;
    }

    // -----------------------------------------------------------------------
    // Floating button
    // -----------------------------------------------------------------------

    function mountButton() {
        if (document.getElementById('co-feedback-btn')) return;
        const btn = document.createElement('button');
        btn.id = 'co-feedback-btn';
        btn.title = 'Deixar feedback';
        btn.setAttribute('aria-label', 'Deixar feedback');
        btn.innerHTML = '📩';
        btn.style.cssText = [
            'position:fixed;bottom:20px;left:20px;z-index:9000',
            'width:44px;height:44px;border-radius:50%;border:none',
            'background:var(--primary,#3b82f6);color:#fff',
            'font-size:18px;cursor:pointer;box-shadow:0 2px 8px rgba(0,0,0,.2)',
            'display:flex;align-items:center;justify-content:center',
            'transition:transform .15s,box-shadow .15s',
        ].join(';');
        btn.addEventListener('mouseenter', () => { btn.style.transform = 'scale(1.08)'; btn.style.boxShadow = '0 4px 16px rgba(0,0,0,.28)'; });
        btn.addEventListener('mouseleave', () => { btn.style.transform = ''; btn.style.boxShadow = '0 2px 8px rgba(0,0,0,.2)'; });
        btn.addEventListener('click', () => { if (!_modal) buildModal(); });
        document.body.appendChild(btn);
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    return {
        init() {
            if (document.readyState === 'loading') {
                document.addEventListener('DOMContentLoaded', () => mountButton());
            } else {
                mountButton();
            }
        },
        setEntryPath(path) { _entryPath = path; },
        setUniverse(key) { _universe = key; },
    };
})();

window.CoFeedbackWidget = CoFeedbackWidget;
CoFeedbackWidget.init();

export { CoFeedbackWidget };
