---
type: doc
title: CLAUDE.md
---

# CO Platform — Board de Desenvolvimento

Universe de rastreamento de desenvolvimento do CO. Contém todas as user-stories,
epics, processos e eventos de CO-1 a CO-161+.

## Universe

- **Slug**: `co`
- **API base**: `/api/v1/universes/co`
- **Viewer**: `/co/co`
- **Visibility**: public-subscribable

## Estrutura

```
work/co/
├── CO-1.md … CO-161.md   # user-stories e epics
├── ROADMAP*.md            # roteiros de release
├── SPRINT-*.md            # planejamento de sprint
├── SPEC-*.md              # especificações
└── _universe.yaml         # schema CO
```

## Content types

- `user-story` — requisito com critérios de aceite BDD (CO-N.md)
- `epic` — agrupamento de user-stories relacionadas
- `task` — subtarefa de uma user-story
- `event` — marco ou evento do projeto
- `page` — página de documentação / roadmap
- `reference` — referência bibliográfica
- `process` — processo de desenvolvimento

## Status do projeto

- **Versão atual**: 1.42.0
- **Total de tasks**: 161+
- **Branch principal**: `main`

## API

```bash
# Listar todas as user-stories
curl /api/v1/universes/co/entries?type=user-story

# Tasks done
curl /api/v1/universes/co/entries?type=user-story&filter={"status":"done"}

# Buscar
curl /api/v1/universes/co/entries?q=visibility+gate

# Re-indexar após sync
curl -X POST /api/v1/universes/co/reindex -H "Authorization: Bearer $TOKEN"
```

## Convenções

- Cada CO-N.md tem `id: N` no frontmatter
- Status: `todo` | `in_progress` | `done` | `blocked` | `cancelled`
- Labels: `type:feat` (minor bump) | `type:fix` (patch) | `type:chore` (no bump)
- `parent: N` referencia o epic pai
