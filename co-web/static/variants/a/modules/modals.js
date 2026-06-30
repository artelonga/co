// ===== Task modal, comments, activity, universe info =====
import { state } from './state.js';
import { api, apiFetch } from './api.js';
import { esc, getSubtasks, getSubtaskProgress, relativeTime } from './helpers.js';
import { STATUSES, STATUS_LABELS, ACTIVITY_ACTION_LABELS } from './constants.js';

let _showToast = () => {};
let _showLoginModal = () => {};
let _refreshTasks = async () => {};
let _render = () => {};
let _renderUniverseInfo = async () => {};
let _renderContent = () => {};
let _ensureOwnUniverse = async () => true;
let _loadMeUniverses = async () => {};
let _renderSidebar = () => {};

export function injectModalCallbacks(callbacks) {
    _showToast = callbacks.showToast;
    _showLoginModal = callbacks.showLoginModal;
    _refreshTasks = callbacks.refreshTasks;
    _render = callbacks.render;
    _renderContent = callbacks.renderContent;
    _ensureOwnUniverse = callbacks.ensureOwnUniverse;
    if (callbacks.loadMeUniverses) _loadMeUniverses = callbacks.loadMeUniverses;
    if (callbacks.renderSidebar) _renderSidebar = callbacks.renderSidebar;
}

// ===== Editor lazy-load =====
let _editorInstance = null;
let _editorBundlePromise = null;
let _taskDraftInterval = null;

export function loadEditorBundle() {
    if (_editorBundlePromise) return _editorBundlePromise;
    if (window.CoEditor) return (_editorBundlePromise = Promise.resolve());
    _editorBundlePromise = new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.src = '/shared/editor.bundle.js';
        s.onload = resolve;
        s.onerror = reject;
        document.head.appendChild(s);
    });
    return _editorBundlePromise;
}

async function initTaskEditor(initialContent, taskId) {
    const container = document.getElementById('task-description-editor');
    const textarea = document.getElementById('task-description');

    if (_taskDraftInterval) { clearInterval(_taskDraftInterval); _taskDraftInterval = null; }
    if (_editorInstance) {
        _editorInstance.destroy();
        _editorInstance = null;
    }

    try {
        await loadEditorBundle();
        if (container && window.CoEditor) {
            container.style.display = '';
            if (textarea) textarea.style.display = 'none';
            _editorInstance = window.CoEditor.initEditor(container, {
                content: initialContent,
                onChange: (val) => { if (textarea) textarea.value = val; },
                readOnly: false,
            });
            if (textarea) textarea.value = initialContent;

            _taskDraftInterval = setInterval(() => {
                try {
                    const val = _editorInstance ? _editorInstance.getValue() : '';
                    const key = taskId ? `co_draft_task_${taskId}` : 'co_draft_new_task';
                    localStorage.setItem(key, val);
                } catch (_) {}
            }, 5000);
            return;
        }
    } catch (_) {}

    if (container) container.style.display = 'none';
    if (textarea) {
        textarea.style.display = '';
        textarea.value = initialContent;
        textarea.rows = 6;
    }
}

