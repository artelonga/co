# Red Team — A: catálogo de cenários

Lista de cenários de ataque considerados contra CO, com **probabilidade**
e **blast radius** estimados. Para o que **fazer** quando um acontece,
ver [incident-playbook.md](incident-playbook.md) (versão B).

Atualizado: 2026-05-13 (CO 2.7.0).

---

## Cenário 1 — VAPID private key leak (standalone)

Ver detalhe completo: [vapid-security.md](../vapid-security.md).

- **Probabilidade**: baixa — só ~2 pessoas têm acesso ao Fly secret store
- **Blast radius standalone**: BAIXO — chave alone é inútil sem subscription DB
- **Blast radius com push_subscriptions DB**: MÉDIO — phishing push notifications

**Detection**: anomalous push delivery patterns; user complaints

**Recovery**: ver playbook §1

---

## Cenário 2 — push_subscriptions DB leak (standalone)

Database dump leaks subscriber endpoints + p256dh + auth keys but NOT
VAPID private key.

- **Probabilidade**: baixa — DB está em Fly volume, no copies externos
  além de backup criptografado
- **Blast radius standalone**: MUITO BAIXO — endpoints sem VAPID assinatura
  não são aceitos pelos push services
- **Combined com VAPID leak**: ver Cenário 1

**Detection**: outside report (em geral, DB leak detection é difícil
sem auditing system)

**Recovery**: rotacionar VAPID (invalida todas as subscriptions); usuários
re-subscrevem na próxima visita

---

## Cenário 3 — JWT_SECRET leak → session forgery

JWT_SECRET é o segredo HS256 usado para assinar todos os session tokens.

- **Probabilidade**: muito baixa — único copy é em Fly secrets
- **Blast radius**: ALTO — attacker pode forjar qualquer sessão de qualquer
  usuário sem precisar de senha

**Como leaks acontecem na prática:**

- Git push acidental de `.env` (mitigado: `.gitignore`)
- Log exposing env (mitigado: tracing não loga env vars)
- Container image inspect (sem AppState dump — secrets só lidos em runtime)
- Phishing operador → flyctl session compromisso → `flyctl secrets list`
  só mostra digests, não values

**Detection**: anomalous session activity; tokens com `iat` antigo mas
fingerprint diferente; **idealmente** auditoria de actions admin (não
implementado hoje — TO BE)

**Recovery**: ver playbook §2

---

## Cenário 4 — Argon2 parameter weakness over time

Não é "compromise" instantâneo, é decay temporal.

- **Probabilidade**: certa (hardware fica mais rápido)
- **Blast radius**: BAIXO se mantivermos rotação; ALTO se não

O parâmetro `m=19 MiB` foi forte em 2024. Em 2030 pode ser fraco.

**Detection**: monitoring de benchmark anual

**Recovery**: bumpar parâmetro + força usuários a re-hashearem na próxima
login (transparent re-hash on successful password verification)

---

## Cenário 5 — Database file leak (full /data/co.db dump)

Inclui plaintext de:

- `users` table (emails, usuarios, display_names, google_subs)
- `chat_messages.body` (TODO o conteúdo de mensagens)
- `universe_invitations`
- `notification_preferences`
- `telemetry_events`
- `push_subscriptions`
- `user_recovery_channels.value_ciphertext` (cifrado, MAS o lookup
  hashes podem permitir correlação)

NÃO inclui plaintext de:

- Senhas (Argon2 hashed)
- Recovery channel values (criptografado com CO_RECOVERY_KEY)

- **Probabilidade**: baixa
- **Blast radius**: CATASTRÓFICO — vazamento massivo de PII + conteúdo
  privado de todos os usuários

**Como acontece:**

- Backup mal-protegido publicado (CO-143 mitiga via cifragem)
- Insider com SSH access
- Cloud provider breach (Fly volume access)

**Detection**: external report; CO não detecta exfiltração de seu próprio
disk

**Recovery**: ver playbook §3 (notificação obrigatória LGPD)

---

## Cenário 6 — Recovery code interception

Email/SMS interception de código de 6 dígitos durante password reset.

- **Probabilidade**: baixa por usuário; média se attacker tem acesso ao
  email account
- **Blast radius**: por-usuário — uma conta tomada por interception

**Mitigações ativas:**

- Codes expiram em 15 min (curto)
- 5 wrong attempts → lockout
- Argon2id hash do code no DB (não retém plaintext)

**Detection**: difícil — parece login válido

**Recovery**: usuário relata, admin força logout em todas sessões + reset

---

## Cenário 7 — CORS misconfiguration

Allow-origin acidentalmente abre um origin malicioso.

- **Probabilidade**: baixa (CORS é code, não config)
- **Blast radius**: depends do endpoint — sessão de usuário pode ser
  capturada via cross-site fetch

CO-205 estabeleceu mirror-request com credentials. CSRF middleware
(`csrf_middleware`) tem allowlist hardcoded. Mudança requer code review.

**Detection**: code review at PR time

---

## Cenário 8 — SQL Injection

- **Probabilidade**: muito baixa (rusqlite + params! macro everywhere)
- **Blast radius**: CATASTRÓFICO se acontecer

**Mitigações ativas:**

- TODOS os queries usam parameterized queries via `params![]`
- `clippy` flags `format!` in SQL strings
- Code review

