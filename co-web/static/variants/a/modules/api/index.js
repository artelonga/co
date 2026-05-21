// ===== api/ — public re-exports =====
// Assembles the `api` object from submodules and preserves every import that
// used `from './api.js'` before this split.
export { apiFetch, injectApiCallbacks } from './client.js';

import * as authFns from './auth.js';
import * as tasksFns from './tasks.js';
import * as universesFns from './universes.js';
import * as entriesFns from './entries.js';

export const api = {
    ...authFns,
    ...tasksFns,
    ...universesFns,
    ...entriesFns,
};
