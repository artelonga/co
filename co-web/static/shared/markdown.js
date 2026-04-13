/**
 * CO Markdown Pipeline — co-web/static/shared/markdown.js
 *
 * Single source of truth for markdown rendering across the CO web app.
 * Exposes window.CoMarkdown with:
 *   - renderMarkdown(text, opts)       → sanitized HTML string
 *   - extractFirstParagraph(text)      → plain-text preview for cards
 *   - extractFrontmatter(text)         → { frontmatter, body }
 *   - wordCount(text)                  → integer
 *   - readingTime(text)                → minutes (integer)
 *   - headingCount(text)               → integer
 *
 * Full HTML rendering delegates to window.CoEditor.renderMarkdown (marked +
 * DOMPurify, loaded lazily with the editor bundle). The lightweight fallback
 * handles basic paragraph wrapping and is safe for anonymous/initial renders.
 *
 * Pure-browser — no Node dependency. Works in Capacitor/Electron shells.
 */
(function (global) {
  'use strict';

  // ===== Frontmatter =====

  function extractFrontmatter(text) {
    if (!text || !text.startsWith('---')) return { frontmatter: {}, body: text || '' };
    const nl = text.indexOf('\n');
    if (nl === -1) return { frontmatter: {}, body: text };
    const rest = text.slice(nl + 1);
    const end = rest.search(/^---/m);
    if (end === -1) return { frontmatter: {}, body: text };
    const yamlStr = rest.slice(0, end);
    const body = rest.slice(end).replace(/^---\n?/, '');
    const frontmatter = {};
    for (const line of yamlStr.split('\n')) {
      const m = line.match(/^([\w-]+)\s*:\s*(.*)$/);
      if (m) {
        let val = m[2].trim();
        val = val.replace(/^["']|["']$/g, '');
        frontmatter[m[1]] = val;
      }
    }
    return { frontmatter, body };
  }

  // ===== Text utilities =====

  /**
   * Returns the first content paragraph as plain text (markdown stripped).
   * Used for kanban card previews — never shows raw ** or \n escapes.
   */
  function extractFirstParagraph(text) {
    if (!text) return '';
    // Delegate to CoEditor version when available (same implementation, tree-shaken)
    if (global.CoEditor && typeof global.CoEditor.extractFirstParagraph === 'function') {
      return global.CoEditor.extractFirstParagraph(text);
    }
    const { body } = extractFrontmatter(text);
    for (const line of body.split('\n')) {
      const t = line.trim();
      if (!t) continue;
      if (/^#{1,6}\s/.test(t)) continue;
      if (t.startsWith('```')) continue;
      if (/^[-*_]{3,}$/.test(t)) continue;
      return t
        .replace(/!\[[^\]]*\]\([^)]+\)/g, '')
        .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
        .replace(/\*\*(.*?)\*\*/g, '$1')
        .replace(/\*(.*?)\*/g, '$1')
        .replace(/~~(.*?)~~/g, '$1')
        .replace(/`([^`]+)`/g, '$1')
        .replace(/\[\[([^\]|]+)(?:\|[^\]]+)?\]\]/g, '$1')
        .trim();
    }
    return '';
  }

  function wordCount(text) {
    if (!text) return 0;
    if (global.CoEditor && typeof global.CoEditor.wordCount === 'function') {
      return global.CoEditor.wordCount(text);
    }
    const { body } = extractFrontmatter(text);
    const clean = body.replace(/```[\s\S]*?```/g, '').replace(/`[^`]+`/g, '');
    return clean.trim().split(/\s+/).filter(Boolean).length;
  }

  function readingTime(text) {
    if (global.CoEditor && typeof global.CoEditor.readingTime === 'function') {
      return global.CoEditor.readingTime(text);
    }
    return Math.max(1, Math.ceil(wordCount(text) / 200));
  }

  function headingCount(text) {
    if (!text) return 0;
    if (global.CoEditor && typeof global.CoEditor.headingCount === 'function') {
      return global.CoEditor.headingCount(text);
    }
    const { body } = extractFrontmatter(text);
    return (body.match(/^#{1,6}\s/gm) || []).length;
  }

  // ===== Rendering =====

  /**
   * Render markdown to sanitized HTML.
   * Uses CoEditor.renderMarkdown (marked + DOMPurify) when the editor bundle
   * has been loaded; falls back to lightweight paragraph-only renderer otherwise.
   */
  function renderMarkdown(text, opts) {
    if (global.CoEditor && typeof global.CoEditor.renderMarkdown === 'function') {
      return global.CoEditor.renderMarkdown(text, opts);
    }
    return _fallbackRender(text);
  }

  function _escHtml(s) {
    return s
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  /**
   * Lightweight fallback: renders paragraphs and code blocks with basic HTML
   * escaping. No inline formatting. Used before the editor bundle loads.
   */
  function _inlineMd(s) {
    // Apply inline markdown to already-escaped HTML.
    // Order matters: images before links (![]() vs []()).
    return s
      .replace(/!\[([^\]]*)\]\(([^)]+)\)/g, '<img src="$2" alt="$1" loading="lazy" class="md-img">')
      .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>')
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      .replace(/\*(.+?)\*/g, '<em>$1</em>')
      .replace(/`([^`]+?)`/g, '<code>$1</code>');
  }

  function _fallbackRender(text) {
    if (!text) return '';
    const { body } = extractFrontmatter(text);
    const blocks = body.split(/\n\n+/).map(p => {
      const t = p.trim();
      if (!t) return '';

      // Code block
      if (t.startsWith('```')) {
        const inner = t.replace(/^```[^\n]*\n?/, '').replace(/```$/, '');
        return `<pre><code>${_escHtml(inner)}</code></pre>`;
      }

      // Heading
      const h = t.match(/^(#{1,6})\s+(.+)$/);
      if (h) {
        const level = h[1].length;
        return `<h${level}>${_inlineMd(_escHtml(h[2]))}</h${level}>`;
      }

      // Blockquote
      if (t.startsWith('> ')) {
        const lines = t.split('\n').map(l => l.replace(/^>\s?/, '')).join(' ');
        return `<blockquote><p>${_inlineMd(_escHtml(lines))}</p></blockquote>`;
      }

      // List (unordered or ordered)
      if (/^[-*]\s/.test(t) || /^\d+\.\s/.test(t)) {
        const items = t.split('\n').filter(l => l.trim()).map(l => {
          const itemText = l.replace(/^[-*]\s+/, '').replace(/^\d+\.\s+/, '');
          return `<li>${_inlineMd(_escHtml(itemText))}</li>`;
        });
        const ordered = /^\d+\.\s/.test(t);
        return ordered ? `<ol>${items.join('')}</ol>` : `<ul>${items.join('')}</ul>`;
      }

      // Image-only block (no <p> wrapper for valid HTML)
      if (/^!\[[^\]]*\]\([^)]+\)$/.test(t)) {
        return `<figure class="md-figure">${_inlineMd(t)}</figure>`;
      }

      // Horizontal rule
      if (/^[-*_]{3,}$/.test(t)) return '<hr>';

      // Paragraph (with inline formatting)
      return `<p>${_inlineMd(_escHtml(t).replace(/\n/g, '<br>'))}</p>`;
    });
    return blocks.filter(Boolean).join('\n');
  }

  // ===== Wikilink resolution =====

  /**
   * Replace [[wikilinks]] in rendered HTML with anchor elements pointing to
   * entries in the current universe.
   *
   * @param {string} html       - Already-rendered HTML from renderMarkdown()
   * @param {string} universeSlug - Current universe key/slug
   * @returns {string} HTML with wikilinks resolved
   */
  function resolveWikilinks(html, universeSlug) {
    if (!html) return html;
    // Wikilinks survive DOMPurify as plain text since [[…]] isn't HTML.
    // After rendering, [[Title]] appears inside text nodes — we look for the
    // literal pattern in the HTML string (safe because DOMPurify already ran).
    return html.replace(/\[\[([^\]|<]+?)(?:\|([^\]<]+?))?\]\]/g, (_, target, label) => {
      const display = _escHtml((label || target).trim());
      const slug = encodeURIComponent(target.trim());
      const href = `/co/${_escHtml(universeSlug)}/entries/${slug}`;
      return `<a href="${href}" class="wikilink" data-wikilink="${_escHtml(target.trim())}">${display}</a>`;
    });
  }

  // ===== Prism lazy loader =====

  let _prismLoaded = false;
  let _prismLoading = null;

  /**
   * Lazy-load PrismJS from CDN and apply syntax highlighting to all
   * <code> blocks inside the given container element.
   *
   * @param {HTMLElement} container
   */
  function highlightCode(container) {
    if (!container) return;
    const blocks = container.querySelectorAll('pre > code');
    if (!blocks.length) return;

    function applyHighlight() {
      if (global.Prism) {
        blocks.forEach(block => global.Prism.highlightElement(block));
      }
    }

    if (_prismLoaded) { applyHighlight(); return; }
    if (_prismLoading) { _prismLoading.then(applyHighlight); return; }

    _prismLoading = new Promise(resolve => {
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.href = 'https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/themes/prism.min.css';
      document.head.appendChild(link);

      const script = document.createElement('script');
      script.src = 'https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/prism.min.js';
      script.onload = () => {
        // Also load common language components
        const autoloader = document.createElement('script');
        autoloader.src = 'https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/plugins/autoloader/prism-autoloader.min.js';
        autoloader.onload = () => { _prismLoaded = true; resolve(); };
        autoloader.onerror = () => { _prismLoaded = true; resolve(); };
        document.head.appendChild(autoloader);
      };
      script.onerror = () => resolve();
      document.head.appendChild(script);
    }).then(applyHighlight);
  }

  // ===== Image zoom =====

  /**
   * Wire up click-to-zoom on all <img> elements inside container.
   */
  function enableImageZoom(container) {
    if (!container) return;
    container.querySelectorAll('img').forEach(img => {
      img.loading = 'lazy';
      img.style.cursor = 'zoom-in';
      img.addEventListener('click', () => {
        const overlay = document.createElement('div');
        overlay.className = 'co-img-zoom-overlay';
        overlay.innerHTML = `<img src="${_escHtml(img.src)}" alt="${_escHtml(img.alt || '')}">`;
        overlay.addEventListener('click', () => overlay.remove());
        document.body.appendChild(overlay);
      });
    });
  }

  // ===== Expose =====

  global.CoMarkdown = {
    renderMarkdown,
    extractFrontmatter,
    extractFirstParagraph,
    wordCount,
    readingTime,
    headingCount,
    resolveWikilinks,
    highlightCode,
    enableImageZoom,
  };
})(window);
