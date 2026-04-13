/**
 * CO Clipper — board UI paste handler and copy-as-Obsidian support
 *
 * Features:
 *   - Ctrl/Cmd+Shift+V → "Paste as CO content" (parse Clipper format)
 *   - On paste: detect Clipper-formatted markdown in clipboard
 *   - Parse frontmatter, show "Paste as task" vs "Paste as content" dialog
 *   - "Copy as Obsidian markdown" option on task cards
 */

(function () {
  'use strict';

  // ─── Frontmatter parser ────────────────────────────────────────────────────

  /**
   * Parse YAML-ish frontmatter from a markdown string.
   * Only handles scalar values and bare arrays (tags: [a, b]).
   * Returns { meta, body } where body is the markdown after the closing ---.
   */
  function parseFrontmatter(text) {
    const meta = {};
    let body = text;

    const match = text.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
    if (!match) return { meta, body };

    const raw = match[1];
    body = match[2];

    for (const line of raw.split('\n')) {
      const kv = line.match(/^(\w[\w-]*):\s*(.*)$/);
      if (!kv) continue;
      const [, key, value] = kv;
      // bare list: [a, b, c]
      const listMatch = value.match(/^\[([^\]]*)\]$/);
      if (listMatch) {
        meta[key] = listMatch[1].split(',').map((s) => s.trim()).filter(Boolean);
      } else {
        meta[key] = value.trim();
      }
    }

    return { meta, body };
  }

  /**
   * Detect whether a string looks like a CO Clipper payload.
   * Requires type: clip in frontmatter.
   */
  function isClipperFormat(text) {
    if (!text || !text.startsWith('---')) return false;
    const { meta } = parseFrontmatter(text);
    return meta.type === 'clip';
  }

  // ─── Obsidian markdown serialiser ─────────────────────────────────────────

  /**
   * Build a Clipper-formatted Obsidian markdown string from a task card object.
   * @param {Object} card  { title, url, author, date, tags, content }
   */
  function toObsidianMarkdown(card) {
    const now = new Date();
    const isoDate = now.toISOString().slice(0, 10);
    const isoFull = now.toISOString().replace(/\.\d+Z$/, 'Z');
    const tags = Array.isArray(card.tags) ? card.tags : (card.tags ? [card.tags] : ['web-clip']);
    const tagStr = '[' + tags.join(', ') + ']';

    return [
      '---',
      `title: ${card.title || ''}`,
      `source: ${card.url || ''}`,
      `author: ${card.author || ''}`,
      `published: ${card.date || isoDate}`,
      `created: ${isoFull}`,
      `tags: ${tagStr}`,
      'type: clip',
      '---',
      '',
      card.content || '',
    ].join('\n');
  }

  // ─── Dialog ────────────────────────────────────────────────────────────────

  function createPasteDialog(meta, body, onChoice) {
    const overlay = document.createElement('div');
    overlay.className = 'clipper-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-label', 'Paste clipped content');

    const title = meta.title || 'Clipped content';
    const source = meta.source || meta.url || '';
    const tagsDisplay = Array.isArray(meta.tags)
      ? meta.tags.join(', ')
      : (meta.tags || '');

    overlay.innerHTML = `
      <div class="clipper-dialog">
        <div class="clipper-dialog-header">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <rect x="8" y="2" width="8" height="4" rx="1" stroke="currentColor" stroke-width="2"/>
            <path d="M6 4H4a2 2 0 00-2 2v14a2 2 0 002 2h16a2 2 0 002-2V6a2 2 0 00-2-2h-2" stroke="currentColor" stroke-width="2"/>
          </svg>
          <span>Colar conteúdo recortado</span>
        </div>
        <div class="clipper-dialog-meta">
          <p class="clipper-title">${escapeHtml(title)}</p>
          ${source ? `<a class="clipper-source" href="${escapeHtml(source)}" target="_blank" rel="noopener">${escapeHtml(source)}</a>` : ''}
          ${tagsDisplay ? `<p class="clipper-tags">${escapeHtml(tagsDisplay)}</p>` : ''}
        </div>
        <div class="clipper-dialog-actions">
          <button class="btn btn-primary" data-action="task">Colar como tarefa</button>
          <button class="btn btn-secondary" data-action="content">Colar como conteúdo</button>
          <button class="btn btn-ghost" data-action="cancel">Cancelar</button>
        </div>
      </div>
    `;

    overlay.addEventListener('click', (e) => {
      const action = e.target.closest('[data-action]')?.dataset.action;
      if (!action) return;
      document.body.removeChild(overlay);
      if (action !== 'cancel') onChoice(action, meta, body);
    });

    // Close on Escape
    const onKey = (e) => {
      if (e.key === 'Escape') {
        document.body.removeChild(overlay);
        document.removeEventListener('keydown', onKey);
      }
    };
    document.addEventListener('keydown', onKey);

    document.body.appendChild(overlay);
    overlay.querySelector('[data-action="task"]').focus();
  }

  function escapeHtml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // ─── Board integration ─────────────────────────────────────────────────────

  /**
   * Called when user chooses how to paste clipped content.
   * Dispatches a custom event that board.js / app.js can listen to.
   */
  function dispatchPasteAction(action, meta, body) {
    const event = new CustomEvent('co:clipper-paste', {
      bubbles: true,
      detail: { action, meta, body },
    });
    document.dispatchEvent(event);
  }

  /**
   * Handle clipboard text that may be Clipper-formatted markdown.
   * Shows dialog if detected, otherwise lets default paste proceed.
   */
  async function handleClipboardText(text, fromShortcut) {
    if (!isClipperFormat(text)) {
      if (fromShortcut) {
        // Shortcut pressed but content is not Clipper format — ignore silently
        return false;
      }
      return false;
    }

    const { meta, body } = parseFrontmatter(text);
    createPasteDialog(meta, body, (action, m, b) => {
      dispatchPasteAction(action, m, b);
    });
    return true;
  }

  // ─── Keyboard shortcut: Ctrl/Cmd+Shift+V ──────────────────────────────────

  document.addEventListener('keydown', async (e) => {
    const isMac = navigator.platform.toUpperCase().includes('MAC');
    const modKey = isMac ? e.metaKey : e.ctrlKey;

    if (!modKey || !e.shiftKey || e.key !== 'V') return;

    e.preventDefault();

    try {
      const text = await navigator.clipboard.readText();
      await handleClipboardText(text, true);
    } catch {
      // Clipboard access denied — nothing to do
    }
  });

  // ─── Paste event on board area ─────────────────────────────────────────────

  document.addEventListener('paste', async (e) => {
    // Only intercept if the paste target is NOT a text input/textarea
    const tag = (e.target?.tagName || '').toLowerCase();
    if (tag === 'input' || tag === 'textarea' || e.target?.isContentEditable) return;

    const text = e.clipboardData?.getData('text/plain') || '';
    if (!text) return;

    const handled = await handleClipboardText(text, false);
    if (handled) e.preventDefault();
  });

  // ─── "Copy as Obsidian markdown" on task cards ─────────────────────────────

  /**
   * Add a "Copy as Obsidian markdown" menu item to task card context menus.
   * Listens for the custom event co:card-context-menu dispatched by board.js.
   *
   * Expected detail: { card: { title, url, author, date, tags, content }, menuEl }
   */
  document.addEventListener('co:card-context-menu', (e) => {
    const { card, menuEl } = e.detail || {};
    if (!card || !menuEl) return;

    const item = document.createElement('button');
    item.className = 'context-menu-item';
    item.textContent = 'Copiar como Obsidian markdown';
    item.addEventListener('click', async () => {
      const md = toObsidianMarkdown(card);
      try {
        await navigator.clipboard.writeText(md);
        showToast('Copiado como Obsidian markdown');
      } catch {
        showToast('Não foi possível copiar', 'error');
      }
      menuEl.remove();
    });

    menuEl.appendChild(item);
  });

  // ─── Toast notification ────────────────────────────────────────────────────

  function showToast(message, type = 'success') {
    const existing = document.querySelector('.clipper-toast');
    if (existing) existing.remove();

    const toast = document.createElement('div');
    toast.className = `clipper-toast clipper-toast--${type}`;
    toast.textContent = message;
    document.body.appendChild(toast);

    requestAnimationFrame(() => toast.classList.add('clipper-toast--visible'));

    setTimeout(() => {
      toast.classList.remove('clipper-toast--visible');
      setTimeout(() => toast.remove(), 300);
    }, 2500);
  }

  // ─── Styles ────────────────────────────────────────────────────────────────

  const style = document.createElement('style');
  style.textContent = `
    .clipper-overlay {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.55);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 10000;
      animation: clipper-fade-in 0.15s ease;
    }
    @keyframes clipper-fade-in {
      from { opacity: 0; }
      to   { opacity: 1; }
    }
    .clipper-dialog {
      background: var(--surface-1, #1e1e2e);
      color: var(--text-1, #cdd6f4);
      border: 1px solid var(--border-color, #313244);
      border-radius: 10px;
      padding: 24px;
      min-width: 320px;
      max-width: 480px;
      width: 90vw;
      box-shadow: 0 8px 32px rgba(0,0,0,0.4);
    }
    .clipper-dialog-header {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.85rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: var(--text-2, #a6adc8);
      margin-bottom: 16px;
    }
    .clipper-dialog-meta {
      margin-bottom: 20px;
    }
    .clipper-title {
      font-size: 1rem;
      font-weight: 600;
      margin: 0 0 6px;
      color: var(--text-1, #cdd6f4);
    }
    .clipper-source {
      display: block;
      font-size: 0.75rem;
      color: var(--accent, #89b4fa);
      text-decoration: none;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      margin-bottom: 6px;
    }
    .clipper-source:hover { text-decoration: underline; }
    .clipper-tags {
      font-size: 0.75rem;
      color: var(--text-2, #a6adc8);
      margin: 0;
    }
    .clipper-dialog-actions {
      display: flex;
      gap: 8px;
      flex-wrap: wrap;
    }
    .clipper-dialog-actions .btn-ghost {
      margin-left: auto;
      color: var(--text-2, #a6adc8);
      background: transparent;
      border: 1px solid var(--border-color, #313244);
    }
    .clipper-toast {
      position: fixed;
      bottom: 24px;
      left: 50%;
      transform: translateX(-50%) translateY(8px);
      background: var(--surface-2, #313244);
      color: var(--text-1, #cdd6f4);
      padding: 10px 20px;
      border-radius: 6px;
      font-size: 0.875rem;
      opacity: 0;
      transition: opacity 0.2s ease, transform 0.2s ease;
      z-index: 10001;
      pointer-events: none;
    }
    .clipper-toast--visible {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
    .clipper-toast--error {
      border-left: 3px solid var(--red, #f38ba8);
    }
  `;
  document.head.appendChild(style);

  // ─── Public API ────────────────────────────────────────────────────────────

  window.COClipper = {
    isClipperFormat,
    parseFrontmatter,
    toObsidianMarkdown,
    handleClipboardText,
  };
})();
