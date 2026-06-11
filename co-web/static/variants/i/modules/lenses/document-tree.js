// CO-393: Document view — tree building utilities (split from conteudo.js).
// These are pure functions with no side effects or DOM access.

// Build a hierarchical folder tree from a flat array of entries.
// Each node: { name, path, items: Entry[], children: Node[] }
export function buildFolderTree(entries) {
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

// Count total entries under a folder node (recursive).
export function countFolderEntries(node) {
    return node.items.length + node.children.reduce((s, c) => s + countFolderEntries(c), 0);
}

// Categorize entries by their type/content_type.
// Returns { tasks, pages, clips, assets, calendar, other }
export function categorizeEntries(entries) {
    const CLIP_TYPES = new Set(['clip', 'reference', 'bookmark', 'link', 'quote']);
    const ASSET_TYPES = /^asset\./;
    const TASK_TYPES = new Set(['task', 'issue', 'ticket', 'todo', 'story', 'epic', 'user-story']);
    const CALENDAR_TYPES = new Set(['event', 'meeting', 'appointment', 'schedule']);

    const tasks = [], pages = [], clips = [], assets = [], calendar = [], other = [];

    for (const e of entries) {
        const type = (e.entry_type || (e.frontmatter || {}).type || '').toLowerCase();
        if (TASK_TYPES.has(type)) { tasks.push(e); continue; }
        if (CLIP_TYPES.has(type)) { clips.push(e); continue; }
        if (ASSET_TYPES.test(type)) { assets.push(e); continue; }
        if (CALENDAR_TYPES.has(type)) { calendar.push(e); continue; }
        if (type === '' || type === 'page' || type === 'note' || type === 'doc') { pages.push(e); continue; }
        other.push(e);
    }

    return { tasks, pages, clips, assets, calendar, other };
}
