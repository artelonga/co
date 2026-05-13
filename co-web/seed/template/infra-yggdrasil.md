---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 32
slug: infra-yggdrasil
tags:
- infra
- yggdrasil
- fly
- compute
- games
title: 'Infra: universo yggdrasil'
type: page
---

# Universo: `yggdrasil`

Repositório: `/Users/artelonga/projects/yggdrasil` · Stack: Rust + Axum + SQLite · Lobby + portais para universos jogáveis.

> Yggdrasil reusa o engine 2D `co/game-core` em **build-time** (path dep). Em runtime, **não fala com `co`**.

Voltar ao [Catálogo](/co/template?page=infra).

---

## Tasks ativas

### `yggdrasil-artelonga` (prod)

- **Status:** running · 1 instância em `gru` · versão 9 · last deploy 2026-05-13 (recém-criado).
- **Fonte:** `fly.toml` no root do repo.

**OS + runtime**

- Base: `debian:trixie-slim`. Mesma razão que `co-web`: `co/game-core` compartilha o toolchain Rust 1.90 e arrasta a mesma necessidade de glibc 2.40 (mesmo que `ort-sys` não esteja aqui, mantém compatibilidade ABI para evitar surpresas).
- Build context **especial:** root é `/Users/artelonga/projects/` (não o repo), porque `yggdrasil/Cargo.toml` ainda tem path dep `../co/game-core`. Comando:
  ```bash
  cd /Users/artelonga/projects
  flyctl deploy --config yggdrasil/fly.toml \
                --dockerfile yggdrasil/yggdrasil-web/Dockerfile
  ```
  YG-17 (tracked) substitui isso por um git rev pin → permitirá deploy a partir do próprio repo.

**Dimensionamento**

- VM: `shared-cpu-1x · 512 MB`. Headroom para game-core carregar maps grandes.
- Volume: `yggdrasil_data` 1 GB. Contém `yggdrasil.db` (lobby + sessões) e `yggdrasil-sementes.db` (carteira de moeda interna — a "semente").
- Auto-stop: `"stop"` com `min_machines_running = 0`. Tolerável porque cold-start Rust é rápido.
- Roda como usuário `ygg` (não-root) — bom padrão.

**Ingress**

- `internal_port = 3030`. Concurrency hard 100 / soft 80.
- DNS: `yggdrasil-artelonga.fly.dev`. (Domínio custom ainda não configurado neste catálogo.)
- IPv6 dedicado: `2a09:8280:1::114:e372:0`. IPv4 compartilhado: `66.241.125.153`.

**Comms**

- **Inbound:** usuários via HTTPS. Nenhuma outra task chama yggdrasil.
- **Outbound:**
  - **SMTP** (`YGGDRASIL_SMTP_*`) — códigos de recuperação por email. Hoje os 4 secrets de SMTP estão **vazios** no `fly.toml`; só `YGGDRASIL_JWT_SECRET` está definido em Fly secrets. **Significa que envio de email está desabilitado em prod hoje** (verificar via `flyctl secrets list -a yggdrasil-artelonga`).

**Secrets em uso:** somente `YGGDRASIL_JWT_SECRET`.

**Custo (upper-bound 24/7):** ~$4/mês compute + $0,15 volume = **~$4,15/mês**. Com auto-stop ativo: provavelmente **<$1/mês** (tráfego mínimo).

---

## Riscos / TO BE

- **SMTP desabilitado** — recuperação por email não funciona em prod até secrets serem populados.
- **Path-dep com `co/game-core`** — release process frágil (YG-17). Se `co` cortar uma release breaking, yggdrasil quebra silenciosamente até o próximo deploy.
- **Sem backup** — mesma situação do co antes do CO-143 (ver [infra-co §planejadas](/co/template?page=infra-co)).

---

Voltar ao [Catálogo](/co/template?page=infra) · [Universo co](/co/template?page=infra-co)
