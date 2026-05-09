// ===== Constants and configuration =====
// i18n-dependent constants (STATUSES, PRIORITY_LABELS, STATUS_LABELS) are
// rebuilt via build* functions and re-exported as mutable let bindings so
// the lang-change event handler in app.js can update them.

// ===== Obsidian Tasks compatibility (CO-37) =====
export const OBSIDIAN_CHECKBOX = {
    todo:        ' ',
    in_progress: '/',
    in_review:   '~',
    done:        'x',
};

export const CHECKBOX_TO_STATUS = {
    ' ': 'todo',
    '/': 'in_progress',
    '~': 'in_review',
    'x': 'done',
    'X': 'done',
};

/** Convert a CO status string to an Obsidian Tasks checkbox character. */
export function statusToCheckbox(status) {
    return OBSIDIAN_CHECKBOX[status] ?? ' ';
}

/** Convert an Obsidian Tasks checkbox character to a CO status string. */
export function checkboxToStatus(c) {
    return CHECKBOX_TO_STATUS[c] ?? 'todo';
}

export function taskToObsidianLine(task) {
    const c = statusToCheckbox(task.status ?? 'todo');
    return `- [${c}] ${task.title ?? 'Untitled'}`;
}

export function parseObsidianCheckboxLine(line) {
    const m = line.match(/^- \[(.)\] /);
    if (!m) return null;
    return checkboxToStatus(m[1]);
}

export function extractStatusFromBody(body) {
    if (!body) return null;
    const firstLine = body.split('\n')[0];
    return parseObsidianCheckboxLine(firstLine);
}

// ===== i18n-rebuilt constants =====
export function buildStatuses() {
    return [
        { key: 'todo', label: window.t('status.todo'), color: '#94a3b8' },
        { key: 'in_progress', label: window.t('status.in_progress'), color: '#3b82f6' },
        { key: 'done', label: window.t('status.done'), color: '#22c55e' },
    ];
}

export function buildPriorityLabels() {
    return {
        low: window.t('priority.low'),
        medium: window.t('priority.medium'),
        high: window.t('priority.high'),
        critical: window.t('priority.critical'),
    };
}

export function buildStatusLabels() {
    return {
        todo: window.t('status.todo'),
        in_progress: window.t('status.in_progress'),
        in_review: window.t('status.in_review'),
        done: window.t('status.done'),
    };
}

export let STATUSES = buildStatuses();
export let PRIORITY_LABELS = buildPriorityLabels();
export const PRIORITY_ORDER = { critical: 0, high: 1, medium: 2, low: 3 };
export const STATUS_ORDER = { todo: 0, in_progress: 1, done: 2, in_review: 3 };
export let STATUS_LABELS = buildStatusLabels();

export function rebuildI18nConstants() {
    STATUSES = buildStatuses();
    PRIORITY_LABELS = buildPriorityLabels();
    STATUS_LABELS = buildStatusLabels();
}

export const ZOOM_DAYS = { week: 7, month: 30, quarter: 90 };
export const COL_WIDTHS = { week: 50, month: 40, quarter: 60 };

export const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
    'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
export const MONTH_NAMES_FULL = ['January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December'];
export const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
export const DAY_NAMES_MINI = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];

export const ACTIVITY_ACTION_LABELS = {
    task_created: 'created task',
    task_deleted: 'deleted task',
    field_changed: 'updated field in',
    comment_added: 'commented on',
    status_changed: 'changed status of',
    task_archived: 'archived task',
    task_unarchived: 'unarchived task',
};

// ===== Theme configuration =====
export const THEME_PALETTE_MAP = {
    'scholarly': 'scholarly',
    'scholarly-light': 'scholarly',
    'scholarly-dark': 'scholarly-dark',
    'relic': 'relic',
    'relic-light': 'relic-light',
    'modern': '',
    '': '',
};

export const THEME_COMPANION = {
    'scholarly': 'scholarly-dark',
    'scholarly-light': 'scholarly-dark',
    'scholarly-dark': 'scholarly',
    'relic': 'relic-light',
    'relic-light': 'relic',
    'modern': 'modern',
};

export const DARK_THEMES = new Set(['scholarly-dark', 'relic']);

// ===== Yggdrasil =====
export const YGGDRASIL_GAMES = [
    { id: 'tetris',    name: 'Tetris',     icon: 'view_agenda',  desc: 'Peças clássicas' },
    { id: 'snake',     name: 'Snake',      icon: 'gesture',       desc: 'A cobra faminta' },
    { id: 'invaders',  name: 'Invaders',   icon: 'rocket_launch', desc: 'Defenda a Terra' },
    { id: 'pointset',  name: 'PointSet',   icon: 'grid_on',       desc: 'Encontre os pares' },
    { id: 'poker',     name: 'Poker',      icon: 'casino',        desc: 'Video poker' },
];

export const GAME_MODULES = {
    tetris:   '/games/tetris.js',
    snake:    '/games/snake.js',
    invaders: '/games/invaders.js',
    pointset: '/games/pointset.js',
    poker:    '/games/poker.js',
};

export const GAME_GLOBALS = {
    tetris:   'CoTetris',
    snake:    'CoSnake',
    invaders: 'CoInvaders',
    pointset: 'CoPointSet',
    poker:    'CoPoker',
};
