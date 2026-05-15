---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 11
slug: seguranca-dependencias
tags:
- seguranca
- dependencias
- transparencia
title: Dependências
type: page
---

# Dependências

Cada biblioteca que Co usa foi escolhida deliberadamente. Esta página lista as **dependências relevantes para segurança** + explica o **por quê** de cada decisão.

Voltar para [Segurança](/seguranca).

## Sumário

- [Stack base](#stack-base)
- [Web + HTTP](#web--http)
- [Banco de dados](#banco-de-dados)
- [Criptografia](#criptografia)
- [Email + notificações](#email--notificações)
- [Frontend](#frontend)
- [Decisões importantes (por quê?)](#decisões-importantes-por-quê)

## Stack base

| Biblioteca | Função |
|---|---|
| **rustc 1.94 pinada** | Compilador. Pinado para evitar lints novas surpreendendo CI. |
| **tokio** | Runtime async multi-thread |
| **serde + serde_json** | (De)serialização JSON em todos os endpoints |

## Web + HTTP

| Biblioteca | Função |
|---|---|
| **axum** | Web framework — routing, extractors, middleware |
| **tower-http** | Middleware: CORS, compression, tracing, request ID |
| **rustls** | TLS em Rust puro (sem dependência OpenSSL) |
| **tokio-tungstenite** | WebSocket — chat, CRDT docs, sync |
| **reqwest** | HTTP client para push delivery, webhooks, JWKS fetch |

## Banco de dados

| Biblioteca | Função |
|---|---|
| **rusqlite** | SQLite — single-file embedded DB |
| **parking_lot** | Mutex **sem semântica de poison** (CO-203) — protege Storage compartilhado sem cascata de falha |

## Criptografia

| Biblioteca | Função | Por quê |
|---|---|---|
| **argon2** | Hash de senhas e códigos | Memory-hard, resistente a GPU/ASIC. Ver "Decisões" abaixo |
| **chacha20poly1305** | AEAD para `user_recovery_channels.value_ciphertext` | Constant-time, independente de AES-NI |
| **blake3** | Hash criptográfico para lookups | 3-5x mais rápido que SHA-256 |
| **sha2** | SHA-256 para ip_hash, tokens de convite | Padrão Internet |
| **p256** | Curva P-256 (ECDSA + ECDH) para VAPID + JWKS + push payload encryption |
| **jsonwebtoken** | Assinatura/validação JWT |
| **rand** com OsRng | RNG criptograficamente seguro |

## Email + notificações

| Biblioteca | Função |
|---|---|
| **lettre** | SMTP client — fallback quando Resend indisponível |
| **Resend** (serviço externo) | Email transactional primário |

## Frontend

Co frontend é **vanilla ES modules, zero npm em runtime**.

| Conceito | Implementação | Por quê |
|---|---|---|
| Module loading | Native `<script type="module">` | Sem bundler em produção; código auditável |
| State management | Single `state.js` module | Sem React/Vue/Svelte; menor surface area |
| Markdown rendering | Parser custom em `modules/views/conteudo.js` | Evita marked.js + sanitizer deps |
| i18n | Object lookup em `i18n.js` | Sem i18next |

## Decisões importantes (por quê?)

### 1. Argon2id em vez de bcrypt

bcrypt tem 56-byte input limit, não é memory-hard, e GPUs aceleram linearmente. Argon2id é memory-hard, sem truncamento, e foi vencedor do Password Hashing Competition. Recomendação oficial PHC.

Parâmetros atuais: `m=19 MiB, t=2, p=1`. Revisão anual — bumpamos para `m=64 MiB` quando hardware típico suportar.

### 2. ChaCha20-Poly1305 em vez de AES-GCM

ChaCha20 tem performance constante (~1 GB/s em qualquer CPU) — não exige AES-NI. AES-GCM sem AES-NI é 5x mais lento, e tem histórico de ataques timing. ChaCha20 é constant-time por design.

### 3. parking_lot::Mutex em vez de std::sync::Mutex

`std::sync::Mutex` é envenenado quando um thread panica enquanto segura o lock — toda aquisição posterior falha com `PoisonError`. Em servidor de longa duração, um panic causa cascata site-wide (incidente 2026-05-12, 12 dias de hotfixes).

`parking_lot::Mutex` não tem poison. Um panic mata o request, não a aplicação inteira.

### 4. ES256 + JWKS para handover, HS256 para sessão

- **Handover cross-domain** (Co → Quilombo, Co → Yggdrasil): ES256 com JWKS público em `/.well-known/jwks.json`. Receivers fetch + verify com a chave pública. Sem compartilhar segredos.
- **Session token** (cookie): HS256 com `JWT_SECRET`. Mesmo servidor emite e verifica. Mais simples, sem overhead public-key.

### 5. SQLite em vez de Postgres

Co roda em uma máquina single-instance. Postgres seria operacional pesadelo sem benefício. SQLite com WAL mode + parking_lot exclusivity dá latência ~1µs/query, backup via copy do arquivo. Trigger para migrar: > 100k usuários ativos OU necessidade multi-region.

### 6. Vanilla JS frontend

Co é uma plataforma sobre soberania digital. Frontend é a superfície mais auditável — usuários técnicos podem ver exatamente o que roda no browser. Sem minificação, sem bundling, sem source maps. Zero npm runtime deps.

Trade-off: mais código manual. Justificativa: menos opacidade, menos surface de ataque supply chain.

### 7. AGPL v3 em vez de MIT

MIT permite que alguém pegue Co, modifique privadamente, e rode como SaaS sem retornar nada à comunidade. Para uma plataforma de soberania digital, isso é incoerente.

AGPL v3 fecha essa brecha: rodar uma versão modificada como serviço obriga liberação de fonte. Ver [/licensa](/licensa).

## Lista completa de deps

Para SBOM (Software Bill of Materials) machine-readable de todas as deps transitivas, ver `docs/security/dependencies.md` no repositório fonte.

CVE monitoring: GitHub Dependabot habilitado + `cargo audit` em CI (em implementação).

## Política de atualização

| Categoria | Cadência |
|---|---|
| Crates criptográficos | Imediato em CVE; revisão trimestral |
| jsonwebtoken | Imediato em CVE |
| rusqlite | Trimestral (acompanhar major releases) |
| tokio, axum | Quando há feature útil |
| Outras | Quando há motivo concreto |

---

Voltar para [Segurança](/seguranca).
