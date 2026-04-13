---
titulo: Auth e sistema de permissoes
status: done
prioridade: critica
prazo: 2026-03-28
etiquetas: [plataforma, seguranca]
criado: 2026-03-15
---

Sistema centralizado de autenticacao e permissoes.

## Entregue

- Matriz declarativa de permissoes (admin/membro)
- Guards server-side: exigirAuth, exigirPermissao
- Helper client-side: pode()
- Ownership check: podeEditarRecurso
- Login com Argon2, sessoes com cookie seguro
- CSRF custom para multi-dominio (fly.dev + quilomboaraucaria.org)
- 301 redirect para dominio canonico
