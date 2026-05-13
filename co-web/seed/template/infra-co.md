---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 31
slug: infra-co
tags:
- infra
- co
- fly
- compute
- backup
title: 'Infra: universo co'
type: page
---

# Universo: `co`

Repositório: `/Users/artelonga/projects/co` · Stack: Rust + Axum + SQLite (LiteFS) · CLI + servidor web.

> CO é a plataforma central — autenticação, telemetria, universos editáveis, chat. Modelo de ameaças e arquitetura completa em [Segurança](/co/template?page=seguranca). Este documento descreve só as **tasks** que materializam CO em produção.

Voltar ao [Catálogo](/co/template?page=infra).

---

## Tasks ativas

### `co-artelonga` (prod)

- **Status:** running · 1 instância em `gru` · versão 229 · last deploy 2026-05-13.
- **Fonte:** `fly.toml` no root do repo.

**OS + runtime**

- Base: `debian:trixie-slim`. Trixie é mandatório (não cosmético) porque `ort-sys` (via `fastembed`, CO-164) compila C++ que referencia `__isoc23_strtoull`/`__isoc23_strtol`, símbolos só presentes a partir de **glibc 2.38**. Bookworm (2.36) quebra no link. Builder e runtime precisam casar — daí o pin nos dois estágios.
- Toolchain de build: multi-stage. Builder com `rust:1.90-slim-trixie` + `protobuf-compiler` (para os schemas wire CO-150/CO-151) + `libssl-dev`. Runtime mantém só `ca-certificates`, `curl` e `git` (necessário para `geoipupdate` futuro).
- Binário: `co-web` único. Entrypoint passa por **LiteFS** (FUSE mount em `/data`, replicação Consul-leased) antes de spawnar o processo — ver `litefs.yml`.

**Dimensionamento**

- VM: `shared-cpu-1x · 512 MB`. Caso normal usa ~200 MB; pico (rebuild de índice MaxMind + WS chat ativo) chega a ~400 MB.
- Volume: `co_data` 1 GB (LUKS encrypted). Contém: `co.db` (SQLite WAL), `meta.db` (LiteFS), `GeoLite2-City.mmdb` (~66 MB), assets per-universe.
- Auto-stop: `"stop"` com `min_machines_running = 0`. Cold-start aceitável (~50 ms) porque é binário Rust. Custo amortizado significativo — a maioria das horas a task não está rodando.
- Health: `/api/health` a cada 30s, grace 90s.

**Ingress**

- `internal_port = 3000`. Concurrency: hard 100 / soft 80 conexões. CSRF via origin allowlist em `csrf_middleware` (CO-205).
- DNS público: `co-artelonga.fly.dev` + `co.artelonga.com.br` (CNAME externo).
- IPv6 dedicado: `2a09:8280:1::f0:15dd:0`. IPv4 compartilhado: `66.241.125.207`.

**Comms**

- **Inbound:**
  - Usuários via HTTPS (Edge Fly LB).
  - `artelonga.com.br` (Quartz site público) → CORS allowlist via `mirror_request` para postar `marketing_events` (CO-177).
  - UAT mirror endpoint (recebe push de prod via `UAT_PROD_TOKEN`).
- **Outbound:**
  - **Resend** (`RESEND_API_KEY`) — email transacional (`senhas@`, `notificacoes@`).
  - **Google OAuth** (`GOOGLE_CLIENT_ID/SECRET`) — federated login.
  - **MaxMind** geoipupdate (`MAXMIND_LICENSE_KEY`) — atualização semanal do `.mmdb`.
  - **Web Push** providers (FCM, APNS via VAPID) — `VAPID_*` keys (ver [VAPID](/co/template?page=seguranca-vapid)).
  - **Evolution API** (opcional, WhatsApp) — `EVOLUTION_API_KEY` se configurado.

**Secrets em uso** (digest only, ver `flyctl secrets list -a co-artelonga`):
`JWT_SECRET`, `CO_SEED_ADMIN_EMAIL`, `CO_SEED_ADMIN_PASSWORD_HASH`, `CO_ASSETS_MASTER_KEY`, `RESEND_API_KEY`, `RESEND_FROM`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `VAPID_PRIVATE_KEY`, `VAPID_PUBLIC_KEY`, `VAPID_SUBJECT`, `MAXMIND_LICENSE_KEY`.

