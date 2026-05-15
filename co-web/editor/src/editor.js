/**
 * CO Editor — CodeMirror 6 markdown editor with live preview
 *
 * Public API: initEditor(container, { content, onChange, readOnly, wsUrl, user })
 *
 * When `wsUrl` + `user` are provided (logged-in), Yjs CRDT sync activates.
 * Anonymous sessions work locally without any server connection.
 */

import { EditorState, Compartment } from '@codemirror/state';
import {
  EditorView,
  keymap,
  drawSelection,
  highlightActiveLine,
  placeholder,
} from '@codemirror/view';
import {
  defaultKeymap,
  history,
  historyKeymap,
} from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { yaml } from '@codemirror/lang-yaml';
import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { Marked } from 'marked';
import DOMPurify from 'dompurify';

// Yjs + CRDT
import * as Y from 'yjs';
import { yCollab, yUndoManagerKeymap } from 'y-codemirror.next';
import * as encoding from 'lib0/encoding';
import * as decoding from 'lib0/decoding';

// ===== Universe key for sha256: asset URL rewriting =====

let _universeKey = '';

/** Set current universe key so sha256: image/video URLs resolve to asset API endpoints. */
export function setUniverseKey(key) {
  _universeKey = key || '';
}

function _assetUrl(sha256hex) {
  if (!_universeKey) return '#asset-' + sha256hex;
  return '/api/v1/universes/' + _universeKey + '/assets/' + sha256hex;
}

// ===== Marked =====

const markedInstance = new Marked({ gfm: true, breaks: false });

markedInstance.use({
  renderer: {
    image({ href, title, text }) {
      let src = href || '';
      if (src.startsWith('sha256:')) {
        src = _assetUrl(src.slice(7));
      }
      const titleAttr = title ? ` title="${title}"` : '';
      return `<img src="${src}" alt="${text || ''}"${titleAttr} loading="lazy" decoding="async" class="md-img">`;
    },
    code({ text, lang }) {
      if (lang === 'video') {
        const raw = (text || '').trim();
        const src = raw.startsWith('sha256:') ? _assetUrl(raw.slice(7)) : raw;
        return `<video src="${src}" preload="none" controls style="max-width:100%;border-radius:4px"></video>`;
      }
      if (lang === 'iframe') {
        const src = (text || '').trim();
        return `<iframe src="${src}" loading="lazy" style="width:100%;min-height:300px;border:none;border-radius:4px"></iframe>`;
      }
      return false;
    },
  },
});

export function renderMarkdown(src, opts) {
  const uk = (opts && opts.universeKey) || _universeKey;
  const prev = _universeKey;
  if (uk) _universeKey = uk;
  const html = markedInstance.parse(src || '');
  _universeKey = prev;
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      'p', 'br', 'strong', 'em', 's', 'del', 'code', 'pre', 'blockquote',
      'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
      'ul', 'ol', 'li', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
      'a', 'img', 'video', 'source', 'iframe', 'figure', 'figcaption',
      'hr', 'input', 'span', 'div',
    ],
    ALLOWED_ATTR: [
      'href', 'src', 'alt', 'title', 'type', 'checked', 'disabled',
      'class', 'id', 'loading', 'decoding', 'preload', 'controls',
      'data-zoom', 'style', 'width', 'height',
    ],
    ALLOW_DATA_ATTR: false,
  });
}

// ===== Markdown utilities (exposed via window.CoEditor) =====

