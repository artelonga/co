# Dependências — A: catálogo

Lista completa de dependências de runtime do servidor CO + frontend.
Para o **porquê** de cada escolha sensível à segurança, ver
[dependency-decisions.md](dependency-decisions.md) (a versão B,
narrativa).

Atualizado: 2026-05-13 (CO 2.6.1).

## Estrutura

CO é arquiteturado em camadas, cada uma com seu próprio universo de
deps. Esta página lista por camada:

1. [Compilador e runtime base](#1-compilador-e-runtime-base)
2. [Web framework e HTTP](#2-web-framework-e-http)
3. [Banco de dados](#3-banco-de-dados)
4. [Criptografia e auth](#4-criptografia-e-auth)
5. [Notificações e mensageria](#5-notificações-e-mensageria)
6. [Telemetria e observabilidade](#6-telemetria-e-observabilidade)
7. [Frontend SPA (vanilla JS)](#7-frontend-spa-vanilla-js)
8. [Ferramentas de build / dev](#8-ferramentas-de-build--dev)
9. [Infraestrutura externa](#9-infraestrutura-externa)

Cada entrada lista versão pinada, papel, e onde no código vive.

---

## 1. Compilador e runtime base

| Crate / Tool | Versão | Papel |
|---|---|---|
| **rustc** | 1.94.0 (pinado em `rust-toolchain.toml`) | Compilador. Pinado para evitar lints novas surpreendendo CI (CO-208 history) |
| **tokio** | workspace | Runtime async multi-thread (work-stealing scheduler) |
| **anyhow** | workspace | Error handling para code paths não-tipados |
| **serde** + **serde_json** | workspace | (De)serialização JSON em todos os endpoints |
| **chrono** | workspace | Date/time com timezone-aware operations |
| **uuid** | workspace | IDs únicos (room_id, message_id, etc.) |
| **nanoid** | workspace | IDs curtos URL-safe (token_hash, etc.) |

## 2. Web framework e HTTP

| Crate | Versão | Papel |
|---|---|---|
| **axum** | 0.8 (features: ws) | Web framework — routing, extractors, middleware |
| **tower** + **tower-http** | latest | Middleware: CORS, compression, tracing, request ID, response headers |
| **hyper** | via axum | HTTP/1.1 + HTTP/2 server |
| **tokio-tungstenite** | 0.26 (rustls-tls-webpki-roots) | WebSocket — chat (CO-194), CRDT docs (CO-150), sync (CO-151) |
| **rustls** | via tokio-tungstenite | TLS implementation in pure Rust (no OpenSSL link) |
| **reqwest** | latest | HTTP client (push delivery, webhooks, JWKS fetch) |

## 3. Banco de dados

| Crate | Versão | Papel |
|---|---|---|
| **rusqlite** | latest | SQLite wrapper. Single-file embedded DB at `/data/co.db` |
| **parking_lot** | 0.12 | Mutex implementation **without poison semantics** (CO-203). Protects shared `Storage` |

**Características operacionais do SQLite:**

- Modo WAL (write-ahead log) habilitado em produção
- Single-writer, multi-reader concurrency
- Schema versioning via `schema_version` table; migrations idempotentes via `ensure_table` / `ensure_column` (ver `docs/ARCHITECTURE.md`)

## 4. Criptografia e auth

| Crate | Versão | Papel | Por quê |
|---|---|---|---|
| **argon2** | latest (variant id) | Hash de senhas + códigos de recuperação | Resistente a ASIC/GPU (memory-hard). Ver dependency-decisions.md §1 |
| **chacha20poly1305** | latest | AEAD (Authenticated Encryption) para `user_recovery_channels.value_ciphertext` | Stream cipher, sem padding oracle, performance constante. dependency-decisions.md §2 |
| **blake3** | latest | Hash criptográfico para lookup hashes de canais de recuperação | Mais rápido que SHA-256, segurança equivalente |
| **sha2** | latest | SHA-256 para `ip_hash`, hashes de tokens de convite | Padrão Internet, escolhido pela compatibilidade |
| **p256** (`ecdh`) | 0.13 | Curva elíptica P-256 para VAPID + handover ES256 (CO-186) + AES-128-GCM key derivation em web push (CO-201) |
| **jsonwebtoken** | latest | Assinatura/validação JWT (HS256 para sessão; ES256 para handover) |
| **base64** | 0.22 (URL_SAFE_NO_PAD) | Codificação base64url para JWT, VAPID keys, push payloads |
| **aes-gcm** | 0.10 | AES-128-GCM payload encryption em web push (RFC 8188/8291) |
| **hkdf** | 0.12 | HKDF-SHA256 para derivar chaves AES de inputs combinados (push subscription keys) |
| **rand** | 0.8 (OsRng) | RNG criptograficamente seguro para gerar chaves, códigos, tokens |

## 5. Notificações e mensageria

| Crate | Versão | Papel |
|---|---|---|
| **lettre** | 0.11 | SMTP client (fallback quando Resend não configurado) |
| **web-push** | indireto via implementação manual com p256+aes-gcm+hkdf | Web Push (CO-201) — RFC 8291 envelope |

**Serviços externos:**

- **Resend** (HTTP API) — email transactional primário. `RESEND_API_KEY` env. Sender `senhas@seguranca.artelonga.com.br`, `notificacoes@…` (CO-200)
- **Evolution API** (HTTP) — WhatsApp delivery opcional (CO-169). `EVOLUTION_API_KEY` env, `EVOLUTION_INSTANCE` config

## 6. Telemetria e observabilidade

| Crate | Versão | Papel |
|---|---|---|
| **tracing** + **tracing-subscriber** | latest | Structured logging. JSON output em prod |
| **maxminddb** | latest | Parser do `.mmdb` para geo enrichment (CO-178) |

**Bases externas:**

- **MaxMind GeoLite2-City** — `/data/GeoLite2-City.mmdb` (~66 MB), atualizável via `geoipupdate` com `MAXMIND_LICENSE_KEY`. Atribuição obrigatória (incluída em `docs/analytics-api.md`)

## 7. Frontend SPA (vanilla JS)

CO frontend é **vanilla ES modules, zero npm runtime deps**. Arquivos
servidos diretamente de `co-web/static/`.

| Conceito | Implementação | Por quê |
|---|---|---|
| **Module loading** | Native `<script type="module">` | Sem bundler em produção; código permanece auditável |
| **State management** | Single `state.js` module com object | Sem React/Vue/Svelte; menos surface area |
| **Reactivity** | Manual re-render on event handlers + WS messages | Trade-off: mais código, zero dependências |
| **Markdown rendering** | Mini parser custom em `modules/views/conteudo.js` | Subset CommonMark (bold/italic/code/link); evita marked.js + sanitizer deps |
| **Date formatting** | `Intl.DateTimeFormat` + `Intl.RelativeTimeFormat` | Built-in; sem moment.js |
| **i18n** | `co-web/static/shared/i18n.js` (PT-BR + EN) | Object lookup; sem i18next |

**Tests (frontend):**

- **Playwright** — E2E. Gated em CI via `CO_BYPASS_RATE_LIMIT=1 + CO_ENV=test` (CO-208)

## 8. Ferramentas de build / dev

| Tool | Papel | Onde |
|---|---|---|
| **cargo** | Build orchestration | All crates |
| **clippy** | Lint | CI gate (cargo clippy --workspace -- -D warnings) |
| **rustfmt** | Format | CI gate (cargo fmt --all -- --check) |
| **protoc** | Compila protobuf schemas (CO-151 sync, CO-150 wire formats) | CI installs via apt; Dockerfile builder stage |
| **co-auto** | Internal task automation (`dev/co-auto/`) | Optional, dev-only |
| **co-pwhash** | Argon2 helper para gerar password_hash de admin seeds | Optional, dev-only |
| **co-token** | Gera tokens de teste para co-cli | Optional, dev-only |

## 9. Infraestrutura externa

| Serviço | Papel | Notas |
|---|---|---|
| **Fly.io** | Compute + edge + block storage + secrets + DNS + healthcheck | Provedor atual. Substituível — ver [SECURITY.md §Portabilidade](SECURITY.md#portabilidade-entre-provedores) |
| **GitHub** | Source control + CI + container registry (GHCR) | Substituível por GitLab / Gitea / Forgejo + qualquer registry |
| **MaxMind GeoLite2** | Geo IP database | Free tier requer signup + license key |
| **Resend** | Transactional email | Substituível por qualquer SMTP-capable provider |
| **Google OAuth** | Federated identity opcional | Substituível ou desabilitável; CO suporta email-code login standalone |

---

## SBOM completo (todas as deps transitivas)

Para um SBOM machine-readable incluindo todas as deps transitivas:

```bash
cargo install cargo-cyclonedx
cargo cyclonedx --format json --output-pattern bom-{name}-{version}.cdx.json
# Gera CycloneDX SBOM por crate
```

CycloneDX é o formato de SBOM padrão da indústria (NTIA, EO 14028).
O resultado pode ser consumido por scanners como Dependency-Track,
Trivy, Snyk para CVE matching contínuo.

A versão pinada deste arquivo cobre os deps com **papel relevante para
segurança** — manuseio de credenciais, criptografia, validação de
input, ou exposição de superfície externa. Outras deps (chrono, uuid,
nanoid, etc.) são úteis mas não fazem decisões de segurança.

---

## Política de atualização

| Categoria | Cadência |
|---|---|
| Crates criptográficos (argon2, chacha20poly1305, p256, etc.) | Atualizar imediatamente em CVE; revisar trimestralmente |
| jsonwebtoken | Atualizar imediatamente em CVE |
| rusqlite | Acompanhar major release; atualizar uma vez por trimestre |
| tokio, axum | Acompanhar minor versions; atualizar quando feature útil |
| Outras | Quando há motivo (bug fix, feature) |

CVE monitoring: `cargo audit` em CI (TO BE — não habilitado hoje) +
GitHub Dependabot security alerts (habilitado).