**Custo (upper-bound 24/7):** ~$4/mês compute + $0,15 volume = **~$4,15/mês**. Com auto-stop ativo: estimo **~$1,30–2,00/mês** realizado.

---

### `co-artelonga-uat` (uat)

- **Status:** running · 1 instância em `gru` · versão 120 · last deploy 2026-05-02.
- **Diferenças vs prod:** `[[vm]] memory = "256mb"`, `auto_stop_machines = false` (UAT precisa estar sempre ligado para receber sync push do prod), grace period 30s (menor), env extra `CO_ENV = "uat"`.

**Comms específico de UAT**

- Recebe sync push de prod via secrets `UAT_MIRROR_PROD`, `UAT_PROD_TOKEN`, `UAT_PROD_URL`. **Token estático** — sem rotação. Risk: lacuna conhecida em [Segurança](/co/template?page=seguranca).
- Não tem secrets de provedores externos (Resend, Google, MaxMind) — UAT roda em modo "isolado": emails são logados, não enviados; OAuth desabilitado.

**Custo:** ~$2/mês compute + $0,15 volume = **~$2,15/mês**. Sem auto-stop → realizado ≈ upper-bound.

---

## Tasks planejadas (em `co/infra/`)

Tudo abaixo tem `fly.toml` no repo mas **nenhum app criado em Fly** ainda. Documentado para definir o orçamento e a ordem de roll-out.

### `co-backup-cron` (CO-143) — DR daily snapshot

- **Papel:** task de cron diária que captura `co.db` + `universes/` da máquina prod e sobe para S3.
- **Dimensão:** `shared-cpu-1x · 256 MB`, sem volume. Roda alguns minutos/dia → custo desprezível (~$0,10/mês).
- **Status:** **app não criado em Fly**. Repo tem `Dockerfile`, `entrypoint.sh`, `fly.toml`, e o script real em `co/scripts/backup-prod.sh`. Falta `flyctl apps create co-backup-cron` + set de secrets + `flyctl deploy`.

**Mecânica concreta** (linha-a-linha de `scripts/backup-prod.sh`, baseada em `flyctl ssh`):

1. **Snapshot SQLite via `.backup` command** — atômico, sem lock (SQLite copia páginas consistentes mesmo com WAL ativo):
   ```
   flyctl ssh console -a co-artelonga -C "sqlite3 /data/co.db '.backup /tmp/co.db.bak'"
   flyctl sftp get  -a co-artelonga /tmp/co.db.bak $WORK/co-$DATE.db
   ```
2. **Tar `universes/` directory:**
   ```
   flyctl ssh console -a co-artelonga -C "tar czf /tmp/universes.tar.gz -C /data universes"
   flyctl sftp get  -a co-artelonga /tmp/universes.tar.gz $WORK/universes-$DATE.tar.gz
   ```
3. **Upload PUT-idempotente para S3:**
   ```
   aws s3 cp $WORK/co-$DATE.db          s3://artelonga-co-backups/co.db/$DATE.db
   aws s3 cp $WORK/universes-$DATE.tar.gz s3://artelonga-co-backups/universes/$DATE.tar.gz
   ```

**Cron schedule:** `17 3 * * *` UTC (03:17 — off-minute para espalhar load). Configurado no Dockerfile via `crond -f -d 8` (busybox `crond`, foreground, debug level 8).

**Imagem:** `alpine:3.21` + `bash`, `busybox-suid` (para crond), `curl`, `aws-cli`, `sqlite`, `tar`, e o `flyctl` instalado via `curl https://fly.io/install.sh`. Path adicionado a `/root/.fly/bin`.

**Comms outbound:**
- **Fly Machines API** via `flyctl ssh`/`sftp` (precisa `FLY_API_TOKEN` com escopo de read+exec na app `co-artelonga`).
- **AWS S3** (`AWS_ACCESS_KEY_ID/SECRET`, default region `us-east-1`, bucket `artelonga-co-backups`).

**Layout S3 resultante:**
```
s3://artelonga-co-backups/
├── co.db/YYYYMMDD-HHMMSS.db          ← snapshot SQLite
└── universes/YYYYMMDD-HHMMSS.tar.gz  ← markdown + assets per-universe
```