export function openTaskModal(taskId) {
    const overlay = document.querySelector('#modal-overlay');
    const form = document.querySelector('#task-form');
    const deleteBtn = document.querySelector('#btn-delete');
    const archiveBtn = document.querySelector('#btn-archive');

    const existingInfo = document.getElementById('task-hierarchy-info');
    if (existingInfo) existingInfo.remove();

    if (taskId) {
        state.editingTaskId = taskId;
        const task = state.tasks.find(t => t.id === taskId);
        if (!task) return;
        document.querySelector('#modal-title').textContent = task.key;
        document.querySelector('#task-title').value = task.title;
        document.querySelector('#task-status').value = task.status;
        document.querySelector('#task-priority').value = task.priority;
        document.querySelector('#task-due-date').value = task.due_date || '';
        document.querySelector('#task-assignee').value = task.assignee || '';
        document.querySelector('#task-labels').value = task.labels.join(', ');
        document.querySelector('#task-description').value = task.description || '';
        initTaskEditor(task.description || '', taskId);
        deleteBtn.classList.remove('hidden');

        if (archiveBtn) {
            archiveBtn.classList.remove('hidden');
            archiveBtn.textContent = task.archived
                ? (window.t ? window.t('unarchive') : 'Desarquivar')
                : (window.t ? window.t('archive') : 'Arquivar');
        }

        const subtaskGroup = document.querySelector('#subtask-btn-group');
        if (subtaskGroup) {
            subtaskGroup.classList.remove('hidden');
            const subtaskBtn = document.querySelector('#btn-add-subtask');
            if (subtaskBtn) {
                subtaskBtn.onclick = () => {
                    closeModal();
                    openTaskModal(null);
                    setTimeout(() => {
                        const parentSel = document.querySelector('#task-parent');
                        if (parentSel) parentSel.value = String(taskId);
                    }, 50);
                };
            }
        }

        const parentTask = task.parent ? state.tasks.find(t => t.id === task.parent) : null;
        const subtasks = getSubtasks(task);
        if (parentTask || subtasks.length > 0) {
            const info = document.createElement('div');
            info.id = 'task-hierarchy-info';
            info.className = 'task-hierarchy-info';
            let infoHtml = '';
            if (parentTask) {
                const pKey = `${state.currentProject.key}-${parentTask.id}`;
                infoHtml += `<div class="hierarchy-parent">
                    <span class="hierarchy-label">Parent:</span>
                    <button class="hierarchy-link" data-task-id="${parentTask.id}">${esc(pKey)} — ${esc(parentTask.title)}</button>
                </div>`;
            }
            if (subtasks.length > 0) {
                const sp = getSubtaskProgress(task);
                infoHtml += `<div class="hierarchy-subtasks">
                    <span class="hierarchy-label">Subtasks</span>
                    ${sp ? `<span class="subtask-badge">${sp.done}/${sp.total}</span>` : ''}
                    <div class="hierarchy-subtask-list">
                        ${subtasks.map(sub => {
                            const si = STATUSES.find(s => s.key === sub.status);
                            return `<div class="hierarchy-subtask-item" data-task-id="${sub.id}">
                                <span class="subtask-item-dot" style="background:${si ? si.color : '#94a3b8'}"></span>
                                <span class="hierarchy-subtask-key">${esc(sub.key)}</span>
                                <span class="hierarchy-subtask-title">${esc(sub.title)}</span>
                                <span class="hierarchy-subtask-status status-${sub.status}">${STATUS_LABELS[sub.status]}</span>
                            </div>`;
                        }).join('')}
                    </div>
                </div>`;
            }
            info.innerHTML = infoHtml;
            document.querySelector('.modal-header').insertAdjacentElement('afterend', info);

            info.querySelectorAll('.hierarchy-subtask-item').forEach(el => {
                el.addEventListener('click', () => openTaskModal(parseInt(el.dataset.taskId)));
            });
            const parentLink = info.querySelector('.hierarchy-link');
            if (parentLink) {
                parentLink.addEventListener('click', () => openTaskModal(parseInt(parentLink.dataset.taskId)));
            }
        }

        loadComments(taskId);
    } else {
        state.editingTaskId = null;
        document.querySelector('#modal-title').textContent = 'New Task';
        form.reset();
        document.querySelector('#task-status').value = 'todo';
        document.querySelector('#task-priority').value = 'medium';
        document.querySelector('#task-assignee').value = '';
        deleteBtn.classList.add('hidden');

        if (archiveBtn) archiveBtn.classList.add('hidden');
        const subtaskGroup2 = document.querySelector('#subtask-btn-group');
        if (subtaskGroup2) subtaskGroup2.classList.add('hidden');
        initTaskEditor('');

        const commentsSection = document.querySelector('#comments-section');
        if (commentsSection) commentsSection.innerHTML = '';
    }

    const parentSelect = document.querySelector('#task-parent');
    parentSelect.innerHTML = '<option value="">None</option>';
    state.tasks.forEach(t => {
        if (t.id !== taskId) {
            const opt = document.createElement('option');
            opt.value = t.id;
            opt.textContent = `${t.key} — ${t.title}`;
            parentSelect.appendChild(opt);
        }
    });

    if (taskId) {
        const task = state.tasks.find(t => t.id === taskId);
        if (task?.parent) parentSelect.value = task.parent;
    }

    overlay.classList.remove('hidden');
    setTimeout(() => document.querySelector('#task-title').focus(), 50);
}

export function closeModal() {
    document.querySelector('#modal-overlay').classList.add('hidden');
    state.editingTaskId = null;
    if (_taskDraftInterval) { clearInterval(_taskDraftInterval); _taskDraftInterval = null; }
    if (_editorInstance) {
        _editorInstance.destroy();
        _editorInstance = null;
    }
}

async function loadComments(taskId) {
    const commentsSection = document.querySelector('#comments-section');
    if (!commentsSection || !state.currentProject) return;

    commentsSection.innerHTML = '<p class="comments-loading">Loading comments...</p>';

    const comments = await api.getComments(state.currentProject.key, taskId);

    let commentsHtml = '<h4 class="comments-title">Comments</h4>';

    if (comments && comments.length > 0) {
        commentsHtml += '<div class="comments-list">';
        for (const c of comments) {
            commentsHtml += `
                <div class="comment-item">
                    <div class="comment-header">
                        <span class="comment-author">${esc(c.author || 'Anonymous')}</span>
                        <span class="comment-time">${relativeTime(c.created_at)}</span>
                    </div>
                    <div class="comment-body">${esc(c.body || '')}</div>
                </div>`;
        }
        commentsHtml += '</div>';
    } else {
        commentsHtml += '<p class="comments-empty">No comments yet</p>';
    }

    commentsHtml += `
        <div class="comment-form">
            <input type="text" class="comment-author-input" id="comment-author" placeholder="Seu nome" />
            <textarea class="comment-body-input" id="comment-body" placeholder="Add a comment..." rows="2"></textarea>
            <button type="button" class="btn btn-primary" id="btn-add-comment">Comment</button>
        </div>`;

    commentsSection.innerHTML = commentsHtml;

    const addBtn = document.getElementById('btn-add-comment');
    if (addBtn) {
        addBtn.addEventListener('click', async () => {
            const author = document.getElementById('comment-author').value.trim();
            const body = document.getElementById('comment-body').value.trim();
            if (!body) return;

            addBtn.disabled = true;
            const result = await api.createComment(state.currentProject.key, taskId, {
                author: author || 'Anonymous',
                body: body,
            });
            addBtn.disabled = false;

            if (result) {
                _showToast('Comment added', 'success');
                loadComments(taskId);
                // increment usage count
                if (state.universeInfo) {
                    state.universeInfo.content_count += 1;
                }
            }
        });
    }
}

