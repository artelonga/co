---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 30
slug: infra
tags:
- infra
- catalogo
- fly
- compute
- transparencia
title: Catálogo de Infraestrutura
type: page
---

# ArteLonga — Catálogo de Infraestrutura

> Infra-as-a-Service interno. Inventário vivo das **tasks** (funções deployáveis e atômicas) que rodam em Fly.io, com decisões de design, custo e relações entre máquinas. Cada universo (projeto) é dono de uma ou mais tasks em ambientes distintos (prod/uat/dev).

**Snapshot:** 2026-05-13 · **Provedor único:** Fly.io · **Região única:** `gru` (São Paulo)

---

## 1. Vocabulário (compute terminology)

| Termo | Significado neste catálogo |
|---|---|
| **Universo** (project) | Repositório-fonte em `/Users/artelonga/projects/<nome>`. Pode produzir 1+ tasks. |
| **Task** (Fly app) | Unidade atômica deployável. 1 imagem Docker + 1 config (`fly.toml`). Identificada pelo nome do app Fly. |
| **Instância** (Fly machine) | Firecracker microVM que executa a task. Uma task pode ter N instâncias. |
| **Ambiente** (env) | Variante de uma task: `prod`, `uat`, `dev`. Tipicamente compartilha a imagem mas tem secrets e dimensionamento distintos. |
| **Stateful mount** (Fly volume) | Disco persistente attachado a uma instância. Não migra entre instâncias — uma instância = um volume. |
| **Edge** | Ingress público: TLS termination + roteamento. Provido pelo Fly LB. |
| **6PN** | Rede privada Fly entre apps da mesma org (`<app>.internal`). Sem TLS, sem firewall extra. |

---

## 2. Inventário (visão única)

| Task (Fly app) | Universo | Env | OS base | Runtime | VM | Mount | Estado | Doc |
|---|---|---|---|---|---|---|---|---|
| `co-artelonga` | [co](/infra-co) | prod | `debian:trixie-slim` | Rust 1.90 / Axum | shared-cpu-1x · 512 MB | `co_data` 1 GB | running | [infra-co](/infra-co) |
| `co-artelonga-uat` | [co](/infra-co) | uat | `debian:trixie-slim` | Rust 1.90 / Axum | shared-cpu-1x · 256 MB | `co_data` 1 GB | running | [infra-co](/infra-co) |
| `yggdrasil-artelonga` | [yggdrasil](/infra-yggdrasil) | prod | `debian:trixie-slim` | Rust 1.90 / Axum | shared-cpu-1x · 512 MB | `yggdrasil_data` 1 GB | running | [infra-yggdrasil](/infra-yggdrasil) |
| `quilombo-araucaria` | [quilomboaraucaria](/infra-quilomboaraucaria) | prod | `node:22-alpine` | Node 22 / SvelteKit | shared-cpu-1x · 2 048 MB | `quilombo_data` 10 GB | running | [infra-quilomboaraucaria](/infra-quilomboaraucaria) |
| `quilombo-araucaria-dev` | quilomboaraucaria | dev | `node:22-alpine` | Node 22 / SvelteKit | shared-cpu-1x · 256 MB | (volume parado) | **parked (suspended)** | [infra-quilomboaraucaria](/infra-quilomboaraucaria) |
| ~~`quilombo-araucaria-uat`~~ | quilomboaraucaria | uat | — | — | — | — | **destruído 2026-05-13** (app + 10 vols órfãos + IPs) | — |
| `artelonga-rfq-gateway` | [rfq-gateway](/infra-rfq-gateway) | **staging** | `debian:bookworm-slim` | Rust 1.91 / Axum | shared-cpu-1x · 256 MB | `rfq_artifacts` 1 GB ×2 | running (1/2) + canário e2e | [infra-rfq-gateway](/infra-rfq-gateway) |
| `rfq` | rfq-gateway | **production** | `debian:bookworm-slim` | Rust 1.91 / Axum | shared-cpu-1x · 256 MB | `rfq_artifacts` 1 GB | running | [infra-rfq-gateway](/infra-rfq-gateway) |
| `co-backup-cron` (planejado, CO-143) | co/infra | cron | (Dockerfile próprio) | shell + awscli | shared-cpu-1x · 256 MB | — | **não criado** | [infra-co §planejadas](/infra-co) |
| `co-clickhouse` (planejado, CO-123) | co/infra | analytics | ClickHouse image | ClickHouse | **performance-cpu · 8 GB / 4 vCPU** | `co_clickhouse_data` 50 GB | **não criado** | [infra-co §planejadas](/infra-co) |
| `co-clickhouse-export` (planejado, CO-123) | co/infra | cron | (Dockerfile próprio) | shell + http | shared-cpu-1x · 256 MB | — | **não criado** | [infra-co §planejadas](/infra-co) |

