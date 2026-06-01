/**
 * CO-332: AI assistant chat widget.
 *
 * Floating button (bottom-right) → modal with SSE chat stream.
 * Connects to POST /api/v1/chat/:slug using fetch + ReadableStream.
 * Conversation history is kept in sessionStorage for the visit.
 *
 * Provider constraint: the server enforces non-Claude — this widget
 * just makes the POST request and streams the SSE response.
 */

const MAX_HISTORY = 20; // pairs of user+assistant messages

// ---------------------------------------------------------------------------
// Detect universe slug from URL or subdomain
// ---------------------------------------------------------------------------

function detectSlug() {
  // Subdomain mode (yuri.artelonga.com.br → slug = "yuri")
  const hostname = window.location.hostname;
  const subdomainMatch = hostname.match(/^([^.]+)\.artelonga\.com\.br$/);
  if (subdomainMatch && subdomainMatch[1] !== 'co') {
    return subdomainMatch[1];
  }
  // Path mode (co.artelonga.com.br/yuri → slug = "yuri")
  const pathMatch = window.location.pathname.match(/^\/([^/]+)/);
  if (pathMatch && pathMatch[1]) return pathMatch[1];
  return null;
}

// ---------------------------------------------------------------------------
// Session history (persisted in sessionStorage)
// ---------------------------------------------------------------------------

const HISTORY_KEY = 'co-chat-history';

function loadHistory() {
  try {
    return JSON.parse(sessionStorage.getItem(HISTORY_KEY) || '[]');
  } catch {
    return [];
  }
}

function saveHistory(history) {
  try {
    sessionStorage.setItem(HISTORY_KEY, JSON.stringify(history.slice(-MAX_HISTORY)));
  } catch { /* storage full */ }
}

// ---------------------------------------------------------------------------
// SSE parser for fetch ReadableStream
// ---------------------------------------------------------------------------

async function* parseSSE(response) {
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    // Split by double-newline (SSE event boundary)
    const parts = buffer.split('\n\n');
    buffer = parts.pop() ?? '';

    for (const part of parts) {
      if (!part.trim()) continue;
      const lines = part.split('\n');
      let eventType = 'message';
      let data = '';
      for (const line of lines) {
        if (line.startsWith('event: ')) eventType = line.slice(7).trim();
        else if (line.startsWith('data: ')) data = line.slice(6);
      }
      if (data) {
        try { yield { type: eventType, data: JSON.parse(data) }; }
        catch { /* malformed JSON — skip */ }
      }
    }
  }
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

function appendMessage(container, role, text) {
  const div = document.createElement('div');
  div.className = `ai-msg ai-msg-${role}`;
  div.textContent = text;
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
  return div;
}

function appendToolNotice(container, name) {
  const div = document.createElement('div');
  div.className = 'ai-msg ai-tool-notice';
  div.textContent = `🔍 ${name.replace(/_/g, ' ')}…`;
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
  return div;
}

// ---------------------------------------------------------------------------
// Main send function
// ---------------------------------------------------------------------------

