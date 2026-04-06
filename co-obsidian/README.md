# CO Universe Sync — Obsidian Plugin

Sync a [CO](https://artelonga.com.br/co) universe ↔ your Obsidian vault.
Pull notes from CO, push changes back, or run bidirectional sync with
last-write-wins conflict resolution.

## Features

- **Pull** — download universe files from CO to your vault
- **Push** — upload vault changes to CO
- **Bidirectional** — pull remote first, push local, last-write-wins on conflict
- **Frontmatter mapping** — `labels` ↔ `tags`, `created_at` ↔ `created`, etc.
- **Wikilinks** — CO task references appear as `[[CO-21|Title]]` with graph support
- **Dataview** — `parent:: [[CO-20]]` inline field for hierarchy traversal
- **Auto-sync** — configurable interval (off / 5 min / 15 min / hourly)
- **On-save push** — debounced 5 s after each file save
- **Status bar** — "CO: synced ✓" / "CO: syncing…" / "CO: offline" / "CO: N conflicts"
- **Ribbon icon** — one-click sync

## Installation

### Community plugins (recommended)

1. Open Obsidian → **Settings → Community plugins → Browse**
2. Search for **CO Universe Sync**
3. Install and enable

### Manual

1. Download `main.js` and `manifest.json` from the latest [release](https://github.com/artelonga/co/releases)
2. Copy both files to `<vault>/.obsidian/plugins/co-universe-sync/`
3. Reload Obsidian and enable the plugin

## Setup

1. **Settings → CO Universe Sync**
2. Set **CO instance URL** (default: `https://artelonga.com.br`)
3. Set **Universe slug** (e.g. `my-notes`)
4. Paste an **API token** from CO → Settings → API Tokens  
   _or_ click **Login with CO** to authenticate via browser
5. Click **Test connection** — you should see "CO: connection OK ✓"
6. Choose sync **direction** and **interval**
7. Click the refresh icon in the ribbon to run your first sync

## Vault structure after sync

```
<vault>/
├── .obsidian/          ← Obsidian config (auto-generated)
├── .co/
│   └── sync.json       ← sync metadata (hashes, last sync timestamp)
├── projects/
│   └── <project-key>/
│       ├── _project.md
│       └── <task-id>.md
├── content/            ← free-form notes (jardim, relatos, …)
└── README.md           ← universe description
```

## Commands

| Command | Description |
|---------|-------------|
| CO: Sync now | Full bidirectional sync |
| CO: Pull from CO | One-way pull (CO → vault) |
| CO: Push to CO | One-way push (vault → CO) |
| CO: Open in CO | Open current file in CO web UI |
| CO: Create task | Quick-add task from current note |
| CO: Link to CO | Insert `[[CO-XX]]` reference picker |

## Frontmatter mapping

| CO field | Obsidian field |
|----------|---------------|
| `labels` | `tags` |
| `created_at` | `created` |
| `updated_at` | `modified` |
| `parent: 21` | `parent: "[[CO-21]]"` + `parent:: [[CO-21]]` inline |
| `id`, `title`, `type`, `status`, `priority` | preserved as-is |
| all other fields | preserved (round-trip safe) |

## Development

```bash
cd co-obsidian
npm install
npm run dev    # watch mode
npm test       # unit + integration tests
npm run build  # production bundle
```

## Publishing

Follow the [Obsidian community plugin submission guidelines](https://docs.obsidian.md/Plugins/Releasing/Submit+your+plugin).

## License

MIT — see [LICENSE](LICENSE)