---

## 3. Camadas de uma task (do bare-metal ao endpoint)

Toda task no catálogo materializa as mesmas camadas. Documentar deliberadamente cada uma, em ordem, evita decisões implícitas.

1. **Hipervisor:** Firecracker (Fly). Não é configurável; é a fronteira de isolamento.
2. **Região:** `gru` (São Paulo). Escolhida por latência aos usuários BR e por compliance LGPD informal (dados nacionais permanecem em território nacional dentro do datacenter Fly, sem sair via replicação multi-região).
3. **Sistema operacional do container:**
   - Rust → `debian:trixie-slim` (glibc 2.40) para co-web, yggdrasil-web; `debian:bookworm-slim` (glibc 2.36) para rfq-gateway. **Por quê Trixie:** `ort-sys`/`fastembed` compila C++ que referencia `__isoc23_strtoull`/`__isoc23_strtol`, símbolos só presentes a partir de glibc 2.38; manter builder + runtime no mesmo `glibc` é mandatório (CO-164). rfq-gateway não tem essas deps → permanece em Bookworm enxuto.
   - Node → `node:22-alpine`. Alpine (musl) escolhido pelo footprint (~5x menor) e por o stack Node/SvelteKit não tocar bindings glibc-only — exceção é `sharp`, instalado explicitamente como `--libc=musl --cpu=x64` (ver Dockerfile do quilombo).
4. **Toolchain de build:** sempre **multi-stage**. Builder pesado (rustc/Cargo, npm) descartado; runtime carrega só o binário/bundle. Princípio: superfície de ataque mínima, imagem reproduzível.
5. **Runtime de aplicação:** binário Rust estático (Axum) ou `node build/index.js` (SvelteKit). Tasks Rust são single-binary; o overhead de cold-start é ~50 ms vs ~10 s do Node (motivo de `min_machines_running = 1` no quilombo-araucaria).
6. **Volume:** Fly Volumes (LUKS-encrypted by default). **Acoplado a uma instância** — Fly não permite multi-attach. Implicação: scaling horizontal só funciona se o estado estiver fora do volume (S3, DB remoto) ou se houver camada de replicação (LiteFS, ver CO).
7. **Ingress:** Fly LB termina TLS, encaminha HTTP/HTTPS ao `internal_port`. IPv6 público dedicado por task (gratuito); IPv4 compartilhado (gratuito). `force_https = true` em todas.
8. **Egress:** sem NAT específico; IP de saída rotaciona dentro do pool Fly. Importante para integrações que exigem allowlist por IP — hoje nenhuma das tasks faz isso.

---

## 4. Topologia de comunicação entre tasks

Hoje **todas as tasks são ilhas** — não há tráfego direto entre apps Fly. Os pontos de contato existentes são:

```
                   ┌──────────────────┐
   usuário (HTTPS) │ co-artelonga     │──┐
   ─────────────►  │ (Edge, 443)      │  │
                   └──────────────────┘  │
                                          │ sync HTTP (push)
                   ┌──────────────────┐  │
                   │ co-artelonga-uat │◄─┘ (UAT_MIRROR_PROD + UAT_PROD_TOKEN)
                   └──────────────────┘

   usuário (HTTPS) ┌──────────────────┐
   ─────────────►  │ quilombo-araucaria│  ── chama webhook ─►  co-artelonga
                   └──────────────────┘     (SYNC_ENABLED, SYNC_TOKEN)

   usuário (HTTPS) ┌──────────────────┐
   ─────────────►  │ artelonga-rfq-gw │  ── HTTPS ─►  Hedix API (externo)
                   └──────────────────┘  ── WSS  ─►  Polymarket WS
                                          ── HTTPS ─►  BCB Olinda / B3
                                          (imagem idêntica à task `rfq`)

   usuário (HTTPS) ┌──────────────────┐
   ─────────────►  │ yggdrasil-artelonga│  (standalone — não fala com co em runtime;
                   └──────────────────┘    depende de co/game-core só em build-time)
```

**Comunicação pendente, prevista pela arquitetura `co/infra`** (não implementada hoje):

- `co-clickhouse-export.internal` (cron) → `co-clickhouse.internal:8123` via 6PN (HTTP, sem TLS, restrito à org).
- `co-artelonga` → `co-clickhouse.internal:9000` via 6PN para queries operacionais (CO-123).
- `co-backup-cron` → S3 externo (`s3://artelonga-co-backups`) + Fly Machines API (precisa de `FLY_API_TOKEN`).

