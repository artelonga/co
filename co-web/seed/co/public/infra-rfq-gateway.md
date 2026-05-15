---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 34
slug: infra-rfq-gateway
tags:
- infra
- rfq-gateway
- fly
- compute
- pricing
title: 'Infra: universo rfq-gateway'
type: page
---

# Universo: `rfq-gateway`

Repositório: `/Users/artelonga/projects/rfq-gateway` · Stack: Rust + Axum · RFQ pricing engine + adapter para Hedix MM, Polymarket, BCB Olinda, B3.

> **Single codebase, two deployments, three env-tiers.** Compreender este split é pré-requisito para qualquer operação:
>
> | Tier | Fly app | `RFQ_ENV` | Hedix | Cohort | Papel |
> |---|---|---|---|---|---|
> | `staging` | `artelonga-rfq-gateway` | `staging` | staging | (tudo não-SMK) | Lane de validação E2E |
> | `smoke-staging` | `artelonga-rfq-gateway` (mesmo app) | `smoke-staging` | staging | SMK* tickers | Dress rehearsal SELIC vs Hedix synthetic |
> | **production** | **`rfq`** | `production` | prod | (real) | **Customer flow real — sem counter-party de teste** |
>
> O tier `smoke-staging` é apenas tag (`RFQ_ENV`) com cohort SMK*-prefix dispatch — não há binário separado. Ambos os tiers `staging` e `smoke-staging` rodam na mesma máquina; o `PrefixDispatchStrategy` roteia pelo prefixo do ticker.

Voltar ao [Catálogo](/infra).

---

## Tasks ativas

### `artelonga-rfq-gateway` (staging + smoke-staging)

- **Status:** running · **3 máquinas** registradas:
  - `dawn-tree-1861` — started, 1/1 checks, volume `vol_rnzyo7p7...`.
  - `empty-fire-4439` — stopped, 0/1 checks, volume `vol_rnzyo7pm...`.
  - `e2e-canary-weekly` — stopped, sem volume, sem process group → instância **destacada** (não parte do Fly Launch group). É uma máquina solta usada para rodar o e2e harness uma vez por semana.
- **Fonte:** `fly.toml` no root do repo.

**OS + runtime**

- Base: `debian:bookworm-slim` (glibc 2.36). **Difere de co/yggdrasil** porque rfq-gateway não tem nenhuma dep nativa C++ que precise dos símbolos glibc 2.38+ — Bookworm fica menor.
- Build com `--platform=linux/amd64` em ambos os estágios (force linux/amd64 mesmo se desenvolvido em mac arm — Fly só roda amd64).
- 3 binários compilados, embarcados no runtime: `rfq-gateway` (servidor), `rfq-snapshot` (debug snapshot), `e2e-harness` (rodado pela máquina canário).
- Roda como usuário `rfq` via `gosu` no entrypoint, mas o entrypoint precisa subir como **root** primeiro para `chown` o volume Fly (volumes montam com `root:root` independente de chown no build).

**Dimensionamento**

- VM: `shared-cpu-1x · 256 MB`. Quote engine; working set pequeno (rings de observabilidade em JSONL).
- Volume: `rfq_artifacts` 1 GB **por máquina** (Fly volumes são per-instance). Contém rings `inbound-YYYY-MM-DD.jsonl`, `rejections-…`, `fills-…`. Comentário no `fly.toml`:
  > `# Volume is per-machine on Fly: one rfq_artifacts volume must exist per app machine`
- Auto-stop: `"stop"` mas **`min_machines_running = 1`**. Sempre 1 quente para responder ao RFQ flow real-time.

**Ingress**

- `internal_port = 8080`. Sem `[concurrency]` configurado → defaults Fly.
- Healthcheck `/health`, grace 10s (binário Rust starta rápido).
- IPv6 dedicado: `2a09:8280:1::fb:558f:0`. IPv4 compartilhado: `66.241.124.178`.

**Comms**

- **Inbound:** clientes externos autenticados via `RFQ_API_KEY` (header bearer).
- **Outbound (todas externas, sem 6PN):**
  - **Hedix MM** (`HEDIX_API_KEY`, `HEDIX_BASE_URL`, `HEDIX_ENV`) — quote requests + (se `HEDIX_WRITE_ENABLED`) fills.
  - **Polymarket WS** (`RFQ_POLYMARKET_WS_ENABLED`) — feed de orderbook em tempo real.
  - **BCB Olinda** (OData) — SELIC + outras taxas BR.
  - **B3 DI** scrape — curva de DI.
- Pricing strategy: `RFQ_STRATEGY` env (Avellaneda-Stoikov + SELIC operator + conviction prefix list).

