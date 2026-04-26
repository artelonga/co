# CO — MVP Público (v1.0)

> Markdown é o conteúdo. CSS é a forma. Cada universo herda, nunca mistura.
> Colaboração é o upgrade. Ansible é a infraestrutura.

---

## O que é o MVP

**artelonga.com.br/co** — plataforma pública, gratuita, open-source.

| Visitante | O que pode fazer |
|-----------|-----------------|
| Anônimo | Ver template, criar universo, editar com CodeMirror, até 100 entradas, 4 temas |
| Logado | Sem limite, todos os temas, CRDT colaborativo, universo compartilhável |

### Fluxo Completo

```
artelonga.com.br/co
  │
  ├─ Vê board template (read-only, live demo)
  ├─ Clica "Criar universo" → modal (nome + slug)
  │   └─ Redireciona para /co/:slug (editável, CodeMirror)
  │
  ├─ Edita conteúdo livremente (CodeMirror, dinamic CSS)
  │   ├─ Até 100 entradas sem conta
  │   ├─ Universo salvo no servidor, acessível por cookie
  │   └─ Mas SEM link compartilhável (404 para outros)
  │
  ├─ Tenta compartilhar ou colaborar →
  │   └─ "Crie uma conta pra colaborar"
  │       └─ Login → universo vira público, CRDT ativa
  │
  └─ Logado:
      ├─ Link compartilhável funciona (/co/:slug visível pra todos)
      ├─ CRDT sync (edição simultânea com cursores remotos)
      ├─ Todos os temas + editor de paleta customizada
      └─ Sem limite de entradas
```

---

## Content ≠ Form

| Camada | O que contém | Onde vive |
|--------|-------------|-----------|
| **Content** | Projetos, tarefas, comentários, atividade | SQLite `co.db` (scoped por universe_key) |
| **Form** | Tema, layout, fontes, tokens CSS customizados | `universes` table + `/api/v1/universes/:slug/theme.css` |

Conteúdo nunca sabe como será exibido. Forma nunca sabe o que contém.

### Dynamic CSS Engine

Cada universo tem um endpoint `GET /theme.css` que gera CSS em tempo real:
- Carrega preset base (scholarly, relic, modern)
- Aplica overrides do owner (custom_tokens JSON)
- Retorna `:root { --bg: ...; --accent: ...; ... }` completo
- Hot-swap no browser sem reload

---

## Temas

| Tier | Palettes | Extras |
|------|----------|--------|
| **Free** | Scholarly Light/Dark, Relic Light/Dark | — |
| **Logado** | Todos + Modern + 8 variants | Editor de paleta customizada |

---

## Editor

**CodeMirror 6** para todos (anônimos e logados):
- Markdown com GFM (tabelas, task lists, code blocks)
- Split-pane: editor + preview
- Toolbar: bold, italic, heading, link, code, list
- Tema respeita palette ativa (CSS custom properties)

**CRDT (Yjs)** apenas para logados:
- WebSocket sync (`/ws/doc/:slug/:doc_id`)
- Cursores remotos com nome do usuário
- "N users editing" badge
- Anônimo → editor funciona local, sem sync

---

## i18n

- pt-BR (default) e en
- Toggle no header, cookie `co_lang`
- Todos os strings da UI com `data-i18n` attributes

---

## Deploy

**Ansible** para provisionamento reproduzível:
- Fly.io (atual) + VPS genérico + self-hosted
- Playbooks: provision, deploy, backup (com rotação)
- systemd + Caddy (auto-SSL) + backup diário

---

## Tarefas (14 tasks, co auto)

| ID | Título | P | Deps |
|----|--------|---|------|
| CO-20 | Epic: MVP plataforma pública | C | — |
| CO-21 | Universe CRUD API + slug routing | C | — |
| CO-22 | Template universe + "Criar universo" | C | 21 |
| CO-23 | Usage gate (100 entradas → conta) | H | 21 |
| CO-24 | Content/form separation (universe config) | H | 21 |
| CO-25 | Theme gating (free vs logado) | H | 24 |
| CO-26 | Web UI i18n (pt/en) | H | — |
| CO-27 | Landing page /co | C | 22, 26 |
| CO-28 | Open source repo setup | M | all |
| CO-29 | CodeMirror 6 editor | C | — |
| CO-30 | Dynamic CSS engine | H | 24 |
| CO-31 | CRDT sync (Yjs, login required) | H | 29, 21 |
| CO-32 | Ansible deploy | H | 21 |
| CO-33 | E2E test suite | H | all features |

P = Priority: C=critical, H=high, M=medium

### Ordem de execução

```
         CO-26 (i18n) ─────────────────────────────┐
         CO-29 (CodeMirror) ───────────────┐       │
                                           │       │
CO-21 ──┬── CO-23 (usage gate)             │       │
        ├── CO-24 ──┬── CO-25 (themes)     │       │
        │           └── CO-30 (dynamic CSS) │       │
        ├── CO-22 (template) ──────────────┼── CO-27 (landing)
        ├── CO-32 (Ansible)                │
        └───────────────── CO-31 (CRDT) ───┘
                                    │
                              CO-33 (E2E)
                                    │
                              CO-28 (release)
```

---

## Stack do MVP

| Camada | Tecnologia | Status |
|--------|-----------|--------|
| Backend | Axum 0.8, SQLite, JWT auth | ✅ Existe |
| Board UI | Vanilla JS, CSS custom props | ✅ Existe |
| Editor | CodeMirror 6 | ❌ Novo |
| CRDT | Yjs + WebSocket | ❌ Novo |
| Dynamic CSS | Theme engine (Rust) | ❌ Novo |
| Palettes | 5 named + 8 variants | ✅ Existe |
| Auth | Email + código, JWT | ✅ Existe |
| Multi-tenant | universe_key scoping | ✅ Parcial |
| i18n | pt/en | ⚠️ CLI only, needs web |
| Deploy | Fly.io + Docker | ✅ Existe |
| Ansible | Playbooks | ❌ Novo |
| E2E | Playwright | ⚠️ Parcial |
| Landing | — | ❌ Novo |

---

## Pós-MVP (v1.1+)

| Feature | Quando |
|---------|--------|
| ContentDB com zstd + FTS5 | Quando universos passarem de 1000 entradas |
| Electron desktop | Quando demanda justificar |
| Capacitor mobile | Quando demanda justificar |
| Schema registry + validation | Quando tipos customizados forem necessários |
| Version history (beyond Git) | Quando colaboração intensiva exigir |