**Princípios de comunicação** (alinhados ao modelo de ameaças em [Segurança](/seguranca)):

- **Ingress público:** somente via Fly LB, sempre TLS 1.2+. HSTS, X-Frame-Options DENY, CSP defaultSrc 'self', CORS por allowlist (`*.artelonga.com.br`).
- **Inter-app interno:** 6PN (`<app>.internal`). Sem TLS, mas isolado por org. Aceita-se sem TLS para comunicação dentro do datacenter Fly por inspeção do modelo de ameaças.
- **Secrets:** sempre `flyctl secrets` (encrypted at rest, runtime injection). Nunca em `[env]` do `fly.toml`. Chave por uso (JWT, VAPID, Resend, MaxMind, recovery) — sem reuse.
- **Auth entre serviços** (UAT↔prod, Quilombo↔CO): tokens bearer estáticos por enquanto (`UAT_PROD_TOKEN`, `SYNC_TOKEN`). Não há mTLS nem rotação automatizada — lacuna conhecida.

---

## 5. Custo (estimativa AS IS)

Fly cobra por **segundo de execução** (auto-stop machines pagam só quando rodam) + storage + bandwidth. Estimativas abaixo assumem o pior caso: máquina **sempre ligada** o mês inteiro. Tarefas com `auto_stop_machines = "stop"` pagam significativamente menos na prática.

### 5.1 Compute (preços de tabela Fly — valores aproximados, USD/mês)

| Tamanho VM | shared-cpu-1x · 256 MB | 512 MB | 1024 MB | 2048 MB | performance-2x · 8 GB / 4 vCPU |
|---|---|---|---|---|---|
| 24/7 estimado | ~$2 | ~$4 | ~$8 | ~$16 | ~$130–160 |
| Auto-stop ativo (8 h/dia eq.) | ~$0,70 | ~$1,30 | ~$2,60 | ~$5,30 | n/a (sempre ligado) |

### 5.2 Storage (volumes encrypted)

$0,15 / GB-mês. Inventário atual: 1 + 1 + 1 + 10 + 1 + 1 = **15 GB de prod** = ~$2,25/mês. UAT/dev volumes adicionam ~$0,30/mês cada. Volumes planejados: `co_clickhouse_data` 50 GB → +$7,50/mês.

### 5.3 IPs e bandwidth

- IPv6 dedicado por app: **grátis** (já aplicado a todas).
- IPv4 compartilhado: **grátis** (todas usam o pool).
- IPv4 dedicado: $2/mês cada — **não usado hoje**.
- Bandwidth: 100 GB egress incluso/mês na free tier (GRU $0,02/GB acima). Tráfego atual abaixo do limite.

### 5.4 Estimativa AS IS (sempre-ligado, upper bound)

| Universo | Total compute | Storage | Total mensal teto |
|---|---|---|---|
| co (prod + uat) | $4 + $2 = $6 | $0,30 | **~$6,30** |
| yggdrasil | $4 | $0,15 | **~$4,15** |
| quilomboaraucaria (prod só) | $16 | $1,50 | **~$17,50** |
| rfq-gateway (staging 24/7 + prod 24/7 + canário) | $2×3 = $6 | $0,30 | **~$6,30** |
| **Total AS IS** | **~$30** | **~$2,25** | **~$34/mês** |

Com auto-stop ativo nas tasks que permitem (`co-artelonga`, `yggdrasil-artelonga`, e `artelonga-rfq-gateway`/`rfq` quando ociosas), o realizado deve ficar entre **$15–25/mês**.

> **Limpeza 2026-05-13:** destruição de `quilombo-araucaria-uat` removeu **10 volumes órfãos** de 1 GB. Economia bruta: ~$1,50/mês. Sem impacto em prod (já estava sem máquinas).

### 5.5 Custo TO BE (se shippar co/infra completo)

| Adicionar | Compute | Storage | Mensal |
|---|---|---|---|
| `co-backup-cron` (1× /dia, ~5 min) | <$0,10 | — | ~$0,10 |
| `co-clickhouse` (performance, 24/7) | ~$140 | $7,50 (50 GB) | **~$147,50** |
| `co-clickhouse-export` (cron) | <$0,10 | — | ~$0,10 |
| IPv4 dedicado (se necessário) | — | — | $2 cada |

ClickHouse é o passo mais caro do roadmap — quase **5× o orçamento mensal atual**. Decisão estratégica explícita: WAE (Cloudflare Analytics Engine) cobre o caso de uso curto-prazo; ClickHouse só justifica-se quando volume de eventos > 10 M/dia.

