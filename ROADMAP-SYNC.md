# Co — Local Sync Roadmap

> Edit locally on your computer, sync to the web. Like Google Drive for markdown.

## Architecture

```
┌──────────────────────┐     ┌──────────────────────┐
│   Local Machine       │     │   Co Server (Fly.io)  │
│                       │     │                       │
│  ~/Co/                │     │  /data/universes/     │
│  ├── yuri/            │◄──►│  ├── yuri/            │
│  │   ├── content/     │sync│  │   ├── content/     │
│  │   │   └── *.md     │    │  │   │   └── *.md     │
│  │   └── projects/    │    │  │   └── projects/    │
│  └── quilombo/        │    │  └── quilombo/        │
│       └── ...         │    │       └── ...         │
└──────────────────────┘     └──────────────────────┘
```

## Phase 1: CLI Sync (v1.2) — `co sync`

**Minimal viable sync via the existing `co` CLI.**

```bash
co sync pull yuri           # Download all entries from yuri universe
co sync push yuri           # Upload local changes
co sync watch yuri          # Watch for changes + auto-push
co sync status              # Show local vs remote diff
```

### Implementation
- Uses Vault REST API (CO-35) — already deployed
- Auth via API token (`co login` → stores token in `~/.co/token`)
- File format: standard `.md` with YAML frontmatter (Obsidian compatible)
- Conflict resolution: last-write-wins (by `modified` timestamp)
- `.co/sync.json` in each universe folder tracks file hashes + last sync time
- Selective sync: `co sync pull yuri --type page` (only pages)

### Deliverables
- [ ] `co login` — authenticate via email code, store API token
- [ ] `co sync pull <universe>` — download entries to `~/Co/<universe>/`
- [ ] `co sync push <universe>` — upload changed files
- [ ] `co sync watch <universe>` — fsnotify watcher + auto push
- [ ] `co sync status` — show diff (new/modified/deleted local vs remote)
- [ ] `.co/sync.json` — hash registry for change detection

---

## Phase 2: Desktop Tray App (v1.3)

**Always-running sync agent with system tray icon.**

```
┌─────────┐
│  Co ⟳   │  ← Tray icon (green = synced, yellow = syncing, red = error)
│─────────│
│ Synced ✓│
│ 3 files │
│ Open... │
│ Pause   │
│ Settings│
│ Quit    │
└─────────┘
```

### Implementation
- **Electron** (same as CO-35 desktop app plan) OR **Tauri** (lighter)
- Bundles `co sync watch` as a background process
- Tray icon shows sync status
- Native file notifications ("Página atualizada: Sobre")
- Settings: sync interval, which universes to sync, conflict strategy
- Auto-start on OS login (optional)

### Deliverables
- [ ] Tray app with sync status indicator
- [ ] Background file watcher (reuses `co sync watch`)
- [ ] Native notifications on sync events
- [ ] Settings UI: universes, interval, conflict strategy
- [ ] Installers: `.dmg` (macOS), `.AppImage` (Linux), `.exe` (Windows)

---

## Phase 3: Obsidian Deep Integration (v1.4)

**Enhance the existing CO-34 Obsidian plugin with real-time sync.**

The Obsidian plugin (co-obsidian/) already syncs entries. Enhance it:

- [ ] **Auto-sync**: watch for vault changes, push on save (debounced)
- [ ] **Pull on open**: when Obsidian opens, pull latest from server
- [ ] **Conflict UI**: show diff when remote changed since last sync
- [ ] **Status bar**: "Co: synced ✓ | 2 pending" indicator
- [ ] **Vault ↔ Universe mapping**: settings to map vault folders to universes
- [ ] **Multi-universe**: sync multiple universes as subfolders in one vault

---

## Phase 4: PWA Offline (v1.5)

**Edit in the browser, works offline, syncs when online.**

The service worker (sw.js) already caches static assets. Add:

- [ ] **IndexedDB cache**: store entries locally in the browser
- [ ] **Offline edit queue**: save changes to IndexedDB when offline
- [ ] **Background sync**: use Service Worker Background Sync API to push when online
- [ ] **Conflict banner**: "You have 3 offline changes — sync now?"
- [ ] **Install prompt**: "Add Co to Home Screen" for mobile PWA

---

## Phase 5: Mobile Sync (v2.0)

**Capacitor app with filesystem access.**

- [ ] Same sync engine as CLI (Rust compiled to mobile via Capacitor + Rust FFI)
- [ ] OR: pure JS sync client using Vault REST API
- [ ] Local storage: SQLite on device (mirrors server entries table)
- [ ] Background sync via WorkManager (Android) / BGTaskScheduler (iOS)
- [ ] Files accessible in system Files app (iOS) / file manager (Android)

---

## Sync Protocol (shared across all clients)

```
1. Client reads .co/sync.json → last known hashes + timestamps
2. GET /api/v1/universes/:slug/vault/notes → server file list with hashes
3. Compare:
   - Local newer (hash differs, local mtime > remote mtime) → PUSH
   - Remote newer (hash differs, remote mtime > local mtime) → PULL
   - Both changed (hash differs, both mtimes newer than last sync) → CONFLICT
4. Resolve conflicts: last-write-wins (default) or prompt user
5. Execute pushes (PUT) and pulls (GET)
6. Update .co/sync.json with new hashes + timestamp
```

### Conflict Strategies
- **Last-write-wins** (default): newer timestamp wins
- **Local-wins**: always keep local version
- **Remote-wins**: always keep remote version
- **Manual**: show diff, let user choose
- **Merge**: attempt auto-merge (for markdown, line-level merge)

---

## Timeline

| Phase | Scope | Depends on |
|-------|-------|-----------|
| 1. CLI sync | `co sync pull/push/watch` | Vault API (done) |
| 2. Desktop tray | Electron/Tauri background agent | Phase 1 |
| 3. Obsidian deep | Auto-sync + conflict UI in plugin | Phase 1 protocol |
| 4. PWA offline | IndexedDB + Background Sync | Service Worker (done) |
| 5. Mobile sync | Capacitor + native background sync | Phase 1 protocol |

**Phase 1 is the foundation** — all other phases reuse the same sync protocol and Vault REST API.
