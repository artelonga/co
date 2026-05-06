(function () {
    'use strict';

    // ===== State =====
    const state = {
        projects: [],
        currentProject: null,
        tasks: [],
        view: 'conteudo',
        editingTaskId: null,
        searchQuery: '',
        loading: false,
        showArchived: false,
        userUniverses: [],
        // 1.62.0 Phase 7: per-slug pin map. When set, the SPA appends
        // `?as_of=<pin>` to entry queries so the user sees the rewind view.
        // Populated on universe-info open and after pin/unpin actions.
        subscriptionPin: {},
        // Calendar
        calendarDate: new Date(),
        // Table
        sortColumn: 'key',
        sortDirection: 'asc',
        selectedIds: new Set(),
        collapsedGroups: new Set(),
        openStatusDropdown: null,
        // Timeline
        zoom: 'month',
        timelineStart: null,
        miniCalDate: new Date(),
        collapsedSwimlanes: {},
        unscheduledCollapsed: false,
        // Subtree expand/collapse (shared across views)
        collapsedSubtasks: new Set(),
        // Universe routing
        currentUniverseSlug: 'template',
        isTemplate: false,
        universeInfo: null,
        // Form config (CO-24): theme, layout, fonts from universe config endpoint
        universeConfig: null,
        // CO-38: Yggdrasil minigames hub
        isYggdrasil: false,
        gameView: null, // active game slug, e.g. 'tetris'
        // CO-73: universe manifest (loaded from /api/v1/universes/:slug/manifest)
        universeManifest: null,
        // CO-73: entries fetched for calendar/gantt views
        calendarEntries: [],
    };

    // ===== Obsidian Tasks compatibility (CO-37) =====
    // Maps CO status values ↔ Obsidian Tasks checkbox characters.
    // Checkbox line format: `- [c] Task title`
    //
    // | CO status   | checkbox |
    // |-------------|----------|
    // | todo        | ' '      |
    // | in_progress | '/'      |
    // | in_review   | '~'      |
    // | done        | 'x'      |

    const OBSIDIAN_CHECKBOX = {
        todo:        ' ',
        in_progress: '/',
        in_review:   '~',
        done:        'x',
    };

    const CHECKBOX_TO_STATUS = {
        ' ': 'todo',
        '/': 'in_progress',
        '~': 'in_review',
        'x': 'done',
        'X': 'done',
    };

    /** Convert a CO status string to an Obsidian Tasks checkbox character. */
    function statusToCheckbox(status) {
        return OBSIDIAN_CHECKBOX[status] ?? ' ';
    }

    /** Convert an Obsidian Tasks checkbox character to a CO status string. */
    function checkboxToStatus(c) {
        return CHECKBOX_TO_STATUS[c] ?? 'todo';
    }

    /**
     * Serialize a task object to an Obsidian Tasks markdown line.
     * Format: `- [c] Task title`
     *
     * @param {object} task - CO task with `status` and `title` fields
     * @returns {string} Obsidian Tasks markdown line
     */
    function taskToObsidianLine(task) {
        const c = statusToCheckbox(task.status ?? 'todo');
        return `- [${c}] ${task.title ?? 'Untitled'}`;
    }

    /**
     * Parse an Obsidian Tasks checkbox line and return the corresponding
     * CO status string.  Returns `null` if the line does not match.
     *
     * @param {string} line - A single line of markdown text
     * @returns {string|null} CO status string or null
     */
    function parseObsidianCheckboxLine(line) {
        const m = line.match(/^- \[(.)\] /);
        if (!m) return null;
        return checkboxToStatus(m[1]);
    }

    /**
     * Extract the CO status from the first line of a markdown body, if it
     * contains an Obsidian Tasks checkbox.  Frontmatter `status` should be
     * treated as canonical; this is only used when frontmatter is absent.
     *
     * @param {string} body - Markdown body text
     * @returns {string|null} CO status or null if no checkbox found
     */
    function extractStatusFromBody(body) {
        if (!body) return null;
        const firstLine = body.split('\n')[0];
        return parseObsidianCheckboxLine(firstLine);
    }

    // ===== Editor lazy-load =====
    let _editorInstance = null;
    let _editorBundlePromise = null;
    let _taskDraftInterval = null;

    function loadEditorBundle() {
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

        const draftKey = taskId ? `co_draft_task_${taskId}` : 'co_draft_new_task';

        // Try CodeMirror editor
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

                // Auto-save draft to localStorage every 5s
                _taskDraftInterval = setInterval(() => {
                    try {
                        const val = _editorInstance ? _editorInstance.getValue() : '';
                        localStorage.setItem(draftKey, val);
                    } catch (_) {}
                }, 5000);
                return;
            }
        } catch (_) { /* CodeMirror not available */ }

        // Fallback: show plain textarea
        if (container) container.style.display = 'none';
        if (textarea) {
            textarea.style.display = '';
            textarea.value = initialContent;
            textarea.rows = 6;
        }
    }

    // i18n is provided by /shared/i18n.js (loaded before this script).
    // window.t(), window.setLang(), window.currentLang are available.

    function buildStatuses() {
        return [
            { key: 'todo', label: window.t('status.todo'), color: '#94a3b8' },
            { key: 'in_progress', label: window.t('status.in_progress'), color: '#3b82f6' },
            { key: 'done', label: window.t('status.done'), color: '#22c55e' },
        ];
    }

    function buildPriorityLabels() {
        return {
            low: window.t('priority.low'),
            medium: window.t('priority.medium'),
            high: window.t('priority.high'),
            critical: window.t('priority.critical'),
        };
    }

    function buildStatusLabels() {
        return {
            todo: window.t('status.todo'),
            in_progress: window.t('status.in_progress'),
            in_review: window.t('status.in_review'),
            done: window.t('status.done'),
        };
    }

    let STATUSES = buildStatuses();
    let PRIORITY_LABELS = buildPriorityLabels();
    const PRIORITY_ORDER = { critical: 0, high: 1, medium: 2, low: 3 };
    const STATUS_ORDER = { todo: 0, in_progress: 1, done: 2, in_review: 3 };
    let STATUS_LABELS = buildStatusLabels();

    const ZOOM_DAYS = { week: 7, month: 30, quarter: 90 };
    const COL_WIDTHS = { week: 50, month: 40, quarter: 60 };

    const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
        'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    const MONTH_NAMES_FULL = ['January', 'February', 'March', 'April', 'May', 'June',
        'July', 'August', 'September', 'October', 'November', 'December'];
    const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
    const DAY_NAMES_MINI = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];

    const ACTIVITY_ACTION_LABELS = {
        task_created: 'created task',
        task_deleted: 'deleted task',
        field_changed: 'updated field in',
        comment_added: 'commented on',
        status_changed: 'changed status of',
        task_archived: 'archived task',
        task_unarchived: 'unarchived task',
    };

    // ===== Toast System =====
    function showToast(message, type) {
        const container = document.getElementById('toast-container');
        if (!container) return;

        const toast = document.createElement('div');
        toast.className = 'toast toast-' + (type || 'success');
        toast.textContent = message;
        container.appendChild(toast);

        setTimeout(() => {
            toast.classList.add('toast-fade-out');
            setTimeout(() => {
                if (toast.parentNode) toast.parentNode.removeChild(toast);
            }, 300);
        }, 3000);
    }

    // ===== Universe Form Config (CO-24 / CO-30) =====

    // Maps API theme_preset names to CSS data-palette attribute values (used for
    // structural palette rules in style.css — modal styling, input styling, etc.).
    const THEME_PALETTE_MAP = {
        'scholarly': 'scholarly',
        'scholarly-light': 'scholarly',
        'scholarly-dark': 'scholarly-dark',
        'relic': 'relic',
        'relic-light': 'relic-light',
        'modern': '',
        '': '',
    };

    // Dark/light companion pairs (CO-30).
    const THEME_COMPANION = {
        'scholarly': 'scholarly-dark',
        'scholarly-light': 'scholarly-dark',
        'scholarly-dark': 'scholarly',
        'relic': 'relic-light',
        'relic-light': 'relic',
        'modern': 'modern',
    };

    const DARK_THEMES = new Set(['scholarly-dark', 'relic']);

    // ---------------------------------------------------------------------------
    // CO-30: Load theme.css from the server for the active universe.
    // Hot-swaps the existing <link> element so there is no page reload.
    // ---------------------------------------------------------------------------
    function loadThemeCss(slug) {
        if (!slug) return;
        // User-level palette override wins: if `co_user_palette` is set, load
        // the preset CSS directly from `/api/v1/themes/<preset>` (this
        // endpoint reads from the compiled-in preset definition, independent
        // of any universe's stored `theme_preset`). Default to 'modern' if
        // unset so first-time visitors get the intended look across boards.
        let userPalette = null;
        try {
            userPalette = localStorage.getItem('co_user_palette');
            if (!userPalette) {
                userPalette = 'modern';
                localStorage.setItem('co_user_palette', 'modern');
            }
        } catch (_) {
            userPalette = 'modern';
        }
        // Cache-bust per-page-load so a recent palette change is picked up
        // even if the browser is sitting on a stale theme.css. The `?v=` is
        // ignored by the server but invalidates the browser cache key.
        const v = (window._coBootTs ||= Math.floor(Date.now() / 1000));
        const href = `/api/v1/themes/${encodeURIComponent(userPalette)}?v=${v}`;
        let link = document.getElementById('co-theme-css');
        if (link) {
            if (link.href !== new URL(href, document.baseURI).href) {
                link.href = href;
            }
        } else {
            link = document.createElement('link');
            link.id = 'co-theme-css';
            link.rel = 'stylesheet';
            link.href = href;
            document.head.appendChild(link);
        }
    }

    // Inject a Google Fonts <link rel="preload"> for custom font families (CO-30).
    function loadCustomFonts(config) {
        const fontFamilies = [config.font_headline, config.font_body]
            .filter(Boolean)
            .map(f => encodeURIComponent(f))
            .join('&family=');
        const existingFontLink = document.getElementById('co-universe-fonts');
        if (fontFamilies) {
            const href = `https://fonts.googleapis.com/css2?family=${fontFamilies}&display=swap`;
            if (existingFontLink) {
                existingFontLink.href = href;
            } else {
                // Preconnect hints (idempotent — only add if absent)
                if (!document.querySelector('link[href="https://fonts.googleapis.com"]')) {
                    const preconn1 = document.createElement('link');
                    preconn1.rel = 'preconnect';
                    preconn1.href = 'https://fonts.googleapis.com';
                    document.head.appendChild(preconn1);
                }
                if (!document.querySelector('link[href="https://fonts.gstatic.com"]')) {
                    const preconn2 = document.createElement('link');
                    preconn2.rel = 'preconnect';
                    preconn2.href = 'https://fonts.gstatic.com';
                    preconn2.crossOrigin = 'anonymous';
                    document.head.appendChild(preconn2);
                }
                const link = document.createElement('link');
                link.id = 'co-universe-fonts';
                link.rel = 'stylesheet';
                link.href = href;
                document.head.appendChild(link);
            }
        } else if (existingFontLink) {
            existingFontLink.remove();
        }
    }

    // Apply universe presentation config: theme.css link, data-palette, fonts, layout.
    function applyUniverseConfig(config) {
        if (!config) return;
        state.universeConfig = config;
        const slug = state.currentUniverseSlug;

        // User-level theme override (CO-94 follow-up): if `co_user_palette` is
        // set in localStorage, it wins over the universe's per-board theme.
        // Default to 'modern' for first-time visitors so the look is
        // consistent across all boards. User can later change via the palette
        // dropdown; clearing the override returns to per-universe themes.
        if (!localStorage.getItem('co_user_palette')) {
            try { localStorage.setItem('co_user_palette', 'modern'); } catch (_) {}
        }
        const userPalette = localStorage.getItem('co_user_palette');

        // Clear the legacy per-session override key so the user-level one wins.
        localStorage.removeItem('co_named_palette');
        document.documentElement.removeAttribute('data-palette');

        // 1. CO-30: Load generated theme.css from the server (hot-swap on change).
        // The user's palette override applies via data-palette attribute on
        // <html> below; CSS rules `[data-palette="modern"]` etc. provide the
        // color tokens that override the universe-themed defaults.
        if (slug) loadThemeCss(slug);

        // 2. Set data-palette attribute for structural CSS rules (modal styling, etc.).
        const effectivePreset = userPalette || config.theme_preset;
        const paletteKey = THEME_PALETTE_MAP[effectivePreset] ?? '';
        if (paletteKey) {
            document.documentElement.setAttribute('data-palette', paletteKey);
        } else {
            document.documentElement.removeAttribute('data-palette');
        }

        // 3. Inject Google Fonts for custom fonts (with <link rel="preload"> for CO-30).
        loadCustomFonts(config);

        // 4. Set default layout / view from config (board → kanban, others map directly).
        // Conteúdo is the universal default — every universe shows entries first,
        // and other views (kanban, calendar, etc.) are opt-in per layout config.
        const layoutToView = {
            'board': 'kanban',
            'table': 'table',
            'timeline': 'timeline',
            'calendar': 'calendar',
            'dashboard': 'dashboard',
            'conteudo': 'conteudo',
        };
        const defaultView = layoutToView[config.layout] || 'conteudo';
        if (state.view !== defaultView) {
            switchView(defaultView);
        }
    }

    // ===== Loading Helpers =====
    function showLoading() {
        state.loading = true;
        const content = $('#content');
        if (content) {
            content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Loading...</p></div>';
        }
    }

    function hideLoading() {
        state.loading = false;
    }

    function setSubmitDisabled(disabled) {
        const btn = $('#btn-submit');
        if (btn) btn.disabled = disabled;
    }

    // ===== API =====
    async function apiFetch(url, options, silent401 = false) {
        try {
            const r = await fetch(url, options);
            if (!r.ok) {
                if (r.status === 401) {
                    if (!silent401) showLoginModal();
                    return null;
                }
                let errData = null;
                try { errData = await r.json(); } catch (_) {}
                if (r.status === 402 && errData && errData.error === 'usage_limit') {
                    showUsageLimitModal(errData);
                    return null;
                }
                const errMsg = (errData && (errData.message || errData.error)) || 'Request error';
                showToast(errMsg, 'error');
                return null;
            }
            // DELETE / NO_CONTENT responses may have no body
            if (r.status === 204 || r.headers.get('content-length') === '0') {
                return {};
            }
            return r.json();
        } catch (err) {
            showToast('Connection error: ' + err.message, 'error');
            return null;
        }
    }

    const api = {
        // Append ?u=<slug> to board API URLs for universe scoping
        _u(url) {
            const slug = state.currentUniverseSlug;
            if (!slug) return url;
            return url + (url.includes('?') ? '&' : '?') + `u=${slug}`;
        },
        async getProjects() {
            const r = await apiFetch(this._u('/api/projects'), {}, true);
            return r || [];
        },
        async getTasks(key, opts) {
            let url = `/api/projects/${key}/tasks`;
            if (opts && typeof opts.archived === 'boolean') {
                url += '?archived=' + (opts.archived ? 'true' : 'false');
            }
            const r = await apiFetch(this._u(url), {}, true);
            return r || [];
        },
        async createTask(key, data) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks`), {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async updateTask(key, id, data) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}`), {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async deleteTask(key, id) {
            await apiFetch(this._u(`/api/projects/${key}/tasks/${id}`), { method: 'DELETE' });
        },
        async getComments(key, id) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}/comments`), {}, true);
            return r || [];
        },
        async createComment(key, id, data) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks/${id}/comments`), {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async getActivity(key, limit) {
            const l = limit || 50;
            const r = await apiFetch(this._u(`/api/projects/${key}/activity?limit=${l}`), {}, true);
            return r || [];
        },
        async getDashboard(key) {
            const r = await apiFetch(this._u(`/api/projects/${key}/dashboard`), {}, true);
            return r;
        },
        async bulkUpdateTasks(key, data) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks/bulk-update`), {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async bulkDeleteTasks(key, data) {
            const r = await apiFetch(this._u(`/api/projects/${key}/tasks/bulk-delete`), {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(data),
            });
            return r;
        },
        async me() {
            return apiFetch('/api/v1/auth/me', {}, true);
        },
        async logout() {
            await apiFetch('/api/v1/auth/logout', { method: 'POST' }, true);
        },
        async loginWithPassword(usuario, senha) {
            // If it looks like an email, use the universal password-login (CO-85).
            // Works on both UAT (yuri@uat.local/uat) and prod (yuri@artelonga.com.br/<password>).
            // The legacy uat-login endpoint is a 404 in prod by design; password-login replaces it.
            if (usuario.includes('@')) {
                const resp = await apiFetch('/api/v1/auth/password-login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ email: usuario, password: senha }),
                }, true);
                if (resp && resp.user_id) {
                    return { usuario: resp.display_name || resp.email, ...resp };
                }
            }
            // Fallback: quilombo legacy username/password login (no @ in identifier)
            return apiFetch('/api/v1/quilombo/auth/login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ usuario, senha }),
            }, true);
        },
        async getUniverses() {
            const r = await apiFetch('/api/v1/universes', {}, true);
            return r || [];
        },
        async getPublicacoes() {
            const r = await apiFetch('/api/v1/quilombo/publicacoes', {}, true);
            return r || [];
        },
        async getEventos() {
            const r = await apiFetch('/api/v1/quilombo/eventos', {}, true);
            return r || [];
        },
        async getMissoes() {
            const r = await apiFetch('/api/v1/quilombo/missoes', {}, true);
            return r || [];
        },
        async getUniverseProjects(slug) {
            const r = await apiFetch(`/api/v1/universes/${slug}/projects`, {}, true);
            return r || [];
        },
        async cloneUniverse(sourceSlug, body) {
            return apiFetch(`/api/v1/universes/${sourceSlug}/clone`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            });
        },
        async getUniverseInfo(slug) {
            return apiFetch(`/api/v1/universes/${slug}`, {}, true);
        },
        async listUniverses() {
            const r = await apiFetch('/api/v1/universes', {}, true);
            return r || [];
        },
        async claimUniverse(slug) {
            return apiFetch(`/api/v1/universes/${slug}/claim`, { method: 'POST' }, true);
        },
        // CO-24: universe form config (theme, layout, fonts)
        async getUniverseConfig(slug) {
            return apiFetch(`/api/v1/universes/${slug}/config`, {}, true);
        },
        async updateUniverseConfig(slug, config) {
            return apiFetch(`/api/v1/universes/${slug}/config`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(config),
            });
        },
        // CO-24: fetch entries by type from the universe.
        // 1.62.0: when `state.subscriptionPin` is set for this slug, append
        // `&as_of=<pin>` so the user gets the rewind view automatically.
        async getUniverseEntries(slug, type) {
            let url = type
                ? `/api/v1/universes/${slug}/entries?type=${encodeURIComponent(type)}`
                : `/api/v1/universes/${slug}/entries`;
            const pin = state.subscriptionPin && state.subscriptionPin[slug];
            if (pin) {
                url += (url.includes('?') ? '&' : '?') + 'as_of=' + encodeURIComponent(pin);
            }
            const r = await apiFetch(url, {}, true);
            return (r && r.entries) || [];
        },
        // CO-73: fetch the universe manifest (parsed _universe.yaml or default)
        async getUniverseManifest(slug) {
            return apiFetch(`/api/v1/universes/${slug}/manifest`, {}, true);
        },
        // CO-73: fetch entries by date semantic and optional UTC date range
        async getEntriesByDate(slug, semantic, from, to) {
            let url = `/api/v1/universes/${slug}/entries?date_semantic=${encodeURIComponent(semantic)}`;
            if (from) url += `&from=${encodeURIComponent(from)}`;
            if (to)   url += `&to=${encodeURIComponent(to)}`;
            const r = await apiFetch(url, {}, true);
            return (r && r.entries) || [];
        },
    };

    // ===== Auto-clone on first interaction =====
    // When a visitor interacts with the template (drag, edit, create),
    // silently clone into an anonymous universe so writes persist server-side.
    // The clone slug is cached in localStorage for subsequent visits.

    let _cloning = false;
    async function ensureOwnUniverse() {
        if (!state.isTemplate) return true; // already on own universe
        // Template is read-only — prompt login to create own community
        showLoginModal();
        return false;
    }

    // Stub for backward compat (drag handler references these)
    function saveLocalTaskOverrides() {}
    function applyLocalTaskOverrides() {}

    // ===== Helpers =====
    function esc(s) {
        const d = document.createElement('div');
        d.textContent = s;
        return d.innerHTML;
    }

    function $(sel) { return document.querySelector(sel); }

    function formatDate(d) {
        if (!d) return '';
        const dt = new Date(d + 'T00:00:00');
        return dt.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
    }

    function isOverdue(d) {
        if (!d) return false;
        return new Date(d + 'T23:59:59') < new Date();
    }

    function assigneeInitials(name) {
        if (!name) return '';
        const parts = name.trim().split(/[\s@.]+/).filter(Boolean);
        if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
        return parts[0].slice(0, 2).toUpperCase();
    }

    function getSubtasks(task) {
        return state.tasks.filter(t => t.parent === task.id);
    }

    function getSubtaskProgress(task) {
        const subs = getSubtasks(task);
        if (subs.length === 0) return null;
        const done = subs.filter(t => t.status === 'done').length;
        return { done, total: subs.length };
    }

    // ===== Subtree State (localStorage) =====
    function loadSubtreeState(projectKey) {
        try {
            const raw = localStorage.getItem('co_subtree_' + projectKey);
            state.collapsedSubtasks = new Set(raw ? JSON.parse(raw) : []);
        } catch (e) {
            state.collapsedSubtasks = new Set();
        }
    }

    function saveSubtreeState() {
        if (!state.currentProject) return;
        localStorage.setItem('co_subtree_' + state.currentProject.key, JSON.stringify([...state.collapsedSubtasks]));
    }

    function toggleSubtree(taskId) {
        if (state.collapsedSubtasks.has(taskId)) {
            state.collapsedSubtasks.delete(taskId);
        } else {
            state.collapsedSubtasks.add(taskId);
        }
        saveSubtreeState();
    }

    function filteredTasks() {
        const q = state.searchQuery.toLowerCase();
        if (!q) return state.tasks;
        return state.tasks.filter(t =>
            t.title.toLowerCase().includes(q) ||
            t.key.toLowerCase().includes(q) ||
            t.labels.some(l => l.toLowerCase().includes(q))
        );
    }

    // ===== Relative Time Helper =====
    function relativeTime(dateStr) {
        if (!dateStr) return '';
        const now = new Date();
        const past = new Date(dateStr);
        const diffMs = now - past;
        if (diffMs < 0) return 'agora';

        const seconds = Math.floor(diffMs / 1000);
        if (seconds < 60) return 'agora';

        const minutes = Math.floor(seconds / 60);
        if (minutes < 60) return minutes + ' min ago';

        const hours = Math.floor(minutes / 60);
        if (hours < 24) return hours + ' h ago';

        const days = Math.floor(hours / 24);
        if (days < 30) return days + (days === 1 ? ' day ago' : ' days ago');

        const months = Math.floor(days / 30);
        if (months < 12) return months + (months === 1 ? ' month ago' : ' months ago');

        const years = Math.floor(months / 12);
        return years + (years === 1 ? ' year ago' : ' years ago');
    }

    // ===== Timeline helpers =====
    function toDateStr(d) {
        const y = d.getFullYear();
        const m = String(d.getMonth() + 1).padStart(2, '0');
        const day = String(d.getDate()).padStart(2, '0');
        return `${y}-${m}-${day}`;
    }

    function parseDate(s) {
        if (!s) return null;
        return new Date(s + 'T00:00:00');
    }

    function addDays(d, n) {
        const r = new Date(d);
        r.setDate(r.getDate() + n);
        return r;
    }

    function daysBetween(a, b) {
        const msPerDay = 86400000;
        return Math.round((b - a) / msPerDay);
    }

    function isWeekend(d) {
        const day = d.getDay();
        return day === 0 || day === 6;
    }

    function getWeekNumber(d) {
        const start = new Date(d.getFullYear(), 0, 1);
        const diff = d - start;
        return Math.ceil((diff / 86400000 + start.getDay() + 1) / 7);
    }

    function todayDate() {
        const now = new Date();
        return new Date(now.getFullYear(), now.getMonth(), now.getDate());
    }

    function formatDateShort(d) {
        return d.toLocaleDateString('en-US', { day: '2-digit', month: 'short' });
    }

    // ===== Table helpers =====
    function sortTasks(tasks) {
        const col = state.sortColumn;
        const dir = state.sortDirection === 'asc' ? 1 : -1;

        return [...tasks].sort((a, b) => {
            let va, vb;
            switch (col) {
                case 'key':
                    va = a.id;
                    vb = b.id;
                    break;
                case 'title':
                    va = a.title.toLowerCase();
                    vb = b.title.toLowerCase();
                    break;
                case 'status':
                    va = STATUS_ORDER[a.status] || 0;
                    vb = STATUS_ORDER[b.status] || 0;
                    break;
                case 'priority':
                    va = PRIORITY_ORDER[a.priority] || 99;
                    vb = PRIORITY_ORDER[b.priority] || 99;
                    break;
                case 'due_date':
                    va = a.due_date || '9999-12-31';
                    vb = b.due_date || '9999-12-31';
                    break;
                case 'labels':
                    va = a.labels.join(',').toLowerCase();
                    vb = b.labels.join(',').toLowerCase();
                    break;
                default:
                    va = a.id;
                    vb = b.id;
            }

            if (va < vb) return -1 * dir;
            if (va > vb) return 1 * dir;
            return 0;
        });
    }

    function groupTasksByStatus(tasks) {
        const groups = {};
        for (const s of STATUSES) {
            groups[s.key] = [];
        }
        for (const t of tasks) {
            if (groups[t.status]) {
                groups[t.status].push(t);
            }
        }
        return groups;
    }

    // ===== CO-73: Timezone-aware date helpers =====

    /** User's local IANA timezone (e.g. "America/Sao_Paulo"). */
    function userTimezone() {
        return Intl.DateTimeFormat().resolvedOptions().timeZone;
    }

    /**
     * Render a UTC ISO-8601 string (e.g. "2026-06-01T00:00:00Z") in the user's
     * local timezone, returning a YYYY-MM-DD string for calendar cell matching.
     */
    function utcToLocalDate(utcStr) {
        if (!utcStr) return '';
        try {
            const d = new Date(utcStr);
            return d.toLocaleDateString('en-CA', { timeZone: userTimezone() }); // en-CA gives YYYY-MM-DD
        } catch (_) {
            return utcStr.slice(0, 10);
        }
    }

    // ===== CO-73: Manifest-driven view tabs =====

    const MANIFEST_TAB_CLASS = 'manifest-injected-tab';

    /** Remove any view tabs injected from a previous manifest load. */
    function removeManifestViewTabs() {
        document.querySelectorAll(`.${MANIFEST_TAB_CLASS}`).forEach(el => el.remove());
    }

    /**
     * Inject extra view tabs declared in the manifest (e.g. gantt views).
     * Called once per universe boot after the manifest is fetched.
     */
    function injectManifestViewTabs(manifest) {
        removeManifestViewTabs();
        const viewTabs = document.getElementById('view-tabs');
        if (!viewTabs) return;
        const ganttViews = (manifest.views || []).filter(v => v.type === 'gantt');
        for (const v of ganttViews) {
            const btn = document.createElement('button');
            btn.className = `view-tab ${MANIFEST_TAB_CLASS}`;
            btn.dataset.view = `gantt:${v.name}`;
            btn.dataset.ganttView = JSON.stringify(v);
            btn.innerHTML = '<span class="view-tab-icon material-symbols-outlined">view_timeline</span>'
                + `<span class="view-tab-label">${esc(v.name)}</span>`;
            btn.addEventListener('click', () => switchView(btn.dataset.view));
            viewTabs.appendChild(btn);
        }
    }

    // ===== Timeline date range =====
    function getTimelineRange() {
        const days = ZOOM_DAYS[state.zoom];
        const start = state.timelineStart || todayDate();
        // For quarter zoom, generate week columns
        if (state.zoom === 'quarter') {
            const columns = [];
            let current = new Date(start);
            // Align to Monday
            const dayOfWeek = current.getDay();
            const offset = dayOfWeek === 0 ? -6 : 1 - dayOfWeek;
            current.setDate(current.getDate() + offset);
            const numWeeks = Math.ceil(days / 7);
            for (let i = 0; i < numWeeks; i++) {
                const weekStart = new Date(current);
                const weekEnd = addDays(weekStart, 6);
                columns.push({ date: weekStart, endDate: weekEnd, type: 'week' });
                current = addDays(current, 7);
            }
            return { columns, startDate: columns[0].date, endDate: columns[columns.length - 1].endDate };
        }
        // Day columns
        const columns = [];
        for (let i = 0; i < days; i++) {
            const date = addDays(start, i);
            columns.push({ date, type: 'day' });
        }
        return { columns, startDate: start, endDate: addDays(start, days - 1) };
    }

    function initTimelineStart() {
        const today = todayDate();
        const days = ZOOM_DAYS[state.zoom];
        // Start a few days before today so today is visible
        const offset = state.zoom === 'week' ? 1 : Math.floor(days * 0.2);
        state.timelineStart = addDays(today, -offset);
    }

    // ===== Render: Sidebar =====
    function renderSidebar() {
        const list = $('#project-list');

        // Universe switcher (visible when logged in with multiple universes).
        // CO-98: hierarchical rendering — universes with `parent_key` referring
        // to another universe in the user's list are nested under that parent
        // with a 16px indent + chevron. Orphan children (parent not in the
        // user's universes) fall back to top-level rendering.
        let universeHtml = '';
        if (state.userUniverses && state.userUniverses.length >= 1) {
            const universes = state.userUniverses;
            const seen = new Set(universes.map(u => u.key));
            const childrenByParent = {};
            const topLevel = [];
            universes.forEach(u => {
                if (u.parent_key && seen.has(u.parent_key)) {
                    (childrenByParent[u.parent_key] = childrenByParent[u.parent_key] || []).push(u);
                } else {
                    topLevel.push(u);
                }
            });

            const renderUniverseItem = (u, depth) => {
                const active = u.key === state.currentUniverseSlug ? ' active' : '';
                const kids = childrenByParent[u.key];
                const hasKids = !!(kids && kids.length);
                const expandKey = `co_universe_tree_${u.key}`;
                const stored = localStorage.getItem(expandKey);
                const containsActive = hasKids
                    && kids.some(k => k.key === state.currentUniverseSlug);
                const expanded = stored !== null ? (stored === '1') : containsActive;
                const indent = 12 + depth * 16;
                const chevron = hasKids
                    ? `<span class="sidebar-universe-chevron" data-toggle="${u.key}" style="display:inline-block;width:14px;text-align:center;cursor:pointer;user-select:none">${expanded ? '▾' : '▸'}</span>`
                    : '<span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>';
                let html = `<div class="sidebar-item sidebar-universe-item${active}" data-universe="${u.key}" style="padding-left:${indent}px">
                    ${chevron}<span class="sidebar-item-name">${esc(u.name || u.key)}</span>
                </div>`;
                if (hasKids && expanded) {
                    for (const k of kids) html += renderUniverseItem(k, depth + 1);
                }
                return html;
            };

            universeHtml = `<div class="sidebar-universes">
                <div class="sidebar-universe-label">${window.t ? window.t('universes') : 'Universos'}</div>
                ${topLevel.map(u => renderUniverseItem(u, 0)).join('')}
                <hr class="sidebar-divider">
            </div>`;
        }

        list.innerHTML = universeHtml + state.projects.map(p => {
            const active = state.currentProject?.key === p.key ? ' active' : '';
            return `
                <div class="sidebar-item${active}" data-key="${p.key}">
                    <span class="sidebar-item-key">${esc(p.key)}</span>
                    <span class="sidebar-item-name">${esc(p.name)}</span>
                </div>`;
        }).join('');

        // CO-98: chevron click toggles expansion without switching universes.
        // Bind BEFORE the universe-item click handler so e.stopPropagation is
        // honored (the universe-item handler does an async universe switch).
        list.querySelectorAll('.sidebar-universe-chevron[data-toggle]').forEach(c => {
            c.addEventListener('click', (e) => {
                e.stopPropagation();
                const k = c.dataset.toggle;
                const expandKey = `co_universe_tree_${k}`;
                const stored = localStorage.getItem(expandKey);
                const wasOpen = stored === null
                    ? (state.userUniverses || []).some(u => u.parent_key === k && u.key === state.currentUniverseSlug)
                    : stored === '1';
                localStorage.setItem(expandKey, wasOpen ? '0' : '1');
                renderSidebar();
            });
        });

        // Universe switching (click) + rename (double-click)
        list.querySelectorAll('.sidebar-universe-item').forEach(el => {
            el.addEventListener('click', async () => {
                const slug = el.dataset.universe;
                if (slug === state.currentUniverseSlug) return;
                if (state.switchingUniverse) return; // ignore rapid double-clicks during transition
                // Mark the clicked item active immediately so the sidebar
                // reflects the user's intent without waiting for the fetch.
                list.querySelectorAll('.sidebar-universe-item').forEach(x => x.classList.remove('active'));
                el.classList.add('active');
                setUniverseSlugInUrl(slug);
                state.currentUniverseSlug = slug;
                state.isTemplate = (slug === 'template');
                if (state.isTemplate) showTemplateBanner(); else hideTemplateBanner();
                await bootAppForUniverse(slug);
            });
            el.addEventListener('dblclick', (e) => {
                e.stopPropagation();
                const slug = el.dataset.universe;
                const nameSpan = el.querySelector('.sidebar-item-name');
                const oldName = nameSpan.textContent;
                const input = document.createElement('input');
                input.type = 'text';
                input.value = oldName;
                input.className = 'sidebar-rename-input';
                nameSpan.replaceWith(input);
                input.focus();
                input.select();
                const save = async () => {
                    const newName = input.value.trim() || oldName;
                    const span = document.createElement('span');
                    span.className = 'sidebar-item-name';
                    span.textContent = newName;
                    input.replaceWith(span);
                    if (newName !== oldName) {
                        await apiFetch(`/api/v1/universes/${slug}`, {
                            method: 'PUT',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ name: newName }),
                        }, true);
                        const u = state.userUniverses.find(u => u.key === slug);
                        if (u) u.name = newName;
                    }
                };
                input.addEventListener('blur', save);
                input.addEventListener('keydown', (e) => {
                    if (e.key === 'Enter') input.blur();
                    if (e.key === 'Escape') { input.value = oldName; input.blur(); }
                });
            });
        });

        // Project selection
        list.querySelectorAll('.sidebar-item:not(.sidebar-universe-item)').forEach(el => {
            if (el.dataset.key) el.addEventListener('click', () => selectProject(el.dataset.key));
        });
    }

    // ===== Render: Header =====
    function renderHeader() {
        const p = state.currentProject;
        $('#project-name').textContent = p ? p.name : (window.t ? window.t('select_project') : 'Selecione um projeto');
        $('#project-desc').textContent = p ? (p.description || '') : '';
        renderUsageCount();
    }

    function renderUsageCount() {
        const el = document.getElementById('usage-count');
        if (!el) return;
        const info = state.universeInfo;
        if (!info || state.isTemplate) {
            el.classList.add('hidden');
            return;
        }
        el.classList.remove('hidden');
        if (info.is_anonymous) {
            el.textContent = `${info.content_count} / 100 entradas`;
        } else {
            el.textContent = `${info.content_count} entradas`;
        }
    }

    function incrementLocalUsageCount() {
        if (state.universeInfo) {
            state.universeInfo.content_count += 1;
            renderUsageCount();
        }
    }

    // ===== Render: Mini Calendar (sidebar, for timeline) =====
    function renderMiniCalendar() {
        const container = $('#mini-calendar');
        if (!container) return;

        // Show mini calendar only when timeline is active
        if (state.view !== 'timeline') {
            container.classList.add('hidden');
            return;
        }
        container.classList.remove('hidden');

        const d = state.miniCalDate;
        const year = d.getFullYear();
        const month = d.getMonth();
        const firstDay = new Date(year, month, 1);
        const lastDay = new Date(year, month + 1, 0);
        const startDay = firstDay.getDay();
        const daysInMonth = lastDay.getDate();
        const today = todayDate();

        // Build set of dates with tasks
        const taskDates = new Set();
        for (const t of state.tasks) {
            if (t.due_date) taskDates.add(t.due_date);
        }

        const totalCells = Math.ceil((startDay + daysInMonth) / 7) * 7;
        let cells = '';

        for (let i = 0; i < totalCells; i++) {
            const dayNum = i - startDay + 1;
            const isCurrentMonth = dayNum >= 1 && dayNum <= daysInMonth;
            let cellDate = null;
            let displayNum = '';
            let classes = 'mini-cal-day';

            if (isCurrentMonth) {
                const dateObj = new Date(year, month, dayNum);
                cellDate = toDateStr(dateObj);
                displayNum = dayNum;
                if (dateObj.getTime() === today.getTime()) classes += ' today';
                if (taskDates.has(cellDate)) classes += ' has-tasks';
            } else if (dayNum < 1) {
                const prevLast = new Date(year, month, 0).getDate();
                displayNum = prevLast + dayNum;
                classes += ' other-month';
            } else {
                displayNum = dayNum - daysInMonth;
                classes += ' other-month';
            }

            cells += `<div class="${classes}" data-date="${cellDate || ''}">${displayNum}</div>`;
        }

        container.innerHTML = `
            <div class="mini-cal-header">
                <span class="mini-cal-title">${MONTH_NAMES_FULL[month]} ${year}</span>
                <div class="mini-cal-nav">
                    <button class="mini-cal-nav-btn" id="mini-cal-prev">&lsaquo;</button>
                    <button class="mini-cal-nav-btn" id="mini-cal-next">&rsaquo;</button>
                </div>
            </div>
            <div class="mini-cal-grid">
                ${DAY_NAMES_MINI.map(n => `<div class="mini-cal-day-header">${n}</div>`).join('')}
                ${cells}
            </div>`;

        // Events
        $('#mini-cal-prev').addEventListener('click', () => {
            state.miniCalDate = new Date(year, month - 1, 1);
            renderMiniCalendar();
        });
        $('#mini-cal-next').addEventListener('click', () => {
            state.miniCalDate = new Date(year, month + 1, 1);
            renderMiniCalendar();
        });

        container.querySelectorAll('.mini-cal-day').forEach(el => {
            el.addEventListener('click', () => {
                const dateStr = el.dataset.date;
                if (!dateStr) return;
                scrollToDate(parseDate(dateStr));
            });
        });
    }

    // ===== Render: Kanban =====
    function renderKanban() {
        const content = $('#content');
        content.className = 'content';
        const tasks = filteredTasks();
        const taskIds = new Set(tasks.map(t => t.id));
        // Only root-level tasks appear as top-level cards; subtasks render inside parent
        const rootTasks = tasks.filter(t => !t.parent || !taskIds.has(t.parent));

        content.innerHTML = `<div class="kanban">${STATUSES.map(s => {
            const colTasks = rootTasks.filter(t => t.status === s.key);
            return `
                <div class="kanban-column" data-status="${s.key}">
                    <div class="kanban-column-header">
                        ${s.label}
                        <span class="column-count">${colTasks.length}</span>
                    </div>
                    <div class="kanban-cards" data-status="${s.key}">
                        ${colTasks.map(t => renderTaskCard(t)).join('')}
                    </div>
                </div>`;
        }).join('')}</div>`;

        setupDragDrop();
        setupCardClicks();
        setupSubtreeToggles();
    }

    function renderTaskCard(task) {
        const subtasks = getSubtasks(task);
        const parentTask = task.parent ? state.tasks.find(t => t.id === task.parent) : null;
        const parentKey = parentTask ? `${state.currentProject.key}-${parentTask.id}` : null;
        const overdue = task.status !== 'done' && isOverdue(task.due_date);
        const hasSubtasks = subtasks.length > 0;
        const collapsed = state.collapsedSubtasks.has(task.id);
        const sp = hasSubtasks ? getSubtaskProgress(task) : null;

        const subtaskHtml = hasSubtasks ? `
            <div class="subtask-toggle" data-task-id="${task.id}">
                <span class="subtask-chevron${collapsed ? '' : ' open'}">&#9660;</span>
                <span class="subtask-toggle-label">${subtasks.length} subtask${subtasks.length !== 1 ? 's' : ''}</span>
                ${sp ? `<span class="subtask-badge">${sp.done}/${sp.total}</span>` : ''}
            </div>
            <div class="subtask-list${collapsed ? ' hidden' : ''}">
                ${subtasks.map(sub => renderSubtaskKanbanItem(sub)).join('')}
            </div>` : '';

        // Description preview: first paragraph as plain text (no raw markdown escapes)
        const rawPreview = window.CoMarkdown
            ? window.CoMarkdown.extractFirstParagraph(task.description || '')
            : (task.description || '').split('\n').find(l => l.trim() && !l.startsWith('#') && !l.startsWith('```')) || '';
        const descSnippet = rawPreview.length > 100 ? rawPreview.slice(0, 100) + '…' : rawPreview;

        return `
            <div class="task-card" draggable="true" data-task-id="${task.id}">
                <div class="task-card-header">
                    <span class="task-key">${esc(task.key)}</span>
                    ${parentKey ? `<span class="task-parent-key">${esc(parentKey)}</span>` : ''}
                </div>
                <div class="task-title">${esc(task.title)}</div>
                ${descSnippet ? `<div class="task-desc-preview">${esc(descSnippet)}</div>` : ''}
                <div class="task-meta">
                    ${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}
                    ${task.due_date ? `<span class="due-date-badge${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
                    ${task.assignee ? `<span class="assignee-badge" title="${esc(task.assignee)}">${esc(assigneeInitials(task.assignee))}</span>` : ''}
                </div>
                ${subtaskHtml}
            </div>`;
    }

    function renderSubtaskKanbanItem(task) {
        const statusInfo = STATUSES.find(s => s.key === task.status);
        const overdue = task.status !== 'done' && isOverdue(task.due_date);
        return `<div class="subtask-item" data-task-id="${task.id}">
            <span class="subtask-item-key">${esc(task.key)}</span>
            <span class="subtask-item-title">${esc(task.title)}</span>
            ${task.due_date ? `<span class="subtask-item-due${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span>` : ''}
        </div>`;
    }

    // ===== Render: Calendar =====
    async function renderCalendar() {
        const content = $('#content');
        content.className = 'content';
        const d = state.calendarDate;
        const year = d.getFullYear();
        const month = d.getMonth();
        const firstDay = new Date(year, month, 1);
        const lastDay = new Date(year, month + 1, 0);
        const startDay = firstDay.getDay(); // 0=Sun
        const daysInMonth = lastDay.getDate();
        const today = new Date();

        const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

        // CO-73: determine calendar date field from manifest
        const manifest = state.universeManifest;
        let calendarSemantic = null;
        if (manifest) {
            for (const ct of manifest.content_types || []) {
                if (ct.presentation && ct.presentation.calendar && ct.presentation.calendar.date_field) {
                    // Find the semantic for this date field
                    const fieldDef = ct.schema && ct.schema[ct.presentation.calendar.date_field];
                    if (fieldDef && fieldDef.semantic) {
                        calendarSemantic = fieldDef.semantic;
                        break;
                    }
                }
            }
        }

        // Build entries/tasks by local date
        const tasksByDate = {};
        if (calendarSemantic) {
            // CO-73: fetch from date-semantic API for the current month
            const fromDate = `${year}-${String(month + 1).padStart(2, '0')}-01`;
            const toDate = `${year}-${String(month + 1).padStart(2, '0')}-${String(daysInMonth).padStart(2, '0')}`;
            const slug = state.currentUniverseSlug;
            try {
                const entries = await api.getEntriesByDate(slug, calendarSemantic, fromDate, toDate);
                state.calendarEntries = entries;
                for (const e of entries) {
                    // Get the date value from frontmatter using the calendar field name
                    const calField = (manifest.content_types || []).find(ct =>
                        ct.presentation && ct.presentation.calendar
                    );
                    const fieldName = calField && calField.presentation.calendar.date_field;
                    const rawDate = fieldName && e.frontmatter && e.frontmatter[fieldName];
                    const localDate = rawDate ? utcToLocalDate(rawDate) : '';
                    if (localDate) {
                        if (!tasksByDate[localDate]) tasksByDate[localDate] = [];
                        tasksByDate[localDate].push({ ...e, _isEntry: true });
                    }
                }
            } catch (err) {
                console.warn('calendar: failed to fetch entries by date', err);
            }
        } else {
            // Legacy: use due_date from tasks
            const tasks = filteredTasks();
            for (const t of tasks) {
                if (t.due_date) {
                    if (!tasksByDate[t.due_date]) tasksByDate[t.due_date] = [];
                    tasksByDate[t.due_date].push(t);
                }
            }
        }

        // Build day cells
        const totalCells = Math.ceil((startDay + daysInMonth) / 7) * 7;
        let cells = '';
        for (let i = 0; i < totalCells; i++) {
            const dayNum = i - startDay + 1;
            const isCurrentMonth = dayNum >= 1 && dayNum <= daysInMonth;
            let cellDate = '';
            let displayNum = '';
            let otherClass = '';

            if (isCurrentMonth) {
                cellDate = `${year}-${String(month + 1).padStart(2, '0')}-${String(dayNum).padStart(2, '0')}`;
                displayNum = dayNum;
            } else if (dayNum < 1) {
                const prevLast = new Date(year, month, 0).getDate();
                displayNum = prevLast + dayNum;
                otherClass = ' other-month';
            } else {
                displayNum = dayNum - daysInMonth;
                otherClass = ' other-month';
            }

            const isToday = isCurrentMonth &&
                today.getFullYear() === year &&
                today.getMonth() === month &&
                today.getDate() === dayNum;

            const dayItems = tasksByDate[cellDate] || [];
            const maxShow = 3;

            cells += `
                <div class="calendar-day${otherClass}${isToday ? ' today' : ''}">
                    <div class="calendar-day-num">${displayNum}</div>
                    ${dayItems.slice(0, maxShow).map(item => item._isEntry
                        ? `<div class="calendar-task" title="${esc(item.title || item.path)}">${esc(item.title || item.path)}</div>`
                        : `<div class="calendar-task status-${item.status}" data-task-id="${item.id}" title="${esc(item.key)}: ${esc(item.title)}">${esc(item.key)} ${esc(item.title)}</div>`
                    ).join('')}
                    ${dayItems.length > maxShow ? `<div class="calendar-more">+${dayItems.length - maxShow} mais</div>` : ''}
                </div>`;
        }

        content.innerHTML = `
            <div class="calendar">
                <div class="calendar-nav">
                    <div class="calendar-nav-buttons">
                        <button class="calendar-nav-btn" id="cal-prev">&larr;</button>
                        <button class="calendar-nav-btn" id="cal-today">Hoje</button>
                        <button class="calendar-nav-btn" id="cal-next">&rarr;</button>
                    </div>
                    <h2>${MONTH_NAMES_FULL[month]} ${year}</h2>
                </div>
                <div class="calendar-grid">
                    ${dayNames.map(n => `<div class="calendar-day-header">${n}</div>`).join('')}
                    ${cells}
                </div>
            </div>`;

        // Calendar nav events
        $('#cal-prev').addEventListener('click', () => {
            state.calendarDate = new Date(year, month - 1, 1);
            renderCalendar();
        });
        $('#cal-next').addEventListener('click', () => {
            state.calendarDate = new Date(year, month + 1, 1);
            renderCalendar();
        });
        $('#cal-today').addEventListener('click', () => {
            state.calendarDate = new Date();
            renderCalendar();
        });

        // Calendar task clicks
        content.querySelectorAll('.calendar-task').forEach(el => {
            el.addEventListener('click', (e) => {
                e.stopPropagation();
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });
    }

    // ===== CO-73: Render: Gantt =====
    // Renders a Gantt chart for a manifest-declared gantt view.
    // `viewDef` is a View object: { name, type: 'gantt', source, date_start, date_end }
    async function renderGantt(viewDef) {
        const content = $('#content');
        content.className = 'content';
        content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div></div>';

        const slug = state.currentUniverseSlug;
        const startField = viewDef.date_start;
        const endField = viewDef.date_end;

        // Fetch entries with date_start semantic to get the date range
        let entries = [];
        try {
            entries = await api.getEntriesByDate(slug, startField, null, null);
        } catch (err) {
            console.warn('gantt: failed to fetch entries', err);
        }

        // Sort by start date ascending
        entries.sort((a, b) => {
            const da = (a.frontmatter && a.frontmatter[startField]) || '';
            const db = (b.frontmatter && b.frontmatter[startField]) || '';
            return da.localeCompare(db);
        });

        // Determine chart date range
        const today = new Date();
        let minDate = today;
        let maxDate = new Date(today.getFullYear(), today.getMonth() + 2, 1);
        for (const e of entries) {
            const start = e.frontmatter && e.frontmatter[startField];
            const end   = e.frontmatter && e.frontmatter[endField];
            if (start) {
                const d = new Date(start);
                if (d < minDate) minDate = d;
            }
            if (end) {
                const d = new Date(end);
                if (d > maxDate) maxDate = d;
            }
        }
        // Pad by a week on each side
        minDate = addDays(minDate, -7);
        maxDate = addDays(maxDate, 7);

        const totalDays = Math.max(1, Math.round((maxDate - minDate) / 86400000));
        const colW = 28; // px per day

        // Build header (month labels)
        let headerHtml = '';
        let cur = new Date(minDate);
        while (cur <= maxDate) {
            headerHtml += `<div class="gantt-month-label" style="left:${Math.round((cur - minDate) / 86400000) * colW}px">${MONTH_NAMES_FULL[cur.getMonth()]} ${cur.getFullYear()}</div>`;
            cur = new Date(cur.getFullYear(), cur.getMonth() + 1, 1);
        }

        // Today marker
        const todayOffset = Math.round((today - minDate) / 86400000) * colW;
        const todayMarker = `<div class="gantt-today-marker" style="left:${todayOffset}px"></div>`;

        // Build rows
        const rows = entries.map(e => {
            const title = (e.frontmatter && e.frontmatter.title) || e.path;
            const startStr = e.frontmatter && e.frontmatter[startField];
            const endStr   = e.frontmatter && e.frontmatter[endField];
            const startD = startStr ? new Date(startStr) : null;
            const endD   = endStr   ? new Date(endStr)   : null;

            let barHtml = '';
            if (startD) {
                const barLeft = Math.max(0, Math.round((startD - minDate) / 86400000)) * colW;
                const barRight = endD ? Math.max(0, Math.round((maxDate - endD) / 86400000)) * colW : 0;
                const barWidth = Math.max(colW, totalDays * colW - barLeft - barRight);
                barHtml = `<div class="gantt-bar" style="left:${barLeft}px;width:${barWidth}px" title="${esc(startStr || '')} → ${esc(endStr || '')}"></div>`;
            }

            return `<div class="gantt-row">
                <div class="gantt-row-label" title="${esc(title)}">${esc(title)}</div>
                <div class="gantt-row-track" style="width:${totalDays * colW}px">
                    ${barHtml}
                </div>
            </div>`;
        }).join('');

        content.innerHTML = `
            <div class="gantt-view">
                <div class="gantt-header">
                    <div class="gantt-header-label">${esc(viewDef.name)}</div>
                    <div class="gantt-header-dates" style="width:${totalDays * colW}px;position:relative">
                        ${headerHtml}
                        ${todayMarker}
                    </div>
                </div>
                <div class="gantt-body">
                    ${rows || '<div class="gantt-empty">Nenhum item com datas declaradas.</div>'}
                </div>
            </div>`;
    }

    // Build a depth-ordered list for a task group, respecting collapse state
    function buildGroupHierarchy(groupTasks) {
        const groupIds = new Set(groupTasks.map(t => t.id));
        const childrenOf = {};
        for (const t of groupTasks) {
            if (t.parent && groupIds.has(t.parent)) {
                if (!childrenOf[t.parent]) childrenOf[t.parent] = [];
                childrenOf[t.parent].push(t);
            }
        }
        const roots = groupTasks.filter(t => !t.parent || !groupIds.has(t.parent));
        const result = [];
        function flatten(task, depth) {
            const children = childrenOf[task.id] || [];
            result.push({ task, depth, hasGroupChildren: children.length > 0 });
            if (!state.collapsedSubtasks.has(task.id)) {
                for (const child of children) flatten(child, depth + 1);
            }
        }
        for (const root of roots) flatten(root, 0);
        return result;
    }

    // ===== Render: Table =====
    function renderTable() {
        const content = $('#content');
        content.className = 'content no-padding';
        const tasks = filteredTasks();
        const sorted = sortTasks(tasks);
        const groups = groupTasksByStatus(sorted);

        const allIds = sorted.map(t => t.id);
        const allSelected = allIds.length > 0 && allIds.every(id => state.selectedIds.has(id));

        let html = '';

        // Bulk actions bar
        if (state.selectedIds.size > 0) {
            html += `
                <div class="bulk-bar" id="bulk-bar">
                    <span><span class="bulk-bar-count">${state.selectedIds.size}</span> selecionada(s)</span>
                    <div class="bulk-bar-actions">
                        <div class="bulk-status-wrapper">
                            <button class="btn btn-bulk" id="bulk-move-btn">Mover para...</button>
                            <div class="bulk-status-dropdown hidden" id="bulk-status-dropdown">
                                ${STATUSES.map(s => `
                                    <button class="bulk-status-option" data-status="${s.key}">
                                        <span class="status-dd-dot" style="background:${s.color}"></span>
                                        ${s.label}
                                    </button>
                                `).join('')}
                            </div>
                        </div>
                        <button class="btn btn-bulk" id="bulk-archive-btn">Archive</button>
                        <button class="btn btn-bulk btn-danger" id="bulk-delete-btn">Delete</button>
                    </div>
                    <button class="btn" id="bulk-bar-close">&times; Limpar</button>
                </div>`;
        }

        html += '<div class="table-container">';

        // Render each status group
        for (const status of STATUSES) {
            const groupTasks = groups[status.key];
            if (groupTasks.length === 0) continue;

            const collapsed = state.collapsedGroups.has(status.key);
            const chevronSvg = `<svg class="group-chevron" viewBox="0 0 16 16" fill="none"><path d="M5 3l5 5-5 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

            html += `<div class="status-group${collapsed ? ' collapsed' : ''}" data-group="${status.key}">`;
            html += `
                <div class="status-group-header" data-group="${status.key}">
                    ${chevronSvg}
                    <span class="group-dot" style="background:${status.color}"></span>
                    <span class="status-group-label">${status.label}</span>
                    <span class="status-group-count">${groupTasks.length}</span>
                </div>`;

            html += '<div class="status-group-body">';
            html += '<table class="data-table">';
            html += `<thead><tr>
                <th class="col-checkbox">
                    <div class="row-checkbox">
                        <input type="checkbox" class="select-all-cb" data-group="${status.key}" ${allSelected ? 'checked' : ''}>
                    </div>
                </th>
                <th class="col-key"><div class="th-inner${sortClass('key')}" data-sort="key">Key${sortArrow('key')}</div></th>
                <th class="col-title"><div class="th-inner${sortClass('title')}" data-sort="title">Title${sortArrow('title')}</div></th>
                <th class="col-status"><div class="th-inner${sortClass('status')}" data-sort="status">Status${sortArrow('status')}</div></th>
                <th class="col-priority"><div class="th-inner${sortClass('priority')}" data-sort="priority">Prioridade${sortArrow('priority')}</div></th>
                <th class="col-due-date"><div class="th-inner${sortClass('due_date')}" data-sort="due_date">Data Limite${sortArrow('due_date')}</div></th>
                <th class="col-assignee"><div class="th-inner">Responsável</div></th>
                <th class="col-labels"><div class="th-inner${sortClass('labels')}" data-sort="labels">Labels${sortArrow('labels')}</div></th>
            </tr></thead>`;
            html += '<tbody>';

            for (const { task, depth, hasGroupChildren } of buildGroupHierarchy(groupTasks)) {
                const selected = state.selectedIds.has(task.id);
                const overdue = task.status !== 'done' && isOverdue(task.due_date);
                const collapsed = state.collapsedSubtasks.has(task.id);
                const indent = depth * 20;
                const connector = depth > 0 ? '<span class="tree-line">└─</span>' : '';
                const toggleBtn = hasGroupChildren
                    ? `<button class="tree-toggle" data-task-id="${task.id}" title="${collapsed ? 'Expand' : 'Collapse'}">${collapsed ? '▶' : '▼'}</button>`
                    : '<span class="tree-toggle-spacer"></span>';
                html += `
                    <tr data-task-id="${task.id}" class="${selected ? 'selected' : ''}${depth > 0 ? ' subtask-row' : ''}">
                        <td>
                            <div class="row-checkbox">
                                <input type="checkbox" class="task-cb" data-task-id="${task.id}" ${selected ? 'checked' : ''}>
                            </div>
                        </td>
                        <td><span class="cell-key">${esc(task.key)}</span></td>
                        <td>
                            <div class="cell-title-tree" style="padding-left:${indent}px">
                                ${connector}${toggleBtn}<span class="cell-title">${esc(task.title)}</span>
                            </div>
                        </td>
                        <td>
                            <span class="status-badge status-${task.status}" data-task-id="${task.id}">
                                <span class="status-badge-dot"></span>
                                ${STATUS_LABELS[task.status]}
                            </span>
                        </td>
                        <td>
                            <span class="cell-priority">
                                <span class="priority-dot ${task.priority}"></span>
                                <span class="priority-label">${PRIORITY_LABELS[task.priority]}</span>
                            </span>
                        </td>
                        <td><span class="cell-due-date${overdue ? ' overdue' : ''}">${formatDate(task.due_date)}</span></td>
                        <td>${task.assignee ? `<span class="assignee-badge" title="${esc(task.assignee)}">${esc(assigneeInitials(task.assignee))}</span>` : ''}</td>
                        <td><span class="cell-labels">${task.labels.map(l => `<span class="label-badge">${esc(l)}</span>`).join('')}</span></td>
                    </tr>`;
            }

            html += '</tbody></table>';
            html += '</div>'; // status-group-body
            html += '</div>'; // status-group
        }

        // Empty state when no tasks match filter
        if (sorted.length === 0) {
            html += '<div class="empty-state"><p>No tasks found</p></div>';
        }

        html += '</div>'; // table-container

        content.innerHTML = html;

        setupTableEvents();
    }

    function sortClass(col) {
        if (state.sortColumn !== col) return '';
        return ` sorted${state.sortDirection === 'desc' ? ' desc' : ''}`;
    }

    function sortArrow(col) {
        if (state.sortColumn !== col) {
            return ' <span class="sort-arrow">&#9650;</span>';
        }
        return state.sortDirection === 'asc'
            ? ' <span class="sort-arrow">&#9650;</span>'
            : ' <span class="sort-arrow">&#9660;</span>';
    }

    // ===== Table Events =====
    function setupTableEvents() {
        // Sort headers
        document.querySelectorAll('.th-inner[data-sort]').forEach(th => {
            th.addEventListener('click', (e) => {
                e.stopPropagation();
                const col = th.dataset.sort;
                if (state.sortColumn === col) {
                    state.sortDirection = state.sortDirection === 'asc' ? 'desc' : 'asc';
                } else {
                    state.sortColumn = col;
                    state.sortDirection = 'asc';
                }
                renderTable();
            });
        });

        // Group toggle
        document.querySelectorAll('.status-group-header').forEach(hdr => {
            hdr.addEventListener('click', () => {
                const group = hdr.dataset.group;
                if (state.collapsedGroups.has(group)) {
                    state.collapsedGroups.delete(group);
                } else {
                    state.collapsedGroups.add(group);
                }
                renderTable();
            });
        });

        // Tree toggle (subtask expand/collapse in table)
        document.querySelectorAll('.tree-toggle').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                toggleSubtree(parseInt(btn.dataset.taskId));
                renderTable();
            });
        });

        // Row clicks (open modal)
        document.querySelectorAll('.data-table tbody tr').forEach(row => {
            row.addEventListener('click', (e) => {
                // Don't open modal when clicking checkbox or status badge
                if (e.target.closest('.row-checkbox') || e.target.closest('.status-badge')) return;
                const taskId = parseInt(row.dataset.taskId);
                openTaskModal(taskId);
            });
        });

        // Checkbox: individual
        document.querySelectorAll('.task-cb').forEach(cb => {
            cb.addEventListener('change', (e) => {
                e.stopPropagation();
                const taskId = parseInt(cb.dataset.taskId);
                if (cb.checked) {
                    state.selectedIds.add(taskId);
                } else {
                    state.selectedIds.delete(taskId);
                }
                renderTable();
            });
            cb.addEventListener('click', (e) => e.stopPropagation());
        });

        // Checkbox: select all (per group)
        document.querySelectorAll('.select-all-cb').forEach(cb => {
            cb.addEventListener('change', (e) => {
                e.stopPropagation();
                const groupKey = cb.dataset.group;
                const groupTasks = filteredTasks().filter(t => t.status === groupKey);
                if (cb.checked) {
                    groupTasks.forEach(t => state.selectedIds.add(t.id));
                } else {
                    groupTasks.forEach(t => state.selectedIds.delete(t.id));
                }
                renderTable();
            });
            cb.addEventListener('click', (e) => e.stopPropagation());
        });

        // Bulk bar close
        const bulkClose = document.getElementById('bulk-bar-close');
        if (bulkClose) {
            bulkClose.addEventListener('click', () => {
                state.selectedIds.clear();
                renderTable();
            });
        }

        // Bulk move (status change)
        const bulkMoveBtn = document.getElementById('bulk-move-btn');
        if (bulkMoveBtn) {
            bulkMoveBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                const dd = document.getElementById('bulk-status-dropdown');
                if (dd) dd.classList.toggle('hidden');
            });
        }

        // Bulk status option clicks
        document.querySelectorAll('.bulk-status-option').forEach(opt => {
            opt.addEventListener('click', async (e) => {
                e.stopPropagation();
                const newStatus = opt.dataset.status;
                const taskIds = Array.from(state.selectedIds);
                if (taskIds.length === 0) return;
                const result = await api.bulkUpdateTasks(state.currentProject.key, {
                    task_ids: taskIds,
                    status: newStatus,
                });
                if (result) {
                    showToast(taskIds.length + ' task(s) updated', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
                const dd = document.getElementById('bulk-status-dropdown');
                if (dd) dd.classList.add('hidden');
            });
        });

        // Bulk archive
        const bulkArchiveBtn = document.getElementById('bulk-archive-btn');
        if (bulkArchiveBtn) {
            bulkArchiveBtn.addEventListener('click', async () => {
                const taskIds = Array.from(state.selectedIds);
                if (taskIds.length === 0) return;
                const result = await api.bulkUpdateTasks(state.currentProject.key, {
                    task_ids: taskIds,
                    archived: true,
                });
                if (result) {
                    showToast(taskIds.length + ' task(s) archived', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
            });
        }

        // Bulk delete
        const bulkDeleteBtn = document.getElementById('bulk-delete-btn');
        if (bulkDeleteBtn) {
            bulkDeleteBtn.addEventListener('click', async () => {
                const taskIds = Array.from(state.selectedIds);
                if (taskIds.length === 0) return;
                if (!confirm('Are you sure you want to delete ' + taskIds.length + ' task(s)? This action cannot be undone.')) return;
                const result = await api.bulkDeleteTasks(state.currentProject.key, {
                    task_ids: taskIds,
                });
                if (result) {
                    showToast(taskIds.length + ' task(s) deleted', 'success');
                    state.selectedIds.clear();
                    await refreshTasks();
                    renderTable();
                }
            });
        }

        // Status badge clicks -> dropdown
        document.querySelectorAll('.status-badge').forEach(badge => {
            badge.addEventListener('click', (e) => {
                e.stopPropagation();
                const taskId = parseInt(badge.dataset.taskId);
                toggleStatusDropdown(badge, taskId);
            });
        });
    }

    // ===== Status Dropdown =====
    function toggleStatusDropdown(badge, taskId) {
        closeStatusDropdown();

        const task = state.tasks.find(t => t.id === taskId);
        if (!task) return;

        const dropdown = document.createElement('div');
        dropdown.className = 'status-dropdown';
        dropdown.id = 'active-status-dropdown';

        dropdown.innerHTML = STATUSES.map(s => `
            <button class="status-dropdown-item${s.key === task.status ? ' active' : ''}" data-status="${s.key}">
                <span class="status-dd-dot" style="background:${s.color}"></span>
                ${s.label}
            </button>
        `).join('');

        badge.style.position = 'relative';
        badge.appendChild(dropdown);

        state.openStatusDropdown = taskId;

        dropdown.querySelectorAll('.status-dropdown-item').forEach(item => {
            item.addEventListener('click', async (e) => {
                e.stopPropagation();
                const newStatus = item.dataset.status;
                if (newStatus !== task.status) {
                    task.status = newStatus;
                    await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
                }
                closeStatusDropdown();
                renderTable();
            });
        });

        // Close on outside click
        setTimeout(() => {
            document.addEventListener('click', closeStatusDropdownOnOutside);
        }, 0);
    }

    function closeStatusDropdownOnOutside(e) {
        const dd = document.getElementById('active-status-dropdown');
        if (dd && !dd.contains(e.target)) {
            closeStatusDropdown();
        }
    }

    function closeStatusDropdown() {
        const existing = document.getElementById('active-status-dropdown');
        if (existing) existing.remove();
        state.openStatusDropdown = null;
        document.removeEventListener('click', closeStatusDropdownOnOutside);
    }

    // ===== Render: Timeline =====
    function renderTimeline() {
        const content = $('#content');
        content.className = 'content timeline-mode';
        const tasks = filteredTasks();
        const range = getTimelineRange();
        const colWidth = COL_WIDTHS[state.zoom];
        const today = todayDate();

        // Update CSS variable for column width
        document.documentElement.style.setProperty('--timeline-col-width', colWidth + 'px');

        // Separate scheduled vs unscheduled
        const scheduled = tasks.filter(t => t.due_date || t.created_at);
        const unscheduled = tasks.filter(t => !t.due_date && !t.created_at);

        // Group by status
        const grouped = {};
        for (const s of STATUSES) grouped[s.key] = [];
        for (const t of scheduled) {
            if (grouped[t.status]) grouped[t.status].push(t);
        }

        // Build header
        const headerCols = range.columns.map((col, idx) => {
            if (col.type === 'week') {
                // Quarter zoom: month label at first week of each month, week number below
                const isFirst = idx === 0;
                const showMonth = isFirst || col.date.getDate() <= 7;
                const monthLabel = showMonth ? MONTH_NAMES[col.date.getMonth()] : '';
                const wn = getWeekNumber(col.date);
                return `<div class="timeline-date-col week-col">
                    <span class="timeline-date-month">${monthLabel || '&nbsp;'}</span>
                    <span class="timeline-date-week-label">W${wn}</span>
                </div>`;
            }
            const d = col.date;
            const isToday = d.getTime() === today.getTime();
            const weekend = isWeekend(d);
            let classes = 'timeline-date-col';
            if (isToday) classes += ' today';
            if (weekend) classes += ' weekend';

            let topLabel;
            if (state.zoom === 'month') {
                // Month zoom: week number on Mondays or first column
                const isMonday = d.getDay() === 1;
                const isFirst = idx === 0;
                topLabel = (isMonday || isFirst)
                    ? `<span class="timeline-date-week-label">W${getWeekNumber(d)}</span>`
                    : '<span class="timeline-date-week-label">&nbsp;</span>';
            } else {
                // Week zoom: month name on 1st or first column
                const showMonth = d.getDate() === 1 || idx === 0;
                topLabel = showMonth
                    ? `<span class="timeline-date-month">${MONTH_NAMES[d.getMonth()]}</span>`
                    : '<span class="timeline-date-month">&nbsp;</span>';
            }

            return `<div class="${classes}">
                ${topLabel}
                <span class="timeline-date-day">${d.getDate()}</span>
                <span class="timeline-date-weekday">${DAY_NAMES[d.getDay()]}</span>
            </div>`;
        }).join('');

        // Build swimlanes
        const swimlanesHtml = STATUSES.map(s => {
            const laneTasks = grouped[s.key];
            const collapsed = state.collapsedSwimlanes[s.key] || false;
            const laneIds = new Set(laneTasks.map(t => t.id));
            // Root tasks: no parent in this same swimlane
            const laneRoots = laneTasks.filter(t => !t.parent || !laneIds.has(t.parent));

            const taskRows = laneRoots.map(t => {
                const laneSubtasks = laneTasks.filter(sub => sub.parent === t.id);
                const hasSubtasks = laneSubtasks.length > 0;
                const subtreeCollapsed = state.collapsedSubtasks.has(t.id);

                const gridCells = range.columns.map(col => {
                    let cls = 'timeline-grid-cell';
                    if (col.type === 'week') cls += ' week-col';
                    else if (isWeekend(col.date)) cls += ' weekend';
                    return `<div class="${cls}"></div>`;
                }).join('');

                const subRows = (hasSubtasks && !subtreeCollapsed) ? laneSubtasks.map(sub => {
                    const subGridCells = range.columns.map(col => {
                        let cls = 'timeline-grid-cell';
                        if (col.type === 'week') cls += ' week-col';
                        else if (isWeekend(col.date)) cls += ' weekend';
                        return `<div class="${cls}"></div>`;
                    }).join('');
                    return `<div class="timeline-task-row timeline-subtask-row" data-task-id="${sub.id}">
                        <div class="timeline-task-label timeline-subtask-label" data-task-id="${sub.id}">
                            <span class="timeline-subtask-indent"></span>
                            <span class="task-label-priority ${sub.priority}"></span>
                            <span class="task-label-text">${esc(sub.title)}</span>
                            <span class="task-label-key">${esc(sub.key)}</span>
                        </div>
                        <div class="timeline-task-grid" data-task-id="${sub.id}">
                            ${subGridCells}
                        </div>
                    </div>`;
                }).join('') : '';

                const toggleBtn = hasSubtasks
                    ? `<button class="timeline-subtree-toggle" data-task-id="${t.id}">${subtreeCollapsed ? '▶' : '▼'}</button>`
                    : '';

                return `<div class="timeline-task-row" data-task-id="${t.id}">
                    <div class="timeline-task-label" data-task-id="${t.id}">
                        ${toggleBtn}
                        <span class="task-label-priority ${t.priority}"></span>
                        <span class="task-label-text">${esc(t.title)}</span>
                        <span class="task-label-key">${esc(t.key)}</span>
                    </div>
                    <div class="timeline-task-grid" data-task-id="${t.id}">
                        ${gridCells}
                    </div>
                </div>${subRows}`;
            }).join('');

            const totalRows = laneRoots.reduce((n, t) => {
                const laneSubtasks = laneTasks.filter(sub => sub.parent === t.id);
                return n + 1 + (state.collapsedSubtasks.has(t.id) ? 0 : laneSubtasks.length);
            }, 0);

            return `<div class="timeline-swimlane" data-status="${s.key}">
                <div class="timeline-swimlane-header" data-status="${s.key}">
                    <span class="swimlane-dot" style="background:${s.color}"></span>
                    <span class="swimlane-label">${s.label}</span>
                    <span class="swimlane-count">${laneTasks.length}</span>
                    <span class="swimlane-toggle${collapsed ? ' collapsed' : ''}">&#9660;</span>
                </div>
                <div class="timeline-swimlane-body${collapsed ? ' collapsed' : ''}" style="max-height:${collapsed ? '0' : totalRows * 50 + 'px'}">
                    ${taskRows}
                </div>
            </div>`;
        }).join('');

        // Unscheduled section
        const unschedCollapsed = state.unscheduledCollapsed;
        const unscheduledHtml = unscheduled.length > 0 ? `
            <div class="timeline-unscheduled">
                <div class="timeline-unscheduled-header" id="unscheduled-header">
                    <span class="swimlane-toggle${unschedCollapsed ? ' collapsed' : ''}">&#9660;</span>
                    <span class="unscheduled-label">Sem Data</span>
                    <span class="unscheduled-count">${unscheduled.length}</span>
                </div>
                <div class="timeline-unscheduled-body${unschedCollapsed ? ' collapsed' : ''}" id="unscheduled-body" style="max-height:${unschedCollapsed ? '0' : unscheduled.length * 40 + 'px'}">
                    ${unscheduled.map(t => {
                        const statusInfo = STATUSES.find(s => s.key === t.status);
                        return `<div class="unscheduled-task-row" data-task-id="${t.id}">
                            <span class="unscheduled-status-dot" style="background:${statusInfo ? statusInfo.color : '#94a3b8'}"></span>
                            <span class="unscheduled-task-title">${esc(t.title)}</span>
                            <span class="unscheduled-task-key">${esc(t.key)}</span>
                            <span class="unscheduled-task-priority ${t.priority}"></span>
                        </div>`;
                    }).join('')}
                </div>
            </div>` : '';

        content.innerHTML = `
            <div class="timeline-wrapper">
                <div class="timeline-container" id="timeline-container">
                    <div class="timeline-header">
                        <div class="timeline-header-label">Tasks</div>
                        <div class="timeline-header-dates">${headerCols}</div>
                    </div>
                    <div class="timeline-body" id="timeline-body">
                        ${swimlanesHtml}
                    </div>
                </div>
                ${unscheduledHtml}
            </div>`;

        // Position task bars
        positionTaskBars(range, colWidth, today);

        // Draw dependency arrows between parent and child tasks
        renderDependencyArrows();

        // Place today marker
        placeTodayMarker(range, colWidth, today);

        // Set up events
        setupTimelineEvents();
        setupSwimlaneToggles();
        setupSubtreeToggles();

        // Scroll to today
        scrollToTodayInitial(range, colWidth, today);
    }

    // ===== Render: Events Timeline (CO-1.44.1) =====
    // Chronological feed of entries-as-events for any universe whose manifest
    // declares a `presentation.calendar.date_field`. Pairs with the calendar
    // grid: same data source, different layout. Used by /time and any other
    // event-shaped universe.
    async function renderEventsTimeline() {
        const content = $('#content');
        content.className = 'content events-timeline';
        const manifest = state.universeManifest;
        if (!manifest) {
            content.innerHTML = '<p class="empty-state" style="padding:24px">Sem manifest — adicione `presentation.calendar.date_field` ao tipo do conteúdo.</p>';
            return;
        }

        // Find the calendar field name + semantic from the manifest.
        const ct = (manifest.content_types || []).find(c =>
            c.presentation && c.presentation.calendar && c.presentation.calendar.date_field
        );
        if (!ct) {
            content.innerHTML = '<p class="empty-state" style="padding:24px">Sem campo de data declarado no manifest deste universo.</p>';
            return;
        }
        const dateField = ct.presentation.calendar.date_field;
        const fieldDef = (ct.schema || {})[dateField];
        const semantic = (fieldDef && fieldDef.semantic) || dateField;

        const slug = state.currentUniverseSlug;
        let entries;
        try {
            entries = await api.getEntriesByDate(slug, semantic, null, null);
        } catch (err) {
            content.innerHTML = `<p class="empty-state" style="padding:24px">Erro ao carregar eventos: ${esc(String(err))}</p>`;
            return;
        }
        if (!entries || entries.length === 0) {
            content.innerHTML = '<p class="empty-state" style="padding:24px">Nenhum evento.</p>';
            return;
        }

        // Sort chronologically (asc) and group by month.
        const items = entries
            .map(e => {
                const raw = e.frontmatter && e.frontmatter[dateField];
                const dt = raw ? new Date(raw) : null;
                return { e, dt, raw };
            })
            .filter(x => x.dt && !isNaN(x.dt.getTime()))
            .sort((a, b) => a.dt - b.dt);

        const groups = new Map();
        for (const x of items) {
            const key = `${x.dt.getFullYear()}-${String(x.dt.getMonth() + 1).padStart(2, '0')}`;
            if (!groups.has(key)) groups.set(key, []);
            groups.get(key).push(x);
        }

        const monthNames = (typeof MONTH_NAMES !== 'undefined' && MONTH_NAMES.length === 12)
            ? MONTH_NAMES
            : ['Jan','Fev','Mar','Abr','Mai','Jun','Jul','Ago','Set','Out','Nov','Dez'];

        const groupHtml = [...groups.entries()].map(([k, xs]) => {
            const [y, m] = k.split('-').map(Number);
            const monthLabel = `${monthNames[m - 1]} ${y}`;
            const rowsHtml = xs.map(({ e, dt }) => {
                const day = dt.getDate();
                const time = dt.getHours() || dt.getMinutes()
                    ? `${String(dt.getHours()).padStart(2, '0')}:${String(dt.getMinutes()).padStart(2, '0')}`
                    : '';
                const title = (e.frontmatter && e.frontmatter.title) || e.path || '';
                const path = e.path || '';
                return `
                    <div class="event-row" data-entry-path="${esc(path)}">
                        <div class="event-date">
                            <span class="event-day">${day}</span>
                            ${time ? `<span class="event-time">${esc(time)}</span>` : ''}
                        </div>
                        <div class="event-body">
                            <span class="event-title">${esc(title)}</span>
                            <span class="event-path">${esc(path)}</span>
                        </div>
                    </div>`;
            }).join('');
            return `
                <section class="events-month">
                    <h2 class="events-month-label">${esc(monthLabel)}</h2>
                    <div class="events-month-rows">${rowsHtml}</div>
                </section>`;
        }).join('');

        content.innerHTML = `<div class="events-feed" style="padding:16px;max-width:880px;margin:0 auto">${groupHtml}</div>`;

        // Open zoom modal on click.
        content.querySelectorAll('.event-row').forEach(row => {
            row.addEventListener('click', async () => {
                const path = row.dataset.entryPath;
                if (!path) return;
                try {
                    const entry = await apiFetch(
                        `/api/v1/universes/${encodeURIComponent(slug)}/entries/${path.split('/').map(encodeURIComponent).join('/')}`
                    );
                    if (entry && entry.path) openZoomModal({ ...entry, _universeSlug: slug }, false);
                } catch (_) {}
            });
        });
    }

    function positionTaskBars(range, colWidth, today) {
        const tasks = filteredTasks().filter(t => t.due_date || t.created_at);

        for (const task of tasks) {
            const gridEl = document.querySelector(`.timeline-task-grid[data-task-id="${task.id}"]`);
            if (!gridEl) continue;

            const dueDate = parseDate(task.due_date);
            const createdDate = parseDate(task.created_at);

            if (dueDate && createdDate && daysBetween(createdDate, dueDate) > 0) {
                // Bar spanning created_at to due_date
                const startOffset = getColumnOffset(createdDate, range, colWidth);
                const endOffset = getColumnOffset(dueDate, range, colWidth) + colWidth;

                if (startOffset === null && endOffset === null) continue;

                const barLeft = startOffset !== null ? startOffset : 0;
                const barRight = endOffset !== null ? endOffset : range.columns.length * colWidth;
                const barWidth = Math.max(barRight - barLeft, 8);

                const bar = document.createElement('div');
                bar.className = `timeline-task-bar status-${task.status}`;
                bar.style.left = barLeft + 'px';
                bar.style.width = barWidth + 'px';
                bar.dataset.taskId = task.id;

                // Short label on bar
                if (barWidth > 60) {
                    bar.textContent = task.title.length > 20 ? task.title.slice(0, 18) + '...' : task.title;
                }

                // Resize handle
                const handle = document.createElement('div');
                handle.className = 'timeline-bar-handle';
                handle.dataset.taskId = task.id;
                bar.appendChild(handle);

                gridEl.appendChild(bar);

                setupBarDrag(handle, task, range, colWidth);
                setupBarTooltip(bar, task);
                bar.addEventListener('click', (e) => {
                    if (!e.target.classList.contains('timeline-bar-handle')) {
                        openTaskModal(task.id);
                    }
                });
            } else if (dueDate) {
                // Single dot for due_date only
                const offset = getColumnOffset(dueDate, range, colWidth);
                if (offset === null) continue;

                const dot = document.createElement('div');
                dot.className = `timeline-task-dot status-${task.status}`;
                dot.style.left = (offset + colWidth / 2) + 'px';
                dot.dataset.taskId = task.id;
                gridEl.appendChild(dot);

                setupBarTooltip(dot, task);
                dot.addEventListener('click', () => openTaskModal(task.id));
            } else if (createdDate) {
                // Dot at created_at
                const offset = getColumnOffset(createdDate, range, colWidth);
                if (offset === null) continue;

                const dot = document.createElement('div');
                dot.className = `timeline-task-dot status-${task.status}`;
                dot.style.left = (offset + colWidth / 2) + 'px';
                dot.dataset.taskId = task.id;
                gridEl.appendChild(dot);

                setupBarTooltip(dot, task);
                dot.addEventListener('click', () => openTaskModal(task.id));
            }
        }
    }

    // ===== Dependency Arrows (SVG overlay) =====
    function renderDependencyArrows() {
        const container = document.getElementById('timeline-container');
        if (!container) return;

        const existing = container.querySelector('.dep-arrows-svg');
        if (existing) existing.remove();

        const tasks = filteredTasks();
        const taskMap = new Map(tasks.map(t => [t.id, t]));
        const containerRect = container.getBoundingClientRect();
        const scrollLeft = container.scrollLeft;
        const scrollTop = container.scrollTop;

        const arrows = [];
        for (const task of tasks) {
            if (!task.parent) continue;
            const parentTask = taskMap.get(task.parent);
            if (!parentTask) continue;

            const parentBar = container.querySelector(`.timeline-task-bar[data-task-id="${parentTask.id}"]`);
            const childBar = container.querySelector(`.timeline-task-bar[data-task-id="${task.id}"]`);
            if (!parentBar || !childBar) continue;

            const pRect = parentBar.getBoundingClientRect();
            const cRect = childBar.getBoundingClientRect();

            const x1 = pRect.right - containerRect.left + scrollLeft;
            const y1 = pRect.top + pRect.height / 2 - containerRect.top + scrollTop;
            const x2 = cRect.left - containerRect.left + scrollLeft;
            const y2 = cRect.top + cRect.height / 2 - containerRect.top + scrollTop;

            arrows.push({ x1, y1, x2, y2 });
        }

        if (arrows.length === 0) return;

        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.classList.add('dep-arrows-svg');
        svg.setAttribute('width', container.scrollWidth);
        svg.setAttribute('height', container.scrollHeight);

        const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
        const marker = document.createElementNS('http://www.w3.org/2000/svg', 'marker');
        marker.setAttribute('id', 'dep-arrowhead');
        marker.setAttribute('markerWidth', '8');
        marker.setAttribute('markerHeight', '8');
        marker.setAttribute('refX', '7');
        marker.setAttribute('refY', '3');
        marker.setAttribute('orient', 'auto');
        const arrowTip = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        arrowTip.setAttribute('d', 'M0,0 L0,6 L8,3 z');
        arrowTip.setAttribute('fill', '#94a3b8');
        marker.appendChild(arrowTip);
        defs.appendChild(marker);
        svg.appendChild(defs);

        for (const { x1, y1, x2, y2 } of arrows) {
            const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
            const mx = x1 + (x2 - x1) * 0.5;
            path.setAttribute('d', `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`);
            path.setAttribute('stroke', '#94a3b8');
            path.setAttribute('stroke-width', '1.5');
            path.setAttribute('stroke-dasharray', '4,3');
            path.setAttribute('fill', 'none');
            path.setAttribute('marker-end', 'url(#dep-arrowhead)');
            svg.appendChild(path);
        }

        container.appendChild(svg);
    }

    function getColumnOffset(date, range, colWidth) {
        if (state.zoom === 'quarter') {
            // Find which week column this date falls in
            for (let i = 0; i < range.columns.length; i++) {
                const col = range.columns[i];
                if (date >= col.date && date <= col.endDate) {
                    // Position within the week
                    const daysIntoWeek = daysBetween(col.date, date);
                    const fraction = daysIntoWeek / 7;
                    return i * colWidth + fraction * colWidth;
                }
            }
            // Out of range
            if (date < range.startDate) return null;
            if (date > range.endDate) return null;
            return null;
        }

        // Day columns
        const dayOffset = daysBetween(range.startDate, date);
        if (dayOffset < 0 || dayOffset >= range.columns.length) return null;
        return dayOffset * colWidth;
    }

    function placeTodayMarker(range, colWidth, today) {
        const container = $('#timeline-container');
        if (!container) return;

        const offset = getColumnOffset(today, range, colWidth);
        if (offset === null) return;

        const labelWidth = 180; // --swimlane-label-width
        const markerX = labelWidth + offset + colWidth / 2;

        // Marker in header
        const marker = document.createElement('div');
        marker.className = 'timeline-today-marker';
        marker.style.left = markerX + 'px';
        container.appendChild(marker);

        // Dashed line through body
        const body = $('#timeline-body');
        if (body) {
            const line = document.createElement('div');
            line.className = 'timeline-today-line';
            line.style.left = markerX + 'px';
            container.appendChild(line);
        }
    }

    function scrollToTodayInitial(range, colWidth, today) {
        const container = $('#timeline-container');
        if (!container) return;

        const offset = getColumnOffset(today, range, colWidth);
        if (offset === null) return;

        // Scroll so today is roughly in view
        const labelWidth = 180;
        const targetScroll = labelWidth + offset - container.clientWidth / 3;
        container.scrollLeft = Math.max(0, targetScroll);
    }

    function scrollToDate(date) {
        // Update timeline start so the date is visible
        const days = ZOOM_DAYS[state.zoom];
        const offset = Math.floor(days * 0.2);
        state.timelineStart = addDays(date, -offset);
        renderContent();
    }

    // ===== Bar Drag (Resize due_date) =====
    function setupBarDrag(handle, task, range, colWidth) {
        let dragging = false;
        let startX = 0;
        let origWidth = 0;
        let barEl = null;

        handle.addEventListener('mousedown', (e) => {
            e.preventDefault();
            e.stopPropagation();
            dragging = true;
            startX = e.clientX;
            barEl = handle.parentElement;
            origWidth = barEl.offsetWidth;
            handle.classList.add('dragging');

            const onMouseMove = (e) => {
                if (!dragging) return;
                const dx = e.clientX - startX;
                const newWidth = Math.max(colWidth, origWidth + dx);
                barEl.style.width = newWidth + 'px';
            };

            const onMouseUp = async (e) => {
                if (!dragging) return;
                dragging = false;
                handle.classList.remove('dragging');
                document.removeEventListener('mousemove', onMouseMove);
                document.removeEventListener('mouseup', onMouseUp);

                // Calculate new due_date from bar width
                const finalWidth = barEl.offsetWidth;
                const barLeft = parseFloat(barEl.style.left);
                const barEnd = barLeft + finalWidth;

                // Find the date at barEnd
                const newDueDate = dateAtOffset(barEnd, range, colWidth);
                if (newDueDate && toDateStr(newDueDate) !== task.due_date) {
                    task.due_date = toDateStr(newDueDate);
                    await api.updateTask(state.currentProject.key, task.id, { due_date: task.due_date });
                    await refreshTasks();
                    renderContent();
                }
            };

            document.addEventListener('mousemove', onMouseMove);
            document.addEventListener('mouseup', onMouseUp);
        });
    }

    function dateAtOffset(offset, range, colWidth) {
        if (state.zoom === 'quarter') {
            const colIndex = Math.floor(offset / colWidth);
            if (colIndex < 0 || colIndex >= range.columns.length) return null;
            const col = range.columns[colIndex];
            const fraction = (offset - colIndex * colWidth) / colWidth;
            const dayOffset = Math.round(fraction * 7);
            return addDays(col.date, Math.min(dayOffset, 6));
        }
        const dayIndex = Math.floor(offset / colWidth);
        if (dayIndex < 0 || dayIndex >= range.columns.length) return null;
        return range.columns[dayIndex].date;
    }

    // ===== Tooltip =====
    let tooltipEl = null;

    function getTooltip() {
        if (!tooltipEl) {
            tooltipEl = document.createElement('div');
            tooltipEl.className = 'timeline-tooltip hidden';
            document.body.appendChild(tooltipEl);
        }
        return tooltipEl;
    }

    function setupBarTooltip(el, task) {
        el.addEventListener('mouseenter', (e) => {
            const tip = getTooltip();
            const statusInfo = STATUSES.find(s => s.key === task.status);

            let dueDateInfo = '';
            if (task.due_date) {
                const dt = parseDate(task.due_date);
                dueDateInfo = `<span class="tooltip-meta-item">
                    <span class="tooltip-meta-label">Prazo:</span>
                    <span class="tooltip-meta-value">${formatDateShort(dt)}</span>
                </span>`;
            }

            tip.innerHTML = `
                <div class="tooltip-title">${esc(task.key)} — ${esc(task.title)}</div>
                <div class="tooltip-meta">
                    <span class="tooltip-meta-item">
                        <span class="tooltip-status-dot" style="background:${statusInfo ? statusInfo.color : '#94a3b8'}"></span>
                        <span class="tooltip-meta-value">${STATUS_LABELS[task.status] || task.status}</span>
                    </span>
                    <span class="tooltip-meta-item">
                        <span class="tooltip-meta-label">Prioridade:</span>
                        <span class="tooltip-meta-value">${PRIORITY_LABELS[task.priority] || task.priority}</span>
                    </span>
                    ${dueDateInfo}
                </div>`;
            tip.classList.remove('hidden');
            positionTooltip(e);
        });

        el.addEventListener('mousemove', positionTooltip);

        el.addEventListener('mouseleave', () => {
            const tip = getTooltip();
            tip.classList.add('hidden');
        });
    }

    function positionTooltip(e) {
        const tip = getTooltip();
        const x = e.clientX + 12;
        const y = e.clientY - 10;
        tip.style.left = x + 'px';
        tip.style.top = y + 'px';

        // Keep in viewport
        const rect = tip.getBoundingClientRect();
        if (rect.right > window.innerWidth) {
            tip.style.left = (e.clientX - rect.width - 12) + 'px';
        }
        if (rect.bottom > window.innerHeight) {
            tip.style.top = (e.clientY - rect.height - 10) + 'px';
        }
    }

    // ===== Timeline Events =====
    function setupTimelineEvents() {
        // Click on task labels
        document.querySelectorAll('.timeline-task-label').forEach(el => {
            el.addEventListener('click', () => {
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });

        // Click on unscheduled rows
        document.querySelectorAll('.unscheduled-task-row').forEach(el => {
            el.addEventListener('click', () => {
                openTaskModal(parseInt(el.dataset.taskId));
            });
        });

        // Unscheduled toggle
        const unschedHeader = document.getElementById('unscheduled-header');
        if (unschedHeader) {
            unschedHeader.addEventListener('click', () => {
                state.unscheduledCollapsed = !state.unscheduledCollapsed;
                const body = document.getElementById('unscheduled-body');
                const toggle = unschedHeader.querySelector('.swimlane-toggle');
                if (body) body.classList.toggle('collapsed', state.unscheduledCollapsed);
                if (toggle) toggle.classList.toggle('collapsed', state.unscheduledCollapsed);
            });
        }
    }

    function setupSwimlaneToggles() {
        document.querySelectorAll('.timeline-swimlane-header').forEach(el => {
            el.addEventListener('click', () => {
                const status = el.dataset.status;
                state.collapsedSwimlanes[status] = !state.collapsedSwimlanes[status];
                const body = el.nextElementSibling;
                const toggle = el.querySelector('.swimlane-toggle');
                if (body) body.classList.toggle('collapsed', state.collapsedSwimlanes[status]);
                if (toggle) toggle.classList.toggle('collapsed', state.collapsedSwimlanes[status]);
            });
        });
    }

    // ===== Drag & Drop (Kanban) - Optimistic =====
    function setupDragDrop() {
        const cards = document.querySelectorAll('.task-card');
        const zones = document.querySelectorAll('.kanban-cards');

        cards.forEach(card => {
            card.addEventListener('dragstart', (e) => {
                card.classList.add('dragging');
                e.dataTransfer.setData('text/plain', card.dataset.taskId);
                e.dataTransfer.effectAllowed = 'move';
            });
            card.addEventListener('dragend', () => {
                card.classList.remove('dragging');
                zones.forEach(z => z.classList.remove('drag-over'));
            });
        });

        zones.forEach(zone => {
            zone.addEventListener('dragover', (e) => {
                e.preventDefault();
                e.dataTransfer.dropEffect = 'move';
                zone.classList.add('drag-over');
            });
            zone.addEventListener('dragleave', () => {
                zone.classList.remove('drag-over');
            });
            zone.addEventListener('drop', async (e) => {
                e.preventDefault();
                zone.classList.remove('drag-over');
                const taskId = parseInt(e.dataTransfer.getData('text/plain'));
                const newStatus = zone.dataset.status;
                const task = state.tasks.find(t => t.id === taskId);
                if (task && task.status !== newStatus) {
                    if (state.isTemplate) {
                        // First interaction: clone template into own universe
                        await ensureOwnUniverse();
                        return; // board will re-render from the cloned universe
                    }
                    const oldStatus = task.status;
                    task.status = newStatus;
                    renderKanban();
                    const result = await api.updateTask(state.currentProject.key, taskId, { status: newStatus });
                    if (!result) {
                        task.status = oldStatus;
                        renderKanban();
                        showToast('Failed to move task. Reverted.', 'error');
                    }
                }
            });
        });
    }

    // ===== Card Clicks (Kanban) =====
    function setupCardClicks() {
        // Subtask items click before card click to avoid bubbling to parent card
        document.querySelectorAll('.subtask-item').forEach(item => {
            item.addEventListener('click', (e) => {
                e.stopPropagation();
                openTaskModal(parseInt(item.dataset.taskId));
            });
        });

        document.querySelectorAll('.task-card').forEach(card => {
            card.addEventListener('click', () => {
                openTaskModal(parseInt(card.dataset.taskId));
            });
        });
    }

    // ===== Subtree Toggles (shared: kanban, table, timeline) =====
    function setupSubtreeToggles() {
        document.querySelectorAll('.subtask-toggle, .tree-toggle, .timeline-subtree-toggle').forEach(el => {
            el.addEventListener('click', (e) => {
                e.stopPropagation();
                const taskId = parseInt(el.dataset.taskId);
                toggleSubtree(taskId);
                renderContent();
            });
        });
    }

    // ===== Dashboard SVG Chart Helpers =====

    function svgVelocityChart(velocity) {
        const W = 380, H = 160, PAD = { top: 10, right: 10, bottom: 36, left: 32 };
        const chartW = W - PAD.left - PAD.right;
        const chartH = H - PAD.top - PAD.bottom;
        const maxCount = Math.max(1, ...velocity.map(v => v.count));
        const barCount = velocity.length || 1;
        const gap = 6;
        const barW = Math.max(4, (chartW - gap * (barCount - 1)) / barCount);

        let bars = '';
        let xLabels = '';
        velocity.forEach((v, i) => {
            const x = PAD.left + i * (barW + gap);
            const barH = Math.max(2, (v.count / maxCount) * chartH);
            const y = PAD.top + chartH - barH;
            bars += `<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${barH.toFixed(1)}" fill="#22c55e" rx="2"/>`;
            if (v.count > 0) {
                bars += `<text x="${(x + barW / 2).toFixed(1)}" y="${(y - 3).toFixed(1)}" text-anchor="middle" font-size="10" fill="#64748b">${v.count}</text>`;
            }
            const label = v.week.slice(-3);
            xLabels += `<text x="${(x + barW / 2).toFixed(1)}" y="${(H - 6).toFixed(1)}" text-anchor="middle" font-size="10" fill="#94a3b8">${esc(label)}</text>`;
        });

        const yTicks = [0, Math.ceil(maxCount / 2), maxCount];
        let yAxis = '';
        yTicks.forEach(t => {
            const y = PAD.top + chartH - (t / maxCount) * chartH;
            yAxis += `<line x1="${PAD.left - 4}" y1="${y.toFixed(1)}" x2="${W - PAD.right}" y2="${y.toFixed(1)}" stroke="#e2e8f0" stroke-width="1"/>`;
            yAxis += `<text x="${(PAD.left - 6).toFixed(1)}" y="${(y + 3).toFixed(1)}" text-anchor="end" font-size="9" fill="#94a3b8">${t}</text>`;
        });

        if (velocity.length === 0) {
            return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block"><text x="${W / 2}" y="${H / 2}" text-anchor="middle" font-size="13" fill="#94a3b8">No completions in last 8 weeks</text></svg>`;
        }

        return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block">${yAxis}${bars}${xLabels}</svg>`;
    }

    function svgBurndownChart(burndown) {
        const W = 380, H = 160, PAD = { top: 24, right: 10, bottom: 36, left: 36 };
        const chartW = W - PAD.left - PAD.right;
        const chartH = H - PAD.top - PAD.bottom;
        const n = burndown.length;
        if (n === 0) {
            return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block"><text x="${W / 2}" y="${H / 2}" text-anchor="middle" font-size="13" fill="#94a3b8">No data</text></svg>`;
        }

        const maxVal = Math.max(1, ...burndown.map(p => Math.max(p.remaining, p.completed)));
        const px = (i) => PAD.left + (n > 1 ? (i / (n - 1)) * chartW : chartW / 2);
        const py = (v) => PAD.top + chartH - (v / maxVal) * chartH;

        let remainingPoints = '';
        let completedPoints = '';
        burndown.forEach((p, i) => {
            remainingPoints += `${px(i).toFixed(1)},${py(p.remaining).toFixed(1)} `;
            completedPoints += `${px(i).toFixed(1)},${py(p.completed).toFixed(1)} `;
        });

        let dots = '';
        burndown.forEach((p, i) => {
            dots += `<circle cx="${px(i).toFixed(1)}" cy="${py(p.remaining).toFixed(1)}" r="3" fill="#ef4444"/>`;
            dots += `<circle cx="${px(i).toFixed(1)}" cy="${py(p.completed).toFixed(1)}" r="3" fill="#22c55e"/>`;
        });

        let xLabels = '';
        burndown.forEach((p, i) => {
            if (i % 2 === 0 || i === n - 1) {
                xLabels += `<text x="${px(i).toFixed(1)}" y="${(H - 6).toFixed(1)}" text-anchor="middle" font-size="10" fill="#94a3b8">${esc(p.date.slice(-3))}</text>`;
            }
        });

        const yTicks = [0, Math.ceil(maxVal / 2), maxVal];
        let yAxis = '';
        yTicks.forEach(t => {
            const y = py(t);
            yAxis += `<line x1="${PAD.left}" y1="${y.toFixed(1)}" x2="${W - PAD.right}" y2="${y.toFixed(1)}" stroke="#e2e8f0" stroke-width="1"/>`;
            yAxis += `<text x="${(PAD.left - 4).toFixed(1)}" y="${(y + 3).toFixed(1)}" text-anchor="end" font-size="9" fill="#94a3b8">${t}</text>`;
        });

        const legend = `<rect x="${PAD.left}" y="4" width="10" height="4" fill="#ef4444" rx="1"/><text x="${PAD.left + 14}" y="10" font-size="10" fill="#64748b">Remaining</text><rect x="${PAD.left + 82}" y="4" width="10" height="4" fill="#22c55e" rx="1"/><text x="${PAD.left + 96}" y="10" font-size="10" fill="#64748b">Completed</text>`;

        return `<svg viewBox="0 0 ${W} ${H}" width="100%" style="display:block">${yAxis}<polyline points="${remainingPoints.trim()}" fill="none" stroke="#ef4444" stroke-width="2" stroke-linejoin="round"/><polyline points="${completedPoints.trim()}" fill="none" stroke="#22c55e" stroke-width="2" stroke-linejoin="round"/>${dots}${xLabels}${legend}</svg>`;
    }

    function svgLabelChart(labels) {
        if (labels.length === 0) {
            return '<p class="dashboard-empty">No labels in use</p>';
        }
        const BAR_H = 20, GAP = 6, PAD_LEFT = 90, PAD_RIGHT = 40, W = 360;
        const maxCount = Math.max(1, ...labels.map(l => l.count));
        const chartW = W - PAD_LEFT - PAD_RIGHT;
        const H = labels.length * (BAR_H + GAP) + 4;

        let rows = '';
        labels.forEach((l, i) => {
            const y = i * (BAR_H + GAP);
            const barW = Math.max(2, (l.count / maxCount) * chartW);
            const labelText = l.label.length > 12 ? l.label.slice(0, 11) + '\u2026' : l.label;
            rows += `<text x="${PAD_LEFT - 6}" y="${(y + BAR_H / 2 + 4).toFixed(1)}" text-anchor="end" font-size="11" fill="#475569">${esc(labelText)}</text>`;
            rows += `<rect x="${PAD_LEFT}" y="${y}" width="${barW.toFixed(1)}" height="${BAR_H}" fill="#3b82f6" rx="3" opacity="0.85"/>`;
            rows += `<text x="${(PAD_LEFT + barW + 5).toFixed(1)}" y="${(y + BAR_H / 2 + 4).toFixed(1)}" font-size="11" fill="#64748b">${l.count}</text>`;
        });

        return `<svg viewBox="0 0 ${W} ${H}" width="100%" height="${H}" style="display:block">${rows}</svg>`;
    }

    function overdueAgeColor(days) {
        if (days >= 8) return '#ef4444';
        if (days >= 4) return '#f97316';
        return '#f59e0b';
    }

    // ===== Render: Conteúdo (CO-42 redesign) =====
    // Wiki-like browser: folder hierarchy, rendered cards, zoom modal, dados panel.

    // Format ISO timestamp as relative "X dias atrás"
    function relativeDate(iso) {
        if (!iso) return '';
        const then = new Date(iso).getTime();
        if (isNaN(then)) return '';
        const diff = Math.floor((Date.now() - then) / 1000);
        if (diff < 60) return 'agora';
        if (diff < 3600) return `${Math.floor(diff / 60)}min atrás`;
        if (diff < 86400) return `${Math.floor(diff / 3600)}h atrás`;
        const days = Math.floor(diff / 86400);
        if (days === 1) return '1 dia atrás';
        if (days < 30) return `${days} dias atrás`;
        const months = Math.floor(days / 30);
        if (months === 1) return '1 mês atrás';
        if (months < 12) return `${months} meses atrás`;
        const years = Math.floor(days / 365);
        return years === 1 ? '1 ano atrás' : `${years} anos atrás`;
    }

    // Build recursive folder tree from flat entries list.
    // Paths are "content/foo/bar.md" — folders derived from segments after "content/".
    function buildFolderTree(entries) {
        const root = { name: '', path: '', items: [], children: [] };
        for (const entry of entries) {
            let rel = entry.path || '';
            if (rel.startsWith('content/')) rel = rel.slice('content/'.length);
            const parts = rel.split('/').filter(Boolean);
            if (parts.length <= 1) {
                root.items.push(entry);
            } else {
                let node = root;
                for (let i = 0; i < parts.length - 1; i++) {
                    const name = parts[i];
                    let child = node.children.find(c => c.name === name);
                    if (!child) {
                        child = { name, path: parts.slice(0, i + 1).join('/'), items: [], children: [] };
                        node.children.push(child);
                    }
                    node = child;
                }
                node.items.push(entry);
            }
        }
        return root;
    }

    function countFolderEntries(node) {
        return node.items.length + node.children.reduce((s, c) => s + countFolderEntries(c), 0);
    }

    async function renderConteudo() {
        const content = $('#content');
        content.className = 'content conteudo-view';
        content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Carregando...</p></div>';

        const slug = state.currentUniverseSlug;

        try { await loadEditorBundle(); } catch (_) {}

        const [taskEntries, eventEntries, pageEntries, clipEntries, allEntries] = await Promise.all([
            api.getUniverseEntries(slug, 'task'),
            api.getUniverseEntries(slug, 'event'),
            api.getUniverseEntries(slug, 'page'),
            api.getUniverseEntries(slug, 'clip'),
            api.getUniverseEntries(slug),
        ]);

        // CO-94 preview: bulk-imported markdown without an explicit `type:` in
        // frontmatter ends up untyped. Fold those into the Pages folder tree
        // so the Conteúdo view actually shows them.
        const knownPaths = new Set([
            ...taskEntries.map(e => e.path),
            ...eventEntries.map(e => e.path),
            ...pageEntries.map(e => e.path),
            ...clipEntries.map(e => e.path),
        ]);
        // 1.59.0: backend / versioning types belong on the universe info page
        // (the ℹ modal), never in Conteúdo. Hide them from the user-facing
        // content view.
        const CONTEUDO_BACKEND_TYPES = new Set(['state', 'branch', 'proposal', 'merge']);
        for (const e of allEntries) {
            const t = (e.frontmatter || {}).type;
            if (CONTEUDO_BACKEND_TYPES.has(t)) continue;
            if (!knownPaths.has(e.path) && (e.path || '').endsWith('.md')) {
                pageEntries.push(e);
            }
        }

        function entryFm(e) { return e.frontmatter || {}; }
        function entryTitle(e) { return e.title || entryFm(e).title || e.path || ''; }
        function entryTags(e) {
            const tags = entryFm(e).tags;
            return Array.isArray(tags) ? tags : [];
        }

        function cardBodyHtml(body, fm) {
            if (body && body.trim().length > 0) {
                const md = window.CoMarkdown;
                if (md) {
                    return `<div class="conteudo-card-body md-body md-fade">${md.renderMarkdown(body)}</div>`;
                }
                const snippet = body.slice(0, 200);
                return `<div class="conteudo-card-body">${esc(snippet)}${body.length > 200 ? '…' : ''}</div>`;
            }
            // Body-empty fallback: many universes encode the actual content as
            // structured frontmatter (e.g. artelonga members have nome, papel,
            // bio, funcao but no body). Render a compact key-value list of the
            // user-meaningful fields so those cards aren't visually blank.
            return frontmatterPreviewHtml(fm);
        }

        // Frontmatter keys that are scaffolding rather than content — skip them
        // in the empty-body preview. (`tags` is rendered separately as chips.)
        const FM_HIDE_KEYS = new Set([
            'type', 'slug', 'order', 'tags', 'created', 'modified',
            'created_at', 'updated_at', 'next_id', 'archived', 'archived_at',
            'frontmatter', 'parent', 'id', 'key', 'project_key',
        ]);

        function frontmatterPreviewHtml(fm) {
            if (!fm || typeof fm !== 'object') return '';
            const entries = Object.entries(fm)
                .filter(([k, v]) => !FM_HIDE_KEYS.has(k))
                .filter(([, v]) => {
                    if (v === null || v === undefined || v === '') return false;
                    if (Array.isArray(v) && v.length === 0) return false;
                    return true;
                });
            if (!entries.length) return '';
            const fmtVal = (v) => {
                if (Array.isArray(v)) return v.map(x => esc(String(x))).join(', ');
                if (typeof v === 'object') {
                    try { return esc(JSON.stringify(v)); } catch (_) { return ''; }
                }
                const s = String(v);
                // If the value looks like a URL (image / link), make it visual.
                if (/^https?:\/\/|^\//.test(s) && /\.(png|jpe?g|gif|svg|webp)(\?|$)/i.test(s)) {
                    return `<img src="${esc(s)}" alt="" loading="lazy" class="conteudo-fm-img">`;
                }
                if (/^https?:\/\//.test(s)) {
                    return `<a href="${esc(s)}" target="_blank" rel="noopener">${esc(s)}</a>`;
                }
                return esc(s);
            };
            const rows = entries.slice(0, 8).map(([k, v]) =>
                `<div class="conteudo-fm-row"><span class="conteudo-fm-key">${esc(k)}</span><span class="conteudo-fm-val">${fmtVal(v)}</span></div>`
            ).join('');
            return `<div class="conteudo-card-body conteudo-fm-preview">${rows}</div>`;
        }

        // Read/write section collapse state from localStorage (1 = open, 0 = closed)
        function sectionOpen(key, defaultOpen) {
            try {
                const v = localStorage.getItem(`co_section_${key}`);
                return v === null ? defaultOpen : v === '1';
            } catch (_) { return defaultOpen; }
        }

        function sectionHtml(key, label, count, bodyHtml, defaultOpen, tooltip, actionBtn) {
            const open = sectionOpen(key, defaultOpen);
            const tooltipAttr = tooltip ? ` title="${esc(tooltip)}"` : '';
            const actionHtml = actionBtn || '';
            return `<div class="co-section" data-section="${esc(key)}">
                <div class="co-section-header" data-section-toggle${tooltipAttr}>
                    <span class="co-section-chevron ${open ? 'open' : 'closed'}">▼</span>
                    <span class="co-section-title">${esc(label)}</span>
                    <span class="co-section-count">${count}</span>
                    ${actionHtml}
                </div>
                <div class="co-section-body${open ? '' : ' collapsed'}">${bodyHtml}</div>
            </div>`;
        }

        // Render a single page card
        function renderPageCard(e) {
            const tags = entryTags(e);
            const fm = entryFm(e);
            const updated = e.updated_at || fm.modified || fm.updated || '';
            const relDate = relativeDate(updated);
            return `<div class="conteudo-card conteudo-card-clickable co-page-card"
                        data-entry-path="${esc(e.path)}"
                        data-entry-title="${esc(entryTitle(e))}">
                <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
                ${cardBodyHtml(e.body, entryFm(e))}
                <div class="conteudo-card-footer">
                    ${tags.length ? `<div class="conteudo-card-tags">${tags.map(t => `<span class="conteudo-tag">#${esc(t)}</span>`).join('')}</div>` : '<span></span>'}
                    ${relDate ? `<span class="conteudo-card-date">${esc(relDate)}</span>` : ''}
                </div>
            </div>`;
        }

        // Render folder node recursively (returns HTML string)
        function renderFolderNode(node, depth) {
            let html = '';
            if (depth === 0) {
                html += node.items.map(renderPageCard).join('');
            }
            for (const child of node.children) {
                const folderKey = `co_folder_${encodeURIComponent(child.path)}`;
                let savedState = null;
                try { savedState = localStorage.getItem(folderKey); } catch (_) {}
                // Folders default to CLOSED; only open if user explicitly opened it before.
                const isOpen = savedState === 'open';
                const count = countFolderEntries(child);
                html += `<div class="co-folder" data-folder-path="${esc(child.path)}" data-folder-key="${esc(folderKey)}">
                    <div class="co-folder-header" data-folder-toggle>
                        <span class="co-folder-chevron">${isOpen ? '▼' : '▶'}</span>
                        <span class="material-symbols-outlined co-folder-icon">${isOpen ? 'folder_open' : 'folder'}</span>
                        <span class="co-folder-name">${esc(child.name)}</span>
                        <span class="co-folder-count">${count}</span>
                    </div>
                    <div class="co-folder-body"${isOpen ? '' : ' style="display:none"'}>
                        ${child.items.map(renderPageCard).join('')}
                        ${renderFolderNode(child, depth + 1)}
                    </div>
                </div>`;
            }
            return html;
        }

        // Pages section — expanded by default, folder tree
        const pagesBodyHtml = pageEntries.length
            ? renderFolderNode(buildFolderTree(pageEntries), 0)
            : '<p class="conteudo-empty">Nenhuma página</p>';

        // Tasks section — collapsed by default (redundante ao Kanban)
        const tasksBodyHtml = taskEntries.length
            ? taskEntries.map(e => {
                const fm = entryFm(e);
                const taskId = fm.id || '';
                const status = fm.status || 'todo';
                const priority = fm.priority || 'medium';
                const tags = entryTags(e);
                return `<div class="conteudo-card conteudo-card-clickable" data-task-id="${taskId}">
                    <div class="conteudo-card-meta">${esc(status)} · ${esc(priority)}</div>
                    <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
                    ${cardBodyHtml(e.body, entryFm(e))}
                    ${tags.length ? `<div class="conteudo-card-tags">${tags.map(t => `<span class="conteudo-tag">${esc(t)}</span>`).join('')}</div>` : ''}
                </div>`;
              }).join('')
            : '<p class="conteudo-empty">Nenhuma tarefa</p>';

        // Events section — collapsed by default
        const today = todayDate();
        const upcomingEvents = eventEntries
            .filter(e => { const d = entryFm(e).date || entryFm(e).data || ''; return d >= today; })
            .sort((a, b) => (entryFm(a).date || entryFm(a).data || '').localeCompare(entryFm(b).date || entryFm(b).data || ''))
            .slice(0, 5);

        const eventsBodyHtml = upcomingEvents.length
            ? upcomingEvents.map(e => {
                const fm = entryFm(e);
                const date = fm.date || fm.data || '';
                const local = fm.local || fm.location || '';
                return `<div class="conteudo-card">
                    <div class="conteudo-card-meta">${esc(date)}${local ? ' · ' + esc(local) : ''}</div>
                    <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
                    ${cardBodyHtml(e.body, entryFm(e))}
                </div>`;
              }).join('')
            : '<p class="conteudo-empty">Nenhum evento próximo</p>';

        // Clips section — collapsed by default if empty
        const clipsBodyHtml = clipEntries.length
            ? clipEntries.slice(0, 6).map(e => {
                const fm = entryFm(e);
                const url = fm.url || fm.link || '';
                return `<div class="conteudo-card">
                    <div class="conteudo-card-title">${url ? `<a href="${esc(url)}" target="_blank" rel="noopener">${esc(entryTitle(e))}</a>` : esc(entryTitle(e))}</div>
                    ${cardBodyHtml(e.body, entryFm(e))}
                </div>`;
              }).join('')
            : '<p class="conteudo-empty">Nenhum clipe</p>';

        const addContentBtn = state.isTemplate
            ? ''
            : `<button class="btn btn-secondary btn-sm co-add-content" id="btn-add-content" style="margin-left:auto;font-size:12px">+ ${window.t ? window.t('add_content') : 'Adicionar conteúdo'}</button>`;

        // All sections start collapsed by default — user expands what they
        // want to look at. Saved-state in localStorage still wins, so once a
        // user opens a section it remembers next time.
        const sectionsHtml = [
            sectionHtml('paginas', 'Páginas', pageEntries.length, pagesBodyHtml, false, null, addContentBtn),
            sectionHtml('tarefas', 'Tarefas', taskEntries.length, tasksBodyHtml, false, 'Redundante ao Kanban'),
            sectionHtml('eventos', 'Próximos Eventos', upcomingEvents.length, eventsBodyHtml, false, null),
            clipEntries.length ? sectionHtml('clipes', 'Clipes', clipEntries.length, clipsBodyHtml, false, null) : '',
        ].join('');

        // Stats strip — quick totals + last activity, derived from allEntries
        // so untyped markdown counts toward the total too. Renders unobtrusively
        // above the sections.
        const totalEntries = allEntries.length;
        const lastUpdated = allEntries
            .map(e => e.updated_at || (e.frontmatter || {}).modified || '')
            .filter(Boolean)
            .sort()
            .slice(-1)[0] || '';
        const lastUpdatedRel = lastUpdated ? relativeDate(lastUpdated) : '';
        const tagCount = (() => {
            const seen = new Set();
            for (const e of allEntries) {
                const t = (e.frontmatter || {}).tags;
                if (Array.isArray(t)) t.forEach(x => seen.add(x));
            }
            return seen.size;
        })();
        const statsHtml = `
            <div class="conteudo-stats">
                <div class="conteudo-stat"><span class="conteudo-stat-value">${totalEntries}</span><span class="conteudo-stat-label">${totalEntries === 1 ? 'entrada' : 'entradas'}</span></div>
                <div class="conteudo-stat"><span class="conteudo-stat-value">${pageEntries.length}</span><span class="conteudo-stat-label">${pageEntries.length === 1 ? 'página' : 'páginas'}</span></div>
                <div class="conteudo-stat"><span class="conteudo-stat-value">${taskEntries.length}</span><span class="conteudo-stat-label">${taskEntries.length === 1 ? 'tarefa' : 'tarefas'}</span></div>
                <div class="conteudo-stat"><span class="conteudo-stat-value">${eventEntries.length}</span><span class="conteudo-stat-label">${eventEntries.length === 1 ? 'evento' : 'eventos'}</span></div>
                <div class="conteudo-stat"><span class="conteudo-stat-value">${tagCount}</span><span class="conteudo-stat-label">${tagCount === 1 ? 'tag' : 'tags'}</span></div>
                ${lastUpdatedRel ? `<div class="conteudo-stat conteudo-stat-meta"><span class="conteudo-stat-label">Última edição</span><span class="conteudo-stat-value-meta">${esc(lastUpdatedRel)}</span></div>` : ''}
            </div>`;

        content.innerHTML = `<div class="conteudo-list">${statsHtml}${sectionsHtml}</div>`;

        // Section toggle handlers
        content.querySelectorAll('[data-section-toggle]').forEach(header => {
            header.addEventListener('click', () => {
                const section = header.closest('[data-section]');
                const key = section.dataset.section;
                const body = section.querySelector('.co-section-body');
                const chevron = header.querySelector('.co-section-chevron');
                const isOpen = !body.classList.contains('collapsed');
                body.classList.toggle('collapsed', isOpen);
                chevron.classList.toggle('open', !isOpen);
                chevron.classList.toggle('closed', isOpen);
                try { localStorage.setItem(`co_section_${key}`, isOpen ? '0' : '1'); } catch (_) {}
            });
        });

        // Folder toggle handlers
        content.querySelectorAll('[data-folder-toggle]').forEach(header => {
            header.addEventListener('click', () => {
                const folder = header.closest('.co-folder');
                const folderKey = folder.dataset.folderKey;
                const body = folder.querySelector('.co-folder-body');
                const chevron = header.querySelector('.co-folder-chevron');
                const icon = header.querySelector('.co-folder-icon');
                const isOpen = body.style.display !== 'none';
                body.style.display = isOpen ? 'none' : '';
                chevron.textContent = isOpen ? '▶' : '▼';
                if (icon) icon.textContent = isOpen ? 'folder' : 'folder_open';
                try { localStorage.setItem(folderKey, isOpen ? 'closed' : 'open'); } catch (_) {}
            });
        });

        // Page card: single-click → zoom view, double-click → zoom edit mode
        content.querySelectorAll('.co-page-card').forEach(card => {
            let clickTimer = null;
            card.addEventListener('click', e => {
                if (e.detail >= 2) return;
                clearTimeout(clickTimer);
                clickTimer = setTimeout(() => {
                    const entryPath = card.dataset.entryPath;
                    const entry = (content._pageEntries || []).find(en => en.path === entryPath)
                        || { path: entryPath, title: card.dataset.entryTitle || entryPath, body: '' };
                    openZoomModal(entry, false);
                }, 220);
            });
            card.addEventListener('dblclick', () => {
                clearTimeout(clickTimer);
                const entryPath = card.dataset.entryPath;
                const entry = (content._pageEntries || []).find(en => en.path === entryPath)
                    || { path: entryPath, title: card.dataset.entryTitle || entryPath, body: '' };
                openZoomModal(entry, true);
            });
        });

        // Task card: single-click → task editor
        content.querySelectorAll('[data-task-id]').forEach(card => {
            card.addEventListener('click', () => {
                const taskId = parseInt(card.dataset.taskId);
                if (taskId) openContentEditor(taskId);
            });
        });

        // Store page entries for click handlers
        content._pageEntries = pageEntries;

        // "Add Content" button → create new page and open editor
        const addBtn = document.getElementById('btn-add-content');
        if (addBtn) {
            addBtn.addEventListener('click', async (e) => {
                e.stopPropagation();
                if (state.isTemplate) { showLoginModal(); return; }

                const title = prompt(window.t ? window.t('new_page_title') : 'Título da nova página:');
                if (!title || !title.trim()) return;

                const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 60);
                const path = `content/${slug}.md`;
                const slug2 = state.currentUniverseSlug;

                const result = await apiFetch(`/api/v1/universes/${slug2}/entries`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        path: path,
                        type: 'page',
                        frontmatter: { type: 'page', title: title.trim(), slug: slug, tags: [], created: new Date().toISOString(), modified: new Date().toISOString() },
                        body: `# ${title.trim()}\n\n`,
                    }),
                }, true);

                if (result) {
                    // Open the new page in edit mode
                    openZoomModal({ path, title: title.trim(), body: `# ${title.trim()}\n\n` }, true);
                } else {
                    showToast('Erro ao criar página', 'error');
                }
            });
        }
    }

    // ===== CO-160: Inline PDF Viewer helpers =====

    function shouldRenderInlinePdf(entry) {
        const fm = entry.frontmatter || {};
        return (
            fm.type === 'reference' &&
            fm.medium === 'pdf' &&
            fm.file &&
            fm.file.endsWith('.pdf')
        );
    }

    function pdfUrlFromCard(universe, entry) {
        const fm = entry.frontmatter || {};
        if (fm.blob_sha256) {
            return `/api/v1/universes/${universe}/assets/${fm.blob_sha256}`;
        }
        // Fall back to the raw blob endpoint which serves files directly from
        // the universe directory without requiring an asset index entry.
        const dir = (entry.path || '').split('/').slice(0, -1).join('/');
        const filename = fm.file || '';
        const filePath = (dir ? dir + '/' : '') + filename;
        return `/api/v1/universes/${universe}/blob/${filePath.split('/').map(encodeURIComponent).join('/')}`;
    }

    function buildPdfViewerHtml(pdfUrl, filename) {
        const viewerSrc = `/pdfjs/web/viewer.html?file=${encodeURIComponent(pdfUrl)}`;
        const dlName = filename || 'documento.pdf';
        return `<div class="pdf-viewer" id="co-pdf-viewer">
  <iframe id="co-pdf-iframe"
          src="${esc(viewerSrc)}"
          width="100%"
          height="800"
          loading="lazy"
          style="border:0;border-radius:8px;display:block"
          allowfullscreen></iframe>
  <div class="pdf-viewer-actions">
    <a href="${esc(pdfUrl)}" download="${esc(dlName)}" class="pdf-download-btn btn btn-secondary">
      <span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">download</span>
      Baixar PDF
    </a>
    <button class="pdf-fullscreen-btn btn btn-secondary" id="co-pdf-fullscreen">
      <span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">fullscreen</span>
      Tela cheia
    </button>
  </div>
</div>`;
    }

    function initPdfViewerActions(container) {
        const btn = container.querySelector('#co-pdf-fullscreen');
        const iframe = container.querySelector('#co-pdf-iframe');
        if (!btn || !iframe) return;
        btn.addEventListener('click', () => {
            const el = iframe;
            if (el.requestFullscreen) el.requestFullscreen();
            else if (el.webkitRequestFullscreen) el.webkitRequestFullscreen();
            else if (el.mozRequestFullScreen) el.mozRequestFullScreen();
        });
    }

    // ===== Zoom Viewer Modal (CO-42) =====
    // PDF-style full-screen overlay for reading/editing content entries.

    async function openZoomModal(entry, startInEditMode) {
        const existing = document.getElementById('co-zoom-overlay');
        if (existing) existing.remove();

        try { await loadEditorBundle(); } catch (_) {}

        // Fetch full entry if body is not yet available (e.g. called outside renderConteudo).
        // _universeSlug override lets callers pin the fetch to a specific universe
        // (e.g. template legal pages opened from any user's context).
        let fullEntry = entry;
        const fetchUniverse = entry._universeSlug || state.currentUniverseSlug;
        if (fullEntry.body === undefined && fullEntry.path && fetchUniverse) {
            try {
                const encodedEntryPath = (fullEntry.path || '').split('/').map(encodeURIComponent).join('/');
                const data = await apiFetch(`/api/v1/universes/${fetchUniverse}/entries/${encodedEntryPath}`);
                if (data) fullEntry = data;
            } catch (_) {}
        }

        const title = fullEntry.title || (fullEntry.frontmatter || {}).title || fullEntry.path || '';

        const overlay = document.createElement('div');
        overlay.className = 'co-zoom-overlay';
        overlay.id = 'co-zoom-overlay';
        overlay.innerHTML = `
            <div class="co-zoom-container" id="co-zoom-container">
                <div class="co-zoom-toolbar">
                    <button class="co-zoom-close" id="co-zoom-close" title="Fechar (Esc)">
                        <span class="material-symbols-outlined" style="font-size:18px">close</span>
                    </button>
                    <span class="co-zoom-title">${esc(title)}</span>
                    <div class="co-zoom-actions">
                        <button class="co-zoom-action" id="co-zoom-edit" title="Editar">
                            <span class="material-symbols-outlined" style="font-size:18px">edit</span>
                        </button>
                        <button class="co-zoom-action" id="co-zoom-dados" title="Ver dados">
                            <span class="material-symbols-outlined" style="font-size:18px">info</span>
                        </button>
                        <button class="co-zoom-action" id="co-zoom-print" title="Imprimir">
                            <span class="material-symbols-outlined" style="font-size:18px">print</span>
                        </button>
                    </div>
                </div>
                <div class="co-zoom-body md-article" id="co-zoom-body"></div>
            </div>`;

        document.body.appendChild(overlay);

        const zoomBody = document.getElementById('co-zoom-body');

        function renderView() {
            const md = window.CoMarkdown;
            let html = md ? md.renderMarkdown(fullEntry.body || '') : esc(fullEntry.body || '');
            if (md && md.resolveWikilinks) html = md.resolveWikilinks(html, state.currentUniverseSlug);
            zoomBody.className = 'co-zoom-body md-article';
            zoomBody.innerHTML = html;

            zoomBody.querySelectorAll('table').forEach(tbl => {
                const wrap = document.createElement('div');
                wrap.className = 'co-table-wrap';
                tbl.parentNode.insertBefore(wrap, tbl);
                wrap.appendChild(tbl);
            });
            const md2 = window.CoMarkdown;
            if (md2 && md2.enableImageZoom) md2.enableImageZoom(zoomBody);
            if (md2 && md2.highlightCode) md2.highlightCode(zoomBody);
            if (md2 && md2.renderMermaidBlocks) md2.renderMermaidBlocks(zoomBody);

            // CO-160: Append inline PDF viewer for reference entries with medium:pdf
            if (shouldRenderInlinePdf(fullEntry)) {
                const pdfUrl = pdfUrlFromCard(fetchUniverse, fullEntry);
                const fm = fullEntry.frontmatter || {};
                // Check if the file is available before rendering the iframe.
                fetch(pdfUrl, { method: 'HEAD' }).then(r => {
                    if (r.ok) {
                        zoomBody.insertAdjacentHTML('beforeend', buildPdfViewerHtml(pdfUrl, fm.file || ''));
                        initPdfViewerActions(zoomBody);
                    } else {
                        zoomBody.insertAdjacentHTML('beforeend',
                            `<div style="margin-top:16px;padding:12px 16px;background:#fef9c3;border-radius:8px;font-size:.85rem;color:#713f12">
                              <strong>PDF não sincronizado</strong> — execute <code>co-sync &lt;token&gt;</code>
                              para enviar os arquivos locais ao servidor.
                              ${fm.file ? `<br><span style="color:#92400e">Arquivo: ${esc(fm.file)}</span>` : ''}
                            </div>`
                        );
                    }
                }).catch(() => {});
            }

            zoomBody.addEventListener('dblclick', enterEditMode, { once: true });
        }

        let _zoomEditorInstance = null;
        let _zoomDraftInterval = null;
        const draftKey = `co_draft_page_${encodeURIComponent(fullEntry.path || '')}`;

        function enterEditMode() {
            const editBtn = document.getElementById('co-zoom-edit');
            if (editBtn) editBtn.classList.add('active');

            zoomBody.className = 'co-zoom-body co-zoom-edit-container';
            zoomBody.innerHTML = `
                <textarea class="content-editor-textarea" id="co-zoom-textarea">${esc(fullEntry.body || '')}</textarea>
                <div class="co-zoom-edit-actions">
                    <button class="btn btn-primary" id="co-zoom-save">Salvar</button>
                    <button class="btn btn-secondary" id="co-zoom-cancel">Cancelar</button>
                </div>`;

            document.getElementById('co-zoom-cancel').addEventListener('click', () => {
                if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
                if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
                if (editBtn) editBtn.classList.remove('active');
                renderView();
            });

            document.getElementById('co-zoom-save').addEventListener('click', async () => {
                const saveBtn = document.getElementById('co-zoom-save');
                const ta = document.getElementById('co-zoom-textarea');
                let newBody = _zoomEditorInstance && _zoomEditorInstance.getContent
                    ? _zoomEditorInstance.getContent()
                    : (ta ? ta.value : (fullEntry.body || ''));

                if (state.isTemplate) {
                    const ok = await ensureOwnUniverse();
                    if (!ok) { showToast('Crie uma conta para salvar', 'error'); return; }
                    showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                    return;
                }

                if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
                const result = await apiFetch(
                    `/api/v1/universes/${state.currentUniverseSlug}/entries/${encodeURIComponent(fullEntry.path)}`,
                    { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ body: newBody }) }
                );
                if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = 'Salvar'; }
                if (result) {
                    fullEntry = { ...fullEntry, body: newBody };
                    try { localStorage.removeItem(draftKey); } catch (_) {}
                    if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
                    if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
                    if (editBtn) editBtn.classList.remove('active');
                    showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                    renderView();
                } else {
                    showToast('Erro ao salvar', 'error');
                }
            });

            if (window.CoEditor) {
                const ta = document.getElementById('co-zoom-textarea');
                if (ta) ta.style.display = 'none';
                const cmDiv = document.createElement('div');
                cmDiv.className = 'content-editor-cm co-zoom-cm';
                zoomBody.insertBefore(cmDiv, zoomBody.querySelector('.co-zoom-edit-actions'));
                _zoomEditorInstance = window.CoEditor.initEditor(cmDiv, {
                    content: fullEntry.body || '',
                    onChange: (val) => { if (ta) ta.value = val; },
                    readOnly: false,
                });

                if (_zoomDraftInterval) clearInterval(_zoomDraftInterval);
                _zoomDraftInterval = setInterval(() => {
                    try {
                        const val = _zoomEditorInstance ? _zoomEditorInstance.getValue() : '';
                        localStorage.setItem(draftKey, val);
                    } catch (_) {}
                }, 5000);
            }
        }

        function closeZoom() {
            if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
            if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
            document.removeEventListener('keydown', onEsc);
            const dadosOverlay = document.getElementById('co-dados-overlay');
            if (dadosOverlay) dadosOverlay.remove();
            overlay.remove();
        }

        function onEsc(e) { if (e.key === 'Escape') closeZoom(); }
        document.addEventListener('keydown', onEsc);

        document.getElementById('co-zoom-close').addEventListener('click', closeZoom);
        overlay.addEventListener('click', e => { if (e.target === overlay) closeZoom(); });

        document.getElementById('co-zoom-edit').addEventListener('click', () => {
            if (!zoomBody.classList.contains('co-zoom-edit-container')) enterEditMode();
        });

        document.getElementById('co-zoom-dados').addEventListener('click', () => {
            openViewDados(fullEntry, overlay);
        });

        document.getElementById('co-zoom-print').addEventListener('click', () => window.print());

        if (startInEditMode) { enterEditMode(); } else { renderView(); }
    }

    // ===== View Dados Panel (CO-42) =====
    // Slide-in metadata + stats panel for a content entry.

    function openViewDados(entry, parentEl) {
        // Toggle: close if already open
        const existingDados = document.getElementById('co-dados-overlay');
        if (existingDados) { existingDados.remove(); return; }

        const fm = entry.frontmatter || {};
        const body = entry.body || '';

        // Stats computed client-side
        const words = body.trim() ? body.trim().split(/\s+/).length : 0;
        const chars = body.length;
        const charsNoSpaces = body.replace(/\s/g, '').length;
        const readMins = Math.max(1, Math.ceil(words / 200));
        const byteSize = new TextEncoder().encode(body).length;
        const sizeHuman = byteSize < 1024 ? `${byteSize} B`
            : byteSize < 1048576 ? `${(byteSize / 1024).toFixed(1)} KB`
            : `${(byteSize / 1048576).toFixed(2)} MB`;

        const md = window.CoMarkdown;
        let h1 = 0, h2 = 0, h3 = 0;
        if (md && md.headingCount) {
            const hc = md.headingCount(body);
            h1 = hc.h1 || 0; h2 = hc.h2 || 0; h3 = hc.h3 || 0;
        } else {
            h1 = (body.match(/^# /gm) || []).length;
            h2 = (body.match(/^## /gm) || []).length;
            h3 = (body.match(/^### /gm) || []).length;
        }
        const intLinks = (body.match(/\[\[.*?\]\]/g) || []).length;
        const extLinks = (body.match(/\[.*?\]\(https?:\/\//g) || []).length;
        const images = (body.match(/!\[.*?\]\(/g) || []).length;
        const codeBlocks = Math.floor((body.match(/^```/gm) || []).length / 2);

        const tags = Array.isArray(fm.tags) ? fm.tags : [];
        const basename = (entry.path || '').split('/').pop() || '';
        const slug = basename.replace(/\.md$/, '');
        const parentPath = (entry.path || '').split('/').slice(0, -1).join('/');
        const created = entry.created_at || fm.created || '';
        const updated = entry.updated_at || fm.modified || fm.updated || '';

        function fmtDate(iso) {
            if (!iso) return '—';
            const d = new Date(iso);
            if (isNaN(d.getTime())) return iso;
            return d.toLocaleDateString('pt-BR') + ' (' + relativeDate(iso) + ')';
        }

        const fmKeys = Object.keys(fm);
        const fmTableHtml = fmKeys.length
            ? `<table class="co-dados-fm-table"><tbody>${fmKeys.map(k => {
                const v = typeof fm[k] === 'object' ? JSON.stringify(fm[k]) : String(fm[k]);
                return `<tr><td>${esc(k)}</td><td>${esc(v)}</td></tr>`;
              }).join('')}</tbody></table>`
            : '<span style="font-size:11px;color:var(--color-text-secondary)">Sem frontmatter</span>';
        const fmRawYaml = fmKeys.map(k => {
            const v = typeof fm[k] === 'object' ? JSON.stringify(fm[k]) : fm[k];
            return `${k}: ${v}`;
        }).join('\n');

        const dadosOverlay = document.createElement('div');
        dadosOverlay.id = 'co-dados-overlay';
        dadosOverlay.className = 'co-dados-overlay';
        dadosOverlay.innerHTML = `
            <div class="co-dados-panel">
                <div class="co-dados-header">
                    <span class="co-dados-title">Dados do arquivo</span>
                    <button class="co-dados-close" id="co-dados-close">
                        <span class="material-symbols-outlined" style="font-size:18px">close</span>
                    </button>
                </div>
                <div class="co-dados-body">
                    <div class="co-dados-section">
                        <div class="co-dados-section-title">Metadados</div>
                        <div class="co-dados-row"><span class="co-dados-label">Arquivo</span><span class="co-dados-value">${esc(basename)}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Caminho</span><span class="co-dados-value" style="font-size:10px">${esc(entry.path || '')}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Tipo</span><span class="co-dados-value">${esc(entry.entry_type || fm.type || 'page')}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Slug</span><span class="co-dados-value">${esc(slug)}</span></div>
                        ${parentPath ? `<div class="co-dados-row"><span class="co-dados-label">Pasta</span><span class="co-dados-value">${esc(parentPath)}</span></div>` : ''}
                        ${tags.length ? `<div class="co-dados-row"><span class="co-dados-label">Tags</span><span class="co-dados-value">${tags.map(t => `<span class="co-dados-tag-chip">${esc(t)}</span>`).join('')}</span></div>` : ''}
                        ${fm.author ? `<div class="co-dados-row"><span class="co-dados-label">Autor</span><span class="co-dados-value">${esc(fm.author)}</span></div>` : ''}
                        <div class="co-dados-row"><span class="co-dados-label">Criado</span><span class="co-dados-value">${fmtDate(created)}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Modificado</span><span class="co-dados-value">${fmtDate(updated)}</span></div>
                    </div>

                    <div class="co-dados-section">
                        <div class="co-dados-section-title">Estatísticas</div>
                        <div class="co-dados-row"><span class="co-dados-label">Palavras</span><span class="co-dados-value">${words.toLocaleString('pt-BR')}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Caracteres</span><span class="co-dados-value">${chars.toLocaleString('pt-BR')}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Sem espaços</span><span class="co-dados-value">${charsNoSpaces.toLocaleString('pt-BR')}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Leitura</span><span class="co-dados-value">~${readMins} min</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Tamanho</span><span class="co-dados-value">${sizeHuman}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Títulos</span><span class="co-dados-value">H1:${h1} H2:${h2} H3:${h3}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Links</span><span class="co-dados-value">int:${intLinks} ext:${extLinks}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Imagens</span><span class="co-dados-value">${images}</span></div>
                        <div class="co-dados-row"><span class="co-dados-label">Blocos código</span><span class="co-dados-value">${codeBlocks}</span></div>
                    </div>

                    <div class="co-dados-section">
                        <div class="co-dados-section-title">
                            Frontmatter
                            <button id="co-dados-fm-toggle" style="font-size:10px;background:none;border:none;cursor:pointer;color:var(--accent);margin-left:8px">Ver YAML bruto</button>
                        </div>
                        <div id="co-dados-fm-table">${fmTableHtml}</div>
                        <pre class="co-dados-fm-raw" id="co-dados-fm-raw">${esc(fmRawYaml || '(vazio)')}</pre>
                    </div>

                    <div class="co-dados-section">
                        <div class="co-dados-section-title">Ações</div>
                        <div class="co-dados-actions">
                            <button class="co-dados-action-btn" id="co-dados-copy-path">
                                <span class="material-symbols-outlined" style="font-size:14px">content_copy</span>Copiar caminho
                            </button>
                            <button class="co-dados-action-btn" id="co-dados-copy-fm">
                                <span class="material-symbols-outlined" style="font-size:14px">data_object</span>Copiar frontmatter como JSON
                            </button>
                            <button class="co-dados-action-btn" id="co-dados-download">
                                <span class="material-symbols-outlined" style="font-size:14px">download</span>Baixar como .md
                            </button>
                        </div>
                    </div>
                </div>
            </div>`;

        (parentEl || document.body).appendChild(dadosOverlay);

        document.getElementById('co-dados-close').addEventListener('click', () => dadosOverlay.remove());

        document.getElementById('co-dados-fm-toggle').addEventListener('click', () => {
            const raw = document.getElementById('co-dados-fm-raw');
            const table = document.getElementById('co-dados-fm-table');
            const btn = document.getElementById('co-dados-fm-toggle');
            const showing = raw.classList.contains('visible');
            raw.classList.toggle('visible', !showing);
            table.style.display = showing ? '' : 'none';
            btn.textContent = showing ? 'Ver YAML bruto' : 'Ver tabela';
        });

        document.getElementById('co-dados-copy-path').addEventListener('click', () => {
            navigator.clipboard.writeText(entry.path || '').then(() => showToast('Caminho copiado', 'success'));
        });

        document.getElementById('co-dados-copy-fm').addEventListener('click', () => {
            navigator.clipboard.writeText(JSON.stringify(fm, null, 2)).then(() => showToast('Frontmatter copiado', 'success'));
        });

        document.getElementById('co-dados-download').addEventListener('click', () => {
            const blob = new Blob([body], { type: 'text/markdown;charset=utf-8' });
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = basename || 'arquivo.md';
            document.body.appendChild(a);
            a.click();
            setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
        });
    }

    // ===== Content Editor (CodeMirror) =====
    let _contentEditorInstance = null;
    let _draftSaveInterval = null;

    async function openContentEditor(taskId) {
        const task = state.tasks.find(t => t.id === taskId);
        if (!task) return;

        const content = $('#content');
        content.className = 'content content-editor-view';
        content.innerHTML = `
            <div class="content-editor-header">
                <button class="btn btn-secondary content-editor-back" id="content-editor-back">
                    <span class="material-symbols-outlined" style="font-size:18px;vertical-align:middle">arrow_back</span>
                    ${window.t ? window.t('back') : 'Voltar'}
                </button>
                <div class="content-editor-title-area">
                    <span class="task-key" style="margin-right:8px">${esc(task.key)}</span>
                    <h2 class="content-editor-title">${esc(task.title)}</h2>
                </div>
                <button class="btn btn-primary" id="content-editor-save">${window.t ? window.t('save') : 'Salvar'}</button>
            </div>
            <div class="content-editor-body" id="content-editor-body"></div>
        `;

        const draftKey = `co_draft_task_${taskId}`;

        // Back button → return to content view
        document.getElementById('content-editor-back').addEventListener('click', () => {
            if (_draftSaveInterval) { clearInterval(_draftSaveInterval); _draftSaveInterval = null; }
            if (_contentEditorInstance) {
                _contentEditorInstance.destroy();
                _contentEditorInstance = null;
            }
            renderConteudo();
        });

        // Always show textarea first, then upgrade to CodeMirror if available
        const editorContainer = document.getElementById('content-editor-body');
        editorContainer.innerHTML = `<textarea class="content-editor-textarea" id="content-editor-textarea">${esc(task.description || '')}</textarea>`;

        try {
            await loadEditorBundle();
            if (window.CoEditor) {
                // Hide textarea, show CodeMirror
                const ta = document.getElementById('content-editor-textarea');
                if (ta) ta.style.display = 'none';
                const cmDiv = document.createElement('div');
                cmDiv.className = 'content-editor-cm';
                editorContainer.appendChild(cmDiv);
                _contentEditorInstance = window.CoEditor.initEditor(cmDiv, {
                    content: task.description || '',
                    onChange: (val) => { if (ta) ta.value = val; },
                    readOnly: false,
                });

                // Auto-save draft to localStorage every 5s
                if (_draftSaveInterval) clearInterval(_draftSaveInterval);
                _draftSaveInterval = setInterval(() => {
                    try {
                        const val = _contentEditorInstance ? _contentEditorInstance.getValue() : '';
                        localStorage.setItem(draftKey, val);
                    } catch (_) {}
                }, 5000);
            }
        } catch (_) { /* keep textarea */ }

        // Save button
        document.getElementById('content-editor-save').addEventListener('click', async () => {
            const saveBtn = document.getElementById('content-editor-save');
            const ta = document.getElementById('content-editor-textarea');
            let newDesc;
            if (_contentEditorInstance && _contentEditorInstance.getContent) {
                newDesc = _contentEditorInstance.getContent();
            } else {
                newDesc = ta ? ta.value : task.description;
            }

            if (state.isTemplate) {
                const ok = await ensureOwnUniverse();
                if (!ok) { showToast('Crie uma conta para salvar', 'error'); return; }
                // After clone, task references changed — just show success
                showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                return;
            }

            if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
            const result = await api.updateTask(state.currentProject.key, taskId, { description: newDesc });
            if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = window.t ? window.t('save') : 'Salvar'; }
            if (result) {
                task.description = newDesc;
                try { localStorage.removeItem(draftKey); } catch (_) {}
                showToast(window.t ? window.t('saved') : 'Salvo', 'success');
            } else {
                showToast('Erro ao salvar', 'error');
            }
        });
    }

    // Page editor — same as content editor but saves via vault/entries API
    async function openPageEditor(entryPath, title, body) {
        const content = $('#content');
        content.className = 'content content-editor-view';
        content.innerHTML = `
            <div class="content-editor-header">
                <button class="btn btn-secondary content-editor-back" id="content-editor-back">
                    <span class="material-symbols-outlined" style="font-size:18px;vertical-align:middle">arrow_back</span>
                    ${window.t ? window.t('back') : 'Voltar'}
                </button>
                <div class="content-editor-title-area">
                    <h2 class="content-editor-title">${esc(title)}</h2>
                </div>
                <button class="btn btn-primary" id="content-editor-save">${window.t ? window.t('save') : 'Salvar'}</button>
            </div>
            <div class="content-editor-body" id="content-editor-body"></div>
        `;

        const pageDraftKey = `co_draft_page_${encodeURIComponent(entryPath)}`;

        document.getElementById('content-editor-back').addEventListener('click', () => {
            if (_draftSaveInterval) { clearInterval(_draftSaveInterval); _draftSaveInterval = null; }
            if (_contentEditorInstance) { _contentEditorInstance.destroy(); _contentEditorInstance = null; }
            renderConteudo();
        });

        const editorContainer = document.getElementById('content-editor-body');
        editorContainer.innerHTML = `<textarea class="content-editor-textarea" id="content-editor-textarea">${esc(body)}</textarea>`;

        try {
            await loadEditorBundle();
            if (window.CoEditor) {
                const ta = document.getElementById('content-editor-textarea');
                if (ta) ta.style.display = 'none';
                const cmDiv = document.createElement('div');
                cmDiv.className = 'content-editor-cm';
                editorContainer.appendChild(cmDiv);
                _contentEditorInstance = window.CoEditor.initEditor(cmDiv, {
                    content: body,
                    onChange: (val) => { if (ta) ta.value = val; },
                    readOnly: false,
                });

                // Auto-save draft to localStorage every 5s
                if (_draftSaveInterval) clearInterval(_draftSaveInterval);
                _draftSaveInterval = setInterval(() => {
                    try {
                        const val = _contentEditorInstance ? _contentEditorInstance.getValue() : '';
                        localStorage.setItem(pageDraftKey, val);
                    } catch (_) {}
                }, 5000);
            }
        } catch (_) { /* keep textarea */ }

        document.getElementById('content-editor-save').addEventListener('click', async () => {
            const saveBtn = document.getElementById('content-editor-save');
            const ta = document.getElementById('content-editor-textarea');
            let newBody;
            if (_contentEditorInstance && _contentEditorInstance.getContent) {
                newBody = _contentEditorInstance.getContent();
            } else {
                newBody = ta ? ta.value : body;
            }

            if (state.isTemplate) {
                const ok = await ensureOwnUniverse();
                if (!ok) { showToast('Crie uma conta para salvar', 'error'); return; }
                showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                return;
            }

            if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
            const slug = state.currentUniverseSlug;
            const result = await apiFetch(`/api/v1/universes/${slug}/entries/${encodeURIComponent(entryPath)}`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ body: newBody }),
            });
            if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = window.t ? window.t('save') : 'Salvar'; }
            if (result) {
                try { localStorage.removeItem(pageDraftKey); } catch (_) {}
                showToast(window.t ? window.t('saved') : 'Salvo', 'success');
            } else {
                showToast('Erro ao salvar', 'error');
            }
        });
    }

    async function renderDashboard() {
        const content = $('#content');
        content.className = 'content';

        if (!state.currentProject) {
            content.innerHTML = '<div class="empty-state"><p>Select a project</p></div>';
            return;
        }

        content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Loading dashboard...</p></div>';

        const data = await api.getDashboard(state.currentProject.key);

        if (!data) {
            content.innerHTML = '<div class="empty-state"><p>Error loading dashboard</p></div>';
            return;
        }

        const statusCounts = data.status_counts || {};
        const totalTasks = Object.values(statusCounts).reduce((a, b) => a + b, 0);
        const upcomingDue = data.upcoming_tasks || [];
        const recentlyUpdated = data.recently_updated || [];
        const velocity = data.velocity || [];
        const burndown = data.burndown || [];
        const labelDist = data.label_distribution || [];
        const overdueDetail = data.overdue_tasks_detail || [];

        // Status bars
        let statusBarsHtml = '';
        for (const s of STATUSES) {
            const count = statusCounts[s.key] || 0;
            const pct = totalTasks > 0 ? ((count / totalTasks) * 100).toFixed(1) : 0;
            statusBarsHtml += `<div class="dashboard-status-row"><div class="dashboard-status-label"><span class="dashboard-status-dot" style="background:${s.color}"></span>${s.label}</div><div class="dashboard-status-bar-track"><div class="dashboard-status-bar-fill" style="width:${pct}%;background:${s.color}"></div></div><span class="dashboard-status-count">${count}</span></div>`;
        }

        // Overdue tasks with aging
        let overdueHtml = '';
        if (overdueDetail.length > 0) {
            overdueHtml = overdueDetail.map(t => {
                const color = overdueAgeColor(t.days_overdue);
                const daysLabel = t.days_overdue === 1 ? '1 day' : `${t.days_overdue} days`;
                return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title" style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(t.title)}</span><span style="font-size:11px;font-weight:600;color:${color};white-space:nowrap;padding:2px 6px;border-radius:4px;background:${color}18">${daysLabel} overdue</span></div>`;
            }).join('');
        } else {
            overdueHtml = '<p class="dashboard-empty">No overdue tasks</p>';
        }

        // Upcoming due list
        let upcomingHtml = '';
        if (upcomingDue.length > 0) {
            upcomingHtml = upcomingDue.map(t => {
                const overdue = t.status !== 'done' && isOverdue(t.due_date);
                return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title">${esc(t.title)}</span><span class="dashboard-task-due${overdue ? ' overdue' : ''}">${formatDate(t.due_date)}</span></div>`;
            }).join('');
        } else {
            upcomingHtml = '<p class="dashboard-empty">No tasks due in the next 7 days</p>';
        }

        // Recently updated list
        let recentHtml = '';
        if (recentlyUpdated.length > 0) {
            recentHtml = recentlyUpdated.map(t => {
                return `<div class="dashboard-task-item" data-task-id="${t.id}"><span class="dashboard-task-key">${esc(t.key)}</span><span class="dashboard-task-title">${esc(t.title)}</span><span class="status-badge status-${t.status}"><span class="status-badge-dot"></span>${STATUS_LABELS[t.status]}</span>${t.updated_at ? `<span class="dashboard-task-time">${relativeTime(t.updated_at)}</span>` : ''}</div>`;
            }).join('');
        } else {
            recentHtml = '<p class="dashboard-empty">No recently updated tasks</p>';
        }

        content.innerHTML = `<div class="dashboard"><div class="dashboard-grid"><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Velocity \u2014 Tasks Completed per Week</h3>${svgVelocityChart(velocity)}</div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Burnup \u2014 Remaining vs Completed</h3>${svgBurndownChart(burndown)}</div><div class="dashboard-card"><h3 class="dashboard-card-title">Status Distribution</h3><div class="dashboard-status-bars">${statusBarsHtml}</div><div class="dashboard-total">Total: ${totalTasks} task(s)</div></div><div class="dashboard-card"><h3 class="dashboard-card-title">Labels</h3>${svgLabelChart(labelDist)}</div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Overdue Tasks</h3><div class="dashboard-task-list">${overdueHtml}</div></div><div class="dashboard-card"><h3 class="dashboard-card-title">Upcoming Deadlines (7 days)</h3><div class="dashboard-task-list">${upcomingHtml}</div></div><div class="dashboard-card dashboard-card-wide"><h3 class="dashboard-card-title">Recently Updated</h3><div class="dashboard-task-list">${recentHtml}</div></div></div></div>`;

        content.querySelectorAll('.dashboard-task-item').forEach(el => {
            el.addEventListener('click', () => {
                const taskId = parseInt(el.dataset.taskId);
                if (taskId) openTaskModal(taskId);
            });
        });
    }

    // ===== Modal =====
    function openTaskModal(taskId) {
        const overlay = $('#modal-overlay');
        const form = $('#task-form');
        const deleteBtn = $('#btn-delete');
        const archiveBtn = $('#btn-archive');

        // Remove any stale hierarchy info from previous open
        const existingInfo = document.getElementById('task-hierarchy-info');
        if (existingInfo) existingInfo.remove();

        if (taskId) {
            state.editingTaskId = taskId;
            const task = state.tasks.find(t => t.id === taskId);
            if (!task) return;
            $('#modal-title').textContent = task.key;
            $('#task-title').value = task.title;
            $('#task-status').value = task.status;
            $('#task-priority').value = task.priority;
            $('#task-due-date').value = task.due_date || '';
            $('#task-assignee').value = task.assignee || '';
            $('#task-labels').value = task.labels.join(', ');
            $('#task-description').value = task.description || '';
            initTaskEditor(task.description || '', taskId);
            deleteBtn.classList.remove('hidden');

            // Archive button logic
            if (archiveBtn) {
                archiveBtn.classList.remove('hidden');
                archiveBtn.textContent = task.archived
                    ? (window.t ? window.t('unarchive') : 'Desarquivar')
                    : (window.t ? window.t('archive') : 'Arquivar');
            }

            // Subtask creation button (above description)
            const subtaskGroup = $('#subtask-btn-group');
            if (subtaskGroup) {
                subtaskGroup.classList.remove('hidden');
                const subtaskBtn = $('#btn-add-subtask');
                if (subtaskBtn) {
                    subtaskBtn.onclick = () => {
                        closeModal();
                        openTaskModal(null);
                        setTimeout(() => {
                            const parentSel = $('#task-parent');
                            if (parentSel) parentSel.value = String(taskId);
                        }, 50);
                    };
                }
            }

            // Inject parent link + subtask list
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

            // Load comments
            loadComments(taskId);
        } else {
            state.editingTaskId = null;
            $('#modal-title').textContent = 'New Task';
            form.reset();
            $('#task-status').value = 'todo';
            $('#task-priority').value = 'medium';
            $('#task-assignee').value = '';
            deleteBtn.classList.add('hidden');

            if (archiveBtn) archiveBtn.classList.add('hidden');
            const subtaskGroup2 = $('#subtask-btn-group');
            if (subtaskGroup2) subtaskGroup2.classList.add('hidden');
            initTaskEditor('');

            // Clear comments section
            const commentsSection = $('#comments-section');
            if (commentsSection) commentsSection.innerHTML = '';
        }

        // Populate parent select
        const parentSelect = $('#task-parent');
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
        setTimeout(() => $('#task-title').focus(), 50);
    }

    // ===== Comments =====
    async function loadComments(taskId) {
        const commentsSection = $('#comments-section');
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

        // Add comment button handler
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
                    showToast('Comment added', 'success');
                    loadComments(taskId);
                    incrementLocalUsageCount();
                }
            });
        }
    }

    // ===== Activity Feed =====
    async function toggleActivityPanel() {
        const panel = $('#activity-panel');
        if (!panel) return;

        if (panel.classList.contains('hidden')) {
            panel.classList.remove('hidden');
            await loadActivity();
        } else {
            panel.classList.add('hidden');
        }
    }

    async function loadActivity() {
        const panel = $('#activity-panel');
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

    function setupActivityClose() {
        const closeBtn = document.getElementById('activity-close');
        if (closeBtn) {
            closeBtn.addEventListener('click', () => {
                const panel = $('#activity-panel');
                if (panel) panel.classList.add('hidden');
            });
        }
    }

    function closeModal() {
        $('#modal-overlay').classList.add('hidden');
        state.editingTaskId = null;
        if (_taskDraftInterval) { clearInterval(_taskDraftInterval); _taskDraftInterval = null; }
        if (_editorInstance) {
            _editorInstance.destroy();
            _editorInstance = null;
        }
    }

    async function handleFormSubmit(e) {
        e.preventDefault();
        if (!state.currentProject) return;

        setSubmitDisabled(true);

        const data = {
            title: $('#task-title').value.trim(),
            status: $('#task-status').value,
            priority: $('#task-priority').value,
            description: $('#task-description').value.trim(),
            labels: $('#task-labels').value.split(',').map(l => l.trim()).filter(Boolean),
        };

        const assigneeVal = $('#task-assignee').value.trim();
        if (assigneeVal) data.assignee = assigneeVal;

        const dueDate = $('#task-due-date').value;
        if (dueDate) data.due_date = dueDate;

        const parentVal = $('#task-parent').value;
        if (parentVal) data.parent = parseInt(parentVal);

        // If on template, auto-clone first so writes go to own universe
        if (state.isTemplate) {
            const ok = await ensureOwnUniverse();
            if (!ok) { setSubmitDisabled(false); return; }
        }

        const key = state.currentProject.key;

        let result;
        if (state.editingTaskId) {
            result = await api.updateTask(key, state.editingTaskId, data);
        } else {
            result = await api.createTask(key, data);
        }

        setSubmitDisabled(false);

        if (result) {
            // Clear draft on successful save
            try {
                const savedId = state.editingTaskId || (result.id);
                if (savedId) localStorage.removeItem(`co_draft_task_${savedId}`);
                else localStorage.removeItem('co_draft_new_task');
            } catch (_) {}
            showToast(state.editingTaskId ? 'Task updated' : 'Task created', 'success');
            if (!state.editingTaskId) incrementLocalUsageCount();
            closeModal();
            await refreshTasks();
            render();
        }
    }

    async function handleDelete() {
        if (!state.editingTaskId || !state.currentProject) return;
        if (!confirm('Delete this task?')) return;

        await api.deleteTask(state.currentProject.key, state.editingTaskId);
        showToast('Task deleted', 'success');
        closeModal();
        await refreshTasks();
        render();
    }

    async function handleArchive() {
        if (!state.editingTaskId || !state.currentProject) return;
        const task = state.tasks.find(t => t.id === state.editingTaskId);
        if (!task) return;

        const newArchived = !task.archived;
        const result = await api.updateTask(state.currentProject.key, state.editingTaskId, {
            archived: newArchived,
        });

        if (result) {
            showToast(newArchived ? 'Task archived' : 'Task unarchived', 'success');
            closeModal();
            await refreshTasks();
            render();
        }
    }

    // ===== Navigation =====
    async function selectProject(key) {
        state.currentProject = state.projects.find(p => p.key === key);
        state.selectedIds.clear();
        loadSubtreeState(key);
        showLoading();
        await refreshTasks();
        hideLoading();
        render();
    }

    async function refreshTasks() {
        if (state.currentProject) {
            // showArchived checkbox: unchecked → only current, checked → only archived
            const opts = { archived: state.showArchived };
            state.tasks = await api.getTasks(state.currentProject.key, opts);
            applyLocalTaskOverrides();
        }
    }

    // ===== View Switching =====
    function switchView(view) {
        state.view = view;

        // Update view tabs
        document.querySelectorAll('#view-tabs .view-tab').forEach(tab => {
            tab.classList.toggle('active', tab.dataset.view === view);
        });

        // Show/hide timeline-specific controls
        const zoomTabs = $('#zoom-tabs');
        const timelineNav = $('#timeline-nav');
        if (view === 'timeline') {
            zoomTabs.classList.remove('hidden');
            timelineNav.classList.remove('hidden');
        } else {
            zoomTabs.classList.add('hidden');
            timelineNav.classList.add('hidden');
        }

        // Update mini calendar visibility
        renderMiniCalendar();

        renderContent();
    }

    function switchZoom(zoom) {
        state.zoom = zoom;
        document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab => {
            tab.classList.toggle('active', tab.dataset.zoom === zoom);
        });
        initTimelineStart();
        renderContent();
    }

    function renderContent() {
        if (state.loading) return;
        if (state.view === 'conteudo') { renderConteudo(); return; }
        // CO-73: manifest-declared gantt views use a 'gantt:<name>' key
        if (state.view.startsWith('gantt:')) {
            const viewName = state.view.slice(6);
            const manifest = state.universeManifest;
            const viewDef = manifest && (manifest.views || []).find(v => v.type === 'gantt' && v.name === viewName);
            if (viewDef) { renderGantt(viewDef); return; }
        }
        // Calendar and Timeline can render entries-as-events when the
        // manifest declares a date-semantic field (CO-73), even without a
        // project. Used by the time universe and any event-shaped universe.
        const manifestHasCalendar = (state.universeManifest?.content_types || [])
            .some(ct => ct.presentation?.calendar?.date_field);
        if (state.view === 'calendar' && manifestHasCalendar) {
            renderCalendar();
            return;
        }
        if (state.view === 'timeline' && manifestHasCalendar) {
            renderEventsTimeline();
            return;
        }
        if (!state.currentProject) {
            // No project in this universe (or fetch failed). Render an empty
            // state instead of returning silently — otherwise the loading
            // spinner stays up forever and the user is stuck.
            renderUniverseHome();
            return;
        }
        if (state.view === 'kanban') renderKanban();
        else if (state.view === 'calendar') renderCalendar();
        else if (state.view === 'table') renderTable();
        else if (state.view === 'timeline') renderTimeline();
        else if (state.view === 'dashboard') renderDashboard();
    }

    // Universe home / front page. Renders the universe's `index.md` body as a
    // hero, falling back to a description + empty-state when no index exists.
    // Also used as the safe-fallback when there's no project to render.
    async function renderUniverseHome() {
        const content = $('#content');
        if (!content) return;
        const slug = state.currentUniverseSlug;
        const info = state.universeInfo || {};
        const name = info.name || slug || '';
        const description = info.description || '';

        // Render skeleton immediately so spinner clears even if fetch is slow.
        content.className = 'content universe-home-view';
        content.innerHTML = `
            <div class="universe-home">
                <header class="universe-home-header">
                    <h1 class="universe-home-title">${esc(name)}</h1>
                    ${description ? `<p class="universe-home-desc">${esc(description)}</p>` : ''}
                </header>
                <div class="universe-home-body" id="universe-home-body">
                    <div class="universe-home-loading">Carregando…</div>
                </div>
            </div>`;

        // Try `index.md` at the universe root. Server responds 404 if absent.
        let indexEntry = null;
        try {
            indexEntry = await apiFetch(
                `/api/v1/universes/${encodeURIComponent(slug)}/entries/${encodeURIComponent('index.md')}`,
                {},
                true
            );
        } catch (_) { /* fall through */ }

        const body = $('#universe-home-body');
        if (!body) return;

        if (indexEntry && indexEntry.body) {
            const md = window.CoMarkdown;
            const html = md ? md.renderMarkdown(indexEntry.body) : esc(indexEntry.body);
            body.innerHTML = `<article class="universe-home-md md-body">${html}</article>`;
            // Post-process Mermaid fenced blocks (CO-107). The helper lazy-loads
            // the bundle only if a `mermaid` block is present, so this is a no-op
            // when index.md has none.
            if (md && typeof md.renderMermaidBlocks === 'function') {
                md.renderMermaidBlocks(body);
            }
            return;
        }

        // No index — show a clear "what to do next" empty state, with hints
        // grounded in what this universe contains. Avoid the spooky blank.
        const totalEntries = (info.content_count || state.projects?.length || 0);
        body.innerHTML = `
            <div class="universe-home-empty">
                <p>Este universo ainda não tem uma página inicial.</p>
                <p style="color:var(--text-muted,#6b7280);font-size:13px">
                    Crie um arquivo <code>index.md</code> na raiz do universo para descrever
                    o que ele é. Ele será renderizado aqui.
                </p>
                ${totalEntries ? `<p style="color:var(--text-muted,#6b7280);font-size:13px">Há <strong>${totalEntries}</strong> entrada(s) neste universo. Acesse pelo menu lateral ou pela visão Conteúdo.</p>` : ''}
            </div>`;
    }

    function render() {
        renderSidebar();
        renderHeader();
        renderMiniCalendar();
        renderContent();
    }

    // ===== Mobile Hamburger Menu =====
    function setupHamburgerMenu() {
        const hamburgerBtn = document.getElementById('hamburger-btn');
        const sidebar = document.getElementById('sidebar');
        const overlay = document.getElementById('sidebar-overlay');

        if (hamburgerBtn && sidebar) {
            hamburgerBtn.addEventListener('click', () => {
                sidebar.classList.toggle('open');
                if (overlay) overlay.classList.toggle('visible');
            });
        }

        if (overlay && sidebar) {
            overlay.addEventListener('click', () => {
                sidebar.classList.remove('open');
                overlay.classList.remove('visible');
            });
        }

        const toggleProjects = document.getElementById('btn-toggle-projects');
        const projectList = document.getElementById('project-list');
        if (toggleProjects && projectList) {
            toggleProjects.addEventListener('click', () => {
                const expanded = toggleProjects.getAttribute('aria-expanded') === 'true';
                toggleProjects.setAttribute('aria-expanded', String(!expanded));
                projectList.classList.toggle('collapsed', expanded);
            });
        }
    }

    // ===== Events =====
    // View tabs
    document.querySelectorAll('#view-tabs .view-tab').forEach(tab => {
        tab.addEventListener('click', () => switchView(tab.dataset.view));
    });

    // Zoom tabs (timeline)
    document.querySelectorAll('#zoom-tabs .view-tab').forEach(tab => {
        tab.addEventListener('click', () => switchZoom(tab.dataset.zoom));
    });

    // Timeline navigation
    $('#btn-prev').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), -shift);
        renderContent();
    });

    $('#btn-today').addEventListener('click', () => {
        initTimelineStart();
        renderContent();
    });

    $('#btn-next').addEventListener('click', () => {
        const shift = Math.floor(ZOOM_DAYS[state.zoom] / 2);
        state.timelineStart = addDays(state.timelineStart || todayDate(), shift);
        renderContent();
    });

    // New task
    $('#btn-new-task').addEventListener('click', async () => {
        if (state.isTemplate) { await ensureOwnUniverse(); return; }
        if (state.currentProject) openTaskModal(null);
    });

    // 1.58.0: Universe info modal — public read-only metadata + stats, plus
    // (auth-gated) state save and state history with diff. Replaces the
    // header-only ⏱/🕓 buttons from 1.55-1.57 — versioning surface lives in
    // the info page, not on the Conteúdo header.
    const btnInfo = $('#btn-universe-info');
    const infoOverlay = $('#universe-info-overlay');
    const infoClose = $('#universe-info-close');
    const infoBody = $('#universe-info-body');
    const infoTitle = $('#universe-info-title');

    if (infoClose) infoClose.addEventListener('click', () => infoOverlay && infoOverlay.classList.add('hidden'));
    if (infoOverlay) infoOverlay.addEventListener('click', e => {
        if (e.target === infoOverlay) infoOverlay.classList.add('hidden');
    });

    if (btnInfo) {
        btnInfo.addEventListener('click', async () => {
            if (!infoOverlay) return;
            infoOverlay.classList.remove('hidden');
            await renderUniverseInfo();
        });
    }

    async function renderUniverseInfo() {
        const slug = state.currentUniverseSlug;
        if (!slug || !infoBody) return;
        if (infoTitle) infoTitle.textContent = `Universo · ${slug}`;
        infoBody.innerHTML = '<p class="empty-state" style="margin:16px">Carregando…</p>';
        try {
            // Always-public: universe metadata.
            const info = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}`, {}, true);
            // Always-public: entry list (used for type breakdown). Errors here
            // are non-fatal — we still render metadata.
            let entries = [];
            try {
                const r = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/entries?limit=5000`, {}, true);
                entries = (r && r.entries) || [];
            } catch (_) {}
            // 1.61.0: fetch user's subscription status (subscribed + pin).
            // 1.62.0: cache the pin so other code paths (Conteúdo, Calendário,
            // …) can append `?as_of=` and get the rewind view automatically.
            let subscription = null;
            try {
                subscription = await apiFetch(`/api/v1/universes/${encodeURIComponent(slug)}/subscription`, {}, true);
            } catch (_) {}
            if (subscription && subscription.pinned_state) {
                state.subscriptionPin[slug] = subscription.pinned_state;
            } else {
                delete state.subscriptionPin[slug];
            }
            // Versioning entries are filtered from the same listing.
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
            infoBody.innerHTML = '<p class="empty-state" style="margin:16px;color:#dc2626">Erro ao carregar informações.</p>';
        }
    }

    function renderInfoSections(slug, info, entries, stateEntries, branchEntries, proposalEntries, mergeEntries, typeCounts, subscription) {
        if (!infoBody) return;
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
        // State save action (auth required) + state history with inline diff.
        // 1.61.0: surface subscription status — subscribed yes/no + pinned-to.
        let subBlock = '';
        if (subscription) {
            const pinPath = subscription.pinned_state || '';
            const pinShort = pinPath ? pinPath.split('/').pop().slice(0, 24) : '';
            const status = subscription.subscribed
                ? (pinPath
                    ? `📌 Pinned to <code>${esc(pinShort)}…</code> <button id="info-unpin" class="btn-text-sm" style="margin-left:8px;color:#dc2626;background:none;border:none;cursor:pointer;font-size:11px;padding:2px 6px">unpin</button>`
                    : `Subscribed (following head)`)
                : `Not subscribed`;
            subBlock = `<div style="font-size:11px;color:var(--text-muted,#6b7280);margin-bottom:8px">${status}</div>`;
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
        // Branches (named pointers to a state).
        const branchSection = `
            <section style="padding:16px;border-bottom:1px solid var(--border,#e5e7eb)">
                <h3 style="margin:0 0 8px;font-size:14px">Branches</h3>
                <div id="info-branch-list" style="font-size:12px"></div>
            </section>`;
        // Proposals + merge events.
        const propSection = `
            <section style="padding:16px">
                <h3 style="margin:0 0 8px;font-size:14px">Propostas e merges</h3>
                <div id="info-proposal-list" style="font-size:12px"></div>
            </section>`;
        infoBody.innerHTML = overview + typesSection + stateSection + branchSection + propSection;
        renderStatesInto(infoBody.querySelector('#info-state-list'), stateEntries, slug, (subscription && subscription.pinned_state) || null);
        renderBranchesInto(infoBody.querySelector('#info-branch-list'), branchEntries, slug, stateEntries);
        renderProposalsInto(infoBody.querySelector('#info-proposal-list'), proposalEntries, mergeEntries, slug, stateEntries);

        // 1.61.0: unpin handler.
        const unpinBtn = infoBody.querySelector('#info-unpin');
        if (unpinBtn) {
            unpinBtn.addEventListener('click', async () => {
                try {
                    await apiFetch(
                        `/api/v1/universes/${encodeURIComponent(slug)}/subscribe/pin`,
                        { method: 'DELETE' }, true,
                    );
                    showToast('Unpinned (now following head)', 'success');
                    delete state.subscriptionPin[slug];
                    await renderUniverseInfo();
                    if (state.currentUniverseSlug === slug) renderContent();
                } catch (err) {
                    console.error('unpin failed', err);
                    showToast('Failed to unpin', 'error');
                }
            });
        }
        const saveBtn = infoBody.querySelector('#info-save-state');
        if (saveBtn) {
            saveBtn.addEventListener('click', async () => {
                if (state.isTemplate) { await ensureOwnUniverse(); return; }
                const msg = window.prompt('Message for this state (optional):', '');
                if (msg === null) return;
                try {
                    const r = await apiFetch(
                        `/api/v1/universes/${encodeURIComponent(slug)}/states`,
                        { method: 'POST', body: JSON.stringify({ message: msg }), headers: { 'Content-Type': 'application/json' } },
                        true,
                    );
                    showToast(`State saved · ${String(r.state_hash).slice(0, 12)}…`, 'success');
                    await renderUniverseInfo(); // re-fetch
                } catch (err) {
                    console.error('save state failed', err);
                    showToast('Failed to save state', 'error');
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

        // 1.60.0: pin handler. Stops propagation so the row's click-to-diff
        // doesn't fire on the same click. 1.61.0: re-render the modal after
        // success so the new pinned state shows the "📌 pinned" indicator.
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
                    showToast(`Pinned to ${statePath.split('/').pop().slice(0, 16)}…`, 'success');
                    state.subscriptionPin[rowSlug] = statePath;
                    await renderUniverseInfo();
                    // 1.62.0: re-render the background view so the rewind
                    // (?as_of=) actually filters entries.
                    if (state.currentUniverseSlug === rowSlug) renderContent();
                } catch (err) {
                    console.error('pin failed', err);
                    showToast('Failed to pin (login required)', 'error');
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
                    console.error('diff fetch failed', err);
                    panel.innerHTML = '<em style="color:#dc2626">Failed to load diff.</em>';
                }
            });
        });
    }

    function renderBranchesInto(container, branches, slug, stateEntries) {
        if (!container) return;
        // 1.63.0: inline create-branch form. Available to authed users only
        // (server gates the POST). Opens on "+ Nova branch" click; submits
        // {name, head_state, default} and re-renders the info modal.
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
                    // 1.64.0: per-branch advance disclosure. PUT /branches/:name
                    // with a different state path to fast-forward (no merge logic
                    // here — just bump the pointer).
                    const branchName = fm.name || '';
                    const advanceForm = (stateEntries && stateEntries.length > 0)
                        ? `
                            <details style="margin-top:4px">
                                <summary style="cursor:pointer;color:var(--accent,#6366f1);font-size:11px">↳ advance head</summary>
                                <form class="info-branch-advance-form" data-branch-name="${esc(branchName)}" style="margin-top:4px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                                    <select name="head_state" required style="padding:3px 6px;font-size:11px;border:1px solid var(--border,#e5e7eb);border-radius:3px;flex:1;min-width:200px">${stateOptions}</select>
                                    <button type="submit" class="btn btn-secondary btn-sm" style="font-size:11px;padding:2px 8px">apply</button>
                                </form>
                            </details>`
                        : '';
                    return `
                        <div style="padding:6px 0;border-bottom:1px solid var(--border,#e5e7eb)">
                            <div><strong>${esc(fm.name || '?')}</strong>${isDefault}</div>
                            <div style="color:var(--text-muted,#6b7280);font-size:11px;margin-top:2px">
                                head: <code>${esc(headShort)}…</code> · updated ${esc(updated)}
                            </div>
                            ${advanceForm}
                        </div>`;
                }).join('');
            })();

        const formHtml = (stateEntries && stateEntries.length > 0)
            ? `
                <details style="margin-top:8px">
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
                    showToast(`Branch "${body.name}" created`, 'success');
                    await renderUniverseInfo();
                } catch (err) {
                    console.error('create branch failed', err);
                    showToast('Failed to create branch (login required?)', 'error');
                }
            });
        }

        // 1.64.0: per-branch advance handlers. Each form has its own branch name.
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
                    showToast(`Branch "${branchName}" advanced`, 'success');
                    await renderUniverseInfo();
                } catch (err) {
                    console.error('advance branch failed', err);
                    showToast('Failed to advance branch (login required?)', 'error');
                }
            });
        });
    }

    function renderProposalsInto(container, proposals, merges, slug, stateEntries) {
        if (!container) return;

        // Build the rows.
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
            // 1.65.0: open proposals get a Merge button.
            const mergeBtn = status === 'open'
                ? `<button class="info-merge-btn" data-proposal-path="${esc(p.path)}" style="background:#1e40af;color:white;border:none;padding:2px 10px;border-radius:3px;font-size:11px;cursor:pointer">⇆ Merge</button>`
                : '';
            items.push({
                ts: fm.created_at || '',
                html: `
                    <div style="padding:8px 0;border-bottom:1px solid var(--border,#e5e7eb)">
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
                html: `
                    <div style="padding:8px 0;border-bottom:1px solid var(--border,#e5e7eb)">
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
        items.sort((a, b) => (a.ts < b.ts ? 1 : -1)); // newest first

        const listHtml = items.length === 0
            ? '<em style="color:var(--text-muted,#6b7280);font-size:11px;display:block;padding:4px 0">Sem propostas ou merges ainda.</em>'
            : items.map(x => x.html).join('');

        // 1.65.0: + Propor mudanças form. Posts to TARGET universe's
        // /proposals endpoint with this universe's state as source.
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
            ? `
                <details style="margin-top:8px">
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

        // Wire up the create-proposal form.
        const propForm = container.querySelector('#info-proposal-form');
        if (propForm) {
            propForm.addEventListener('submit', async (ev) => {
                ev.preventDefault();
                const fd = new FormData(propForm);
                const targetUniverse = String(fd.get('target_universe') || '').trim();
                if (!targetUniverse) return;
                const body = {
                    source_universe: slug, // current universe is the source
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
                    showToast(`Proposal created in ${targetUniverse} · ${(r.path || '').split('/').pop().slice(0, 24)}`, 'success');
                    await renderUniverseInfo();
                } catch (err) {
                    console.error('create proposal failed', err);
                    showToast('Failed to create proposal', 'error');
                }
            });
        }

        // Wire up the per-proposal merge buttons (only on open proposals,
        // listed in THIS universe — i.e. someone proposed changes to us).
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
                    showToast(`Merged · ${r.entries_copied || 0} entries copied`, 'success');
                    await renderUniverseInfo();
                } catch (err) {
                    console.error('merge failed', err);
                    showToast('Failed to merge', 'error');
                }
            });
        });
    }

    // ---- (legacy 1.56-only state-history modal handlers; not wired) ----
    // eslint-disable-next-line no-unused-vars
    function _legacy_renderStateHistory(entries) {
        const historyBody = $('#state-history-body');
        if (!historyBody) return;
        if (!entries || entries.length === 0) {
            historyBody.innerHTML = '<p class="empty-state" style="margin:0">Sem estados ainda. Clique <strong>⏱ Estado</strong> para criar o primeiro.</p>';
            return;
        }
        const sorted = entries.slice().sort((a, b) => (a.path < b.path ? 1 : -1));
        const slug = state.currentUniverseSlug;
        const rows = sorted.map(e => {
            const fm = e.frontmatter || {};
            const hash = (fm.state_hash || '').slice(0, 12);
            const msg = fm.message || '';
            const count = fm.entry_count != null ? fm.entry_count : '?';
            const created = fm.created_at || '';
            const author = fm.author ? `<span style="color:var(--text-muted,#6b7280);font-size:11px">by ${esc(String(fm.author).slice(0, 12))}</span>` : '';
            const parentPath = fm.parent || '';
            const parentTag = parentPath ? `<div style="font-size:11px;color:var(--text-muted,#6b7280);margin-top:2px">parent: <code>${esc(parentPath.split('/').pop().slice(0, 24))}…</code></div>` : '';
            const ts = created ? new Date(created).toLocaleString() : '';
            return `
                <div class="state-row" style="padding:10px 0;border-bottom:1px solid var(--border,#e5e7eb);cursor:pointer"
                     data-state-path="${esc(e.path)}" data-parent-path="${esc(parentPath)}" data-slug="${esc(slug)}"
                     title="Click to see what changed in this state">
                    <div style="display:flex;justify-content:space-between;gap:12px;align-items:baseline">
                        <code style="font-size:12px;font-family:var(--mono,monospace);background:var(--bg-subtle,#f3f4f6);padding:2px 6px;border-radius:3px">${esc(hash)}…</code>
                        <span style="font-size:11px;color:var(--text-muted,#6b7280)">${esc(ts)}</span>
                    </div>
                    <div style="margin-top:4px;font-size:13px">${esc(msg) || '<em style="color:var(--text-muted,#6b7280)">(no message)</em>'}</div>
                    <div style="font-size:11px;color:var(--text-muted,#6b7280);margin-top:2px">${count} entries ${author}</div>
                    ${parentTag}
                </div>`;
        }).join('');
        historyBody.innerHTML = rows;

        // 1.57.0: click a state row → inline diff panel showing what changed
        // vs its parent state. Uses the /states/diff API (1.51.0). Click again
        // to collapse. First state (no parent) shows a friendly note.
        historyBody.querySelectorAll('.state-row').forEach(row => {
            row.addEventListener('click', async () => {
                const existing = row.querySelector('.state-diff');
                if (existing) { existing.remove(); return; }
                const statePath = row.dataset.statePath;
                const parentPath = row.dataset.parentPath;
                const rowSlug = row.dataset.slug;
                const panel = document.createElement('div');
                panel.className = 'state-diff';
                panel.style.cssText = 'margin-top:8px;padding:8px 10px;background:var(--bg-subtle,#f9fafb);border-radius:4px;font-size:12px;border:1px solid var(--border,#e5e7eb)';
                panel.innerHTML = '<em>Loading diff…</em>';
                row.appendChild(panel);
                if (!parentPath) {
                    panel.innerHTML = '<em style="color:var(--text-muted,#6b7280)">First state — no parent to diff against.</em>';
                    return;
                }
                try {
                    const url = `/api/v1/universes/${encodeURIComponent(rowSlug)}/states/diff?from=${encodeURIComponent(parentPath)}&to=${encodeURIComponent(statePath)}`;
                    const diff = await apiFetch(url, {}, true);
                    const parts = [];
                    const fmtList = (xs, sign, color) => xs.slice(0, 50).map(e =>
                        `<div style="padding-left:12px;color:${color};font-family:var(--mono,monospace);font-size:11px">${sign} ${esc(e.path)}</div>`
                    ).join('') + (xs.length > 50 ? `<div style="padding-left:12px;color:var(--text-muted,#6b7280);font-size:11px">… and ${xs.length - 50} more</div>` : '');
                    if (diff.added.length) {
                        parts.push(`<div style="color:#059669;font-weight:600">+${diff.added.length} added</div>` + fmtList(diff.added, '+', '#059669'));
                    }
                    if (diff.modified.length) {
                        parts.push(`<div style="color:#d97706;font-weight:600;margin-top:4px">~${diff.modified.length} modified</div>` + fmtList(diff.modified, '~', '#d97706'));
                    }
                    if (diff.removed.length) {
                        parts.push(`<div style="color:#dc2626;font-weight:600;margin-top:4px">-${diff.removed.length} removed</div>` + fmtList(diff.removed, '-', '#dc2626'));
                    }
                    if (parts.length === 0) {
                        panel.innerHTML = `<em style="color:var(--text-muted,#6b7280)">No changes — ${diff.unchanged} entries unchanged from parent.</em>`;
                    } else {
                        panel.innerHTML = parts.join('') + `<div style="margin-top:6px;color:var(--text-muted,#6b7280);font-size:11px">${diff.unchanged} unchanged</div>`;
                    }
                } catch (err) {
                    console.error('diff fetch failed', err);
                    panel.innerHTML = '<em style="color:#dc2626">Failed to load diff.</em>';
                }
            });
        });
    }

    // Modal
    $('#modal-close').addEventListener('click', closeModal);
    $('#btn-cancel').addEventListener('click', closeModal);
    $('#task-form').addEventListener('submit', handleFormSubmit);
    $('#btn-delete').addEventListener('click', handleDelete);

    // Archive button
    const archiveBtn = $('#btn-archive');
    if (archiveBtn) {
        archiveBtn.addEventListener('click', handleArchive);
    }

    $('#modal-overlay').addEventListener('click', (e) => {
        if (e.target === e.currentTarget) closeModal();
    });

    // Search
    $('#search-input').addEventListener('input', (e) => {
        state.searchQuery = e.target.value;
        renderContent();
    });

    // Archive toggle
    const showArchivedCb = document.getElementById('show-archived');
    if (showArchivedCb) {
        showArchivedCb.addEventListener('change', async () => {
            state.showArchived = showArchivedCb.checked;
            await refreshTasks();
            render();
        });
    }

    // Activity panel toggle button
    const activityBtn = document.getElementById('btn-activity');
    if (activityBtn) {
        activityBtn.addEventListener('click', toggleActivityPanel);
    }

    // Keyboard shortcuts
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            closeStatusDropdown();
            closeModal();
            // Close activity panel on Escape
            const panel = $('#activity-panel');
            if (panel && !panel.classList.contains('hidden')) {
                panel.classList.add('hidden');
            }
            // Close sidebar on mobile
            const sidebar = document.getElementById('sidebar');
            const sidebarOverlay = document.getElementById('sidebar-overlay');
            if (sidebar) sidebar.classList.remove('open');
            if (sidebarOverlay) sidebarOverlay.classList.remove('visible');
            return;
        }

        // Only handle shortcuts when no input/textarea is focused
        const active = document.activeElement;
        const isEditing = active && (
            active.tagName === 'INPUT' ||
            active.tagName === 'TEXTAREA' ||
            active.tagName === 'SELECT' ||
            active.isContentEditable
        );
        if (isEditing) return;

        switch (e.key) {
            case 'n':
                e.preventDefault();
                if (state.currentProject) openTaskModal(null);
                break;
            case '/':
                e.preventDefault();
                const searchInput = $('#search-input');
                if (searchInput) searchInput.focus();
                break;
            case '1':
                e.preventDefault();
                switchView('kanban');
                break;
            case '2':
                e.preventDefault();
                switchView('table');
                break;
            case '3':
                e.preventDefault();
                switchView('timeline');
                break;
            case '4':
                e.preventDefault();
                switchView('calendar');
                break;
            case '5':
                e.preventDefault();
                switchView('dashboard');
                break;
            case '6':
                e.preventDefault();
                switchView('conteudo');
                break;
        }
    });

    // ===== i18n — delegated to /shared/i18n.js =====
    // window.t(), window.setLang(), window.currentLang are provided by i18n.js.

    document.addEventListener('co:langchange', () => {
        STATUSES = buildStatuses();
        PRIORITY_LABELS = buildPriorityLabels();
        STATUS_LABELS = buildStatusLabels();
        render();
    });

    // ===== Usage Limit Modal =====

    function showUsageLimitModal(data) {
        const overlay = document.getElementById('usage-limit-overlay');
        if (!overlay) return;
        const titleEl = document.getElementById('usage-limit-title');
        const msgEl = document.getElementById('usage-limit-msg');
        const current = data && data.current != null ? data.current : 100;
        if (titleEl) titleEl.textContent = window.t('universe.limit');
        if (msgEl) msgEl.textContent = window.t('universe.limit_msg').replace('{n}', current);
        overlay.classList.remove('hidden');
    }

    function hideUsageLimitModal() {
        const overlay = document.getElementById('usage-limit-overlay');
        if (overlay) overlay.classList.add('hidden');
    }

    function setupUsageLimitModal() {
        const btnLogin = document.getElementById('btn-usage-login');
        const btnLang = document.getElementById('btn-usage-lang');
        if (btnLogin) {
            btnLogin.addEventListener('click', () => {
                hideUsageLimitModal();
                showLoginModal();
            });
        }
        if (btnLang) {
            btnLang.addEventListener('click', () => {
                window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
                render();
            });
        }
    }

    // ===== Auth UI =====

    function showLoginModal() {
        const overlay = document.getElementById('login-modal-overlay');
        if (overlay) {
            overlay.classList.remove('hidden');
            window.setLang(window.currentLang);
            const usuarioInput = document.getElementById('login-usuario');
            if (usuarioInput) usuarioInput.focus();
        }
    }

    function hideLoginModal() {
        const overlay = document.getElementById('login-modal-overlay');
        if (overlay) overlay.classList.add('hidden');
    }

    function renderUserBadge(me) {
        const sidebarUser = document.getElementById('sidebar-user');
        const nameEl = document.getElementById('user-display-name');
        if (sidebarUser) sidebarUser.classList.remove('hidden');
        if (nameEl) nameEl.textContent = me.display_name || me.email;
        renderHeaderUserArea(me);
    }

    function setupLoginModal() {
        const btnEntrar = document.getElementById('btn-entrar');
        const btnLogout = document.getElementById('btn-logout');
        const btnLang = document.getElementById('btn-lang-toggle');

        if (btnLang) {
            btnLang.addEventListener('click', () => {
                window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
                render();
            });
        }

        async function attemptLogin() {
            const usuario = document.getElementById('login-usuario').value.trim();
            const senha = document.getElementById('login-senha').value;
            if (!usuario || !senha) return;

            const errEl = document.getElementById('login-error');
            errEl.classList.add('hidden');
            btnEntrar.disabled = true;
            btnEntrar.textContent = window.t('signing_in');

            const r = await api.loginWithPassword(usuario, senha);

            btnEntrar.disabled = false;
            btnEntrar.textContent = window.t('sign_in');

            if (r && r.usuario) {
                hideLoginModal();
                // Clear anonymous local clone reference (disposable)
                localStorage.removeItem('co_local_universe');

                const me = await api.me();
                if (me) renderUserBadge(me);

                // Get user's communities — only keep unique ones
                const owned = await api.listUniverses();
                const mine = (owned || []).filter(u => !u.is_template);
                state.userUniverses = mine;

                let targetSlug;
                // Prefer personal community (owned by this user) over shared ones
                const userId = me?.user_id || '';
                const personal = mine.find(u => u.owner_id === userId);
                if (personal) {
                    targetSlug = personal.key;
                } else {
                    // No personal community yet — create one (even if shared communities exist)
                    const displayName = r.display_name || r.usuario || me?.display_name || 'Minha comunidade';
                    const slug = displayName.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 40) || 'minha-comunidade';

                    // Try creating empty universe
                    let result = await apiFetch('/api/v1/universes', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ key: slug, name: displayName, description: '' }),
                    }, true);

                    // If create failed (auth issue, duplicate slug), try clone as fallback
                    if (!result) {
                        const fallbackSlug = slug + '-' + Math.random().toString(36).slice(2, 6);
                        result = await api.cloneUniverse('template', {
                            name: displayName,
                            key: fallbackSlug,
                            description: '',
                        });
                    }

                    if (result) {
                        targetSlug = result.key;
                        state.userUniverses.push(result);
                    }
                }

                if (targetSlug) {
                    setUniverseSlugInUrl(targetSlug);
                    state.currentUniverseSlug = targetSlug;
                    state.isTemplate = false;
                    hideTemplateBanner();
                    // Re-fetch full universe list (includes quilombo membership)
                    const refreshed = await api.listUniverses();
                    state.userUniverses = (refreshed || []).filter(u => !u.is_template);
                    await bootAppForUniverse(targetSlug);
                    return;
                }
                await bootApp();
            } else if (r && r.error === 'unauthorized') {
                errEl.textContent = window.t('invalid_credentials');
                errEl.classList.remove('hidden');
                document.getElementById('login-senha').value = '';
                document.getElementById('login-senha').focus();
            } else {
                errEl.textContent = window.t('login_error');
                errEl.classList.remove('hidden');
            }
        }

        if (btnEntrar) btnEntrar.addEventListener('click', attemptLogin);

        // Enter on either field submits
        ['login-usuario', 'login-senha'].forEach(id => {
            const el = document.getElementById(id);
            if (el) el.addEventListener('keydown', e => { if (e.key === 'Enter') attemptLogin(); });
        });

        if (btnLogout) {
            btnLogout.addEventListener('click', async () => {
                await api.logout();
                document.getElementById('sidebar-user').classList.add('hidden');
                document.getElementById('login-usuario').value = '';
                document.getElementById('login-senha').value = '';
                showLoginModal();
            });
        }
    }

    // ===== Universe routing helpers =====

    function readUniverseSlugFromUrl() {
        // Path-based routing: `/` → template, `/{slug}` → slug, `/{slug}/...`
        // → slug. Reserved top-level paths return 'template' (the hub).
        const RESERVED = ['', 'admin', 'settings', 'yggdrasil', 'static', 'health', '_app'];
        if (window.location.pathname.match(/^\/yggdrasil\/[a-z0-9-]+/)) return 'yggdrasil';
        const m = window.location.pathname.match(/^\/([a-z0-9-]+)(\/|$)/);
        if (m && !RESERVED.includes(m[1])) return m[1];
        return 'template';
    }

    /**
     * Extract the entry path from a deep URL like /{slug}/docs/file.md.
     * Returns null when the URL has no subpath beyond the universe slug.
     */
    function readEntryPathFromUrl(universeSlug) {
        const prefix = `/${universeSlug}/`;
        const p = window.location.pathname;
        if (!p.startsWith(prefix)) return null;
        const sub = p.slice(prefix.length);
        if (!sub) return null;
        // Strip trailing .md so path matches the entry index key
        return sub.replace(/\.md$/i, '');
    }

    /**
     * If the current URL contains a deep entry path (e.g. /mbya/refs/foo),
     * fetch and open that entry in the zoom modal after the universe boots.
     */
    async function maybeOpenEntryFromUrl(universeSlug) {
        let entryPath = readEntryPathFromUrl(universeSlug);
        if (!entryPath) { maybeOpenPageFromUrl(universeSlug); return; }
        // Strip a spurious leading 'entries/' if old-format URLs are clicked
        if (entryPath.startsWith('entries/')) entryPath = entryPath.slice('entries/'.length);
        // Try both without .md (clean URL) and with .md (stored path)
        const tryPaths = [entryPath + '.md', entryPath];
        for (const p of tryPaths) {
            try {
                const encodedPath = p.split('/').map(encodeURIComponent).join('/');
                const entry = await apiFetch(
                    `/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${encodedPath}`
                );
                if (entry && entry.path) {
                    window.history.replaceState({}, '', `/${universeSlug}`);
                    openZoomModal({ ...entry, _universeSlug: universeSlug }, false);
                    return;
                }
            } catch (_) {}
        }

        // Exact path failed — try full-text search for the last path segment.
        // Handles wikilinks like [[GNDicInt]] pointing to refs/GNDicInt.md.
        const stem = entryPath.split('/').pop().replace(/\.md$/i, '');
        try {
            const res = await apiFetch(
                `/api/v1/universes/${encodeURIComponent(universeSlug)}/entries?q=${encodeURIComponent(stem)}&limit=5`
            );
            if (res && res.entries && res.entries.length > 0) {
                // Prefer an entry whose path stem matches exactly.
                const exact = res.entries.find(e => {
                    const s = (e.path || '').split('/').pop().replace(/\.md$/i, '');
                    return s.toLowerCase() === stem.toLowerCase();
                }) || res.entries[0];
                window.history.replaceState({}, '', `/${universeSlug}`);
                openZoomModal({ ...exact, _universeSlug: universeSlug }, false);
                return;
            }
        } catch (_) {}

        window.history.replaceState({}, '', `/${universeSlug}`);
        maybeOpenPageFromUrl(universeSlug);
    }

    function readGameFromUrl() {
        const m = window.location.pathname.match(/^\/yggdrasil\/([a-z0-9-]+)$/);
        return m ? m[1] : null;
    }

    function setUniverseSlugInUrl(slug) {
        // Path-based routing: hub at `/`, universes at `/{slug}`.
        const newPath = slug === 'template' ? '/' : `/${slug}`;
        window.history.pushState({}, '', newPath);
        // Persist preferred universe so login → correct board on next visit.
        if (slug && slug !== 'template') {
            try { localStorage.setItem('co_preferred_universe', slug); } catch (_) {}
        }
    }

    function showTemplateBanner() {
        const banner = document.getElementById('template-banner');
        if (banner) banner.classList.remove('hidden');
    }

    function hideTemplateBanner() {
        const banner = document.getElementById('template-banner');
        if (banner) banner.classList.add('hidden');
        const app = document.getElementById('app');
        if (app) app.classList.remove('is-template');
    }

    function applyTemplateReadonlyTooltips() {
        // No-op: users always work on their own clone, never on read-only template
    }

    // ===== Onboarding Banner (CO-99) =====
    //
    // Three-step coach mark for first-time anonymous visitors on the template
    // universe. Non-blocking floating card. Visible only when:
    //   - state.isTemplate === true
    //   - api.me() returned null (anonymous)
    //   - cookie `co_onboarded` is NOT set
    //   - viewport width >= 720px
    // Once dismissed (Pular) or completed (after step 3), sets the cookie for
    // a year to avoid re-display on subsequent visits.

    function setupOnboarding() {
        const banner = document.getElementById('onboarding-banner');
        if (!banner) return;

        // Cookie gate. If already onboarded, never show.
        if (/(?:^|;\s*)co_onboarded=1(?:;|$)/.test(document.cookie || '')) return;

        // Viewport gate. Mobile UX is deferred per the ticket.
        if (window.innerWidth < 720) return;

        // Anonymous-on-template gate. setupOnboarding is invoked only from the
        // anonymous-template branch in init(); we re-check state defensively
        // so a future caller can't show the banner for the wrong audience.
        if (!state.isTemplate) return;

        const titleEl = document.getElementById('onboarding-title');
        const bodyEl = document.getElementById('onboarding-body');
        const stepEl = document.getElementById('onboarding-step-indicator');
        const skipBtn = document.getElementById('onboarding-skip');
        const nextBtn = document.getElementById('onboarding-next');
        if (!titleEl || !bodyEl || !stepEl || !skipBtn || !nextBtn) return;

        const steps = [
            {
                title: 'Visões',
                bodyHtml: 'O Co tem várias visões do mesmo conteúdo: <strong>Quadro</strong>, <strong>Tabela</strong>, <strong>Conteúdo</strong>, <strong>Linha do tempo</strong>. Clique nas abas acima para alternar.',
                cta: 'Próximo',
            },
            {
                title: 'Linha do tempo',
                bodyHtml: 'A linha do tempo mostra eventos numa escala log: do Big Bang ao agora. <a href="/shared/timeline.html?u=tempo,universo,humanity" target="_blank" rel="noopener">Abrir linha do tempo →</a>',
                cta: 'Próximo',
            },
            {
                title: 'Crie seu universo',
                bodyHtml: 'Quando quiser organizar suas próprias coisas, faça login e clique em <strong>+ Novo universo</strong> na barra lateral.',
                cta: 'Concluir',
            },
        ];

        let current = 0; // 0-indexed step

        function render() {
            const s = steps[current];
            titleEl.textContent = s.title;
            bodyEl.innerHTML = s.bodyHtml;
            stepEl.textContent = `${current + 1} / 3`;
            nextBtn.textContent = s.cta;
        }

        function dismiss() {
            // Year-long cookie. Path=/ so it applies across the whole SPA.
            document.cookie = 'co_onboarded=1; Path=/; Max-Age=31536000; SameSite=Lax';
            banner.classList.add('hidden');
        }

        nextBtn.addEventListener('click', () => {
            if (current < steps.length - 1) {
                current += 1;
                render();
            } else {
                dismiss();
            }
        });
        skipBtn.addEventListener('click', dismiss);

        render();
        banner.classList.remove('hidden');
    }

    // ===== Criar Universo Modal =====

    function setupCriarModal() {
        const overlay = document.getElementById('criar-modal-overlay');
        if (!overlay) return;

        const closeBtn = document.getElementById('criar-modal-close');
        const cancelBtn = document.getElementById('criar-cancel');
        const form = document.getElementById('criar-form');
        const nameInput = document.getElementById('criar-name');
        const slugInput = document.getElementById('criar-slug');
        const descInput = document.getElementById('criar-description');
        const copyToggle = document.getElementById('criar-copy-from-toggle');
        const copySource = document.getElementById('criar-copy-from-source');
        const errorEl = document.getElementById('criar-error');

        // CO-96 P1: populate the copy-from source select from the user's universes.
        // Always offers `template` as a stable fallback. Re-populated on every open
        // so newly-added universes appear without a page reload.
        function populateCopyFromSources() {
            if (!copySource) return;
            const universes = (state.userUniverses || [])
                .filter(u => u.key && u.key !== 'template');
            copySource.innerHTML = '<option value="template">Template (CO)</option>'
                + universes.map(u =>
                    `<option value="${u.key}">${esc(u.name || u.key)}</option>`
                ).join('');
        }

        // CO-96 P1: open() accepts an optional `defaults` object to prefill the
        // form. The banner CTA passes { copyFromTemplate: true } so the legacy
        // "always clone template" flow still works; the sidebar CTA passes
        // nothing for a fresh empty form (visibility=private, no copy-from).
        function open(defaults) {
            const d = defaults || {};
            populateCopyFromSources();
            overlay.classList.remove('hidden');
            nameInput.value = '';
            slugInput.value = '';
            if (descInput) descInput.value = '';
            // Reset visibility radios to private (default for new universes).
            const privacyRadio = form.querySelector('input[name="criar-visibility"][value="private"]');
            if (privacyRadio) privacyRadio.checked = true;
            // Copy-from toggle handling.
            if (copyToggle) {
                copyToggle.checked = !!d.copyFromTemplate;
                if (copySource) {
                    copySource.classList.toggle('hidden', !copyToggle.checked);
                    if (d.copyFromTemplate) copySource.value = 'template';
                }
            }
            if (errorEl) errorEl.classList.add('hidden');
            nameInput.focus();
        }

        function close() {
            overlay.classList.add('hidden');
        }

        // Toggle visibility of the source-select when the checkbox flips.
        if (copyToggle && copySource) {
            copyToggle.addEventListener('change', () => {
                copySource.classList.toggle('hidden', !copyToggle.checked);
            });
        }

        const slugPreview = document.getElementById('criar-slug-preview');
        const slugValEl = document.getElementById('criar-slug-val');

        function updateSlugPreview() {
            const slug = slugInput.value.trim();
            if (slugValEl) slugValEl.textContent = slug || '…';
            if (slugPreview) {
                if (slug) {
                    slugPreview.setAttribute('data-active', '');
                } else {
                    slugPreview.removeAttribute('data-active');
                }
            }
        }

        // Auto-generate slug from name
        nameInput.addEventListener('input', () => {
            const slug = nameInput.value
                .toLowerCase()
                .replace(/[^a-z0-9]+/g, '-')
                .replace(/^-+|-+$/g, '')
                .slice(0, 40);
            slugInput.value = slug;
            updateSlugPreview();
        });

        // Manual slug edit also updates preview
        slugInput.addEventListener('input', updateSlugPreview);

        closeBtn && closeBtn.addEventListener('click', close);
        cancelBtn && cancelBtn.addEventListener('click', close);
        overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

        form.addEventListener('submit', async e => {
            e.preventDefault();
            const name = nameInput.value.trim();
            const key = slugInput.value.trim();
            if (!name || !key) return;
            const description = descInput ? descInput.value.trim() : '';
            const visibilityEl = form.querySelector('input[name="criar-visibility"]:checked');
            const visibility = visibilityEl ? visibilityEl.value : 'private';
            const copyFrom = (copyToggle && copyToggle.checked && copySource)
                ? copySource.value
                : null;

            const submitBtn = document.getElementById('criar-submit');
            if (submitBtn) submitBtn.disabled = true;
            if (errorEl) errorEl.classList.add('hidden');

            // CO-96 P1: branch between empty-create and copy-from paths.
            //  - Copy-from → POST /api/v1/universes/<source>/duplicate (CO-95).
            //  - Empty     → POST /api/v1/universes (existing endpoint).
            // Either way: visibility is set client-side; if the empty-create
            // endpoint doesn't honor the field today, follow up with a PUT.
            let result;
            if (copyFrom) {
                result = await api.cloneUniverse(copyFrom, { name, key, description });
            } else {
                result = await apiFetch('/api/v1/universes', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ key, name, description }),
                }, true);
                // Visibility cannot be set on POST today; follow up with PUT.
                // No-op for the default `private` since that's the server default.
                if (result && visibility !== 'private') {
                    await apiFetch(`/api/v1/universes/${encodeURIComponent(key)}`, {
                        method: 'PUT',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ visibility }),
                    }, true);
                }
            }
            if (submitBtn) submitBtn.disabled = false;

            if (!result) return; // apiFetch already showed error toast

            close();
            showToast('Universo criado! Redirecionando...', 'success');
            setUniverseSlugInUrl(result.key);
            state.currentUniverseSlug = result.key;
            state.isTemplate = false;
            // Seed universe info from response (content_count present on
            // clone path; defaults to 0 on empty-create).
            state.universeInfo = {
                key: result.key,
                name: result.name,
                description: result.description,
                content_count: result.content_count || 0,
                is_anonymous: result.owner_id ? result.owner_id.startsWith('anon-') : true,
                is_template: false,
            };
            renderUsageCount();
            hideTemplateBanner();
            await bootAppForUniverse(result.key);
        });

        // CO-96 P1: sidebar `+ Novo universo` button opens the modal with no
        // defaults — fresh empty form, visibility=private, copy-from off.
        const btnSidebarNew = document.getElementById('btn-sidebar-new-universe');
        if (btnSidebarNew) btnSidebarNew.addEventListener('click', () => open());

        // Wire the CTA button in the banner — anonymous-visitor flow always
        // wants a clone of `template`, so prefill copy-from accordingly.
        const btnCriar = document.getElementById('btn-criar-universo');
        if (btnCriar) btnCriar.addEventListener('click', () => open({ copyFromTemplate: true }));

        // Wire "Entrar" in the banner to the login modal
        const btnEntrar = document.getElementById('btn-banner-entrar');
        if (btnEntrar) btnEntrar.addEventListener('click', showLoginModal);

        // Wire language toggle in banner
        const btnBannerLang = document.getElementById('btn-banner-lang');
        if (btnBannerLang) {
            btnBannerLang.addEventListener('click', () => {
                window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
                render();
            });
        }
    }

    // ===== CO-38: Yggdrasil Minigames Hub =====

    const YGGDRASIL_GAMES = [
        { id: 'tetris',    name: 'Tetris',     icon: 'view_agenda',  desc: 'Peças clássicas' },
        { id: 'snake',     name: 'Snake',      icon: 'gesture',       desc: 'A cobra faminta' },
        { id: 'invaders',  name: 'Invaders',   icon: 'rocket_launch', desc: 'Defenda a Terra' },
        { id: 'pointset',  name: 'PointSet',   icon: 'grid_on',       desc: 'Encontre os pares' },
        { id: 'poker',     name: 'Poker',      icon: 'casino',        desc: 'Video poker' },
    ];

    const GAME_MODULES = {
        tetris:   '/games/tetris.js',
        snake:    '/games/snake.js',
        invaders: '/games/invaders.js',
        pointset: '/games/pointset.js',
        poker:    '/games/poker.js',
    };

    const GAME_GLOBALS = {
        tetris:   'CoTetris',
        snake:    'CoSnake',
        invaders: 'CoInvaders',
        pointset: 'CoPointSet',
        poker:    'CoPoker',
    };

    async function bootYggdrasil() {
        state.isYggdrasil = true;
        document.documentElement.setAttribute('data-palette', 'relic');

        const me = await api.me();
        if (me) {
            hideLoginModal();
            renderUserBadge(me);
        }

        const gameId = state.gameView || readGameFromUrl();
        if (gameId) {
            state.gameView = gameId;
            if (me) {
                await renderYggdrasilGame(gameId, me);
            } else {
                renderYggdrasilLoginWall(gameId);
            }
        } else {
            if (me) {
                await renderYggdrasilHub(me);
            } else {
                renderYggdrasilLoginWall(null);
            }
        }
        hideLoading();
    }

    function renderYggdrasilLoginWall(gameId) {
        const content = document.getElementById('content');
        if (!content) return;
        // Hide standard header controls
        document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
        document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
        const title = gameId ? `Faça login para jogar ${gameId.charAt(0).toUpperCase() + gameId.slice(1)}` : 'Yggdrasil — Hub de Jogos';
        content.innerHTML = `
            <div class="ygg-login-wall">
                <div class="ygg-login-card">
                    <div class="ygg-login-icon material-symbols-outlined">sports_esports</div>
                    <h2 class="ygg-login-title">${esc(title)}</h2>
                    <p class="ygg-login-subtitle">Crie uma conta gratuita para acessar o hub de minijogos,<br>perfis de jogadores e rankings globais.</p>
                    <div class="ygg-login-actions">
                        <button class="btn btn-primary ygg-btn-login" id="ygg-btn-login">Entrar</button>
                        <button class="btn btn-ghost ygg-btn-register" id="ygg-btn-register">Criar conta</button>
                    </div>
                </div>
            </div>
            <style>
                .ygg-login-wall { display:flex; align-items:center; justify-content:center; min-height:60vh; padding:32px; }
                .ygg-login-card { text-align:center; padding:48px 40px; background:rgba(255,255,255,.04); border-radius:16px; border:1px solid rgba(255,255,255,.1); max-width:400px; width:100%; backdrop-filter:blur(8px); }
                .ygg-login-icon { font-size:56px; color:var(--accent, #e0505f); margin-bottom:16px; }
                .ygg-login-title { font-family:'Newsreader', serif; font-size:24px; margin:0 0 12px; }
                .ygg-login-subtitle { color:rgba(255,255,255,.6); font-size:14px; line-height:1.6; margin:0 0 28px; }
                .ygg-login-actions { display:flex; gap:12px; justify-content:center; flex-wrap:wrap; }
                .ygg-btn-login, .ygg-btn-register { min-width:120px; }
            </style>
        `;
        document.getElementById('ygg-btn-login') && document.getElementById('ygg-btn-login').addEventListener('click', () => {
            document.getElementById('btn-header-entrar') && document.getElementById('btn-header-entrar').click();
        });
        document.getElementById('ygg-btn-register') && document.getElementById('ygg-btn-register').addEventListener('click', () => {
            document.getElementById('btn-header-entrar') && document.getElementById('btn-header-entrar').click();
        });
    }

    async function renderYggdrasilHub(me) {
        const content = document.getElementById('content');
        if (!content) return;
        // Hide standard board controls
        document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
        document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
        const projectName = document.getElementById('project-name');
        if (projectName) projectName.textContent = 'Yggdrasil';

        // Fetch data in parallel
        const [globalBoard, recentAct, myProfile] = await Promise.all([
            apiFetch('/api/v1/games/leaderboard/global?limit=10', {}, true).catch(() => []),
            apiFetch('/api/v1/games/recent?limit=20', {}, true).catch(() => []),
            apiFetch('/api/v1/profile', {}, true).catch(() => null),
        ]);

        // Build per-game personal stats
        const gameStats = {};
        for (const g of YGGDRASIL_GAMES) {
            try {
                const s = await apiFetch(`/api/v1/games/${g.id}/stats`, {}, true);
                if (s) gameStats[g.id] = s;
            } catch (_) { /* not played yet */ }
        }

        content.innerHTML = `
            <div class="ygg-hub">
                <style>
                    .ygg-hub { padding:24px; display:grid; grid-template-columns:1fr 320px; gap:24px; max-width:1100px; margin:0 auto; }
                    @media(max-width:768px){ .ygg-hub { grid-template-columns:1fr; } }
                    .ygg-section { margin-bottom:24px; }
                    .ygg-section-title { font-family:'Newsreader', serif; font-size:18px; margin:0 0 14px; color:rgba(255,255,255,.8); border-bottom:1px solid rgba(255,255,255,.08); padding-bottom:8px; }
                    /* Profile card */
                    .ygg-profile { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:20px; backdrop-filter:blur(4px); }
                    .ygg-profile-head { display:flex; align-items:center; gap:16px; margin-bottom:16px; }
                    .ygg-avatar { width:52px; height:52px; border-radius:50%; background:var(--accent,#e0505f); display:flex; align-items:center; justify-content:center; font-size:22px; font-weight:700; color:#fff; font-family:'Newsreader',serif; }
                    .ygg-profile-name { font-size:18px; font-family:'Newsreader',serif; }
                    .ygg-profile-sub { font-size:12px; color:rgba(255,255,255,.5); margin-top:2px; }
                    .ygg-hp-bar { height:6px; background:rgba(255,255,255,.1); border-radius:3px; overflow:hidden; margin-top:8px; }
                    .ygg-hp-fill { height:100%; background:var(--accent,#e0505f); border-radius:3px; transition:width .4s; }
                    .ygg-profile-stats { display:grid; grid-template-columns:1fr 1fr 1fr; gap:12px; margin-top:16px; }
                    .ygg-pstat { text-align:center; font-size:11px; color:rgba(255,255,255,.5); }
                    .ygg-pstat strong { display:block; font-size:18px; font-family:'Newsreader',serif; color:#fff; }
                    /* Game grid */
                    .ygg-games { display:grid; grid-template-columns:repeat(auto-fill,minmax(160px,1fr)); gap:14px; }
                    .ygg-game-card { background:rgba(255,255,255,.04); border:1px solid rgba(255,255,255,.08); border-radius:12px; padding:18px 14px; cursor:pointer; transition:transform .15s,border-color .15s,box-shadow .15s; text-align:center; }
                    .ygg-game-card:hover { transform:scale(1.03); border-color:var(--accent,#e0505f); box-shadow:0 4px 20px rgba(224,80,95,.2); }
                    .ygg-game-icon { font-size:32px; color:var(--accent,#e0505f); margin-bottom:8px; }
                    .ygg-game-name { font-family:'Newsreader',serif; font-size:16px; margin-bottom:4px; }
                    .ygg-game-desc { font-size:11px; color:rgba(255,255,255,.4); margin-bottom:10px; }
                    .ygg-game-best { font-size:12px; color:rgba(255,255,255,.6); margin-bottom:10px; }
                    .ygg-play-btn { display:inline-block; padding:6px 18px; background:var(--accent,#e0505f); color:#fff; border-radius:6px; font-size:12px; font-weight:600; letter-spacing:.5px; pointer-events:none; }
                    /* Right column */
                    .ygg-right { display:flex; flex-direction:column; gap:20px; }
                    /* Leaderboard */
                    .ygg-lb { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; backdrop-filter:blur(4px); }
                    .ygg-lb-row { display:flex; align-items:center; gap:10px; padding:6px 0; border-bottom:1px solid rgba(255,255,255,.05); font-size:13px; }
                    .ygg-lb-row:last-child { border-bottom:none; }
                    .ygg-lb-rank { width:20px; color:rgba(255,255,255,.4); font-size:11px; text-align:right; }
                    .ygg-lb-rank.gold { color:#E9C349; font-weight:700; }
                    .ygg-lb-name { flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
                    .ygg-lb-score { color:var(--accent2,#E9C349); font-family:'Newsreader',serif; }
                    /* Activity */
                    .ygg-feed { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; backdrop-filter:blur(4px); }
                    .ygg-feed-item { font-size:12px; color:rgba(255,255,255,.6); padding:5px 0; border-bottom:1px solid rgba(255,255,255,.04); }
                    .ygg-feed-item:last-child { border-bottom:none; }
                    .ygg-feed-item strong { color:#fff; }
                    .ygg-empty { color:rgba(255,255,255,.3); font-size:12px; font-style:italic; }
                </style>
                <div class="ygg-left">
                    <!-- Profile -->
                    <div class="ygg-section">
                        ${renderProfileCard(me, myProfile, gameStats)}
                    </div>
                    <!-- Game grid -->
                    <div class="ygg-section">
                        <div class="ygg-section-title">Jogos</div>
                        <div class="ygg-games">
                            ${YGGDRASIL_GAMES.map(g => {
                                const st = gameStats[g.id];
                                const best = st ? st.high_score : 0;
                                return `<div class="ygg-game-card" data-game="${esc(g.id)}">
                                    <div class="ygg-game-icon material-symbols-outlined">${esc(g.icon)}</div>
                                    <div class="ygg-game-name">${esc(g.name)}</div>
                                    <div class="ygg-game-desc">${esc(g.desc)}</div>
                                    <div class="ygg-game-best">Melhor: ${best.toLocaleString()}</div>
                                    <div class="ygg-play-btn">JOGAR</div>
                                </div>`;
                            }).join('')}
                        </div>
                    </div>
                </div>
                <div class="ygg-right">
                    <!-- Global Leaderboard -->
                    <div class="ygg-lb">
                        <div class="ygg-section-title" style="margin-bottom:10px">Ranking Global</div>
                        ${globalBoard && globalBoard.length > 0
                            ? globalBoard.map(e => `
                                <div class="ygg-lb-row">
                                    <span class="ygg-lb-rank${e.rank <= 3 ? ' gold' : ''}">${e.rank}</span>
                                    <span class="ygg-lb-name">${esc(e.display_name || e.username || 'Jogador')}</span>
                                    <span class="ygg-lb-score">${(e.total_score || 0).toLocaleString()}</span>
                                </div>`).join('')
                            : '<div class="ygg-empty">Nenhum jogador ainda.</div>'}
                    </div>
                    <!-- Recent Activity -->
                    <div class="ygg-feed">
                        <div class="ygg-section-title" style="margin-bottom:10px">Atividade Recente</div>
                        ${recentAct && recentAct.length > 0
                            ? recentAct.slice(0, 10).map(e => {
                                const ago = formatTimeAgo(e.played_at);
                                return `<div class="ygg-feed-item">
                                    <strong>${esc(e.display_name || e.username || 'Jogador')}</strong>
                                    marcou ${(e.score || 0).toLocaleString()} em ${esc(capitalize(e.game_name))}
                                    <span style="color:rgba(255,255,255,.3)"> · ${ago}</span>
                                </div>`;
                            }).join('')
                            : '<div class="ygg-empty">Nenhuma atividade ainda.</div>'}
                    </div>
                </div>
            </div>
        `;

        // Wire game card clicks
        content.querySelectorAll('.ygg-game-card').forEach(card => {
            card.addEventListener('click', () => {
                const gameId = card.dataset.game;
                navigateToGame(gameId);
            });
        });
    }

    function renderProfileCard(me, profile, gameStats) {
        const name = (me && me.display_name) || (me && me.email ? me.email.split('@')[0] : 'Aventureiro');
        const initial = name.charAt(0).toUpperCase();
        const totalScore = Object.values(gameStats).reduce((sum, s) => sum + (s.high_score || 0), 0);
        const gamesPlayed = Object.values(gameStats).reduce((sum, s) => sum + (s.games_played || 0), 0);
        const memberSince = (profile && profile.created_at)
            ? new Date(profile.created_at * 1000).getFullYear()
            : new Date().getFullYear();
        const level = Math.max(1, Math.floor(gamesPlayed / 5) + 1);
        const levelPct = Math.min(100, ((gamesPlayed % 5) / 5) * 100);

        return `<div class="ygg-profile">
            <div class="ygg-profile-head">
                <div class="ygg-avatar">${esc(initial)}</div>
                <div>
                    <div class="ygg-profile-name">${esc(name)}</div>
                    <div class="ygg-profile-sub">Nível ${level} · Aventureiro</div>
                    <div class="ygg-hp-bar"><div class="ygg-hp-fill" style="width:${levelPct}%"></div></div>
                </div>
            </div>
            <div class="ygg-profile-stats">
                <div class="ygg-pstat"><strong>${totalScore.toLocaleString()}</strong>Pontuação Total</div>
                <div class="ygg-pstat"><strong>${gamesPlayed}</strong>Partidas</div>
                <div class="ygg-pstat"><strong>${memberSince}</strong>Membro desde</div>
            </div>
        </div>`;
    }

    async function renderYggdrasilGame(gameId, me) {
        const content = document.getElementById('content');
        if (!content) return;
        document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
        document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
        const projectName = document.getElementById('project-name');
        if (projectName) projectName.textContent = 'Yggdrasil';

        const info = YGGDRASIL_GAMES.find(g => g.id === gameId);
        if (!info) { renderYggdrasilHub(me); return; }

        content.innerHTML = `
            <div class="ygg-game-view">
                <div class="ygg-game-header">
                    <button class="ygg-back-btn" id="ygg-back-btn">
                        <span class="material-symbols-outlined">arrow_back</span>
                        <span>Yggdrasil</span>
                    </button>
                    <h2 class="ygg-game-title">${esc(info.name)}</h2>
                </div>
                <div class="ygg-game-area">
                    <div id="ygg-game-canvas-area" class="ygg-game-canvas-area"></div>
                    <div class="ygg-game-side">
                        <div id="ygg-game-lb" class="ygg-side-lb">
                            <div class="ygg-section-title">Top 10 — ${esc(info.name)}</div>
                            <div id="ygg-lb-list">Carregando...</div>
                        </div>
                    </div>
                </div>
            </div>
            <style>
                .ygg-game-view { padding:16px 24px; }
                .ygg-game-header { display:flex; align-items:center; gap:16px; margin-bottom:20px; }
                .ygg-back-btn { display:flex; align-items:center; gap:6px; background:rgba(255,255,255,.08); border:1px solid rgba(255,255,255,.12); border-radius:8px; padding:6px 14px; cursor:pointer; color:inherit; font-size:14px; }
                .ygg-back-btn:hover { background:rgba(255,255,255,.12); }
                .ygg-game-title { font-family:'Newsreader',serif; font-size:22px; margin:0; }
                .ygg-game-area { display:flex; gap:24px; align-items:flex-start; flex-wrap:wrap; }
                .ygg-game-canvas-area { flex:1; min-width:280px; }
                .ygg-game-side { width:220px; flex-shrink:0; }
                .ygg-side-lb { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; }
                @media(max-width:600px){ .ygg-game-side { width:100%; } }
            </style>
        `;

        document.getElementById('ygg-back-btn').addEventListener('click', () => {
            state.gameView = null;
            window.history.pushState({}, '', '/yggdrasil');
            renderYggdrasilHub(me);
        });

        // Load game leaderboard
        apiFetch(`/api/v1/games/${gameId}/leaderboard?limit=10`, {}, true).then(lb => {
            const el = document.getElementById('ygg-lb-list');
            if (!el) return;
            if (!lb || lb.length === 0) { el.innerHTML = '<div class="ygg-empty">Sem dados ainda.</div>'; return; }
            el.innerHTML = lb.map(e => `
                <div class="ygg-lb-row">
                    <span class="ygg-lb-rank${e.rank <= 3 ? ' gold' : ''}">${e.rank}</span>
                    <span class="ygg-lb-name">${esc(e.display_name || e.username || 'Jogador')}</span>
                    <span class="ygg-lb-score">${(e.high_score || 0).toLocaleString()}</span>
                </div>`).join('');
        }).catch(() => {});

        // Load and init the game
        const canvasArea = document.getElementById('ygg-game-canvas-area');
        await loadGameScript(GAME_MODULES[gameId]);
        const gameGlobal = window[GAME_GLOBALS[gameId]];
        if (gameGlobal && gameGlobal.init) {
            gameGlobal.init(canvasArea);
        }
    }

    function loadGameScript(src) {
        return new Promise((resolve, reject) => {
            // If already loaded, resolve immediately
            if (document.querySelector(`script[src="${src}"]`)) { resolve(); return; }
            const s = document.createElement('script');
            s.src = src;
            s.onload = resolve;
            s.onerror = reject;
            document.head.appendChild(s);
        });
    }

    function navigateToGame(gameId) {
        state.gameView = gameId;
        window.history.pushState({}, '', `/yggdrasil/${gameId}`);
        api.me().then(me => {
            if (me) renderYggdrasilGame(gameId, me);
            else renderYggdrasilLoginWall(gameId);
        });
    }

    function formatTimeAgo(ts) {
        if (!ts) return '?';
        const diffSecs = Math.floor(Date.now() / 1000) - ts;
        if (diffSecs < 60) return `${diffSecs}s`;
        if (diffSecs < 3600) return `${Math.floor(diffSecs / 60)}m`;
        if (diffSecs < 86400) return `${Math.floor(diffSecs / 3600)}h`;
        return `${Math.floor(diffSecs / 86400)}d`;
    }

    function capitalize(s) {
        return s ? s.charAt(0).toUpperCase() + s.slice(1) : s;
    }

    // ===== App boot (post-auth) =====
    async function bootApp() {
        showLoading();
        state.projects = await api.getProjects();
        if (state.projects.length > 0) {
            await selectProject(state.projects[0].key);
        }
        hideLoading();
        render();
    }

    // Universe switching: a single atomic transition rather than a chain of
    // partial updates. The previous version cleared/replaced state piece by
    // piece while fetching, so old cards, old theme, and old gear icon would
    // briefly coexist with the new universe's URL and sidebar state.
    //
    // New flow:
    //   1. Mark `state.switchingUniverse = true` so render() / view callbacks
    //      can short-circuit and avoid painting stale data.
    //   2. Reset transient per-universe state (tasks, projects, info, config,
    //      universeInfo) so subsequent reads don't surface leftovers.
    //   3. Show the loading state (clears the content area).
    //   4. Apply the new theme/config FIRST — this is cheap and prevents the
    //      brief flash of the prior universe's palette under the spinner.
    //   5. Fetch projects + first project's data in parallel where possible.
    //   6. Drop the switching flag, render once.
    // Wraps a promise with a hard timeout so a hanging fetch cannot leave the
    // spinner up forever. Resolves to `null` instead of rejecting, so callers
    // can treat timeout as a soft failure.
    function withTimeout(promise, ms, label) {
        return new Promise((resolve) => {
            let done = false;
            const t = setTimeout(() => {
                if (done) return;
                done = true;
                console.warn(`withTimeout(${label}): timed out after ${ms}ms`);
                resolve(null);
            }, ms);
            promise.then(
                (val) => { if (done) return; done = true; clearTimeout(t); resolve(val); },
                (err) => { if (done) return; done = true; clearTimeout(t); console.warn(`withTimeout(${label}):`, err); resolve(null); }
            );
        });
    }

    // Open a page entry from the ?page=<slug> URL param (e.g. /co?page=privacidade).
    // Maps to content/<slug>.md in the given universe.
    function maybeOpenPageFromUrl(universeSlug) {
        const params = new URLSearchParams(window.location.search);
        const page = params.get('page');
        if (!page) return;
        // Reading the disclosure pages counts as informed consent.
        if (page === 'dados-rastreados' || page === 'privacidade') {
            try { localStorage.setItem('co_cookie_consent', '1'); } catch (_) {}
        }
        const entryPath = `content/${page}.md`;
        // ?page= links always point at template universe content (legal pages,
        // sobre, dados-rastreados). Fetch from template regardless of which
        // universe the logged-in user landed on.
        const fetchSlug = 'template';
        openZoomModal({ path: entryPath, body: undefined, _universeSlug: fetchSlug }, false);
        const clean = new URL(window.location.href);
        clean.searchParams.delete('page');
        window.history.replaceState({}, '', clean.toString());
    }

    async function bootAppForUniverse(slug) {
        state.switchingUniverse = true;
        // Wipe per-universe collections so old cards never leak through.
        state.tasks = [];
        state.projects = [];
        state.currentProject = null;
        state.universeInfo = null;
        state.universeConfig = null;
        state.universeManifest = null;
        state.calendarEntries = [];
        removeManifestViewTabs();

        showLoading();

        // Outer watchdog: if the whole boot doesn't finish in 20s, force a
        // recovery so the spinner cannot stick. The user can refresh / pick
        // another universe / hit /reset-sw.html if something is genuinely
        // wrong server-side.
        let watchdogFired = false;
        const watchdogTimer = setTimeout(() => {
            watchdogFired = true;
            state.switchingUniverse = false;
            const content = $('#content');
            if (content) {
                content.innerHTML = `
                    <div style="padding:24px;max-width:520px;margin:40px auto;background:var(--card-bg,#fff);border:1px solid var(--border,#e5e7eb);border-radius:8px">
                        <h2 style="margin:0 0 12px;font-size:18px">Carregamento demorou demais</h2>
                        <p style="margin:0 0 12px;color:var(--text-muted,#6b7280)">
                            Não conseguimos carregar <strong>${esc(slug)}</strong> em 20s. Pode ser uma falha de rede,
                            um universo grande, ou um service worker antigo.
                        </p>
                        <p style="margin:0;display:flex;gap:8px;flex-wrap:wrap">
                            <button class="btn btn-primary btn-sm" onclick="window.location.reload()">Recarregar</button>
                            <a class="btn btn-secondary btn-sm" href="/" style="text-decoration:none">Voltar ao template</a>
                            <a class="btn btn-secondary btn-sm" href="/reset-sw.html" style="text-decoration:none">Reset cache</a>
                        </p>
                    </div>`;
            }
        }, 20000);

        try {
            let info = null;
            let config = null;
            state.universeManifest = null;
            try {
                const [i, c, m] = await Promise.all([
                    withTimeout(api.getUniverseInfo(slug), 8000, 'getUniverseInfo'),
                    withTimeout(api.getUniverseConfig(slug), 8000, 'getUniverseConfig'),
                    withTimeout(api.getUniverseManifest(slug), 8000, 'getUniverseManifest'),
                ]);
                info = i; config = c;
                if (m) {
                    state.universeManifest = m;
                    injectManifestViewTabs(m);
                }
            } catch (e) {
                console.warn('bootAppForUniverse: info/config fetch failed', e);
            }

            if (watchdogFired) return;

            if (info) {
                state.universeInfo = info;
                renderUsageCount();
            }
            if (config) {
                applyUniverseConfig(config);
            } else {
                loadThemeCss(slug);
            }
            renderSettingsGear(info);

            const projects = await withTimeout(api.getUniverseProjects(slug), 8000, 'getUniverseProjects');
            if (watchdogFired) return;
            state.projects = Array.isArray(projects) ? projects : [];

            if (state.projects.length > 0) {
                try {
                    // selectProject internally calls refreshTasks which fetches
                    // /api/projects/:key/tasks — bound the same way.
                    await withTimeout(selectProject(state.projects[0].key), 8000, 'selectProject');
                } catch (e) {
                    console.warn('bootAppForUniverse: selectProject threw', e);
                }
            }
        } finally {
            clearTimeout(watchdogTimer);
            if (!watchdogFired) {
                state.switchingUniverse = false;
                hideLoading();
                try { render(); } catch (e) { console.warn('bootAppForUniverse: render threw', e); }
            }
        }
    }

    // ===== Settings Panel (CO-24) =====

    // Show gear icon in header if caller is the universe owner (non-template only).
    function renderSettingsGear(universeInfo) {
        // Remove any existing gear button to avoid duplicates.
        const existing = document.getElementById('btn-settings-gear');
        if (existing) existing.remove();

        // Only show for non-template universes.
        if (!universeInfo || universeInfo.is_template) return;

        const headerRight = document.querySelector('.header-right');
        if (!headerRight) return;

        const gearBtn = document.createElement('button');
        gearBtn.id = 'btn-settings-gear';
        gearBtn.className = 'btn-icon';
        gearBtn.title = 'Configurações do universo';
        gearBtn.setAttribute('aria-label', 'Configurações');
        gearBtn.innerHTML = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/>
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>`;

        // Insert before the new task button.
        const btnNewTask = document.getElementById('btn-new-task');
        if (btnNewTask) {
            headerRight.insertBefore(gearBtn, btnNewTask);
        } else {
            headerRight.appendChild(gearBtn);
        }

        gearBtn.addEventListener('click', openSettingsPanel);
    }

    function openSettingsPanel() {
        const overlay = document.getElementById('settings-modal-overlay');
        if (!overlay) return;

        const config = state.universeConfig || {};

        // Pre-fill form fields.
        const themeSelect = document.getElementById('settings-theme');
        const layoutSelect = document.getElementById('settings-layout');
        const fontHeadlineInput = document.getElementById('settings-font-headline');
        const fontBodyInput = document.getElementById('settings-font-body');
        const customTokensInput = document.getElementById('settings-custom-tokens');

        // Normalise scholarly-light → scholarly for the new select options.
        const preset = config.theme_preset === 'scholarly-light' ? 'scholarly' : (config.theme_preset || 'scholarly');
        if (themeSelect) themeSelect.value = preset;
        if (layoutSelect) layoutSelect.value = config.layout || 'board';
        if (fontHeadlineInput) fontHeadlineInput.value = config.font_headline || '';
        if (fontBodyInput) fontBodyInput.value = config.font_body || '';
        // CO-30: pre-fill custom tokens JSON textarea.
        if (customTokensInput) {
            customTokensInput.value = config.custom_tokens
                ? JSON.stringify(config.custom_tokens, null, 2)
                : '';
        }

        // CO-30: update dark-toggle icon to reflect current theme.
        updateDarkToggleIcon(preset);

        overlay.classList.remove('hidden');
    }

    // Update the dark/light toggle button icon to show current mode.
    function updateDarkToggleIcon(preset) {
        const btn = document.getElementById('settings-dark-toggle');
        if (!btn) return;
        const isDark = DARK_THEMES.has(preset);
        // Sun icon when dark (clicking switches to light), moon icon when light.
        if (isDark) {
            btn.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>`;
            btn.title = 'Modo claro';
        } else {
            btn.innerHTML = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>`;
            btn.title = 'Modo escuro';
        }
    }

    function setupSettingsPanel() {
        const overlay = document.getElementById('settings-modal-overlay');
        if (!overlay) return;

        const closeBtn = document.getElementById('settings-modal-close');
        const cancelBtn = document.getElementById('settings-cancel');
        const form = document.getElementById('settings-form');

        function close() { overlay.classList.add('hidden'); }

        closeBtn && closeBtn.addEventListener('click', close);
        cancelBtn && cancelBtn.addEventListener('click', close);
        overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

        // CO-30: dark/light toggle — swap to companion preset.
        const darkToggleBtn = document.getElementById('settings-dark-toggle');
        if (darkToggleBtn) {
            darkToggleBtn.addEventListener('click', () => {
                const themeSelect = document.getElementById('settings-theme');
                if (!themeSelect) return;
                const current = themeSelect.value;
                const companion = THEME_COMPANION[current] || 'scholarly';
                themeSelect.value = companion;
                updateDarkToggleIcon(companion);
            });
        }

        // Update toggle icon when user manually changes the select.
        const themeSelectEl = document.getElementById('settings-theme');
        if (themeSelectEl) {
            themeSelectEl.addEventListener('change', () => {
                updateDarkToggleIcon(themeSelectEl.value);
            });
        }

        form && form.addEventListener('submit', async e => {
            e.preventDefault();
            const slug = state.currentUniverseSlug;

            const themeSelect = document.getElementById('settings-theme');
            const layoutSelect = document.getElementById('settings-layout');
            const fontHeadlineInput = document.getElementById('settings-font-headline');
            const fontBodyInput = document.getElementById('settings-font-body');
            const customTokensInput = document.getElementById('settings-custom-tokens');

            // CO-30: parse custom tokens JSON (null = clear).
            let customTokens = undefined;
            if (customTokensInput) {
                const raw = customTokensInput.value.trim();
                if (raw) {
                    try {
                        customTokens = JSON.parse(raw);
                    } catch {
                        showToast('JSON inválido nos tokens CSS', 'error');
                        return;
                    }
                } else {
                    customTokens = null; // explicit null clears existing overrides
                }
            }

            const update = {
                theme_preset: themeSelect ? themeSelect.value : undefined,
                layout: layoutSelect ? layoutSelect.value : undefined,
                font_headline: fontHeadlineInput ? fontHeadlineInput.value : undefined,
                font_body: fontBodyInput ? fontBodyInput.value : undefined,
                custom_tokens: customTokens,
            };

            const saveBtn = document.getElementById('settings-save');
            if (saveBtn) saveBtn.disabled = true;

            const result = await api.updateUniverseConfig(slug, update);
            if (saveBtn) saveBtn.disabled = false;

            if (result) {
                // CO-30: hot-swap theme.css link by reloading with a cache-bust.
                const themeLink = document.getElementById('co-theme-css');
                if (themeLink && slug) {
                    themeLink.href = `/api/v1/universes/${slug}/theme.css?_=${Date.now()}`;
                }
                applyUniverseConfig(result);
                showToast('Configurações salvas', 'success');
                close();
            }
        });
    }

    // ===== Header user area =====

    function renderHeaderUserArea(me) {
        const area = document.getElementById('header-user-area');
        if (!area) return;
        if (me) {
            const name = me.display_name || me.usuario || me.email || '';
            area.innerHTML = `
                <span class="header-user-badge" title="${esc(name)}">${esc(name)}</span>
                <button class="btn btn-ghost btn-signout" id="btn-signout" title="${window.t ? window.t('sign_out') : 'Sair'}">
                    <span class="material-symbols-outlined" style="font-size:18px">logout</span>
                </button>`;
            document.getElementById('btn-signout').addEventListener('click', async () => {
                await api.logout();
                // Clear state
                state.userUniverses = [];
                state.currentUniverseSlug = 'template';
                state.isTemplate = true;
                localStorage.removeItem('co_local_universe');
                // Reload to template
                window.location.href = '/';
            });
        } else {
            area.innerHTML = `<button class="btn btn-ghost header-entrar" id="btn-header-entrar" data-i18n="action.login">${window.t ? window.t('action.login') : 'Entrar'}</button>`;
            const btn = document.getElementById('btn-header-entrar');
            if (btn) btn.addEventListener('click', showLoginModal);
        }
    }

    // ===== Footer version =====

    async function initFooter() {
        const versionEl = document.getElementById('footer-version');
        if (!versionEl) return;
        try {
            const r = await fetch('/api/health');
            if (r.ok) {
                const data = await r.json();
                const tagline = window.t ? window.t('footer.tagline') : 'CO — código aberto';
                versionEl.textContent = `CO v${data.version} — ${tagline.replace('CO — ', '')}`;
            }
        } catch {}
    }

    // ===== Init =====
    async function init() {
        // Apply initial language (from cookie, set by i18n.js)
        window.setLang(window.currentLang);

        // Wire header language toggle
        const btnHeaderLang = document.getElementById('btn-header-lang');
        if (btnHeaderLang) {
            btnHeaderLang.addEventListener('click', () => {
                window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
                render();
            });
        }

        // Wire initial header "Entrar" button (before auth check)
        renderHeaderUserArea(null);

        initTimelineStart();
        setupHamburgerMenu();
        setupLoginModal();
        setupCriarModal();
        setupUsageLimitModal();
        setupSettingsPanel();
        initFooter();

        const slug = readUniverseSlugFromUrl();
        state.currentUniverseSlug = slug;
        state.isTemplate = slug === 'template';
        state.isYggdrasil = slug === 'yggdrasil';

        // CO-38: Yggdrasil universe of universes — minigames hub
        if (state.isYggdrasil) {
            state.gameView = readGameFromUrl();
            showLoading();
            await bootYggdrasil();
            return;
        }

        if (state.isTemplate) {
            // Anonymous: show template board directly (static, read-only).
            // No cloning. Logged-in users get redirected to their community.
            const me = await api.me();
            if (me) {
                hideLoginModal();
                renderUserBadge(me);
                // Logged-in → go to their community
                const owned = await api.listUniverses();
                const mine = (owned || []).filter(u => !u.is_template);
                state.userUniverses = mine;
                if (mine.length > 0) {
                    // Prefer last-used universe; fall back to first in list.
                    const preferred = (() => { try { return localStorage.getItem('co_preferred_universe'); } catch (_) { return null; } })();
                    const target = (preferred && mine.find(u => u.key === preferred)) ? preferred : mine[0].key;
                    state.currentUniverseSlug = target;
                    state.isTemplate = false;
                    hideTemplateBanner();
                    await bootAppForUniverse(target);
                    return;
                }
            }
            // Anonymous or logged-in with no community → show template
            showTemplateBanner();
            // CO-99: onboarding coach mark for first-time anonymous visitors.
            // Internally gated on cookie + viewport + state.isTemplate.
            setupOnboarding();
            await bootAppForUniverse('template');
            maybeOpenPageFromUrl('template');
            return;
        }

        // Non-template universe: try auth silently.
        const me = await api.me();
        if (me) {
            hideLoginModal();
            renderUserBadge(me);
            await bootAppForUniverse(slug);
            await maybeOpenEntryFromUrl(slug);
            return;
        }

        // Not authenticated via full account — check if this is an anonymous universe.
        // Anonymous users have a session cookie (anon JWT) set during clone.
        const info = await api.getUniverseInfo(slug);
        if (info && info.is_anonymous) {
            // Anonymous universe owner: boot without requiring login
            state.universeInfo = info;
            renderUsageCount();
            state.projects = await api.getUniverseProjects(slug);
            if (state.projects.length > 0) {
                await selectProject(state.projects[0].key);
            }
            render();
            return;
        }

        // Universe not accessible or doesn't exist — fall back to template
        state.currentUniverseSlug = 'template';
        state.isTemplate = true;
        setUniverseSlugInUrl('template');
        showTemplateBanner();
        // CO-99: onboarding for the fallback-to-template path too.
        setupOnboarding();
        await bootAppForUniverse('template');
    }

    init();
})();