---

## 6. Convenções de catálogo (para crescer)

- **Um arquivo por universo** em `universos/<slug>.md`. Cada arquivo segue o template em §7.
- **Apps suspended/sem-máquinas permanecem no catálogo** — desligar não significa apagar, e a documentação ajuda a reativar.
- **Toda task tem um repo de origem** — se faltar, ou registrar como "não-rastreável" (action item), ou criar.
- **Mudanças de dimensionamento** (memória, CPU, volume) **devem citar a causa** no `fly.toml` — exemplo de ouro: `quilombo-araucaria` documenta `# 412 MB upload double-buffers during parse — 1 GB OOM'd at 866 MB rss`.
- **Não duplicar conteúdo de [Segurança](/seguranca)** — referenciar. Este catálogo é sobre máquinas; aquele é sobre o modelo de ameaças.

## 7. Template para nova task

```markdown
# <nome-do-app-fly>

- **Universo:** <repo path>
- **Env:** prod | uat | dev | cron | analytics
- **Status:** running | suspended | planejado

## OS + Runtime
- Imagem base e por quê (glibc, musl, ABI...)
- Toolchain de build (multi-stage?)

## Dimensionamento
- VM size + justificativa empírica (OOM observado, latência, etc.)
- Volume size + por quê
- Auto-stop policy + min instances

## Ingress
- Internal port, healthcheck path, concurrency limits

## Comms
- Quem chama esta task (auth)
- Quem esta task chama (auth, dependências externas)
- Secrets necessários (lista, sem valores)

## Custo
- Estimativa upper-bound (24/7) + realista (com auto-stop)

## Riscos / TO BE
- Lacunas conhecidas
```

---

## 8. Action items derivados deste inventário

1. ~~**`artelonga-dev` (universo `ArteLonga`)**~~ — **resolvido 2026-05-13**: `fly.toml` removido do repo. Sem footprint em Fly.
2. ~~**`quilombo-araucaria-uat`**~~ — **resolvido 2026-05-13**: app + 10 volumes órfãos + IPs destruídos via `flyctl apps destroy`.
3. **`quilombo-araucaria-dev`** — decisão tomada: **parked**. Permanece suspended (zero custo de compute) e o volume de 1 GB (~$0,15/mês) é aceito como tradeoff pela opção de reativar sem rebuild de schema. Não destruir.
4. ~~**`rfq` vs `artelonga-rfq-gateway`**~~ — **clarificado**:
   - `artelonga-rfq-gateway` = **staging** (`RFQ_ENV=staging`) + **smoke-staging** (cohort SMK*-prefix, `RFQ_ENV=smoke-staging`) — lane de validação contra Hedix staging.
   - `rfq` = **production** (`RFQ_ENV=production`) — Hedix prod real, customer flow.
   Um único codebase, dois deploys, três env-tiers. Detalhes em [infra-rfq-gateway](/infra-rfq-gateway).
5. **CO-143 backup-cron** — DR daily snapshot. Mecânica em [infra-co §planejadas](/infra-co): cron Alpine + busybox que roda `flyctl ssh` para `sqlite3 .backup` e `tar czf` do diretório `universes/`, e faz upload para S3 (`artelonga-co-backups`). Status: app **não criado em Fly**, mas Dockerfile + entrypoint + script `scripts/backup-prod.sh` prontos. Bloqueio de release stable.
6. **ADRs formais** — migrar decisões deste catálogo (ex: por que Trixie, por que `auto_stop_machines`) para `docs/decisions/`.

---

## Páginas relacionadas

- [Segurança](/seguranca) — modelo de ameaças e camadas de defesa
- [Dependências](/seguranca-dependencias) — bibliotecas, decisões, custos cripto
- [Cenários de Red Team](/seguranca-cenarios) — ataques considerados + playbook
- Tasks por universo: [infra-co](/infra-co) · [infra-yggdrasil](/infra-yggdrasil) · [infra-quilomboaraucaria](/infra-quilomboaraucaria) · [infra-rfq-gateway](/infra-rfq-gateway)

## Links cross-universe (novo)

Co suporta links que atravessam universos com a sintaxe `[[/universo/caminho]]`. Exemplo:

- `[[/template/infra]]` aponta para esta página
- `[[/template/seguranca]]` aponta para a página de segurança

A regra: se o universo é público (como `template`), qualquer um vê. Se é privado e você não tem permissão, o link retorna 404 — silenciosamente. **Cuidado ao expor links de repos privados em conteúdo público** — o link em si não vaza dados, mas vaza a existência do path.