**Encryption strategy:** S3 bucket-level (SSE-S3 ou SSE-KMS — definido por `co/infra/s3/setup.sh` + `lifecycle.json`). Chave **separada** da `CO_RECOVERY_KEY` / `CO_ASSETS_MASTER_KEY` que cifram dados na app — princípio de key-separation. Restore só requer o bucket + acesso à conta AWS.

**Por quê isso é o gating de release stable:**
- Volume Fly é LUKS-encrypted mas single-region — falha de datacenter `gru` = perda total sem off-site.
- Sem snapshot diário, RPO (Recovery Point Objective) é "última hora em que alguém lembrou de rodar `sqlite3 .backup` manualmente". Não é DR; é deixar a recuperação ao acaso.
- O caminho hot-path (`co-artelonga`) **não muda** com este deploy; CO-143 é estritamente aditivo e baixo-risco. O atraso é falta de tempo, não obstáculo técnico.

**Restore drill (a documentar):** sem isso, não há garantia de que o backup é restaurável. TO BE: drill trimestral.

### `co-clickhouse` (CO-123) — analytics warehouse

- **Papel:** node ClickHouse single-instance para queries ad-hoc + WAE bridge.
- **Dimensão (no `fly.toml`):** `performance-cpu · 4 vCPU · 8 GB · 50 GB volume`. **Maior task do catálogo por uma ordem de grandeza.**
- **Custo estimado:** ~$140/mês compute + $7,50 volume = **~$147,50/mês**.
- **Ingress:** **sem `[http_service]`** — acesso interno-only via 6PN (`co-clickhouse.internal:8123` HTTP, `:9000` native). Admin local via `flyctl proxy`.
- **Por quê tão grande:** ClickHouse cold-start + working set de queries OLAP justificam memória dedicada. Auto-stop não se aplica.
- **Decisão de orçamento ainda não tomada** — WAE (Cloudflare Analytics Engine) atende o caso atual. ClickHouse só rampa quando volume justifica (>10M eventos/dia).

### `co-clickhouse-export` (CO-123) — cron WAE→CH

- **Papel:** cron diário; baixa eventos de ontem da Cloudflare Analytics Engine SQL API e insere em ClickHouse.
- **Dimensão:** `shared-cpu-1x · 256 MB`. Custo desprezível.
- **Comms outbound:** Cloudflare (`CF_ACCOUNT_ID`, `CF_API_TOKEN`) + `co-clickhouse.internal:8123` via 6PN.
- **Decisão:** só acionar quando ClickHouse estiver de pé.

### Subdirs sem `fly.toml` (não destinados a Fly)

- `co/infra/cloudflare/` — Terraform para configurar CDN/Workers/WAE. Provedor externo.
- `co/infra/minio/` — `docker-compose.yml` para MinIO local de dev (substituto offline do S3).
- `co/infra/s3/` — `lifecycle.json` + `setup.sh` para criar bucket de backup com retention policy.

---

## Relação entre máquinas (resumo)

```
co-artelonga (prod)
   │
   │  push sync HTTP (token bearer)
   ▼
co-artelonga-uat
```

Outras tasks externas que **consomem** CO:
- `quilombo-araucaria` envia eventos via webhook (`SYNC_TOKEN`).
- `artelonga.com.br` (site Quartz, fora deste catálogo) envia telemetry para `/marketing_events`.

CO **não chama** nenhuma outra task interna em runtime hoje. As deps planejadas (ClickHouse) seriam via 6PN.

---

## Riscos / TO BE explícitos

- **DR ausente** (CO-143 não shipado). Único snapshot é o volume LUKS — sem off-site backup.
- **Single region** — falha do datacenter `gru` é total outage. Migração planejada não tem ETA.
- **Tokens UAT/Sync sem rotação** — `UAT_PROD_TOKEN`, `SYNC_TOKEN` são estáticos.
- **Sem WAF/CDN** — Cloudflare planejado (CO-111) ainda não na frente.
- **LiteFS rodando solo** — `lease.type = consul` está configurado mas só há 1 instância. Quando escalar horizontal, o Consul-lease torna-se load-bearing — testar antes.

---

Voltar ao [Catálogo](/co/template?page=infra) · [Segurança](/co/template?page=seguranca) · [Dependências](/co/template?page=seguranca-dependencias)
