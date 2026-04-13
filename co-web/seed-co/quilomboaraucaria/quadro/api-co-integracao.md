---
titulo: Integracao com API do co-web
status: todo
prioridade: media
prazo: 2026-06-30
etiquetas: [plataforma, api, co-web]
criado: 2026-04-01
---

Conectar quilombo-blog ao co-web server para unificar backend.

## Contexto

Hoje quilombo-blog tem seu proprio SQLite e filesystem. O co-web server ja expoe:
- /api/projects/{key}/tasks — board/quadro
- /api/v1/quilombo/ — membros, missoes, eventos, auth
- Universo trait para conteudo markdown

## O que precisa ser feito

- [ ] Definir qual backend sera fonte de verdade (co-web vs quilombo-blog SQLite)
- [ ] SvelteKit fetch server-side para API do co-web
- [ ] Sincronizar usuarios entre os dois sistemas
- [ ] Migrar quadro/missoes para usar co-web API
- [ ] Avaliar: manter quilombo-blog autonomo ou tornar frontend puro do co-web

## Decisao arquitetural

Opcao A: quilombo-blog autonomo, co-web como backoffice/board
Opcao B: quilombo-blog vira frontend do co-web (server-side fetch)
Opcao C: Migrar tudo para co-web com SSR em Rust (longo prazo)
