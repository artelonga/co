# CO — Dev Testing Guide

Complete test plan for validating all MVP features before v1.0 release.

## 1. Start Dev Server

```bash
# Terminal 1: Build editor bundle (one-time)
cd co-web/editor && npm install && npm run build && cd ../..

# Terminal 2: Run server
JWT_SECRET=dev-test-secret cargo run -p co-web

# Server starts at http://localhost:3000
# Default variant: Modern (a)
# Template universe auto-seeded on first boot
```

## 2. Smoke Test (5 min)

```bash
# Health check
curl http://localhost:3000/api/health
# → {"status":"ok","version":"0.29.0"}

# Template universe exists
curl http://localhost:3000/api/v1/universes/template
# → {"key":"template","name":"CO","is_template":true,...}

# Template has projects
curl http://localhost:3000/api/v1/universes/template/projects
# → [{"key":"MP","name":"Meu Projeto",...}]
```

## 3. Feature Tests

### 3a. Landing Page + Universe Flow

1. Open http://localhost:3000 (or http://localhost:3000/?u=template)
2. Verify: hero banner visible, "Criar universo" button, template board read-only
3. Try editing a task on template → should fail (403 / read-only notice)
4. Click "Criar universo" → enter name + slug → submit
5. Verify: redirected to own universe, board is editable
6. Create a project, add tasks, drag between columns
7. Verify content count badge in header

### 3b. Usage Gate (100 entries)

```bash
# Create anonymous universe
SLUG=test-gate-$(date +%s)
curl -c cookies.txt -X POST http://localhost:3000/api/v1/universes/template/clone \
  -H 'Content-Type: application/json' \
  -d "{\"key\":\"$SLUG\",\"name\":\"Test Gate\"}"

# Create 100 tasks rapidly
for i in $(seq 1 101); do
  curl -b cookies.txt -X POST "http://localhost:3000/api/projects/MP/tasks?u=$SLUG" \
    -H 'Content-Type: application/json' \
    -d "{\"title\":\"Task $i\"}" 2>/dev/null
  [ $i -eq 100 ] && echo "--- GATE SHOULD TRIGGER ON NEXT ---"
done
# Task 101 should return 402 with "Crie uma conta para continuar"
```

### 3c. Auth + Claim

1. Click "Entrar" → enter email → receive code (check server logs for dev mode)
2. Enter code → JWT issued, session cookie set
3. Verify: anonymous universe claimed (owner_id updated)
4. Verify: usage gate lifted, can create task 101+
5. Verify: palette switcher shows ALL themes (not just free 4)

### 3d. Theme System

1. **Anonymous user:** palette switcher shows 4 options (Scholarly Light/Dark, Relic Light/Dark)
2. Switch theme → verify CSS variables update instantly (no reload)
3. Refresh → theme persists (cookie)
4. **Logged-in user:** switcher shows all 5 named palettes + 8 variants
5. Universe owner: click gear icon → settings → change theme preset → verify visitor sees new theme

```bash
# Verify dynamic CSS endpoint
curl http://localhost:3000/api/v1/universes/template/theme/scholarly
# → :root { --bg: #FFF9ED; --accent: #CD7F32; ... }
```

### 3e. i18n (pt/en)

1. Default language should be pt-BR (check "Projetos", "Criar", "A fazer")
2. Click language toggle (header) → switch to English
3. Verify: all strings change ("Projects", "Create", "To do")
4. Refresh → language persists (cookie `co_lang`)
5. Check task status labels, priority labels, modal forms, error messages

### 3f. CodeMirror Editor

1. Create or edit a task → click description field
2. Verify: CodeMirror initializes (syntax highlighting, line numbers)
3. Type markdown: `**bold**, *italic*, # Heading, \`code\`, - list item`
4. Verify: live preview renders formatted HTML
5. Use toolbar: bold (Ctrl+B), italic (Ctrl+I), link (Ctrl+K)
6. Save → reopen → content persists with formatting

### 3g. CRDT Collaboration

1. **Must be logged in** (anonymous gets local-only editing)
2. Open a task editor in Browser Tab A
3. Open SAME task in Browser Tab B (same universe, same slug)
4. Type in Tab A → verify text appears in Tab B (real-time)
5. Type in Tab B → verify text appears in Tab A
6. Check: colored cursors with username labels
7. Check: "N users editing" badge in toolbar
8. Disconnect Tab B → edit in Tab A → reconnect Tab B → verify merge

```bash
# Verify WebSocket endpoint requires auth
wscat -c ws://localhost:3000/ws/doc/template/test-doc
# → Should disconnect (401, no token)

# With token (get JWT from login flow):
wscat -c "ws://localhost:3000/ws/doc/my-universe/task-1?token=YOUR_JWT"
# → Should connect, receive sync messages
```

### 3h. Entry Abstraction

```bash
# List entries (new API)
curl http://localhost:3000/api/v1/universes/template/entries?type=task
# → EntryList with .md-backed entries

# Create entry
curl -X POST http://localhost:3000/api/v1/universes/$SLUG/entries \
  -H 'Content-Type: application/json' \
  -d '{"type":"task","title":"Test Entry","frontmatter":{"status":"todo","priority":"high","tags":["test"]},"body":"Entry description"}'
# → Created entry, verify .md file exists in universe data dir

# Protobuf response
curl -H 'Accept: application/x-protobuf' \
  http://localhost:3000/api/v1/universes/template/entries?type=task
# → Binary protobuf response

# Tags
curl http://localhost:3000/api/v1/universes/template/entries/tags
# → [{"tag":"setup","count":2},{"tag":"backend","count":3},...]

# Tree
curl http://localhost:3000/api/v1/universes/template/entries/tree?type=task
# → Hierarchical JSON with parent/child nesting
```