export async function handleFormSubmit(e) {
    e.preventDefault();
    if (!state.currentProject) return;

    const submitBtn = document.querySelector('#btn-submit');
    if (submitBtn) submitBtn.disabled = true;

    const data = {
        title: document.querySelector('#task-title').value.trim(),
        status: document.querySelector('#task-status').value,
        priority: document.querySelector('#task-priority').value,
        description: document.querySelector('#task-description').value.trim(),
        labels: document.querySelector('#task-labels').value.split(',').map(l => l.trim()).filter(Boolean),
    };

    const assigneeVal = document.querySelector('#task-assignee').value.trim();
    if (assigneeVal) data.assignee = assigneeVal;

    const dueDate = document.querySelector('#task-due-date').value;
    if (dueDate) data.due_date = dueDate;

    const parentVal = document.querySelector('#task-parent').value;
    if (parentVal) data.parent = parseInt(parentVal);

    const ok = await _ensureOwnUniverse();
    if (!ok) { if (submitBtn) submitBtn.disabled = false; return; }

    const key = state.currentProject.key;

    let result;
    if (state.editingTaskId) {
        result = await api.updateTask(key, state.editingTaskId, data);
    } else {
        result = await api.createTask(key, data);
    }

    if (submitBtn) submitBtn.disabled = false;

    if (result) {
        try {
            const savedId = state.editingTaskId || (result.id);
            if (savedId) localStorage.removeItem(`co_draft_task_${savedId}`);
            else localStorage.removeItem('co_draft_new_task');
        } catch (_) {}
        _showToast(state.editingTaskId ? 'Task updated' : 'Task created', 'success');
        closeModal();
        await _refreshTasks();
        _render();
    }
}

export async function handleDelete() {
    if (!state.editingTaskId || !state.currentProject) return;
    if (!(await _ensureOwnUniverse())) return;
    if (!confirm('Delete this task?')) return;

    await api.deleteTask(state.currentProject.key, state.editingTaskId);
    _showToast('Task deleted', 'success');
    closeModal();
    await _refreshTasks();
    _render();
}

export async function handleArchive() {
    if (!state.editingTaskId || !state.currentProject) return;
    const task = state.tasks.find(t => t.id === state.editingTaskId);
    if (!task) return;

    const newArchived = !task.archived;
    const result = await api.updateTask(state.currentProject.key, state.editingTaskId, {
        archived: newArchived,
    });

    if (result) {
        _showToast(newArchived ? 'Task archived' : 'Task unarchived', 'success');
        closeModal();
        await _refreshTasks();
        _render();
    }
}

// ===== Activity panel =====
export async function toggleActivityPanel() {
    const panel = document.querySelector('#activity-panel');
    if (!panel) return;

    if (panel.classList.contains('hidden')) {
        panel.classList.remove('hidden');
        await loadActivity();
    } else {
        panel.classList.add('hidden');
    }
}

async function loadActivity() {
    const panel = document.querySelector('#activity-panel');
    if (!panel || !state.currentProject) return;

    panel.innerHTML = '<div class="activity-loading"><div class="spinner"></div><p>Loading activity...</p></div>';

    const activities = await api.getActivity(state.currentProject.key, 50);

    if (!activities || activities.length === 0) {
        panel.innerHTML = '<div class="activity-header"><h3>Activity</h3><button class="activity-close-btn" id="activity-close">&times;</button></div><p class="activity-empty">No recent activity</p>';
        setupActivityClose();
        return;
    }

    let html = '<div class="activity-header"><h3>Activity</h3><button class="activity-close-btn" id="activity-close">&times;</button></div>';
    html += '<div class="activity-list">';

    for (const entry of activities) {
        const actionLabel = ACTIVITY_ACTION_LABELS[entry.action] || entry.action;
        const timeLabel = relativeTime(entry.created_at || entry.timestamp);
        const actor = entry.actor || entry.user || 'Sistema';
        const target = entry.task_key || entry.target || '';

        let detailHtml = '';
        if (entry.action === 'field_changed' && entry.field) {
            detailHtml = `<span class="activity-detail">${esc(entry.field)}: ${esc(entry.old_value || '?')} &rarr; ${esc(entry.new_value || '?')}</span>`;
        }

        html += `
            <div class="activity-entry">
                <div class="activity-entry-header">
                    <span class="activity-actor">${esc(actor)}</span>
                    <span class="activity-action">${actionLabel}</span>
                    ${target ? `<span class="activity-target">${esc(target)}</span>` : ''}
                </div>
                ${detailHtml}
                <span class="activity-time">${timeLabel}</span>
            </div>`;
    }

    html += '</div>';
    panel.innerHTML = html;

    setupActivityClose();
}

export function setupActivityClose() {
    const closeBtn = document.getElementById('activity-close');
    if (closeBtn) {
        closeBtn.addEventListener('click', () => {
            const panel = document.querySelector('#activity-panel');
            if (panel) panel.classList.add('hidden');
        });
    }
}

// ===== Subscribe prompt modal (CO-253) =====
export function showSubscribePromptModal() {
    const overlay = document.getElementById('subscribe-prompt-overlay');
    if (!overlay) return;
    const msg = document.getElementById('subscribe-prompt-msg');
    const cta = document.getElementById('btn-subscribe-cta');
    const t = k => window.t ? window.t(k) : k;
    if (state.me) {
        if (msg) msg.textContent = t('subscribe.prompt.logged_in');
        if (cta) {
            cta.textContent = t('subscribe.cta');
            cta.style.display = '';
        }
    } else {
        if (msg) msg.textContent = t('subscribe.prompt.anonymous');
        if (cta) {
            cta.textContent = t('subscribe.login_cta');
            cta.style.display = '';
        }
    }
    overlay.classList.remove('hidden');
}

export function hideSubscribePromptModal() {
    const overlay = document.getElementById('subscribe-prompt-overlay');
    if (overlay) overlay.classList.add('hidden');
}

