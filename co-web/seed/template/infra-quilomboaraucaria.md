---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 33
slug: infra-quilomboaraucaria
tags:
- infra
- quilomboaraucaria
- fly
- compute
- mídia
title: 'Infra: universo quilomboaraucaria'
type: page
---

# Universo: `quilomboaraucaria`

Repositório: `/Users/artelonga/projects/quilomboaraucaria` · Stack: SvelteKit + Node 22 + SQLite · Site de conteúdo da comunidade Quilombo Araucária com upload de mídia.

> Universo de conteúdo "primeiro cliente real" da plataforma. Caso de uso intensivo em upload (fotos raw, vídeos até ~500 MB) — o motivo do dimensionamento atípico.

Voltar ao [Catálogo](/co/template?page=infra).

---

## Tasks ativas

### `quilombo-araucaria` (prod)

- **Status:** running · 1 instância em `gru` · versão 132 · last deploy 2026-05-10.
- **Fonte:** `fly.toml` no root do repo.

**OS + runtime**

- Base: `node:22-alpine`. Alpine escolhido pelo footprint (~5x menor que slim-debian). Trade-off: musl libc + `sharp` precisam de wheel específico (`npm install --os=linux --libc=musl --cpu=x64 sharp` no Dockerfile).
- `ffmpeg` + `exiftool` instalados via `apk`: probe de vídeo + extração de poster + preview de raw (DNG/CR2/CR3/NEF/ARW/RAF têm JPEG embutido full-res que vai pelo pipeline de variantes).
- Toolchain: multi-stage. Builder roda `npm ci` + `npm run build`; runtime carrega só `build/`, `node_modules` prunado, scripts de seed + `data-seed/` + `content-seed/`.

**Dimensionamento**

- VM: `shared-cpu-1x · 2 048 MB`. **A maior task entre os universos de conteúdo.** Comentário literal no `fly.toml`:
  > `# 412 MB upload double-buffers during parse — 1 GB OOM'd at 866 MB rss`

  Esse é o tipo de decisão que justifica este catálogo: a memória extra é **empírica**, não chute. Upload de vídeos grandes faz double-buffering durante parse multipart; com 1 GB o processo caía a 866 MB residente.
- `BODY_SIZE_LIMIT = 1 073 741 824` (1 GiB) — cap explícito de upload alinhado com o motivo do dimensionamento.
- Volume: `quilombo_data` **10 GB** (maior do catálogo). Contém: `quilombo.db` (SQLite), `uploads/` (mídia), `content/` (markdown).
- Auto-stop: `"stop"` mas com `min_machines_running = 1` — comentado:
  > `# keep one warm 24/7 — kills the ~10s cold-start on first visit after idle`

  Node cold-start é uma ordem de grandeza pior que Rust, decisão de UX. Custo é assumido como necessário.

**Ingress**

- `internal_port = 3000`. Concurrency hard 50 / soft 25 (mais conservador que CO/Yggdrasil — uploads consomem mais por request).
- Healthcheck `/api/v1/quilombo/versao`.
- IPv6 dedicado: `2a09:8280:1::ed:fb5f:0`. IPv4 compartilhado: `66.241.125.48`.

**Comms**

- **Inbound:** usuários via HTTPS.
- **Outbound:**
  - Envia eventos para `co-artelonga` via webhook (`SYNC_TOKEN`, `SYNC_ENABLED = "true"`).
  - `ORIGIN` setado em secrets (necessário para SvelteKit form actions atrás de proxy).
- **Sem secrets de email/OAuth** — quilombo-araucaria autentica via CO handover (JWT ES256 cross-domain, ver [Segurança](/co/template?page=seguranca)).

**Secrets em uso:** `ORIGIN`, `SESSION_SECRET`, `SYNC_ENABLED`, `SYNC_TOKEN`.

**Custo (upper-bound 24/7):** ~$16/mês compute + $1,50 volume = **~$17,50/mês**. Com `min_machines_running = 1` o realizado fica perto do teto — esta é a task mais cara em produção hoje.

---

## Tasks degradadas / dormentes

### `quilombo-araucaria-dev` (dev) — **parked**

- **Status:** **suspended** · 1 instância parada em `gru` · last deploy 2026-04-14.
- **Decisão (2026-05-13):** manter como está. Compute custa $0; volume `re133dg5zm0536o4` (1 GB) custa ~$0,15/mês — aceito como tradeoff pela opção de reativar dev sem rebuild de schema.
- **Fonte:** sem `fly.dev.toml` no repo (apenas `fly.toml` e `fly.staging.toml`). Para reativar, será necessário criar o toml ou usar o `fly.staging.toml` como template, ajustando o nome do app.
- **VM (snapshot):** `shared-cpu-1x · 256 MB`.

### ~~`quilombo-araucaria-uat`~~ — **destruído 2026-05-13**

- App + 10 volumes órfãos (`quilombo_uat_data` ×2, `quilombo_uat_volume` ×2, `quilombo_uat_r3` … `quilombo_uat_r8`) + IPv6 dedicado + IPv4 compartilhado removidos via `flyctl apps destroy quilombo-araucaria-uat`.
- Motivo: histórico de UAT acumulado em ~1 mês, sem máquinas attachadas, sem plano de reativação. Os 10 volumes vinham de tentativas sucessivas (`_r3` a `_r8` parecia ser "release 3" a "release 8") que nunca foram limpas.
- **Economia:** ~$1,50/mês.
- O `fly.staging.toml` permanece no repo como referência. **Apagar do repo se a estratégia mudou** (não há mais UAT planejada) — TO BE.

---

## Riscos / TO BE

- **10 GB de mídia + DB em um único volume LUKS** — backup off-site não existe. Perda do volume = perda de todo conteúdo da comunidade. **Prioridade DR alta.**
- **Sem CDN na frente** — todas as fotos e vídeos servidos diretamente pelo Node. Bandwidth ainda dentro da free-tier (100 GB) mas escala mal.
- **`sharp` musl-bound** — atualização major do Node ou Alpine pode quebrar build (vivência conhecida).
- **UAT divergente do prod** — `fly.staging.toml` tem `min_machines_running = 0` enquanto prod tem 1; é o que se espera, mas garantir que testes UAT não dependam de keep-warm.

---

Voltar ao [Catálogo](/co/template?page=infra) · [Universo co](/co/template?page=infra-co)