### 3i. Vault REST API (Obsidian Compat)

```bash
# Generate API token (must be logged in)
TOKEN=$(curl -b cookies.txt -X POST http://localhost:3000/api/v1/auth/token \
  -H 'Content-Type: application/json' | jq -r '.token')

# List vault files
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/universes/$SLUG/vault/notes
# → File listing with paths and stats

# Read a file
curl -H "Authorization: Bearer $TOKEN" \
  "http://localhost:3000/api/v1/universes/$SLUG/vault/read?path=projects/MP/1.md"
# → { path, content (markdown), frontmatter, tags, stat }

# Create a file
curl -H "Authorization: Bearer $TOKEN" \
  -X PUT "http://localhost:3000/api/v1/universes/$SLUG/vault/notes" \
  -H 'Content-Type: application/json' \
  -d '{"path":"content/test-note.md","content":"---\ntype: page\ntitle: Test Note\ntags: [test]\n---\n\nHello from vault API."}'
# → Created, verify entry index updated

# Search
curl -H "Authorization: Bearer $TOKEN" \
  -X POST "http://localhost:3000/api/v1/universes/$SLUG/vault/search" \
  -H 'Content-Type: application/json' \
  -d '{"query":"autenticação"}'
# → Search results with context

# Tags
curl -H "Authorization: Bearer $TOKEN" \
  "http://localhost:3000/api/v1/universes/$SLUG/vault/tags"
# → [{"tag":"backend","count":3},...]
```

## 4. Obsidian Plugin Test

### Setup

```bash
# Build the plugin
cd co-obsidian && npm install && npm run build && cd ..

# Create test vault
mkdir -p ~/co-test-vault/.obsidian/plugins/co-universe-sync
cp co-obsidian/manifest.json co-obsidian/main.js co-obsidian/styles.css \
   ~/co-test-vault/.obsidian/plugins/co-universe-sync/
```

### Test in Obsidian

1. Open Obsidian → Open vault: `~/co-test-vault`
2. Settings → Community Plugins → Enable "CO Universe Sync"
3. Plugin settings:
   - CO Instance URL: `http://localhost:3000`
   - API Token: paste token from vault API test above
   - Universe Slug: your test universe slug
4. Click "Sync Now" (or ribbon icon)
5. Verify: `.md` files appear in vault matching universe content
6. Verify: frontmatter maps correctly (`tags`, `created`, `modified`)
7. Verify: `[[wikilinks]]` between files work (click to navigate)
8. Edit a file in Obsidian → save → click "Push to CO"
9. Verify: change appears in CO board UI
10. Edit same task in CO board → "Pull from CO" in Obsidian → verify change
11. Status bar: "CO: synced ✓"
12. Run Dataview query (if Dataview plugin installed):
    ```dataview
    TABLE status, priority, tags
    FROM "projects"
    WHERE type = "task" AND status != "done"
    SORT priority DESC
    ```

### Clipper Test

1. Install Obsidian Clipper browser extension
2. Configure destination to CO vault API (or use default Obsidian → sync)
3. Clip a web page → verify it lands in `content/clips/` directory
4. Verify frontmatter: `type: clip`, `source: URL`, `tags`

## 5. E2E Tests (Playwright)

```bash
cd co-web

# Install Playwright browsers (one-time)
npx playwright install chromium

# Run all E2E tests
npx playwright test --project=chromium-desktop

# Run specific test suites
npx playwright test e2e/smoke.spec.ts
npx playwright test e2e/universe.spec.ts
npx playwright test e2e/usage-gate.spec.ts
npx playwright test e2e/theme.spec.ts
npx playwright test e2e/i18n.spec.ts
npx playwright test e2e/codemirror.spec.ts
npx playwright test e2e/auth-crdt.spec.ts
npx playwright test e2e/co-landing.spec.ts

# View report
npx playwright show-report
```

## 6. Ansible Deploy Test

```bash
cd co-deploy

# Dry run against local (no changes)
ansible-playbook playbooks/provision.yml -i inventory/vps.yml --check

# Test backup playbook
ansible-playbook playbooks/backup.yml -i inventory/vps.yml --check

# Full Fly.io deploy (production-like)
ansible-playbook playbooks/fly-deploy.yml -i inventory/fly.yml
```

## 7. Security Checklist

```bash
# No secrets in source
grep -rn "sk_\|pk_\|ghp_\|password.*=.*['\"]" co-web/src/ core/src/
# → Should return nothing

# Clippy clean
cargo clippy -p co-web -p co -- -D warnings

# Cargo audit (if installed)
cargo audit

# Node audit
cd co-web/editor && npm audit && cd ../..
cd co-obsidian && npm audit && cd ..
```

## 8. Release Checklist

- [ ] All tests pass: `cargo test` (34 passed)
- [ ] Clippy clean: `cargo clippy -- -D warnings`
- [ ] E2E pass: `npx playwright test`
- [ ] Editor bundle built: `co-web/static/shared/editor.bundle.js`
- [ ] Obsidian plugin built: `co-obsidian/main.js`
- [ ] VERSION bump: `Cargo.toml` → `1.0.0`
- [ ] CHANGELOG.md updated
- [ ] Docker build: `docker build -t co-web co-web/`
- [ ] Smoke test on Docker container
- [ ] Fly.io deploy: `flyctl deploy` or `ansible-playbook fly-deploy.yml`
- [ ] Tag: `git tag v1.0.0`
- [ ] GitHub Release with binaries
- [ ] Obsidian plugin: PR to `obsidianmd/obsidian-releases`
