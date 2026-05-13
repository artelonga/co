# Red Team — B: playbook de resposta

Para cada cenário em [red-team-scenarios.md](red-team-scenarios.md), o
que fazer quando acontecer. Estruturado por gravidade.

**Princípios de resposta:**

1. **Conter primeiro, investigar depois** — parar o sangramento antes
   de fazer análise forense
2. **Comunicar honestamente** — usuários afetados são informados
3. **Documentar tudo** — timeline + actions taken em postmortem
4. **Iterar** — incident produz CHANGELOG entry + ticket de prevenção
   no próximo sprint

---

## Resposta padrão (universal a todos os incidentes)

Antes de qualquer playbook específico:

1. **Acknowledge** no terminal compartilhado / Slack interno: "Investigando incidente X às HH:MM"
2. **Snapshot atual** do estado: `flyctl logs > /tmp/incident-{date}.log`
3. **Stop the bleeding** se aplicável: kill compromised connections, revoke secret, etc.
4. **Notify operador disponível** (yuri@artelonga.com.br)
5. **Status page** se incidente está externamente visível (TO BE — não existe ainda)

---

## §1 — VAPID private key compromise (Cenário 1)

### Inicialização

- Sinal: usuário reporta push estranho, OU log mostra push delivery anômalo
- Avaliar: chave standalone ou combinada com push_subscriptions leak?

### Containment

1. **Imediato**: `flyctl secrets unset VAPID_PRIVATE_KEY -a co-artelonga`
2. `flyctl machine restart 1850920b111d38 -a co-artelonga`
3. Worker degrada para log-only — push para de delivery imediatamente

### Recovery

1. Gerar nova VAPID keypair (ver `docs/vapid-security.md` rotation section)
2. `flyctl secrets set VAPID_PUBLIC_KEY=... VAPID_PRIVATE_KEY=... VAPID_SUBJECT=...`
3. `flyctl machine restart`
4. **All existing subscriptions become invalid** — usuários re-subscrevem

### Communication

- Email para todos os usuários com push subscription ativa
- "Atualizamos nossa configuração de notificações por segurança. Por favor, clique em 'Ativar notificações' novamente."
- NÃO mencionar o incidente publicamente até saber a extensão

### Postmortem

- Como o secret leaked? (1Password leak? flyctl session? screenshot?)
- Pode ser detectado mais cedo via auditing?
- Ticket: melhorar auditing

---

## §2 — JWT_SECRET leak → session forgery (Cenário 3)

### Containment

1. **Imediato**: gerar novo `JWT_SECRET` (`openssl rand -base64 48`)
2. `flyctl secrets set JWT_SECRET=$NEW -a co-artelonga`
3. `flyctl machine restart`
4. **TODOS os usuários são deslogados** (todos os JWTs antigos ficam inválidos)
5. Usuários precisam re-fazer login

### Recovery

- Nada mais — JWT rotation é self-healing
- Investigation: como leaked?

### Communication

- Banner público no site: "Por questões de segurança, foi necessário deslogar todos os usuários. Por favor, faça login novamente."
- Não mencionar comprometimento até pôstmortem se possível

### Postmortem

- Audit access trail: quem teve acesso ao secret?
- Hardening: rotação periódica + hardware key para flyctl?

---

## §3 — Full database leak (Cenário 5)

### Containment

1. **Imediato**: avaliar se vazamento ainda está em andamento (insider with SSH active? backup endpoint exposed?)
2. Stop the source (revoke SSH key, take backup endpoint offline)
3. Snapshot do dataset atual para preservação forense

### Critical assessment

- **O que vazou**: identificar exatamente quais tabelas
- **Quantos usuários afetados**: query `SELECT COUNT(*) FROM users`
- **Tipos de dados**: PII (LGPD!) + conteúdo privado + tokens

### Recovery (técnico)

1. **Rotacionar TODOS os secrets**:
   - JWT_SECRET (invalida sessões — todos relogam)
   - CO_RECOVERY_KEY (re-encrypt em batch dos canais — script de migração)
   - VAPID, Resend, MaxMind keys (rotacionar todas as APIs)
2. **Force password reset** para todos os usuários (set `password_hash = NULL` em batch; força recovery flow no próximo login)
3. **Revogar todas as push subscriptions** (drop table contents)

### Communication (LGPD obrigatório)

- **48 horas**: notificar ANPD (Autoridade Nacional de Proteção de Dados)
- **Imediato**: notificar todos os usuários afetados via email com:
  - O que vazou
  - O que NÃO vazou (senhas hashed, recovery values cifradas)
  - O que fizemos
  - O que o usuário deve fazer

### Postmortem

- Public CHANGELOG entry + dedicated incident page
- Engagement: bug bounty oferecido se externamente reportado

---

## §4 — Argon2 parameter staleness (Cenário 4)

### Detection (proactive, não reactive)

- Benchmark anual: medir `argon2id` com m=19 MiB em hardware típico
- Se > 500 ms → ok
- Se < 100 ms → bumpar imediatamente

### Recovery

1. Bumpar `m` em código (`co-web/src/auth.rs`)
2. Implementar **transparent re-hash on successful login**:
   ```rust
   if verify(stored_hash, password) {
       if hash_uses_old_params(stored_hash) {
           // Re-hash with new params, update DB
           let new_hash = argon2_new_params(password);
           storage.update_password_hash(user_id, &new_hash);
       }
       // proceed with login
   }
   ```
3. Deploy
4. Over time, all users gradually upgrade as they login

### Postmortem

- Schedule next review (annual)
- Update CHANGELOG with new params

---

## §5 — Recovery code interception (Cenário 6)

### Per-user incident — não system-wide