async function sendMessage(slug, message, history, messagesEl, input, sendBtn) {
  input.disabled = true;
  sendBtn.disabled = true;

  appendMessage(messagesEl, 'user', message);

  const assistantDiv = appendMessage(messagesEl, 'assistant', '');
  let assistantText = '';
  let toolNotice = null;

  try {
    const resp = await fetch(`/api/v1/chat/${encodeURIComponent(slug)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message, history }),
    });

    if (!resp.ok) {
      assistantDiv.textContent = `Erro ${resp.status}: não foi possível conectar ao assistente.`;
      return;
    }

    for await (const event of parseSSE(resp)) {
      switch (event.type) {
        case 'token':
          assistantText += event.data.text ?? '';
          assistantDiv.textContent = assistantText;
          messagesEl.scrollTop = messagesEl.scrollHeight;
          break;

        case 'tool_start':
          if (toolNotice) toolNotice.remove();
          toolNotice = appendToolNotice(messagesEl, event.data.name);
          break;

        case 'tool_result':
          if (toolNotice) { toolNotice.remove(); toolNotice = null; }
          break;

        case 'done':
          break;

        case 'error':
          assistantDiv.textContent = `Erro: ${event.data.message ?? 'falha desconhecida'}`;
          break;
      }
    }

    if (!assistantText) {
      assistantDiv.textContent = '(sem resposta)';
    }
  } catch (err) {
    assistantDiv.textContent = `Erro de rede: ${err.message}`;
  } finally {
    if (toolNotice) toolNotice.remove();
    input.disabled = false;
    sendBtn.disabled = false;
    input.focus();
  }

  return assistantText;
}

// ---------------------------------------------------------------------------
// Widget initialisation
// ---------------------------------------------------------------------------

function injectStyles() {
  const style = document.createElement('style');
  style.textContent = `
    #ai-chat-btn {
      position: fixed;
      bottom: 1.5rem;
      right: 1.5rem;
      z-index: 9000;
      width: 3rem;
      height: 3rem;
      border-radius: 50%;
      background: var(--co-accent, #6366f1);
      color: #fff;
      border: none;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      box-shadow: 0 4px 16px rgba(0,0,0,.25);
      transition: transform .15s;
    }
    #ai-chat-btn:hover { transform: scale(1.08); }
    .ai-chat-panel {
      position: fixed;
      bottom: 5rem;
      right: 1.5rem;
      z-index: 9001;
      width: min(22rem, calc(100vw - 2rem));
      max-height: 60vh;
      background: var(--card-bg, #fff);
      border: 1px solid var(--border, #e5e7eb);
      border-radius: 12px;
      box-shadow: 0 8px 32px rgba(0,0,0,.18);
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }
    .ai-chat-panel.hidden { display: none; }
    .ai-chat-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: .6rem 1rem;
      background: var(--co-accent, #6366f1);
      color: #fff;
      font-weight: 600;
      font-size: .9rem;
    }
    .ai-chat-header button {
      background: none;
      border: none;
      color: #fff;
      cursor: pointer;
      font-size: 1rem;
      line-height: 1;
      padding: 0;
    }
    .ai-chat-messages {
      flex: 1;
      overflow-y: auto;
      padding: .75rem 1rem;
      display: flex;
      flex-direction: column;
      gap: .5rem;
      min-height: 6rem;
    }
    .ai-msg {
      max-width: 90%;
      padding: .45rem .75rem;
      border-radius: 10px;
      font-size: .875rem;
      line-height: 1.45;
      white-space: pre-wrap;
      word-break: break-word;
    }
    .ai-msg-user {
      align-self: flex-end;
      background: var(--co-accent, #6366f1);
      color: #fff;
    }
    .ai-msg-assistant {
      align-self: flex-start;
      background: var(--input-bg, #f3f4f6);
      color: var(--text, #111);
    }
    .ai-tool-notice {
      align-self: flex-start;
      font-size: .78rem;
      color: var(--text-muted, #6b7280);
      font-style: italic;
    }
    .ai-chat-input-row {
      display: flex;
      gap: .5rem;
      padding: .6rem .75rem;
      border-top: 1px solid var(--border, #e5e7eb);
    }
    #ai-chat-input {
      flex: 1;
      border: 1px solid var(--border, #e5e7eb);
      border-radius: 6px;
      padding: .4rem .65rem;
      font-size: .875rem;
      background: var(--input-bg, #f9fafb);
      color: var(--text, #111);
      outline: none;
    }
    #ai-chat-input:focus { border-color: var(--co-accent, #6366f1); }
    #ai-chat-send {
      background: var(--co-accent, #6366f1);
      color: #fff;
      border: none;
      border-radius: 6px;
      padding: .4rem .75rem;
      cursor: pointer;
      font-size: 1rem;
      transition: opacity .15s;
    }
    #ai-chat-send:disabled, #ai-chat-input:disabled { opacity: .5; cursor: not-allowed; }
  `;
  document.head.appendChild(style);
}

function initWidget() {
  const slug = detectSlug();
  if (!slug) return; // Don't show widget when slug is unknown

  const widget = document.getElementById('ai-assistant-widget');
  if (!widget) return;

  injectStyles();
  widget.style.display = '';

  const btn = document.getElementById('ai-chat-btn');
  const panel = document.getElementById('ai-chat-panel');
  const closeBtn = document.getElementById('ai-chat-close');
  const messagesEl = document.getElementById('ai-chat-messages');
  const input = document.getElementById('ai-chat-input');
  const sendBtn = document.getElementById('ai-chat-send');

  let history = loadHistory();

  // Restore history in UI
  for (const msg of history) {
    if (msg.role === 'user' || msg.role === 'assistant') {
      appendMessage(messagesEl, msg.role, msg.content);
    }
  }

  btn.addEventListener('click', () => {
    panel.classList.toggle('hidden');
    if (!panel.classList.contains('hidden')) input.focus();
  });

  closeBtn.addEventListener('click', () => panel.classList.add('hidden'));

  async function submit() {
    const message = input.value.trim();
    if (!message || input.disabled) return;
    input.value = '';

    const reply = await sendMessage(slug, message, history, messagesEl, input, sendBtn);

    // Update history
    history.push({ role: 'user', content: message });
    if (reply) history.push({ role: 'assistant', content: reply });
    saveHistory(history);
  }

  sendBtn.addEventListener('click', submit);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
  });
}

// Run after DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initWidget);
} else {
  initWidget();
}
