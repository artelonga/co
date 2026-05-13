---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 12
slug: seguranca-cenarios
tags:
- seguranca
- red-team
- transparencia
title: Cenários de Red Team
type: page
---

# Cenários de Red Team

Lista de cenários de ataque considerados contra Co, com **probabilidade** e **blast radius** estimados, mais o que **fazer** quando cada um acontece.

Voltar para [Segurança](/co/template?page=seguranca).

## Sumário por severidade

| Severidade | Cenários |
|---|---|
| **Catastrófica** | DB leak, JWT_SECRET leak, SQL injection, operator takeover, physical seizure |
| **Média** | VAPID compromise, Argon2 decay, CORS misconfig, IDOR, phishing via channels, subdomain takeover, supply chain, DDoS |
| **Baixa** | push_subscriptions standalone, recovery code interception |

Defense in depth: assumir que eventos catastróficos podem acontecer e otimizar para **menor blast radius por evento**.

## Cenários detalhados

### 1. VAPID private key leak

Ver detalhe completo em [VAPID e segurança](/co/template?page=seguranca-vapid).

- **Standalone**: BAIXO — chave alone é inútil sem subscription DB
- **Combined com push_subscriptions leak**: MÉDIO — phishing push notifications

### 2. JWT_SECRET leak → session forgery

JWT_SECRET é o segredo HS256 usado para assinar todos os tokens de sessão.

- **Probabilidade**: muito baixa — único copy em Fly secrets
- **Blast radius**: ALTO — attacker forja qualquer sessão

**Como leaks acontecem**: git push acidental de `.env` (mitigado por `.gitignore`), log expondo env (mitigado: tracing não loga env), phishing operador → `flyctl secrets list` mostra digests não values.

**Recovery**: rotacionar `JWT_SECRET` imediato; todos usuários deslogados; precisam relogar. Self-healing.

### 3. Full database leak

Vazamento do `/data/co.db` inclui plaintext de:
- `users` (emails, usuarios, display_names, google_subs)
- `chat_messages.body` (TODO o conteúdo)
- `universe_invitations`, `notification_preferences`, `telemetry_events`

NÃO inclui:
- Senhas (Argon2 hashed)
- Recovery channel values (criptografados com CO_RECOVERY_KEY)

- **Probabilidade**: baixa
- **Blast radius**: CATASTRÓFICO

**Recovery**: rotacionar TODOS os secrets; force password reset; revogar push subscriptions; notificar ANPD em 48h (LGPD); notificar usuários afetados.

### 4. Argon2 parameter staleness

Não é compromise instantâneo — é decay temporal. Hardware fica mais rápido.

- **Probabilidade**: certa
- **Blast radius**: BAIXO se mantivermos rotação

**Recovery**: bumpar `m` em código; transparent re-hash on successful login. Usuários upgradam gradualmente.

### 5. CORS misconfiguration

Allow-origin acidentalmente abre origin malicioso.

- **Probabilidade**: baixa (CORS é code, não config)
- **Blast radius**: depends do endpoint

CO-205 estabeleceu mirror-request com credentials. CSRF middleware tem allowlist hardcoded. Mudança requer code review.

### 6. SQL Injection

- **Probabilidade**: muito baixa
- **Blast radius**: CATASTRÓFICO se acontecer

**Mitigações**: rusqlite parameterized queries em TODOS os call sites; clippy flags `format!` em SQL strings; code review.

### 7. IDOR (Insecure Direct Object Reference)

Attacker tenta `/api/v1/universes/<other_user_universe>/...` e recebe dados que não deveria.

- **Probabilidade**: baixa
- **Blast radius**: por-objeto

**Mitigações**: cada endpoint valida ownership; `universe_members.role` check em endpoints membership-gated; `chat_room_members` check em chat endpoints.

### 8. Phishing via legitimate channels

Attacker compromete uma conta → usa o sistema de notificação do CO para enviar phishing via DMs.

- **Probabilidade**: média (depends da segurança de cada usuário)
- **Blast radius**: por-usuário, mas espalha viralmente

**Mitigações futuras**: rate limit DMs mais agressivo para contas novas; "Esta conta tem N dias" tag em DMs.

### 9. Subdomain takeover

Se um subdomínio `.artelonga.com.br` é abandonado, attacker pode reclamá-lo:
- Receber cookies (cookie domain wildcard `.artelonga.com.br`)
- Servir conteúdo "from artelonga.com.br" para social engineering

**Mitigações**: DNS reviews periódicos; `SameSite=Lax` cookies; HttpOnly cookies bloqueiam JS read.

### 10. Supply chain attack

Dep transitivo é comprometido (event-stream-style).

- **Probabilidade**: baixa por dep individual; cumulativa cresce com #deps
- **Blast radius**: pode ser full RCE

**Mitigações**: vanilla JS frontend = zero npm runtime deps; Cargo.lock commitado; GitHub Dependabot security alerts.

### 11. Operator account takeover

Se conta GitHub / Fly / Resend / 1Password do operador é comprometida.

- **Probabilidade**: baixa
- **Blast radius**: CATASTRÓFICO — game over

**Mitigações ativas**: 2FA em GitHub, Fly, Resend, 1Password; master password offline.

**TO BE**: YubiKey para tudo; separar operator accounts (admin vs daily).

### 12. DDoS / resource exhaustion

Attacker spamma endpoints para esgotar rate limit ou Resend quota.

- **Probabilidade**: média (zero-cost attack)
- **Blast radius**: BAIXO-MÉDIO — degraded service, no data loss

**Mitigações**: rate limit per-IP + per-user; Resend rate limit no provider side.

**TO BE**: Cloudflare na frente.

### 13. Physical seizure / court order

Provider compelido por ordem judicial.

- **Probabilidade**: jurisdiction-dependent
- **Blast radius**: CATASTRÓFICO — full DB plaintext

**Mitigações ativas**: volume cripto pelo provedor (mitiga roubo físico, não court order); recovery values criptografadas em coluna.

**Mitigações futuras** (CO-115 Phase 4): zona de computação criptografada — operator-cannot-read.

## Playbook de resposta

Para cada cenário acima, ver o playbook completo de resposta em `docs/security/incident-playbook.md` no repositório fonte.

### Resposta padrão a qualquer incidente

1. **Acknowledge** internamente: "Investigando X às HH:MM"
2. **Snapshot** do estado atual via `flyctl logs > incident.log`
3. **Conter** se aplicável: kill connections, revoke secret, etc.
4. **Notify** operadores disponíveis
5. **Status page** se externamente visível (TO BE)

### Comunicação após incidente

Template de notificação aos usuários:

```
Em DATA, detectamos INCIDENT_TYPE afetando SCOPE.

O que vazou: LIST
O que NÃO vazou: senhas (Argon2 hashed), recovery values (cifrados)
O que estamos fazendo: ACTIONS TAKEN
O que você precisa fazer: USER ACTIONS

Detalhes técnicos: link para postmortem público
```

### Após cada incidente

- **Postmortem** escrito em 1 semana
- **CHANGELOG entry** no próximo release (transparência)
- **Prevention ticket** filed para fix sistêmico

Cultura de disclosure honesto constrói confiança. Esconder incidentes custa confiança desproporcionalmente quando emergem depois.

---

Voltar para [Segurança](/co/template?page=seguranca).