1. User reports unauthorized access
2. Force logout all sessions for that user_id:
   ```bash
   flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "UPDATE users SET jwt_revoke_at = datetime(\"now\") WHERE id = ?1" -- "$USER_ID"'
   ```
   (requires `users.jwt_revoke_at` column + JWT validation check — TO BE)
3. Force password reset via recovery flow OR manually set hash to require re-recovery
4. Audit user's data: check for IDOR-style data exfil

### Postmortem

- Better: 2FA TOTP would have prevented (TO BE ticket)

---

## §6 — CORS misconfiguration discovered (Cenário 7)

### Containment

1. Revert the offending commit
2. Deploy fix
3. Audit access logs since the misconfig went live

### Recovery

- If sessions were captured via CSRF: rotate JWT_SECRET (§2 protocol)
- If users' data was exfiltrated: §3 protocol scoped to affected endpoints

### Postmortem

- CORS changes need PR review tag (CODEOWNERS for `csrf_middleware`)

---

## §7 — Suspected SQL injection (Cenário 8)

### Containment

1. Take read-only mode imediato — disable writes:
   ```bash
   flyctl ssh console -a co-artelonga -C 'sqlite3 /data/co.db "PRAGMA query_only=1;"'
   ```
   (Não permanente, mas para imediato)
2. Block the vulnerable endpoint via WAF or Cloudflare
3. Deploy fix

### Investigation

- Audit `co-web/src/**/*.rs` para `format!` + SQL strings
- Audit logs para suspicious queries (large LIKE patterns, OR conditions)

### Recovery

- Same as §3 (assume data may have been exfiltrated)

---

## §8 — IDOR discovered (Cenário 9)

### Containment

- Patch endpoint (add owner_id check) + emergency deploy

### Investigation

- Audit logs for unauthorized access pattern
- Identify affected users

### Communication

- Email affected users with what was accessed

---

## §9 — Phishing via legitimate channels (Cenário 10)

### Per-incident — não system-wide

1. Identify compromised account
2. Force logout + force password reset for that account
3. Notify recipients of malicious DMs:
   ```sql
   SELECT DISTINCT recipient_id FROM chat_messages
   WHERE author_id = ? AND created_at > ?
   ```
4. Public banner: "Watch for unusual DMs from {handle}"

### Future mitigation

- Rate limit DMs more aggressively for new accounts
- "Esta conta tem N dias" tag in DM UI

---

## §10 — Subdomain takeover (Cenário 11)

### Containment

1. Identify all `.artelonga.com.br` subdomains via DNS dump
2. For each: verify active provider, valid cert, expected content
3. Take down any orphaned or expired entries

### Recovery

- Rotate any cookies that may have been captured (set new JWT_SECRET — §2)

---

## §11 — Supply chain compromise (Cenário 12)

### Detection

- Dependabot alerts
- `cargo audit` flags CVE
- External CVE database (e.g., RustSec advisory database)

### Containment

1. Pin to known-good version in Cargo.lock
2. Deploy

### Recovery

- If RCE in compromised dep: assume server was compromised, follow §3
- If lesser CVE: monitor + accept

---

## §12 — Operator account takeover (Cenário 13)

### Catastrophic — assume game over

1. **Immediate**: somebody else with operator access rotates ALL secrets
2. Disable compromised account's access (revoke GitHub access, Fly token, Resend API)
3. Provision new operator credentials
4. Audit ALL recent admin actions

### Recovery

- §3 protocol (assume DB leak)
- §2 protocol (rotate JWT)
- New deploys to ensure no malicious code

### Communication

- Public statement about incident
- Notify all users

### Prevention

- HARDWARE keys (YubiKey) for ALL operator accounts
- Multi-operator approval for secret rotation
- Audit trail of admin actions

---

## §13 — DDoS (Cenário 14)

### Containment

1. Activate Cloudflare in front (manual switch via DNS, if CF account ready)
2. Rate limit per-IP at edge
3. Block source ASNs if pattern is clear

### Recovery

- Service may degrade; no data loss
- After attack subsides, review logs for novel attack pattern

---

## §14 — Physical seizure / court order (Cenário 15)

### Legal first

1. **Consult lawyer immediately**
2. Comply with valid order; resist overbroad ones via counsel
3. Document everything

### Technical

- If volume seized: assume DB leak per §3
- Notify users post-compliance if legally permitted

---

## After every incident

1. **Postmortem** written within 1 week
2. **CHANGELOG entry** in the next release (transparent disclosure)
3. **Prevention ticket** filed (CO-N for system fix)
4. **Communication** to users if any data was at risk

A culture of honest disclosure builds trust. Hiding incidents costs trust
disproportionately when they surface anyway.

---

## Templates de comunicação

### Template: notificação de breach (Português)

```
Assunto: Incidente de segurança em CO — sua ação necessária

Olá,

Em {DATA}, detectamos {INCIDENT_TYPE} afetando {SCOPE}.

O que vazou:
- {LIST}

O que NÃO vazou:
- Suas senhas (são armazenadas como hash Argon2id, não como texto)
- {OTHER PROTECTED THINGS}

O que estamos fazendo:
- {ACTIONS TAKEN}

O que você precisa fazer:
- {USER ACTIONS}

Detalhes técnicos: https://co.artelonga.com.br/seguranca/incidentes/{ID}

Lamentamos o impacto. Estamos comprometidos com a sua autonomia digital
e com transparência sobre nossos sistemas.

— equipe CO
```

### Template: notificação ANPD (LGPD)

Ver template oficial em <https://www.gov.br/anpd/pt-br>. Notificação
deve incluir:

- Natureza dos dados afetados
- Categorias de titulares afetados (número aproximado)
- Medidas técnicas e organizacionais empregadas
- Riscos relacionados
- Motivos da demora (se >72h)
- Medidas adotadas ou propostas para reverter ou mitigar