**Detection**: bug discovery; **idealmente** SQL audit log (não impl)

---

## Cenário 9 — IDOR (Insecure Direct Object Reference)

Attacker tries `/api/v1/universes/<other_user_universe>/...` and gets
data they shouldn't.

- **Probabilidade**: baixa (cada endpoint valida ownership)
- **Blast radius**: por-objeto

**Mitigações ativas:**

- Owner_id check em endpoints stateful
- `universe_members.role` check em membership-gated endpoints
- `chat_room_members` check em chat endpoints

**Detection**: pen test, bug bounty (não há um formal hoje)

---

## Cenário 10 — Phishing via legitimate channels

Attacker compromises one user's account → uses CO's own notification
system to send phishing to that user's contacts via DMs.

- **Probabilidade**: média (depends da segurança de cada usuário)
- **Blast radius**: por-usuário initially; spreads viralmente

Não é "CO comprometido" — é "uma conta comprometida". Mas CO o amplifica.

**Mitigações possíveis (futuras):**

- Rate limit DMs (CO já tem 20/min — pode ser menor pra novos accounts)
- Account age + verification flags
- "Esta conta foi criada há 1 hora" tag em DMs

**Detection**: user reports; behavioral anomaly (mass DM in short window)

---

## Cenário 11 — Subdomain takeover

Se um subdomínio de `.artelonga.com.br` é abandonado e seu DNS aponta
para um provider expirado, attacker pode reclamar o subdomain e:

- Receber cookies (cookie domain é `.artelonga.com.br` wildcard)
- Servir conteúdo "from artelonga.com.br" para social engineering

- **Probabilidade**: baixa se DNS é higienizado
- **Blast radius**: ALTO — cookie capture

**Mitigações:**

- DNS reviews periódicos
- Cookie `SameSite=Lax` mitiga partly
- HttpOnly cookies bloqueiam JS read

---

## Cenário 12 — Supply chain attack (npm / cargo dep compromise)

Um dep transitivo é comprometido (event-stream-style attack).

- **Probabilidade**: baixa por dep individual; cumulativa cresce com #deps
- **Blast radius**: depends — could be full RCE

**Mitigações ativas:**

- Vanilla JS frontend = zero npm runtime deps (CO chose this trade-off)
- Cargo.lock commitado e checked
- GitHub Dependabot security alerts habilitado
- TO BE: `cargo audit` em CI

---

## Cenário 13 — Operator account takeover

Se yuri's GitHub / Fly / Resend / 1Password account é comprometido.

- **Probabilidade**: baixa (depends da segurança de cada operator)
- **Blast radius**: CATASTRÓFICO — game over completo

**Mitigações:**

- 2FA habilitado em GitHub
- 2FA habilitado em Fly
- 2FA habilitado em Resend  
- Master password do 1Password offline + 2FA
- TO BE: hardware key (YubiKey) para tudo
- TO BE: separar operator accounts (admin vs daily-driver)

---

## Cenário 14 — DDoS / resource exhaustion

Attacker spams `/api/v1/auth/onboard-with-email` to:

- Esgotar rate limit
- Esgotar Resend quota
- Saturar disk via fake universe creates

- **Probabilidade**: média (zero-cost attack)
- **Blast radius**: BAIXO-MÉDIO — degraded service, no data loss

**Mitigações ativas:**

- Rate limit per-IP + per-user
- Resend rate limit no provider side
- Universe creation requires authenticated user (rate limit applies)

**Mitigações futuras:**

- CO-111 Cloudflare na frente (DDoS absorption)
- Per-tier quota (CO-80)

---

## Cenário 15 — Physical seizure of server / volume

Provider compelled by legal order to hand over disk image (police raid,
court order, foreign jurisdiction).

- **Probabilidade**: jurisdiction-dependent. Brasil: rare for tech ops.
- **Blast radius**: CATASTRÓFICO — full DB plaintext (chat messages,
  emails, universe content)

**Mitigações ativas:**

- Volume cripto provedor-side (LUKS) — mitiga roubo físico, NÃO court order
- Recovery channel values criptografadas em coluna (escapam plaintext)
- Senhas hashed

**Mitigações futuras (CO-115 Phase 4):**

- Operator-cannot-read zone
- Per-user encryption keys derived from user password (zero-knowledge
  como Signal/ProtonMail)
- Trade-off: perde recurso de "veja minhas mensagens em novo dispositivo"

**Política operacional:**

- Operator deve seguir process devido em qualquer requisição legal
- Notify affected users após order ser cumprido (se permitido)
- Backup criptografado offline (CO-143) reduz risco de ataque
  in-flight

---

## Sumário por severidade

| Severidade | Cenários |
|---|---|
| **Catastrófica** | 3 (JWT leak), 5 (DB leak), 8 (SQL injection), 13 (operator takeover), 15 (physical seizure) |
| **Alta** | — |
| **Média** | 1 (VAPID), 4 (Argon2 decay), 7 (CORS), 9 (IDOR), 10 (phishing via channels), 11 (subdomain), 12 (supply chain), 14 (DDoS) |
| **Baixa** | 2 (push_sub alone), 6 (recovery interception) |

Defense in depth: assumir QUE eventos catastróficos podem acontecer e
otimizar para **menor blast radius por evento** em vez de **zero
eventos**.

Para procedimentos de resposta, ir para [incident-playbook.md](incident-playbook.md).
