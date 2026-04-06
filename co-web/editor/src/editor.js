/**
 * CO Editor — CodeMirror 6 markdown editor with live preview
 * Public API: initEditor(container, { content, onChange, readOnly })
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
  insertNewlineAndIndent,
} from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { yaml } from '@codemirror/lang-yaml';
import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { Marked } from 'marked';
import DOMPurify from 'dompurify';

// ===== Marked (GFM + task lists) =====

const markedInstance = new Marked({
  gfm: true,
  breaks: false,
});

function renderMarkdown(src) {
  const html = markedInstance.parse(src || '');
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      'p', 'br', 'strong', 'em', 's', 'del', 'code', 'pre', 'blockquote',
      'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
      'ul', 'ol', 'li', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
      'a', 'img', 'hr', 'input', 'span', 'div',
    ],
    ALLOWED_ATTR: ['href', 'src', 'alt', 'title', 'type', 'checked', 'disabled', 'class', 'id'],
  });
}

// ===== CSS custom-property theme =====

const coTheme = EditorView.theme({
  '&': {
    background: 'var(--card-bg, var(--bg, #fff))',
    color: 'var(--text-primary, #111827)',
    height: '100%',
    fontSize: '14px',
  },
  '&.cm-focused': {
    outline: 'none',
  },
  '.cm-scroller': {
    overflow: 'auto',
    fontFamily: 'var(--font-mono, "SF Mono", Consolas, monospace)',
    lineHeight: '1.6',
  },
  '.cm-content': {
    padding: '12px 16px',
    caretColor: 'var(--accent, #6366f1)',
    minHeight: '180px',
  },
  '.cm-cursor, .cm-dropCursor': {
    borderLeftColor: 'var(--accent, #6366f1)',
  },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    background: 'var(--accent-light, #e0e7ff)',
  },
  '.cm-activeLine': {
    backgroundColor: 'rgba(0,0,0,0.03)',
  },
  '.cm-gutters': {
    display: 'none',
  },
  '.cm-line': {
    padding: '0',
  },
}, { dark: false });

// ===== Syntax highlight style (reads CSS vars via injected class) =====

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

// ===== Editor styles injected once =====

const EDITOR_CSS = `
.co-editor-wrap {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: var(--radius-md, 8px);
  overflow: hidden;
  background: var(--card-bg, #fff);
  resize: vertical;
  min-height: 300px;
  height: 400px;
}
.co-editor-wrap:focus-within {
  border-color: var(--accent, #6366f1);
  box-shadow: 0 0 0 2px var(--accent-light, #e0e7ff);
}
.co-editor-toolbar {
  display: flex;
  gap: 2px;
  padding: 6px 8px;
  background: var(--bg, #f0f2f5);
  border-bottom: 1px solid var(--border, #e5e7eb);
  flex-wrap: wrap;
  align-items: center;
  flex-shrink: 0;
}
.co-editor-btn {
  background: none;
  border: none;
  cursor: pointer;
  padding: 3px 7px;
  border-radius: var(--radius-sm, 4px);
  font-size: 13px;
  color: var(--text-secondary, #6b7280);
  font-family: var(--font, sans-serif);
  line-height: 1.4;
  white-space: nowrap;
  transition: background 0.1s, color 0.1s;
  font-weight: 500;
}
.co-editor-btn:hover {
  background: var(--bg-hover, #e8eaed);
  color: var(--text-primary, #111827);
}
.co-editor-btn.active {
  background: var(--accent-light, #e0e7ff);
  color: var(--accent, #6366f1);
}
.co-editor-sep {
  width: 1px;
  height: 18px;
  background: var(--border, #e5e7eb);
  margin: 0 4px;
  flex-shrink: 0;
}
.co-editor-panes {
  display: flex;
  flex: 1;
  overflow: hidden;
  min-height: 0;
}
.co-editor-left {
  flex: 1;
  overflow: auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.co-editor-left .cm-editor {
  flex: 1;
  height: 100%;
}
.co-editor-right {
  flex: 1;
  overflow: auto;
  padding: 12px 20px;
  border-left: 1px solid var(--border, #e5e7eb);
  background: var(--card-bg, #fff);
  font-family: var(--font, sans-serif);
  font-size: 14px;
  line-height: 1.7;
  color: var(--text-primary, #111827);
  min-width: 0;
}
.co-editor-right.co-preview-hidden {
  display: none;
}
.co-editor-left.co-editor-full {
  flex: 1 1 100%;
}
/* Preview markdown styles */
.co-editor-right h1 { font-size: 1.6em; font-weight: bold; margin: 0.4em 0 0.2em; border-bottom: 1px solid var(--border, #e5e7eb); padding-bottom: 0.2em; }
.co-editor-right h2 { font-size: 1.3em; font-weight: bold; margin: 0.8em 0 0.2em; }
.co-editor-right h3 { font-size: 1.1em; font-weight: 600; margin: 0.8em 0 0.2em; }
.co-editor-right h4, .co-editor-right h5, .co-editor-right h6 { font-weight: 600; margin: 0.6em 0 0.1em; }
.co-editor-right p { margin: 0.4em 0; }
.co-editor-right code { font-family: var(--font-mono, monospace); font-size: 0.9em; background: rgba(0,0,0,0.06); padding: 1px 4px; border-radius: 3px; }
.co-editor-right pre { background: rgba(0,0,0,0.05); border-radius: var(--radius-sm, 4px); padding: 10px 14px; overflow: auto; margin: 0.5em 0; }
.co-editor-right pre code { background: none; padding: 0; }
.co-editor-right blockquote { border-left: 3px solid var(--accent, #6366f1); margin: 0.5em 0; padding: 0 0 0 12px; color: var(--text-secondary, #6b7280); }
.co-editor-right ul, .co-editor-right ol { margin: 0.4em 0; padding-left: 1.5em; }
.co-editor-right li { margin: 0.15em 0; }
.co-editor-right li input[type="checkbox"] { margin-right: 6px; }
.co-editor-right table { border-collapse: collapse; width: 100%; margin: 0.5em 0; font-size: 0.9em; }
.co-editor-right th, .co-editor-right td { border: 1px solid var(--border, #e5e7eb); padding: 6px 10px; text-align: left; }
.co-editor-right th { background: var(--bg, #f0f2f5); font-weight: 600; }
.co-editor-right a { color: var(--accent, #6366f1); text-decoration: underline; }
.co-editor-right hr { border: none; border-top: 1px solid var(--border, #e5e7eb); margin: 1em 0; }
.co-editor-right img { max-width: 100%; border-radius: var(--radius-sm, 4px); }
.co-editor-right del { text-decoration: line-through; color: var(--text-muted, #9ca3af); }
/* Mobile */
@media (max-width: 767px) {
  .co-editor-right { display: none; }
  .co-editor-right.co-preview-mobile { display: block; border-left: none; }
  .co-editor-left.co-editor-mobile-hide { display: none; }
  .co-editor-toolbar .co-desktop-only { display: none; }
}
`;

let styleInjected = false;
function injectStyles() {
  if (styleInjected) return;
  styleInjected = true;
  const style = document.createElement('style');
  style.id = 'co-editor-styles';
  style.textContent = EDITOR_CSS;
  document.head.appendChild(style);
}

// ===== Text manipulation helpers =====

function wrapSelection(view, before, after, cursorOffset) {
  const { state } = view;
  const tr = state.changeByRange(range => {
    const text = state.sliceDoc(range.from, range.to);
    const insert = before + (text || cursorOffset || '') + after;
    const anchor = range.from + before.length;
    const head = range.from + before.length + (text || cursorOffset || '').length;
    return {
      changes: { from: range.from, to: range.to, insert },
      range: { anchor, head },
    };
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
      return {
        changes: { from: start, to: start + prefix.length, insert: '' },
        range: { anchor: range.from - prefix.length, head: range.from - prefix.length },
      };
    }
    return {
      changes: { from: line.from, to: line.from, insert: prefix },
      range: { anchor: range.from + prefix.length, head: range.from + prefix.length },
    };
  });
  view.dispatch(tr);
  view.focus();
}

// ===== Public API =====

/**
 * Initialize the CO markdown editor.
 * @param {HTMLElement} container - Target element (will be replaced by editor)
 * @param {{ content?: string, onChange?: (value: string) => void, readOnly?: boolean }} opts
 * @returns {{ getValue: () => string, setValue: (v: string) => void, destroy: () => void }}
 */
export function initEditor(container, { content = '', onChange, readOnly = false } = {}) {
  injectStyles();

  // Build wrapper
  const wrap = document.createElement('div');
  wrap.className = 'co-editor-wrap';

  // Toolbar
  const toolbar = document.createElement('div');
  toolbar.className = 'co-editor-toolbar';

  // Panes
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

  // Mount into container
  container.innerHTML = '';
  container.appendChild(wrap);

  // Preview state
  let previewVisible = true;
  const isMobile = () => window.innerWidth < 768;

  function updatePreview(text) {
    rightPane.innerHTML = renderMarkdown(text);
  }

  // ===== CodeMirror setup =====

  const readOnlyCompartment = new Compartment();

  const onChangeListener = EditorView.updateListener.of(update => {
    if (update.docChanged) {
      const value = update.state.doc.toString();
      updatePreview(value);
      if (onChange) onChange(value);
    }
  });

  const saveKeymap = keymap.of([
    {
      key: 'Mod-s',
      run: (view) => {
        if (onChange) onChange(view.state.doc.toString());
        return true;
      },
    },
    {
      key: 'Mod-b',
      run: (view) => { wrapSelection(view, '**', '**', 'bold'); return true; },
    },
    {
      key: 'Mod-i',
      run: (view) => { wrapSelection(view, '*', '*', 'italic'); return true; },
    },
    {
      key: 'Mod-k',
      run: (view) => {
        const sel = view.state.sliceDoc(
          view.state.selection.main.from,
          view.state.selection.main.to
        );
        if (sel) {
          wrapSelection(view, '[', '](url)', '');
        } else {
          wrapSelection(view, '[', '](url)', 'link text');
        }
        return true;
      },
    },
  ]);

  const extensions = [
    history(),
    drawSelection(),
    highlightActiveLine(),
    syntaxHighlighting(coHighlightStyle),
    coTheme,
    keymap.of([...defaultKeymap, ...historyKeymap]),
    saveKeymap,
    markdown({
      base: markdownLanguage,
      codeLanguages: languages,
      yaml: yaml().language,
    }),
    onChangeListener,
    EditorView.lineWrapping,
    readOnlyCompartment.of(EditorState.readOnly.of(readOnly)),
    placeholder('Escreva em markdown…'),
  ];

  const startState = EditorState.create({
    doc: content,
    extensions,
  });

  const view = new EditorView({
    state: startState,
    parent: leftPane,
  });

  // Initial preview
  updatePreview(content);

  // Mobile: start with preview hidden
  if (isMobile()) {
    rightPane.classList.add('co-preview-hidden');
    leftPane.classList.remove('co-editor-mobile-hide');
  }

  // ===== Toolbar buttons =====

  const TOOLBAR_ITEMS = [
    {
      label: 'B',
      title: 'Bold (Ctrl+B)',
      style: 'font-weight:bold',
      action: (v) => wrapSelection(v, '**', '**', 'bold'),
    },
    {
      label: 'I',
      title: 'Italic (Ctrl+I)',
      style: 'font-style:italic',
      action: (v) => wrapSelection(v, '*', '*', 'italic'),
    },
    {
      label: 'H',
      title: 'Heading',
      action: (v) => insertLine(v, '## '),
    },
    { sep: true },
    {
      label: 'Link',
      title: 'Link (Ctrl+K)',
      desktopOnly: true,
      action: (v) => {
        const sel = v.state.sliceDoc(v.state.selection.main.from, v.state.selection.main.to);
        if (sel) {
          wrapSelection(v, '[', '](url)', '');
        } else {
          wrapSelection(v, '[', '](url)', 'link text');
        }
      },
    },
    {
      label: '`code`',
      title: 'Inline code',
      desktopOnly: true,
      action: (v) => wrapSelection(v, '`', '`', 'code'),
    },
    { sep: true },
    {
      label: '• List',
      title: 'Unordered list',
      desktopOnly: true,
      action: (v) => insertLine(v, '- '),
    },
    {
      label: '☑ Task',
      title: 'Task list item',
      desktopOnly: true,
      action: (v) => insertLine(v, '- [ ] '),
    },
    { sep: true },
    {
      label: 'Preview',
      title: 'Toggle preview',
      id: 'preview-toggle',
      action: () => {
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
      },
    },
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
    if (item.id === 'preview-toggle') {
      btn.classList.add('active');
    }
    btn.addEventListener('click', () => {
      if (item.action) item.action(view);
    });
    toolbar.appendChild(btn);
  });

  // ===== Public interface =====

  return {
    getValue() {
      return view.state.doc.toString();
    },
    setValue(newContent) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: newContent },
      });
      updatePreview(newContent);
    },
    setReadOnly(ro) {
      view.dispatch({
        effects: readOnlyCompartment.reconfigure(EditorState.readOnly.of(ro)),
      });
    },
    focus() {
      view.focus();
    },
    destroy() {
      view.destroy();
      if (container.contains(wrap)) container.removeChild(wrap);
    },
  };
}
