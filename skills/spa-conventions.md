# SPA Conventions — CO Platform

## File Locations

```
co-web/static/variants/a/
├── js/
│   ├── app.ts          # Main entry, state init
│   ├── api.ts          # HTTP fetch wrappers
│   ├── state.ts        # Client-side state
│   └── sidebar.ts      # Sidebar logic
├── css/
│   └── themes/         # Per-theme CSS variables
└── index.html
```

## TypeScript Patterns

CO migrated to TypeScript incrementally (CO-218). New files MUST be `.ts`.

```typescript
// API call pattern
async function fetchEntries(universeKey: string): Promise<Entry[]> {
    const resp = await fetch(`/api/v1/universes/${universeKey}/entries`);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return resp.json();
}
```

## State Pattern

State is global (non-framework) with explicit update functions:

```typescript
// state.ts
let _universeKey = '';
export const getUniverseKey = () => _universeKey;
export const setUniverseKey = (key: string) => { _universeKey = key; render(); };
```

## i18n

Strings use the `t()` helper (CO-26):

```typescript
import { t } from './i18n.js';
element.textContent = t('board.column.todo');  // → "A fazer" / "To do"
```

## Theme Variables

Themes inject CSS custom properties at runtime (CO-30):

```css
:root {
    --co-bg: var(--theme-bg, #fff);
    --co-text: var(--theme-text, #000);
}
```

Do not hardcode hex colors — always use `--co-*` variables.

## Cache Headers

Static JS/CSS assets use short cache (`max-age=60`), NOT immutable, because filenames are not hashed (CO-feedback). Never add `immutable` to cache headers.