**Secrets relevantes:** `HEDIX_API_KEY`, `RFQ_API_KEY`, `HEDIX_BASE_URL`, `HEDIX_ENV`, `HEDIX_WRITE_ENABLED`, `RFQ_AS_*` (parâmetros AS), `RFQ_FAIR_VALUE_*`, `RFQ_POLYMARKET_WS_ENABLED`, `RFQ_STRATEGY`, `RFQ_CURRENT_SELIC_BPS`, `RFQ_SELIC_OPERATOR_CENTER_BPS`, `RFQ_MAPPING_FILE`.

**Custo (upper-bound):** 2 máquinas (uma 24/7, outra stopped) ≈ $2/mês + canário stopped ≈ $0. Volumes 2× $0,15 = $0,30. **Total ~$2,30/mês.**

---

### `rfq` (production)

- **Status:** running · 1 instância · versão 3 · last deploy 2026-05-12.
- **Imagem:** **mesma image SHA** que `artelonga-rfq-gateway` (`artelonga-rfq-gateway:deployment-01KR6HF4PZQXESFS6AHFACZ8KZ`). Não há `fly.toml` separado para esta task no repo — o deploy é feito apontando a image registry promovida pelo staging:
  ```
  fly deploy --image registry.fly.io/rfq:deployment-<DIGEST> --app rfq
  ```
  (documentado em `rfq-gateway/docs/release-runbook.md:308`).

**Diferenças vs staging (justificadas por serem prod):**

| Secret | staging (`artelonga-rfq-gateway`) | prod (`rfq`) | Razão |
|---|---|---|---|
| `HEDIX_API_KEY` | tenant Hedix staging | tenant Hedix prod | ambientes Hedix distintos |
| `RFQ_API_KEY` | gate staging | gate prod | rotação independente |
| `RFQ_ENV` | (inferido `staging`) | `production` | tier tag — usado por logging + metrics |
| `RFQ_HEDIX_INCENTIVE_*` | desligado | `ENABLED`, `QUOTE_CENTS`, `RATIO` | feature de incentivo só roda quando há contraparte real |
| `RFQ_SELIC_OPERATOR_ENABLED` | desligado | ligado | operator override só faz sentido contra mercado real |
| `RFQ_SELIC_STRATEGY` | placeholder | full | strategy real |
| `RFQ_CONVICTION_PREFIX_LIST` | ausente | ex.: `SELIC-` | dispatch prefix-based |
| `RFQ_RING_PERSIST_DIR` | ausente (rings só em memória) | `/app/artifacts/rings` | flush JSONL para análise post-trade |

**DNS planejado:** `rfq.artelonga.com.br` (cert + A/AAAA Fly). Hoje acessível via `rfq.fly.dev`.

**Dimensão:** `shared-cpu-1x · 256 MB`, volume `rfq_artifacts` 1 GB. Mesmo perfil que staging (workload de quote engine é uniforme; o que muda é a contraparte, não o cálculo).

**Custo:** ~$2/mês compute + $0,15 volume = **~$2,15/mês**.

---

## Topologia (escopo rfq-gateway)

```
   Hedix staging ◄───────┐                            ┌───────► Hedix prod
   Polymarket    ◄───┐   │     ┌────────────────┐     │
   BCB Olinda    ◄───┼───┼─────│ artelonga-     │     │     ┌──────────┐
   B3 DI         ◄───┘   │     │ rfq-gateway    │     ├─────│   rfq    │── customer flow real
                         │     │ (staging +     │     │     │ (prod)   │   (HTTPS + RFQ_API_KEY)
                         └─────│  smoke-staging │     │     └──────────┘
                               │  no MESMO bin) │     │
                               └────────────────┘     │
                                       ▲              │
                                       │              │
                               canário e2e weekly     │
                               (instância destacada)  │
                                                      │
   Polymarket WSS, BCB, B3 ──────────────────────────┘  (mesmas deps externas
                                                         consumidas em prod)
```

Pipeline de release (do runbook): commit → CI → deploy staging (`artelonga-rfq-gateway`) → validação E2E + smoke-staging (cohort SMK*) → promote image para `rfq` por `flyctl deploy --image registry.fly.io/rfq:deployment-<DIGEST>`. Sem rebuild — mesma imagem cruza o gate.

**Nenhuma comunicação inter-task interna em 6PN** entre staging e prod. São deploys independentes da mesma imagem com secret-sets distintos.

---

## Riscos / TO BE

- **Stateful ring em volume per-machine** — escalar horizontal exige uma estratégia de merge offline dos rings (não há broker). Aceitável enquanto cada tier rodar 1 instância.
- **Canário `e2e-canary-weekly`** é uma máquina destacada do Fly Launch. Se for destruída por engano, recriar exige `flyctl machine run` manual, não `flyctl deploy`.
- **Dependência forte de provedores externos** (Hedix, Polymarket, BCB, B3). Sem fallback ou cache — outage de qualquer um degrada pricing.
- **DNS canônico `rfq.artelonga.com.br`** ainda não configurado. Hoje prod só responde em `rfq.fly.dev`.

---

Voltar ao [Catálogo](/infra) · [Universo co](/infra-co)