export function setupSubscribePromptModal() {
    const overlay = document.getElementById('subscribe-prompt-overlay');
    const cta = document.getElementById('btn-subscribe-cta');
    const cancel = document.getElementById('btn-subscribe-cancel');

    if (cancel) cancel.addEventListener('click', hideSubscribePromptModal);
    if (overlay) overlay.addEventListener('click', e => { if (e.target === overlay) hideSubscribePromptModal(); });

    if (cta) {
        cta.addEventListener('click', async () => {
            if (!state.me) {
                hideSubscribePromptModal();
                _showLoginModal();
                return;
            }
            const slug = state.currentUniverseSlug;
            cta.disabled = true;
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscribe`, { method: 'POST' }, true);
                hideSubscribePromptModal();
                await _loadMeUniverses();
                _renderSidebar();
                _showToast(window.t ? window.t('subscribe.cta') : 'Inscrito!', 'success');
            } catch (err) {
                _showToast('Erro ao se inscrever.', 'error');
            } finally {
                cta.disabled = false;
            }
        });
    }
}

// ===== Usage limit modal =====
export function showUsageLimitModal(data) {
    const overlay = document.getElementById('usage-limit-overlay');
    if (!overlay) return;
    const titleEl = document.getElementById('usage-limit-title');
    const msgEl = document.getElementById('usage-limit-msg');
    const current = data && data.current != null ? data.current : 100;
    if (titleEl) titleEl.textContent = window.t('universe.limit');
    if (msgEl) msgEl.textContent = window.t('universe.limit_msg').replace('{n}', current);
    overlay.classList.remove('hidden');
}

export function hideUsageLimitModal() {
    const overlay = document.getElementById('usage-limit-overlay');
    if (overlay) overlay.classList.add('hidden');
}

export function setupUsageLimitModal() {
    const btnLogin = document.getElementById('btn-usage-login');
    const btnLang = document.getElementById('btn-usage-lang');
    if (btnLogin) {
        btnLogin.addEventListener('click', () => {
            hideUsageLimitModal();
            _showLoginModal();
        });
    }
    if (btnLang) {
        btnLang.addEventListener('click', () => {
            // CO-556: setLang() dispatches co:langchange → single global re-render
            // (refetch:false). No explicit _render() — it double-fired and refetched.
            window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
        });
    }
}

// ===== Universe info modal =====
// These are bound at app.js level because they close over infoBody/infoTitle/infoOverlay
// We export a setup function that wires everything and uses injected callbacks.
let _infoBody = null;
let _infoTitle = null;
let _infoOverlay = null;

export function setupUniverseInfoModal(callbacks) {
    _infoBody = document.querySelector('#universe-info-body');
    _infoTitle = document.querySelector('#universe-info-title');
    _infoOverlay = document.querySelector('#universe-info-overlay');
    const infoClose = document.querySelector('#universe-info-close');
    const btnInfo = document.querySelector('#btn-universe-info');

    if (infoClose) infoClose.addEventListener('click', () => _infoOverlay && _infoOverlay.classList.add('hidden'));
    if (_infoOverlay) _infoOverlay.addEventListener('click', e => {
        if (e.target === _infoOverlay) _infoOverlay.classList.add('hidden');
    });

    if (btnInfo) {
        btnInfo.addEventListener('click', async () => {
            if (!_infoOverlay) return;
            _infoOverlay.classList.remove('hidden');
            await renderUniverseInfo();
        });
    }
}

export async function renderUniverseInfo() {
    const slug = state.currentUniverseSlug;
    if (!slug || !_infoBody) return;
    if (_infoTitle) _infoTitle.textContent = `Universo · ${slug}`;
    _infoBody.innerHTML = '<p class="empty-state" style="margin:16px">Carregando…</p>';
    try {
        const info = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}`, {}, true);
        let entries = [];
        try {
            const r = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/entries?limit=5000`, {}, true);
            entries = (r && r.entries) || [];
        } catch (_) {}
        let subscription = null;
        try {
            subscription = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscription`, {}, true);
        } catch (_) {}
        if (subscription && subscription.pinned_state) {
            state.subscriptionPin[slug] = subscription.pinned_state;
        } else {
            delete state.subscriptionPin[slug];
        }
        const stateEntries = entries.filter(e => (e.frontmatter || {}).type === 'state');
        const branchEntries = entries.filter(e => (e.frontmatter || {}).type === 'branch');
        const proposalEntries = entries.filter(e => (e.frontmatter || {}).type === 'proposal');
        const mergeEntries = entries.filter(e => (e.frontmatter || {}).type === 'merge');
        const typeCounts = {};
        for (const e of entries) {
            const t = (e.frontmatter || {}).type || '(none)';
            typeCounts[t] = (typeCounts[t] || 0) + 1;
        }
        renderInfoSections(slug, info, entries, stateEntries, branchEntries, proposalEntries, mergeEntries, typeCounts, subscription);
    } catch (err) {
        console.error('universe info fetch failed', err);
        _infoBody.innerHTML = '<p class="empty-state" style="margin:16px;color:#dc2626">Erro ao carregar informações.</p>';
    }
}