export function extractFrontmatter(text) {
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

export function extractFirstParagraph(text) {
  if (!text) return '';
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

export function wordCount(text) {
  if (!text) return 0;
  const { body } = extractFrontmatter(text);
  const clean = body.replace(/```[\s\S]*?```/g, '').replace(/`[^`]+`/g, '');
  return clean.trim().split(/\s+/).filter(Boolean).length;
}

export function readingTime(text) {
  return Math.max(1, Math.ceil(wordCount(text) / 200));
}

export function headingCount(text) {
  if (!text) return 0;
  const { body } = extractFrontmatter(text);
  return (body.match(/^#{1,6}\s/gm) || []).length;
}

// ===== CodeMirror theme =====

const coTheme = EditorView.theme({
  '&': { background: 'var(--card-bg, var(--bg, #fff))', color: 'var(--text-primary, #111827)', height: '100%', fontSize: '14px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { overflow: 'auto', fontFamily: 'var(--font-mono, "SF Mono", Consolas, monospace)', lineHeight: '1.6' },
  '.cm-content': { padding: '12px 16px', caretColor: 'var(--accent, #6366f1)', minHeight: '180px' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent, #6366f1)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { background: 'var(--accent-light, #e0e7ff)' },
  '.cm-activeLine': { backgroundColor: 'rgba(0,0,0,0.03)' },
  '.cm-gutters': { display: 'none' },
  '.cm-line': { padding: '0' },
  // Remote cursor labels (y-codemirror.next)
  '.cm-ySelectionInfo': {
    position: 'absolute', top: '-1.4em', left: '-1px',
    padding: '1px 4px', borderRadius: '4px 4px 4px 0',
    fontSize: '11px', fontFamily: 'var(--font, sans-serif)',
    whiteSpace: 'nowrap', color: '#fff', lineHeight: '1.4',
    pointerEvents: 'none', userSelect: 'none',
  },
}, { dark: false });

const coHighlightStyle = HighlightStyle.define([
  { tag: tags.heading1, fontSize: '1.5em', fontWeight: 'bold', color: 'var(--text-primary, #111)' },
  { tag: tags.heading2, fontSize: '1.3em', fontWeight: 'bold', color: 'var(--text-primary, #111)' },
  { tag: tags.heading3, fontSize: '1.15em', fontWeight: '600', color: 'var(--text-primary, #111)' },
  { tag: [tags.heading4, tags.heading5, tags.heading6], fontWeight: '600', color: 'var(--text-primary, #111)' },
  { tag: tags.strong, fontWeight: 'bold' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through', color: 'var(--text-muted, #9ca3af)' },
  { tag: tags.link, color: 'var(--accent, #6366f1)', textDecoration: 'underline', textDecorationColor: 'var(--accent, #6366f1)' },
  { tag: tags.url, color: 'var(--accent, #6366f1)' },
  { tag: tags.code, fontFamily: 'var(--font-mono, monospace)', background: 'rgba(0,0,0,0.06)', borderRadius: '3px', padding: '0 3px' },
  { tag: tags.monospace, fontFamily: 'var(--font-mono, monospace)' },
  { tag: tags.comment, color: 'var(--text-muted, #9ca3af)', fontStyle: 'italic' },
  { tag: tags.keyword, color: 'var(--accent-hover, #4f46e5)', fontWeight: '500' },
  { tag: tags.string, color: 'var(--text-secondary, #6b7280)' },
  { tag: tags.number, color: 'var(--text-secondary, #6b7280)' },
  { tag: tags.bool, color: 'var(--accent-hover, #4f46e5)' },
  { tag: tags.null, color: 'var(--text-muted, #9ca3af)' },
  { tag: tags.propertyName, color: 'var(--accent, #6366f1)' },
  { tag: tags.punctuation, color: 'var(--text-muted, #9ca3af)' },
  { tag: tags.processingInstruction, color: 'var(--text-secondary, #6b7280)' },
  { tag: tags.meta, color: 'var(--text-muted, #9ca3af)', fontStyle: 'italic' },
]);

// ===== CSS =====

const EDITOR_CSS = `
.co-editor-wrap {
  display: flex; flex-direction: column;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: var(--radius-md, 8px);
  overflow: hidden; background: var(--card-bg, #fff);
  resize: vertical; min-height: 300px; height: 400px;
}
.co-editor-wrap:focus-within {
  border-color: var(--accent, #6366f1);
  box-shadow: 0 0 0 2px var(--accent-light, #e0e7ff);
}
.co-editor-toolbar {
  display: flex; gap: 2px; padding: 6px 8px;
  background: var(--bg, #f0f2f5);
  border-bottom: 1px solid var(--border, #e5e7eb);
  flex-wrap: wrap; align-items: center; flex-shrink: 0;
}
.co-editor-btn {
  background: none; border: none; cursor: pointer;
  padding: 3px 7px; border-radius: var(--radius-sm, 4px);
  font-size: 13px; color: var(--text-secondary, #6b7280);
  font-family: var(--font, sans-serif); line-height: 1.4;
  white-space: nowrap; transition: background 0.1s, color 0.1s; font-weight: 500;
}
.co-editor-btn:hover { background: var(--bg-hover, #e8eaed); color: var(--text-primary, #111827); }
.co-editor-btn.active { background: var(--accent-light, #e0e7ff); color: var(--accent, #6366f1); }
.co-editor-sep { width: 1px; height: 18px; background: var(--border, #e5e7eb); margin: 0 4px; flex-shrink: 0; }
.co-editor-panes { display: flex; flex: 1; overflow: hidden; min-height: 0; }
.co-editor-left { flex: 1; overflow: auto; min-width: 0; display: flex; flex-direction: column; }
.co-editor-left .cm-editor { flex: 1; height: 100%; }
.co-editor-right {
  flex: 1; overflow: auto; padding: 12px 20px;
  border-left: 1px solid var(--border, #e5e7eb);
  background: var(--card-bg, #fff);
  font-family: var(--font, sans-serif); font-size: 14px; line-height: 1.7;
  color: var(--text-primary, #111827); min-width: 0;
}
.co-editor-right.co-preview-hidden { display: none; }
.co-editor-left.co-editor-full { flex: 1 1 100%; }
.co-editor-right h1 { font-size:1.6em; font-weight:bold; margin:.4em 0 .2em; border-bottom:1px solid var(--border,#e5e7eb); padding-bottom:.2em; }
.co-editor-right h2 { font-size:1.3em; font-weight:bold; margin:.8em 0 .2em; }
.co-editor-right h3 { font-size:1.1em; font-weight:600; margin:.8em 0 .2em; }
.co-editor-right h4,.co-editor-right h5,.co-editor-right h6 { font-weight:600; margin:.6em 0 .1em; }
.co-editor-right p { margin:.4em 0; }
.co-editor-right code { font-family:var(--font-mono,monospace); font-size:.9em; background:rgba(0,0,0,.06); padding:1px 4px; border-radius:3px; }
.co-editor-right pre { background:rgba(0,0,0,.05); border-radius:var(--radius-sm,4px); padding:10px 14px; overflow:auto; margin:.5em 0; }
.co-editor-right pre code { background:none; padding:0; }
.co-editor-right blockquote { border-left:3px solid var(--accent,#6366f1); margin:.5em 0; padding:0 0 0 12px; color:var(--text-secondary,#6b7280); }
.co-editor-right ul,.co-editor-right ol { margin:.4em 0; padding-left:1.5em; }
.co-editor-right li { margin:.15em 0; }
.co-editor-right li input[type="checkbox"] { margin-right:6px; }
.co-editor-right table { border-collapse:collapse; width:100%; margin:.5em 0; font-size:.9em; }
.co-editor-right th,.co-editor-right td { border:1px solid var(--border,#e5e7eb); padding:6px 10px; text-align:left; }
.co-editor-right th { background:var(--bg,#f0f2f5); font-weight:600; }
.co-editor-right a { color:var(--accent,#6366f1); text-decoration:underline; }
.co-editor-right hr { border:none; border-top:1px solid var(--border,#e5e7eb); margin:1em 0; }
.co-editor-right img { max-width:100%; border-radius:var(--radius-sm,4px); }
.co-editor-right del { text-decoration:line-through; color:var(--text-muted,#9ca3af); }
@media (max-width:767px) {
  .co-editor-right { display:none; }
  .co-editor-right.co-preview-mobile { display:block; border-left:none; }
  .co-editor-left.co-editor-mobile-hide { display:none; }
  .co-editor-toolbar .co-desktop-only { display:none; }
}
/* Collab badge (connection status + user count) */
.co-collab-badge {
  display:inline-flex; align-items:center; gap:5px;
  font-size:12px; color:var(--text-secondary,#6b7280);
  padding:2px 8px; border-radius:12px;
  background:var(--bg-hover,#e8eaed); margin-left:4px; white-space:nowrap;
}
.co-collab-dot { width:7px; height:7px; border-radius:50%; flex-shrink:0; }
.co-collab-dot.green  { background:#22c55e; }
.co-collab-dot.yellow { background:#eab308; }
.co-collab-dot.grey   { background:#9ca3af; }
/* Toast */
.co-toast {
  position:fixed; bottom:24px; left:50%; transform:translateX(-50%);
  background:#1f2937; color:#f9fafb;
  padding:10px 20px; border-radius:8px; font-size:14px;
  box-shadow:0 4px 16px rgba(0,0,0,.2); z-index:9999;
  display:flex; align-items:center; gap:12px;
  opacity:0; transition:opacity 0.3s; pointer-events:none;
}
.co-toast.visible { opacity:1; pointer-events:auto; }
.co-toast a { color:var(--accent,#6366f1); text-decoration:underline; }
`;

let styleInjected = false;
function injectStyles() {
  if (styleInjected) return;
  styleInjected = true;
  const s = document.createElement('style');
  s.id = 'co-editor-styles';
  s.textContent = EDITOR_CSS;
  document.head.appendChild(s);
}

// ===== Helpers =====

function wrapSelection(view, before, after, cursorOffset) {
  const { state } = view;
  const tr = state.changeByRange(range => {
    const text = state.sliceDoc(range.from, range.to);
    const insert = before + (text || cursorOffset || '') + after;
    const anchor = range.from + before.length;
    const head = range.from + before.length + (text || cursorOffset || '').length;
    return { changes: { from: range.from, to: range.to, insert }, range: { anchor, head } };
  });
  view.dispatch(tr);
  view.focus();
}

function insertLine(view, prefix) {
  const { state } = view;
  const tr = state.changeByRange(range => {
    const line = state.doc.lineAt(range.from);
    const existing = line.text;
    const hasPrefix = existing.trimStart().startsWith(prefix.trim());
    if (hasPrefix) {
      const start = line.from + existing.indexOf(prefix.trim());
      return { changes: { from: start, to: start + prefix.length, insert: '' }, range: { anchor: range.from - prefix.length, head: range.from - prefix.length } };
    }
    return { changes: { from: line.from, to: line.from, insert: prefix }, range: { anchor: range.from + prefix.length, head: range.from + prefix.length } };
  });
  view.dispatch(tr);
  view.focus();
}

// ===== Cursor colour palette =====

const CURSOR_COLORS = [
  '#6366f1', '#ec4899', '#f59e0b', '#10b981', '#3b82f6',
  '#8b5cf6', '#ef4444', '#06b6d4', '#84cc16', '#f97316',
];
let _colorIdx = 0;
function nextColor() { return CURSOR_COLORS[(_colorIdx++) % CURSOR_COLORS.length]; }

// ===== Toast =====

let _toastEl = null;
let _toastTimeout = null;
function showToast(html) {
  if (!_toastEl) {
    _toastEl = document.createElement('div');
    _toastEl.className = 'co-toast';
    document.body.appendChild(_toastEl);
  }
  _toastEl.innerHTML = html;
  _toastEl.classList.add('visible');
  if (_toastTimeout) clearTimeout(_toastTimeout);
  _toastTimeout = setTimeout(() => _toastEl.classList.remove('visible'), 7000);
}

// ===== Minimal awareness shim for y-codemirror.next =====
//
// y-codemirror.next uses awareness for remote cursor rendering.
// Interface: doc, clientID, getLocalState(), setLocalStateField(), getStates(), on(), off()

function createAwareness(ydoc, user) {
  const color = nextColor();
  const listeners = new Map(); // event → Set<fn>
  const states = new Map([[ydoc.clientID, { user: { name: user.name, color } }]]);

  function emit(event, data) {
    const fns = listeners.get(event);
    if (fns) fns.forEach(fn => fn(data));
  }

  return {
    doc: ydoc,
    clientID: ydoc.clientID,

    getLocalState() {
      return states.get(ydoc.clientID) ?? null;
    },

    setLocalStateField(key, value) {
      const cur = states.get(ydoc.clientID) ?? {};
      if (value === null) { delete cur[key]; } else { cur[key] = value; }
      states.set(ydoc.clientID, cur);
      emit('change', { added: [], updated: [ydoc.clientID], removed: [] });
      emit('update', { added: [], updated: [ydoc.clientID], removed: [] });
    },

    getStates() { return states; },

    on(event, fn) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event).add(fn);
    },

    off(event, fn) {
      listeners.get(event)?.delete(fn);
    },

    // Used by CoYjsProvider to encode/decode awareness messages.
    encodeUpdate(clientIDs) {
      try {
        const enc = encoding.createEncoder();
        encoding.writeVarUint(enc, clientIDs.length);
        for (const id of clientIDs) {
          encoding.writeVarUint(enc, id);
          encoding.writeVarUint(enc, 0); // clock (unused)
          const state = states.get(id) ?? null;
          encoding.writeVarString(enc, state === null ? 'null' : JSON.stringify(state));
        }
        return encoding.toUint8Array(enc);
      } catch { return new Uint8Array(0); }
    },

    applyUpdate(update) {
      try {
        const dec = decoding.createDecoder(update);
        const n = decoding.readVarUint(dec);
        const changed = { added: [], updated: [], removed: [] };
        for (let i = 0; i < n; i++) {
          const id = decoding.readVarUint(dec);
          decoding.readVarUint(dec); // clock
          const stateStr = decoding.readVarString(dec);
          const state = stateStr === 'null' ? null : JSON.parse(stateStr);
          if (id === ydoc.clientID) continue; // skip own updates
          if (state === null) {
            states.delete(id);
            changed.removed.push(id);
          } else if (states.has(id)) {
            states.set(id, state);
            changed.updated.push(id);
          } else {
            states.set(id, state);
            changed.added.push(id);
          }
        }
        emit('change', changed);
        emit('update', changed);
      } catch { /* ignore malformed */ }
    },

    destroy() { listeners.clear(); },
  };
}

// ===== Yjs WebSocket provider =====

const MSG_SYNC = 0;
const MSG_AWARENESS = 1;
const SYNC_STEP1 = 0;
const SYNC_UPDATE = 2;

class CoYjsProvider {
  constructor(ydoc, wsUrl, awareness) {
    this.ydoc = ydoc;
    this.wsUrl = wsUrl;
    this.awareness = awareness;
    this.ws = null;
    this.status = 'reconnecting';
    this._destroyed = false;
    this._reconnectDelay = 1000;
    this.onStatusChange = null;

    // Forward local doc updates to server.
    this._docHandler = (update, origin) => {
      if (origin === this || !this.ws || this.ws.readyState !== WebSocket.OPEN) return;
      const enc = encoding.createEncoder();
      encoding.writeVarUint(enc, MSG_SYNC);
      encoding.writeVarUint(enc, SYNC_UPDATE);
      encoding.writeVarUint8Array(enc, update);
      this.ws.send(encoding.toUint8Array(enc));
    };
    ydoc.on('update', this._docHandler);

    // Forward local awareness changes.
    this._awarenessHandler = ({ added, updated, removed }) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
      const clients = [...added, ...updated, ...removed];
      if (!clients.length) return;
      const enc = encoding.createEncoder();
      encoding.writeVarUint(enc, MSG_AWARENESS);
      encoding.writeVarUint8Array(enc, awareness.encodeUpdate(clients));
      this.ws.send(encoding.toUint8Array(enc));
    };
    awareness.on('update', this._awarenessHandler);

    this._connect();
  }

  _setStatus(s) {
    this.status = s;
    if (this.onStatusChange) this.onStatusChange(s);
  }

  _connect() {
    if (this._destroyed) return;
    this._setStatus('reconnecting');
    const ws = new WebSocket(this.wsUrl);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;

    ws.onopen = () => {
      this._reconnectDelay = 1000;
      this._setStatus('connected');

      // Sync step 1: send our state vector.
      const sv = Y.encodeStateVector(this.ydoc);
      const enc = encoding.createEncoder();
      encoding.writeVarUint(enc, MSG_SYNC);
      encoding.writeVarUint(enc, SYNC_STEP1);
      encoding.writeVarUint8Array(enc, sv);
      ws.send(encoding.toUint8Array(enc));

      // Broadcast our awareness state.
      const aw = encoding.createEncoder();
      encoding.writeVarUint(aw, MSG_AWARENESS);
      encoding.writeVarUint8Array(aw, this.awareness.encodeUpdate([this.ydoc.clientID]));
      ws.send(encoding.toUint8Array(aw));
    };

    ws.onmessage = ({ data }) => {
      const buf = new Uint8Array(data);
      const dec = decoding.createDecoder(buf);
      const msgType = decoding.readVarUint(dec);
      if (msgType === MSG_SYNC) {
        const syncType = decoding.readVarUint(dec);
        if (syncType === 1 /* step2 */ || syncType === SYNC_UPDATE) {
          Y.applyUpdate(this.ydoc, decoding.readVarUint8Array(dec), this);
        }
      } else if (msgType === MSG_AWARENESS) {
        this.awareness.applyUpdate(decoding.readVarUint8Array(dec));
      }
    };

    ws.onclose = () => {
      if (this._destroyed) return;
      this._setStatus('reconnecting');
      setTimeout(() => this._connect(), this._reconnectDelay);
      this._reconnectDelay = Math.min(this._reconnectDelay * 2, 30_000);
    };

    ws.onerror = () => ws.close();
  }

  destroy() {
    this._destroyed = true;
    this.ydoc.off('update', this._docHandler);
    this.awareness.off('update', this._awarenessHandler);
    if (this.ws) this.ws.close();
    this._setStatus('disconnected');
  }
}

// ===== Public API =====

/**
 * @param {HTMLElement} container
 * @param {{
 *   content?: string,
 *   onChange?: (v: string) => void,
 *   readOnly?: boolean,
 *   wsUrl?: string | null,
 *   user?: { id: string, name: string } | null,
 * }} opts
 */
export function initEditor(container, {
  content = '',
  onChange,
  readOnly = false,
  wsUrl = null,
  user = null,
} = {}) {
  injectStyles();

  // --- Layout ---
  const wrap = document.createElement('div');
  wrap.className = 'co-editor-wrap';

  const toolbar = document.createElement('div');
  toolbar.className = 'co-editor-toolbar';

  const panes = document.createElement('div');
  panes.className = 'co-editor-panes';

  const leftPane = document.createElement('div');
  leftPane.className = 'co-editor-left';

  const rightPane = document.createElement('div');
  rightPane.className = 'co-editor-right co-editor-preview';

  panes.appendChild(leftPane);
  panes.appendChild(rightPane);
  wrap.appendChild(toolbar);
  wrap.appendChild(panes);
  container.innerHTML = '';
  container.appendChild(wrap);

  // Preview
  let previewVisible = true;
  const isMobile = () => window.innerWidth < 768;
  // Render markdown to the preview pane. Debounced via _scheduleUpdatePreview
  // for keystrokes so we don't reparse the whole document on every
  // character — typing felt laggy on big docs (~7KB+) when the
  // markdown parse + innerHTML swap ran on every input event.
  function updatePreview(text) { rightPane.innerHTML = renderMarkdown(text); }
  let _previewTimer = null;
  function scheduleUpdatePreview(text) {
    if (_previewTimer) clearTimeout(_previewTimer);
    _previewTimer = setTimeout(() => updatePreview(text), 180);
  }

  // Collab status badge
  const badge = document.createElement('span');
  badge.className = 'co-collab-badge';
  badge.style.display = 'none';

  function updateBadge(status, userCount) {
    const dot = status === 'connected' ? 'green' : status === 'reconnecting' ? 'yellow' : 'grey';
    const label = status === 'connected'
      ? (userCount > 1 ? `${userCount} editando` : 'sincronizado')
      : (status === 'reconnecting' ? 'reconectando…' : 'offline');
    badge.innerHTML = `<span class="co-collab-dot ${dot}"></span>${label}`;
    badge.style.display = '';
  }

  // --- CRDT setup (logged-in only) ---
  const isCrdt = !!(wsUrl && user);
  let provider = null;
  let awareness = null;
  let collabExtensions = [];

  if (isCrdt) {
    const ydoc = new Y.Doc();
    const yText = ydoc.getText('content');
    awareness = createAwareness(ydoc, user);

    provider = new CoYjsProvider(ydoc, wsUrl, awareness);
    provider.onStatusChange = (status) => {
      updateBadge(status, awareness.getStates().size);
    };
    awareness.on('change', () => {
      if (provider.status === 'connected') {
        updateBadge('connected', awareness.getStates().size);
      }
    });
    updateBadge('reconnecting', 1);

    const undoManager = new Y.UndoManager(yText);
    collabExtensions = [
      ...yCollab(yText, awareness, { undoManager }),
      keymap.of(yUndoManagerKeymap),
    ];

    // Toast for anonymous (wsUrl set but no user)
  } else if (wsUrl && !user) {
    // Document has a WS URL but user is anonymous — show collaboration prompt
    setTimeout(() => {
      showToast(
        'Crie uma conta pra colaborar 🤝 &mdash; ' +
        '<a href="/api/v1/auth/login">Entrar</a>'
      );
    }, 2000);
  }

  // --- CodeMirror setup ---
  const readOnlyCompartment = new Compartment();

  const onChangeExt = EditorView.updateListener.of(update => {
    if (update.docChanged) {
      const value = update.state.doc.toString();
      // Debounce preview rendering — keeps typing latency at the
      // speed of CodeMirror's own input handling regardless of
      // document size.
      scheduleUpdatePreview(value);
      if (onChange) onChange(value);
    }
  });

  const saveKeymap = keymap.of([
    { key: 'Mod-s', run: (view) => { if (onChange) onChange(view.state.doc.toString()); return true; } },
    { key: 'Mod-b', run: (view) => { wrapSelection(view, '**', '**', 'bold'); return true; } },
    { key: 'Mod-i', run: (view) => { wrapSelection(view, '*', '*', 'italic'); return true; } },
    { key: 'Mod-k', run: (view) => {
      const sel = view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to);
      if (sel) wrapSelection(view, '[', '](url)', ''); else wrapSelection(view, '[', '](url)', 'link text');
      return true;
    }},
  ]);

  const extensions = [
    history(),
    drawSelection(),
    highlightActiveLine(),
    syntaxHighlighting(coHighlightStyle),
    coTheme,
    keymap.of([...defaultKeymap, ...historyKeymap]),
    saveKeymap,
    markdown({ base: markdownLanguage, codeLanguages: languages, yaml: yaml().language }),
    onChangeExt,
    EditorView.lineWrapping,
    readOnlyCompartment.of(EditorState.readOnly.of(readOnly)),
    placeholder('Escreva em markdown…'),
    ...collabExtensions,
  ];

  // In CRDT mode, start empty — server sync-step-2 fills it.
  const startDoc = isCrdt ? '' : content;
  const startState = EditorState.create({ doc: startDoc, extensions });
  const view = new EditorView({ state: startState, parent: leftPane });

  updatePreview(startDoc);
  if (isMobile()) rightPane.classList.add('co-preview-hidden');

  // --- Toolbar buttons ---
  const TOOLBAR_ITEMS = [
    { label: 'B', title: 'Bold (Ctrl+B)', style: 'font-weight:bold', action: v => wrapSelection(v, '**', '**', 'bold') },
    { label: 'I', title: 'Italic (Ctrl+I)', style: 'font-style:italic', action: v => wrapSelection(v, '*', '*', 'italic') },
    { label: 'H', title: 'Heading', action: v => insertLine(v, '## ') },
    { sep: true },
    { label: 'Link', title: 'Link (Ctrl+K)', desktopOnly: true, action: v => {
      const sel = v.state.sliceDoc(v.state.selection.main.from, v.state.selection.main.to);
      if (sel) wrapSelection(v, '[', '](url)', ''); else wrapSelection(v, '[', '](url)', 'link text');
    }},
    { label: '`code`', title: 'Inline code', desktopOnly: true, action: v => wrapSelection(v, '`', '`', 'code') },
    { sep: true },
    { label: '• List', title: 'Unordered list', desktopOnly: true, action: v => insertLine(v, '- ') },
    { label: '☑ Task', title: 'Task list item', desktopOnly: true, action: v => insertLine(v, '- [ ] ') },
    { sep: true },
    { label: 'Preview', title: 'Toggle preview', id: 'preview-toggle', action: () => {
      previewVisible = !previewVisible;
      if (isMobile()) {
        rightPane.classList.toggle('co-preview-mobile', previewVisible);
        leftPane.classList.toggle('co-editor-mobile-hide', previewVisible);
      } else {
        rightPane.classList.toggle('co-preview-hidden', !previewVisible);
        leftPane.classList.toggle('co-editor-full', !previewVisible);
      }
      const btn = toolbar.querySelector('[data-id="preview-toggle"]');
      if (btn) btn.classList.toggle('active', previewVisible);
    }},
  ];

  TOOLBAR_ITEMS.forEach(item => {
    if (item.sep) {
      const sep = document.createElement('span');
      sep.className = 'co-editor-sep' + (item.desktopOnly ? ' co-desktop-only' : '');
      toolbar.appendChild(sep);
      return;
    }
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'co-editor-btn' + (item.desktopOnly ? ' co-desktop-only' : '');
    btn.textContent = item.label;
    btn.title = item.title || item.label;
    if (item.style) btn.setAttribute('style', item.style);
    if (item.id) btn.dataset.id = item.id;
    if (item.id === 'preview-toggle') btn.classList.add('active');
    btn.addEventListener('click', () => { if (item.action) item.action(view); });
    toolbar.appendChild(btn);
  });

  toolbar.appendChild(badge);

  // --- CO-150: Drag-and-drop / paste image upload ---
  // Extracts the universe key from the current URL (/co/{slug}/...) for uploads.
  function _getEditorUniverseKey() {
    if (_universeKey) return _universeKey;
    const m = window.location.pathname.match(/^\/co\/([^\/]+)/);
    return m ? m[1] : '';
  }

  async function _uploadAndInsert(file) {
    const uKey = _getEditorUniverseKey();
    if (!uKey) return;
    const mime = file.type || 'application/octet-stream';
    const filename = encodeURIComponent(file.name || 'upload');
    try {
      const resp = await fetch(
        `/api/v1/universes/${uKey}/assets?filename=${filename}`,
        { method: 'POST', headers: { 'Content-Type': mime }, body: file },
      );
      if (!resp.ok) return;
      const { sha256 } = await resp.json();
      const isVideo = mime.startsWith('video/');
      const alt = file.name || 'media';
      const syntax = isVideo
        ? `\`\`\`video\nsha256:${sha256}\n\`\`\``
        : `![${alt}](sha256:${sha256})`;
      const cursor = view.state.selection.main.head;
      const sep = cursor > 0 ? '\n' : '';
      view.dispatch({
        changes: { from: cursor, insert: sep + syntax + '\n' },
        selection: { anchor: cursor + sep.length + syntax.length + 1 },
      });
      view.focus();
    } catch (_) { /* silent — network errors shouldn't break the editor */ }
  }

  // Drag-over: accept files
  wrap.addEventListener('dragover', e => {
    if (e.dataTransfer.types.includes('Files')) {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
      wrap.style.outline = '2px dashed var(--accent,#6366f1)';
    }
  });
  wrap.addEventListener('dragleave', () => { wrap.style.outline = ''; });
  wrap.addEventListener('drop', e => {
    wrap.style.outline = '';
    if (!e.dataTransfer.files.length) return;
    e.preventDefault();
    const file = e.dataTransfer.files[0];
    if (file.type.startsWith('image/') || file.type.startsWith('video/')) {
      _uploadAndInsert(file);
    }
  });

  // Paste: handle image/video from clipboard
  wrap.addEventListener('paste', e => {
    const items = Array.from(e.clipboardData?.items || []);
    const fileItem = items.find(i => i.kind === 'file' && (i.type.startsWith('image/') || i.type.startsWith('video/')));
    if (!fileItem) return;
    e.preventDefault();
    const file = fileItem.getAsFile();
    if (file) _uploadAndInsert(file);
  });

  // --- Public interface ---
  return {
    getValue() { return view.state.doc.toString(); },
    setValue(newContent) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: newContent } });
      updatePreview(newContent);
    },
    setReadOnly(ro) {
      view.dispatch({ effects: readOnlyCompartment.reconfigure(EditorState.readOnly.of(ro)) });
    },
    focus() { view.focus(); },
    destroy() {
      provider?.destroy();
      awareness?.destroy();
      view.destroy();
      if (container.contains(wrap)) container.removeChild(wrap);
    },
  };
}
