---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 21
slug: renderers
tags:
- renderers
- markdown
- obsidian
- viewer
title: Visualizadores de Markdown
type: page
---

# Visualizadores de Markdown

Co é Markdown-first. Você pode browsear seu universo (e outros públicos) através de várias ferramentas — todas consomem a mesma API REST + Vault.

## Opções

### 1. Web SPA nativa (incluso)

`https://co.artelonga.com.br/<seu-universo>`

- **O que é**: a SPA principal de Co, escrita em ES modules vanilla
- **Para que serve**: operacional + admin + edição rápida
- **Vantagens**: zero setup, sempre atualizado, integrado com chat + notificações + tarefas
- **Limitações**: minimal por design; não é o lugar para reading prolongado

### 2. Cloud markdown viewer (Svelte + TS — em desenvolvimento)

**Status: planejado em CO-212.** Será servido em `co.artelonga.com.br/viewer/<universo>`.

- **O que é**: app Svelte focado em **leitura bonita** de markdown
- **Vantagens**: tipografia cuidada, links cross-universe, citações com hover preview, dark mode polished, melhor para leitura longa
- **Limitações**: read-mostly (editing acontece em outra ferramenta)

### 3. Obsidian (recomendado para edição local)

[Obsidian](https://obsidian.md/) é um editor Markdown desktop popular.

**Como conectar** (plugin Co-Obsidian — polish em CO-213):

1. Instale Obsidian
2. Em Co settings → "Vault API token" → gere um token
3. Em Obsidian → instale plugin "Co Vault Sync" (community plugins ou via BRAT)
4. Configure URL `https://co.artelonga.com.br`, token, e universo
5. Sync acontece a cada save, pull-on-open também

**Vantagens**: full editing power do Obsidian (gráfico, backlinks, daily notes, plugins community), offline, local files.

**Limitações**: setup requer configuração; mudanças não propagam em tempo real (apenas no próximo sync).

### 4. CLI (`co` binary)

Para usuários técnicos que querem fluxo terminal:

```bash
cargo install co-cli

# Browse seu universo no terminal
co browse seu-universo

# Servir o universo localmente (renderer próprio web)
co serve seu-universo
```

- **Vantagens**: pode trabalhar offline com SQLite local; rápido
- **Limitações**: apenas para devs

### 5. Browsers de markdown genéricos

Como o conteúdo é Markdown standard (CommonMark + frontmatter YAML), qualquer tool pode renderizar:

- **VS Code / Cursor**: open .md file, preview com Cmd+Shift+V
- **Marked** (macOS): drag .md file
- **MarkText**, **Zettlr**, **Typora**: editores Markdown standalone
- **Glow** (CLI): `glow .` em diretório do universo

Esses não conectam à API; precisam dos arquivos localmente (via Obsidian sync, git clone, ou `co pull`).

### 6. GitHub web UI

Universos backed por git repo (CO-89, futuro) são navegáveis em `github.com/<user>/<universo>` diretamente. Funciona sem nenhum setup. Limitado à renderização padrão do GitHub.

## Comparação rápida

| Tool | Setup | Edição | Live updates | Offline | Best for |
|---|---|---|---|---|---|
| Web SPA | zero | rápida | sim (WS) | não | uso diário |
| Cloud viewer (CO-212) | zero | não | sim | não | leitura longa |
| Obsidian + plugin | médio | excelente | sync no save | sim | edição séria |
| CLI `co serve` | dev setup | via editor | manual | sim | técnicos |
| MarkText etc | mínimo | standalone | não | sim | quick view |
| GitHub web | zero | via PR | manual | não | visitas casuais |

## Contrato de API

Para implementar um novo renderer, consuma estes endpoints:

### Públicos (sem auth para universos públicos)

```
GET /api/v1/universes/<slug>                      → metadata
GET /api/v1/universes/<slug>/entries              → lista todos os arquivos
GET /api/v1/universes/<slug>/entries/<path>       → conteúdo de 1 arquivo
GET /api/v1/universes/<slug>/manifest             → schema do universo
```

### Autenticados (para universos privados)

```
GET /api/v1/universes/<slug>/vault/notes
GET /api/v1/universes/<slug>/vault/note/<path>
PUT /api/v1/universes/<slug>/vault/note/<path>
```

Auth: Bearer token via `Authorization: Bearer <token>` header (emitido em settings → API tokens).

OpenAPI spec completo está em desenvolvimento (CO-211). Versionado v1.

## Telemetria no cloud viewer

O cloud viewer (CO-212) tem **telemetria embarcada** para que possamos melhorar a experiência sem que você precise relatar problemas manualmente:

### O que coletamos

- **Page views**: qual universo + qual rota foi visitada
- **404s**: quais URLs retornam não-encontrado (detectar links quebrados)
- **Errors**: JavaScript errors no renderer
- **Performance**: tempo de load por página

### O que NÃO coletamos

- **Conteúdo lido**: não logamos qual texto específico você viu, só a rota
- **Mensagens privadas / DMs**: nunca
- **PII**: emails, nomes pessoais não vão para telemetry
- **Sessões individuais**: usamos visitor_id opaco, não user_id

### Como desativar

Em settings → Privacidade → "Não enviar telemetria de viewer". Stored em `localStorage["co_viewer_telemetry"] = "0"`. O viewer respeita esse flag.

## Roadmap

- **CO-210**: serve `/seguranca`, `/dependencias`, `/licensa`, `/renderers` no SPA com telemetria + 404 tracing
- **CO-211**: formalizar Universe Content API contract v1 + OpenAPI spec
- **CO-212**: build Svelte + TS cloud viewer
- **CO-213**: polish Obsidian plugin (CO-68 follow-up)

Quando esses 4 landam, qualquer Markdown renderer (terceiro, custom, novo viewer) consome a mesma API uniformemente. Co se torna uma plataforma multi-frontend.

---

Voltar para [Segurança](/co/template?page=seguranca).