function renderInfoSections(slug, info, entries, stateEntries, branchEntries, proposalEntries, mergeEntries, typeCounts, subscription) {
    if (!_infoBody) return;
    const visBadge = info.visibility === 'private'
        ? '<span style="background:#fef3c7;color:#92400e;padding:2px 8px;border-radius:10px;font-size:11px">private</span>'
        : info.visibility === 'template'
            ? '<span style="background:#e0e7ff;color:#3730a3;padding:2px 8px;border-radius:10px;font-size:11px">template</span>'
            : '<span style="background:#d1fae5;color:#065f46;padding:2px 8px;border-radius:10px;font-size:11px">public-subscribable</span>';
    const typesHtml = Object.entries(typeCounts)
        .sort((a, b) => b[1] - a[1])
        .map(([t, n]) => `<div style="display:flex;justify-content:space-between;padding:3px 0;border-bottom:1px dashed var(--border,#e5e7eb);font-size:13px"><span>${esc(t)}</span><strong>${n}</strong></div>`)
        .join('');
    const overview = `
        <section style="padding:16px;border-bottom:1px solid var(--border,#e5e7eb)">
            <div style="display:flex;justify-content:space-between;align-items:baseline;gap:12px">
                <h3 style="margin:0;font-size:16px">${esc(info.name || slug)}</h3>
                ${visBadge}
            </div>
            <p style="margin:6px 0 0;color:var(--text-muted,#6b7280);font-size:13px">${esc(info.description || '')}</p>
            <div style="display:grid;grid-template-columns:repeat(2,1fr);gap:8px;margin-top:12px;font-size:13px">
                <div><strong>${info.content_count != null ? info.content_count : entries.length}</strong> entries</div>
                <div><strong>${stateEntries.length}</strong> states</div>
                <div><strong>${branchEntries.length}</strong> branches</div>
                <div style="color:var(--text-muted,#6b7280);font-size:11px">slug: <code>${esc(slug)}</code></div>
            </div>
        </section>`;
    const typesSection = `
        <section style="padding:16px;border-bottom:1px solid var(--border,#e5e7eb)">
            <h3 style="margin:0 0 8px;font-size:14px">Tipos de conteúdo</h3>
            ${typesHtml || '<em style="color:var(--text-muted,#6b7280);font-size:12px">Sem entradas.</em>'}
        </section>`;

    let subBlock = '';
    if (subscription && info.visibility === 'public-subscribable') {
        const pinPath = subscription.pinned_state || '';
        const pinShort = pinPath ? pinPath.split('/').pop().slice(0, 24) : '';
        let statusHtml;
        if (subscription.subscribed && pinPath) {
            statusHtml = `📌 Pinned to <code>${esc(pinShort)}…</code> <button id="info-unpin" class="btn-text-sm" style="margin-left:8px;color:#dc2626;background:none;border:none;cursor:pointer;font-size:11px;padding:2px 6px">unpin</button>`;
        } else if (subscription.subscribed) {
            statusHtml = `Subscribed (following head) <button id="info-unsubscribe" class="btn-text-sm" style="margin-left:8px;color:#dc2626;background:none;border:none;cursor:pointer;font-size:11px;padding:2px 6px">unsubscribe</button>`;
        } else {
            statusHtml = `Not subscribed <button id="info-subscribe" class="btn-text-sm" style="margin-left:8px;color:#1e40af;background:none;border:1px solid #1e40af;cursor:pointer;font-size:11px;padding:2px 8px;border-radius:3px">+ Subscribe</button>`;
        }
        subBlock = `<div style="font-size:11px;color:var(--text-muted,#6b7280);margin-bottom:8px">${statusHtml}</div>`;
    }
    const stateSection = `
        <section style="padding:16px;border-bottom:1px solid var(--border,#e5e7eb)">
            <div style="display:flex;justify-content:space-between;align-items:baseline;margin-bottom:8px">
                <h3 style="margin:0;font-size:14px">Estados (versões)</h3>
                <button class="btn btn-secondary btn-sm" id="info-save-state">⏱ Capturar estado</button>
            </div>
            ${subBlock}
            <div id="info-state-list" style="font-size:12px"></div>
        </section>`;
    const branchSection = `
        <section style="padding:16px;border-bottom:1px solid var(--border,#e5e7eb)">
            <h3 style="margin:0 0 8px;font-size:14px">Branches</h3>
            <div id="info-branch-list" style="font-size:12px"></div>
        </section>`;
    const propSection = `
        <section style="padding:16px">
            <h3 style="margin:0 0 8px;font-size:14px">Propostas e merges</h3>
            <div id="info-proposal-list" style="font-size:12px"></div>
        </section>`;
    _infoBody.innerHTML = overview + typesSection + stateSection + branchSection + propSection;
    renderStatesInto(_infoBody.querySelector('#info-state-list'), stateEntries, slug, (subscription && subscription.pinned_state) || null);
    renderBranchesInto(_infoBody.querySelector('#info-branch-list'), branchEntries, slug, stateEntries);
    renderProposalsInto(_infoBody.querySelector('#info-proposal-list'), proposalEntries, mergeEntries, slug, stateEntries);

    const unpinBtn = _infoBody.querySelector('#info-unpin');
    if (unpinBtn) {
        unpinBtn.addEventListener('click', async () => {
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscribe/pin`, { method: 'DELETE' }, true);
                _showToast('Unpinned (now following head)', 'success');
                delete state.subscriptionPin[slug];
                await renderUniverseInfo();
                if (state.currentUniverseSlug === slug) _renderContent();
            } catch (err) {
                _showToast('Failed to unpin', 'error');
            }
        });
    }

    const subBtn = _infoBody.querySelector('#info-subscribe');
    if (subBtn) {
        subBtn.addEventListener('click', async () => {
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscribe`, { method: 'POST' }, true);
                _showToast('Subscribed', 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to subscribe (login required?)', 'error');
            }
        });
    }
    const unsubBtn = _infoBody.querySelector('#info-unsubscribe');
    if (unsubBtn) {
        unsubBtn.addEventListener('click', async () => {
            try {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscribe`, { method: 'DELETE' }, true);
                _showToast('Unsubscribed', 'success');
                delete state.subscriptionPin[slug];
                await renderUniverseInfo();
                if (state.currentUniverseSlug === slug) _renderContent();
            } catch (err) {
                _showToast('Failed to unsubscribe', 'error');
            }
        });
    }
    const saveBtn = _infoBody.querySelector('#info-save-state');
    if (saveBtn) {
        saveBtn.addEventListener('click', async () => {
            if (state.isTemplate) { await _ensureOwnUniverse(); return; }
            const msg = window.prompt('Message for this state (optional):', '');
            if (msg === null) return;
            try {
                const r = await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(slug)}/states`,
                    { method: 'POST', body: JSON.stringify({ message: msg }), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`State saved · ${String(r.state_hash).slice(0, 12)}…`, 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to save state', 'error');
            }
        });
    }
}

