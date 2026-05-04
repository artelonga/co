---
created: 2026-05-04T00:00:00+00:00
modified: 2026-05-04T00:00:00+00:00
order: 1
slug: co-plataforma
tags:
- featured
- co
- plataforma
- marcos
title: Plataforma CO — marcos e tarefas
type: page
---

# Plataforma CO

CO é software livre desenvolvido publicamente. Cada funcionalidade nasce como uma tarefa no quadro de desenvolvimento.

## Marcos (releases)

| Versão | Data | Destaques |
|--------|------|-----------|
| **1.42.0** | 2026-05-04 | Template scaffold, reindex, blob endpoint, wikilinks corrigidos |
| 1.41.1 | 2026-05-03 | Gate de visibilidade — privacidade de universos |
| 1.41.0 | 2026-05-03 | Leitor inline de PDF (PDF.js) |
| 1.40.0 | 2026-05-03 | Versionamento de referências (work_id + editions) |
| 1.38.0 | 2026-05-03 | Relações cross-universe (co:// URI) |
| 1.36.0 | 2026-05-03 | Tipo `reference` — cartões para PDFs, vídeos, URLs |
| 1.22.4 | 2026-04-30 | Correção de bug em migração (incidente prod) |
| 1.0.0 | 2026-04-01 | MVP público — multi-tenant, board, auth, clone |

## Tarefas por status

O desenvolvimento é rastreado como user-stories (CO-1 a CO-161+) no universo `co`. Cada tarefa tem critérios de aceite em formato BDD.

Ver: [github.com/artelonga/co](https://github.com/artelonga/co)

## Funcionalidades principais

- **Universos** — espaços isolados de conteúdo (public / private / template)
- **Entry abstraction** — cada `.md` é uma entrada tipada com YAML
- **Vault API** — compatível com Obsidian Local REST API
- **CRDT sync** — colaboração em tempo real via WebSocket + Yjs
- **Visibilidade** — gate de acesso por middleware (CO-161)
- **Template scaffold** — CLAUDE.md + type-check automático
- **Timeline** — eventos em escala log, do Big Bang ao presente
- **Referências** — cartões de metadados para mídia com leitor inline