function renderStatesInto(container, entries, slug, pinnedPath) {
    if (!container) return;
    if (!entries || entries.length === 0) {
        container.innerHTML = '<em style="color:var(--text-muted,#6b7280)">Sem estados ainda. Clique "⏱ Capturar estado" para criar o primeiro.</em>';
        return;
    }
    const sorted = entries.slice().sort((a, b) => (a.path < b.path ? 1 : -1));
    container.innerHTML = sorted.map(e => {
        const fm = e.frontmatter || {};
        const hash = (fm.state_hash || '').slice(0, 12);
        const msg = fm.message || '';
        const count = fm.entry_count != null ? fm.entry_count : '?';
        const created = fm.created_at || '';
        const author = fm.author ? `<span style="color:var(--text-muted,#6b7280);font-size:11px">by ${esc(String(fm.author).slice(0, 12))}</span>` : '';
        const parentPath = fm.parent || '';
        const ts = created ? new Date(created).toLocaleString() : '';
        const isPinned = pinnedPath && e.path === pinnedPath;
        const rowBg = isPinned ? 'background:rgba(59,130,246,0.06)' : '';
        const pinBtn = isPinned
            ? `<span title="Pinned to this state" style="font-size:10px;color:#1e40af">📌 pinned</span>`
            : `<button class="state-pin-btn" data-state-path="${esc(e.path)}" data-slug="${esc(slug)}" title="Pin your subscription to this version" style="background:none;border:1px solid var(--border,#e5e7eb);padding:1px 8px;border-radius:10px;font-size:10px;cursor:pointer;color:var(--text-muted,#6b7280)">📌 pin</button>`;
        return `
            <div class="state-row" style="padding:8px 0;border-bottom:1px solid var(--border,#e5e7eb);cursor:pointer;${rowBg}"
                 data-state-path="${esc(e.path)}" data-parent-path="${esc(parentPath)}" data-slug="${esc(slug)}"
                 title="Click to see what changed in this state">
                <div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline">
                    <code style="font-size:11px;font-family:var(--mono,monospace);background:var(--bg-subtle,#f3f4f6);padding:2px 6px;border-radius:3px">${esc(hash)}…</code>
                    <div style="display:flex;gap:6px;align-items:center">
                        ${pinBtn}
                        <span style="font-size:11px;color:var(--text-muted,#6b7280)">${esc(ts)}</span>
                    </div>
                </div>
                <div style="margin-top:4px">${esc(msg) || '<em style="color:var(--text-muted,#6b7280)">(no message)</em>'} · ${count} entries ${author}</div>
            </div>`;
    }).join('');

    container.querySelectorAll('.state-pin-btn').forEach(btn => {
        btn.addEventListener('click', async (ev) => {
            ev.stopPropagation();
            const statePath = btn.dataset.statePath;
            const rowSlug = btn.dataset.slug;
            try {
                await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(rowSlug)}/subscribe/pin`,
                    { method: 'PUT', body: JSON.stringify({ state: statePath }), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`Pinned to ${statePath.split('/').pop().slice(0, 16)}…`, 'success');
                state.subscriptionPin[rowSlug] = statePath;
                await renderUniverseInfo();
                if (state.currentUniverseSlug === rowSlug) _renderContent();
            } catch (err) {
                _showToast('Failed to pin (login required)', 'error');
            }
        });
    });
    container.querySelectorAll('.state-row').forEach(row => {
        row.addEventListener('click', async () => {
            const existing = row.querySelector('.state-diff');
            if (existing) { existing.remove(); return; }
            const statePath = row.dataset.statePath;
            const parentPath = row.dataset.parentPath;
            const rowSlug = row.dataset.slug;
            const panel = document.createElement('div');
            panel.className = 'state-diff';
            panel.style.cssText = 'margin-top:8px;padding:8px 10px;background:var(--bg-subtle,#f9fafb);border-radius:4px;font-size:11px;border:1px solid var(--border,#e5e7eb)';
            panel.innerHTML = '<em>Loading diff…</em>';
            row.appendChild(panel);
            if (!parentPath) {
                panel.innerHTML = '<em style="color:var(--text-muted,#6b7280)">First state — no parent to diff against.</em>';
                return;
            }
            try {
                const url = `/api/v1/universes/${encodeURIComponent(rowSlug)}/states/diff?from=${encodeURIComponent(parentPath)}&to=${encodeURIComponent(statePath)}`;
                const diff = await apiFetch(url, {}, true);
                const fmtList = (xs, sign, color) => xs.slice(0, 50).map(e =>
                    `<div style="padding-left:12px;color:${color};font-family:var(--mono,monospace)">${sign} ${esc(e.path)}</div>`
                ).join('') + (xs.length > 50 ? `<div style="padding-left:12px;color:var(--text-muted,#6b7280)">… and ${xs.length - 50} more</div>` : '');
                const parts = [];
                if (diff.added.length) parts.push(`<div style="color:#059669;font-weight:600">+${diff.added.length} added</div>` + fmtList(diff.added, '+', '#059669'));
                if (diff.modified.length) parts.push(`<div style="color:#d97706;font-weight:600;margin-top:4px">~${diff.modified.length} modified</div>` + fmtList(diff.modified, '~', '#d97706'));
                if (diff.removed.length) parts.push(`<div style="color:#dc2626;font-weight:600;margin-top:4px">-${diff.removed.length} removed</div>` + fmtList(diff.removed, '-', '#dc2626'));
                if (parts.length === 0) {
                    panel.innerHTML = `<em style="color:var(--text-muted,#6b7280)">No changes — ${diff.unchanged} entries unchanged.</em>`;
                } else {
                    panel.innerHTML = parts.join('') + `<div style="margin-top:6px;color:var(--text-muted,#6b7280)">${diff.unchanged} unchanged</div>`;
                }
            } catch (err) {
                panel.innerHTML = '<em style="color:#dc2626">Failed to load diff.</em>';
            }
        });
    });
}

function renderBranchesInto(container, branches, slug, stateEntries) {
    if (!container) return;
    const stateOptions = (stateEntries || [])
        .slice()
        .sort((a, b) => (a.path < b.path ? 1 : -1))
        .map(e => {
            const fm = e.frontmatter || {};
            const hash = (fm.state_hash || '').slice(0, 10);
            const msg = (fm.message || '').slice(0, 40);
            return `<option value="${esc(e.path)}">${esc(hash)}… ${msg ? '· ' + esc(msg) : ''}</option>`;
        }).join('');
    const list = (!branches || branches.length === 0)
        ? '<em style="color:var(--text-muted,#6b7280);font-size:11px;display:block;padding:4px 0">Sem branches ainda.</em>'
        : (() => {
            const sorted = branches.slice().sort((a, b) => {
                const da = (a.frontmatter || {}).default ? 0 : 1;
                const db = (b.frontmatter || {}).default ? 0 : 1;
                return da - db || (a.path < b.path ? -1 : 1);
            });
            return sorted.map(e => {
                const fm = e.frontmatter || {};
                const head = fm.head_state || '';
                const headShort = head ? head.split('/').pop().slice(0, 24) : '?';
                const isDefault = fm.default ? '<span style="background:#e0e7ff;color:#3730a3;padding:1px 6px;border-radius:8px;font-size:10px;margin-left:6px">default</span>' : '';
                const updated = fm.updated_at ? new Date(fm.updated_at).toLocaleString() : '';
                const branchName = fm.name || '';
                const advanceForm = (stateEntries && stateEntries.length > 0)
                    ? `<details style="margin-top:4px">
                            <summary style="cursor:pointer;color:var(--accent,#6366f1);font-size:11px">↳ advance head</summary>
                            <form class="info-branch-advance-form" data-branch-name="${esc(branchName)}" style="margin-top:4px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                                <select name="head_state" required style="padding:3px 6px;font-size:11px;border:1px solid var(--border,#e5e7eb);border-radius:3px;flex:1;min-width:200px">${stateOptions}</select>
                                <button type="submit" class="btn btn-secondary btn-sm" style="font-size:11px;padding:2px 8px">apply</button>
                            </form>
                        </details>`
                    : '';
                return `<div style="padding:6px 0;border-bottom:1px solid var(--border,#e5e7eb)">
                    <div><strong>${esc(fm.name || '?')}</strong>${isDefault}</div>
                    <div style="color:var(--text-muted,#6b7280);font-size:11px;margin-top:2px">
                        head: <code>${esc(headShort)}…</code> · updated ${esc(updated)}
                    </div>
                    ${advanceForm}
                </div>`;
            }).join('');
        })();

    const formHtml = (stateEntries && stateEntries.length > 0)
        ? `<details style="margin-top:8px">
                <summary style="cursor:pointer;color:var(--accent,#6366f1);font-size:12px">+ Nova branch</summary>
                <form id="info-branch-form" style="margin-top:8px;display:grid;gap:6px">
                    <input type="text" name="name" placeholder="branch name (e.g. feat/foo)" pattern="[a-zA-Z0-9_/-]{1,100}" required style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">
                    <select name="head_state" required style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">${stateOptions}</select>
                    <label style="font-size:11px;display:flex;gap:4px;align-items:center"><input type="checkbox" name="default"> default branch</label>
                    <button type="submit" class="btn btn-secondary btn-sm" style="font-size:12px">Criar branch</button>
                </form>
            </details>`
        : '<div style="font-size:11px;color:var(--text-muted,#6b7280);margin-top:6px">Capture um estado primeiro para poder criar uma branch.</div>';

    container.innerHTML = list + formHtml;

    const form = container.querySelector('#info-branch-form');
    if (form) {
        form.addEventListener('submit', async (ev) => {
            ev.preventDefault();
            const fd = new FormData(form);
            const body = {
                name: String(fd.get('name') || '').trim(),
                head_state: String(fd.get('head_state') || ''),
                default: !!fd.get('default'),
            };
            try {
                await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(slug)}/branches`,
                    { method: 'POST', body: JSON.stringify(body), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`Branch "${body.name}" created`, 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to create branch (login required?)', 'error');
            }
        });
    }

    container.querySelectorAll('.info-branch-advance-form').forEach(advForm => {
        advForm.addEventListener('submit', async (ev) => {
            ev.preventDefault();
            const branchName = advForm.dataset.branchName;
            const fd = new FormData(advForm);
            const headState = String(fd.get('head_state') || '');
            if (!branchName || !headState) return;
            try {
                await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(slug)}/branches/${encodeURIComponent(branchName)}`,
                    { method: 'PUT', body: JSON.stringify({ head_state: headState }), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`Branch "${branchName}" advanced`, 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to advance branch (login required?)', 'error');
            }
        });
    });
}

function renderProposalsInto(container, proposals, merges, slug, stateEntries) {
    if (!container) return;

    const items = [];
    for (const p of (proposals || [])) {
        const fm = p.frontmatter || {};
        const status = fm.status || 'open';
        const statusBadge = status === 'open'
            ? '<span style="background:#dbeafe;color:#1e40af;padding:1px 6px;border-radius:8px;font-size:10px">open</span>'
            : status === 'merged'
                ? '<span style="background:#d1fae5;color:#065f46;padding:1px 6px;border-radius:8px;font-size:10px">merged</span>'
                : `<span style="background:#fee2e2;color:#991b1b;padding:1px 6px;border-radius:8px;font-size:10px">${esc(status)}</span>`;
        const created = fm.created_at ? new Date(fm.created_at).toLocaleString() : '';
        const mergeBtn = status === 'open'
            ? `<button class="info-merge-btn" data-proposal-path="${esc(p.path)}" style="background:#1e40af;color:white;border:none;padding:2px 10px;border-radius:3px;font-size:11px;cursor:pointer">⇆ Merge</button>`
            : '';
        items.push({
            ts: fm.created_at || '',
            html: `<div style="padding:8px 0;border-bottom:1px solid var(--border,#e5e7eb)">
                <div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline">
                    <strong style="font-size:13px">${esc(fm.title || p.path)}</strong>
                    <div style="display:flex;gap:6px;align-items:center">${statusBadge}${mergeBtn}</div>
                </div>
                <div style="color:var(--text-muted,#6b7280);font-size:11px;margin-top:2px">
                    from <code>${esc(fm.source_universe || '?')}</code> · ${esc(created)}
                </div>
            </div>`,
        });
    }
    for (const m of (merges || [])) {
        const fm = m.frontmatter || {};
        const merged = fm.merged_at ? new Date(fm.merged_at).toLocaleString() : '';
        items.push({
            ts: fm.merged_at || '',
            html: `<div style="padding:8px 0;border-bottom:1px solid var(--border,#e5e7eb)">
                <div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline">
                    <strong style="font-size:13px">⇆ ${esc(fm.title || 'merge')}</strong>
                    <span style="background:#d1fae5;color:#065f46;padding:1px 6px;border-radius:8px;font-size:10px">${fm.entries_copied || 0} entries</span>
                </div>
                <div style="color:var(--text-muted,#6b7280);font-size:11px;margin-top:2px">
                    from <code>${esc(fm.source_universe || '?')}</code> → branch <code>${esc(fm.target_branch || 'main')}</code> · ${esc(merged)}
                </div>
            </div>`,
        });
    }
    items.sort((a, b) => (a.ts < b.ts ? 1 : -1));

    const listHtml = items.length === 0
        ? '<em style="color:var(--text-muted,#6b7280);font-size:11px;display:block;padding:4px 0">Sem propostas ou merges ainda.</em>'
        : items.map(x => x.html).join('');

    const stateOptions = (stateEntries || [])
        .slice()
        .sort((a, b) => (a.path < b.path ? 1 : -1))
        .map(e => {
            const fm = e.frontmatter || {};
            const hash = (fm.state_hash || '').slice(0, 10);
            const msg = (fm.message || '').slice(0, 40);
            return `<option value="${esc(e.path)}">${esc(hash)}… ${msg ? '· ' + esc(msg) : ''}</option>`;
        }).join('');
    const proposeHtml = (stateEntries && stateEntries.length > 0)
        ? `<details style="margin-top:8px">
                <summary style="cursor:pointer;color:var(--accent,#6366f1);font-size:12px">+ Propor mudanças a outro universo</summary>
                <form id="info-proposal-form" style="margin-top:8px;display:grid;gap:6px">
                    <input type="text" name="target_universe" placeholder="target universe slug (e.g. comunicacao)" required style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">
                    <input type="text" name="title" placeholder="proposal title" required maxlength="200" style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">
                    <textarea name="description" placeholder="description (optional)" rows="2" style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px;resize:vertical"></textarea>
                    <select name="source_state" required style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">${stateOptions}</select>
                    <input type="text" name="target_branch" value="main" placeholder="target_branch (default: main)" style="padding:4px 8px;font-size:12px;border:1px solid var(--border,#e5e7eb);border-radius:3px">
                    <button type="submit" class="btn btn-secondary btn-sm" style="font-size:12px">Criar proposta</button>
                </form>
            </details>`
        : '<div style="font-size:11px;color:var(--text-muted,#6b7280);margin-top:6px">Capture um estado primeiro para poder propor mudanças.</div>';

    container.innerHTML = listHtml + proposeHtml;

    const propForm = container.querySelector('#info-proposal-form');
    if (propForm) {
        propForm.addEventListener('submit', async (ev) => {
            ev.preventDefault();
            const fd = new FormData(propForm);
            const targetUniverse = String(fd.get('target_universe') || '').trim();
            if (!targetUniverse) return;
            const body = {
                source_universe: slug,
                source_state: String(fd.get('source_state') || ''),
                title: String(fd.get('title') || '').trim(),
                description: String(fd.get('description') || ''),
                target_branch: String(fd.get('target_branch') || 'main').trim() || 'main',
            };
            try {
                const r = await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(targetUniverse)}/proposals`,
                    { method: 'POST', body: JSON.stringify(body), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`Proposal created in ${targetUniverse} · ${(r.path || '').split('/').pop().slice(0, 24)}`, 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to create proposal', 'error');
            }
        });
    }

    container.querySelectorAll('.info-merge-btn').forEach(btn => {
        btn.addEventListener('click', async () => {
            const proposalPath = btn.dataset.proposalPath;
            if (!proposalPath) return;
            if (!window.confirm(`Merge ${proposalPath.split('/').pop()}? This will copy source-state entries into this universe.`)) return;
            try {
                const r = await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(slug)}/merges`,
                    { method: 'POST', body: JSON.stringify({ proposal: proposalPath }), headers: { 'Content-Type': 'application/json' } },
                    true,
                );
                _showToast(`Merged · ${r.entries_copied || 0} entries copied`, 'success');
                await renderUniverseInfo();
            } catch (err) {
                _showToast('Failed to merge', 'error');
            }
        });
    });
}
