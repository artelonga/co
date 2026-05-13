# Segurança — CO

**Princípio:** seus dados são seus. CO é uma plataforma de soberania
digital. Documento vivo: estado AS IS hoje + roadmap TO BE para
endereçar lacunas conhecidas.

A página pública `co.artelonga.com.br/seguranca` renderiza este
documento com navegação em dropdown menu por seção. Cada seção tem
estado atual + próximo passo concreto.

## Sumário

1. [Modelo de ameaças](#modelo-de-ameaças)
2. [Taxonomia de dados](#taxonomia-de-dados)
3. [Camadas de defesa](#camadas-de-defesa)
4. [Lacunas conhecidas (AS IS → TO BE)](#lacunas-conhecidas-as-is--to-be)
5. [Cenários de red team](red-team-scenarios.md) — separado
6. [Dependências e por quê](dependencies.md) — separado
7. [Portabilidade entre provedores](#portabilidade-entre-provedores)
8. [Como reportar uma vulnerabilidade](#como-reportar-uma-vulnerabilidade)

---

## Modelo de ameaças

### Quem queremos proteger

| Persona | Vetor principal |
|---|---|
| **Usuário comum** | Phishing, conta tomada, mensagens vazadas, dados privados expostos |
| **Operador (yuri)** | Comprometimento de chaves/secrets, perda de backup, ransomware no dataset |
| **Comunidade Quilombo / Universo** | Vazamento de membro privado para fora do universo |

### De quê queremos proteger

| Ameaça | Prioridade |
|---|---|
| Tomada de conta (credential stuffing, phishing, sessão sequestrada) | Alta |
| Vazamento de mensagens privadas / DMs | Alta |
| Acesso não autorizado a universo privado | Alta |
| Vazamento de dados pessoais (email, eventualmente CPF/nome) | Alta |
| Adulteração de conteúdo (sabotagem) | Média |
| Negação de serviço (DoS, esgotar storage) | Média |
| Análise de tráfego (correlacionar visitantes) | Baixa (apenas hash diário de IP) |
| Comprometimento físico do servidor (apreensão, raid policial) | **Especial** — ver discussão em "Cenários de red team" |

### O que NÃO promete (ainda)

Honestidade explícita:

- **End-to-end encryption** para chat: NÃO. Mensagens são criptografadas
  em trânsito (TLS) e o volume de disco do servidor é criptografado pelo
  provedor, mas o servidor lê o texto puro. Operador honesto pode ler
  qualquer mensagem se quiser — ver "Lacunas conhecidas" abaixo.
- **Zero-knowledge**: NÃO. CO é "trust the operator" hoje. Roadmap
  Phase 4 (CO-115) prevê zona de computação criptografada onde nem o
  operador lê.
- **Anonimato perfeito**: NÃO. CO sabe o IP do navegador na requisição
  (hashed diariamente, descartado). Em ambientes hostis (jornalismo,
  ativismo) use Tor.
- **Defesa contra ataque de estado-nação**: NÃO. CO é defesa contra
  ataque oportunista e operador comprometido, não contra adversário
  com recursos arbitrários.

---

## Taxonomia de dados

Categorizado por sensibilidade. Cada linha cita a tabela/coluna real e
o estado atual de proteção.

### 1. Identidade

| Dado | Tabela.coluna | Proteção AS IS | TO BE |
|---|---|---|---|
| Email | `users.email` | Plaintext em SQLite no volume cripto do provedor. Single primary identifier. | Adicionar opção de email-hash-only (provide email only at recovery moment) |
| Usuario | `users.usuario` | Plaintext, único, exibido em públicos | Mantém |
| Display name | `users.display_name` | Plaintext | Mantém |
| Google sub (OAuth) | `users.google_sub` | Plaintext, é um ID opaco da Google | Mantém |
| CPF / RG / nome legal | — | **NÃO ARMAZENADO HOJE.** Quando armazenar (CO-144 dados pessoais): criptografia em coluna via [recovery_crypto pattern], chave em secret store | Coluna criptografada com ChaCha20-Poly1305 antes do v1 lançar |
| Foto / avatar | — | Não armazenado | Quando armazenar: blob CAS criptografado (CO-145) |

### 2. Credenciais e autenticação

| Dado | Onde vive | Proteção AS IS | TO BE |
|---|---|---|---|
| Senha do usuário | — | **Nunca armazenada.** Só o hash Argon2id (m=19 MiB, t=2, p=1) em `users.password_hash` | Bumpar parâmetros (m=64 MiB) quando hardware deixar |
| Códigos de recuperação | `recovery_verifications.code_hash` | Argon2id hash, válido 15 min, soft-lockout após 5 tentativas | Adicionar 2FA TOTP opcional |
| JWT de sessão | Cookie HTTP-only + Authorization Bearer | Assinado HS256 com `JWT_SECRET` (Fly secret). TTL 7 dias. SameSite=Lax | Migrar sessão para ES256 também (CO-187 só migrou handover); rotação automática anual |
| JWT handover (cross-domain) | URL param (60s TTL) | Assinado ES256 via JWKS público em `/.well-known/jwks.json` | Mantém. Padrão sólido. |
| Senha "como password manager"? | — | **NÃO SUPORTADO HOJE.** Se um usuário quiser guardar outras senhas no CO, não há feature de password vault. Quando construir: criptografia derivada da senha mestra com Argon2id + ChaCha20-Poly1305 | Roadmap futuro (não priorizado). Veja Bitwarden self-hosted como alternativa. |
| Google OAuth client secret | Fly secret `GOOGLE_CLIENT_SECRET` | Injetado em runtime, nunca em log | Mantém |
| Resend API key | Fly secret `RESEND_API_KEY` | Same | Mantém |
| MaxMind license key | Fly secret `MAXMIND_LICENSE_KEY` | Same | Mantém |
| VAPID private key | Fly secret `VAPID_PRIVATE_KEY` | Same. Ver [vapid-security.md] para impacto detalhado | Rotação anual |

### 3. Conteúdo sensível dos usuários

| Dado | Tabela.coluna | Proteção AS IS | TO BE |
|---|---|---|---|
| Mensagens de chat (sala de universo) | `chat_messages.body` | **Plaintext.** Lido somente por membros do universo via gate `chat_room_members`. | E2E encryption opcional (CO-N futuro, não trivial) |
| Mensagens privadas (DM) | Mesma tabela, `chat_rooms.kind='dm'` | **Plaintext.** Lido somente pelas 2 partes. | E2E encryption mesma situação |
| Conteúdo do universo (markdown) | `entries.body` por universo | Plaintext. Lido por membros + visibilidade. | CO-145: encrypted assets at rest |
| Convites por email | `universe_invitations.invited_email` | Plaintext. Token só hashed (sha256). | Mantém |
| Endpoints de push subscription | `push_subscriptions.endpoint`, `.p256dh`, `.auth` | Plaintext. Necessário para encriptar payloads RFC 8291. | Mantém — campo é semi-sensível mas inutilizável sem VAPID key |
| Canais de recuperação (email/telefone) | `user_recovery_channels.value_ciphertext` + `value_nonce` | **Criptografados.** ChaCha20-Poly1305 com `CO_RECOVERY_KEY` (Fly secret). Hash de lookup separado. | Modelo certo. Replicar em outros campos sensíveis. |

### 4. Telemetria e comportamento

| Dado | Tabela.coluna | Proteção AS IS | TO BE |
|---|---|---|---|
| IP do visitante | `telemetry_events.ip_hash` | **SHA-256 com salt diário**, IP raw nunca persistido | Mantém |
| User Agent | `telemetry_events.user_agent` (truncado 256 chars) | Plaintext | Mantém |
| Country / city | `telemetry_events.country`, `.city` | Enriquecimento server-side via MaxMind antes do IP ser hashed | Mantém |
| Visitor token (vid) | `telemetry_events.vid` | UUID opaco | Pode ser correlacionado entre sessões; aceitar para analytics, ou desativar via opt-out por usuário (futuro) |
| Session token (sid) | `telemetry_events.sid` | UUID por sessão | Mantém |
| Eventos de página (`page_view`) | `telemetry_events` table | Per-universe scoped (CO-177); admin pode ver agregado | Endpoints públicos `/analytics/public/*` retornam apenas agregados sem PII |

### 5. Estado do sistema

| Dado | Tabela | Sensibilidade |
|---|---|---|
| Feature flags | `feature_flags`, `ab_assignments` | Baixa — exposição mínima |
| Rate-limit state | em memória (parking_lot::Mutex) | N/A em disco |
| Schema migrations | `schema_version` | Não-sensível |
| Audit log | `activity_log` | Existe parcialmente; precisa expandir para ações de admin (CO-N TO BE) |

---

## Camadas de defesa

CO usa modelo de **defense in depth** — uma falha em uma camada não
deve causar comprometimento total. Cada camada é definida por
propriedades, não por provedor específico. Ver
[portability.md](#portabilidade-entre-provedores) para como cada
camada se traduz em AWS Fargate, Cloudflare, ou auto-hospedado.

### Camada 1 — Transporte (rede)

| Propriedade | AS IS | TO BE |
|---|---|---|
| TLS 1.2+ obrigatório | Fly LB termina TLS automaticamente | Mantém em qualquer provedor (Caddy/Traefik/CF) |
| HSTS header | Sim, `max-age=63072000; includeSubDomains; preload` | Mantém |
| X-Frame-Options DENY | Sim | Mantém |
| X-Content-Type-Options nosniff | Sim | Mantém |
| Referrer-Policy strict-origin-when-cross-origin | Sim | Mantém |
| CSP (Content-Security-Policy) | Parcial — defaultSrc 'self' | TO BE: nonce-based para scripts inline |

### Camada 2 — Borda (perímetro de aplicação)

| Propriedade | AS IS | TO BE |
|---|---|---|
| WAF (Web App Firewall) | Não — confia no provedor (Fly não inclui) | CO-111: Cloudflare na frente |
| DDoS protection | Provedor only | Cloudflare camada |
| Rate limiting por IP | Token bucket por user_id, não por IP raw (problemas com NAT) | TO BE: combinar com IP hash diário |
| Bot filter | UA-based em `/telemetry/events` (CO-46) | Mantém; expandir para outros endpoints |

### Camada 3 — Aplicação (lógica de negócio)

| Propriedade | AS IS |
|---|---|
| **SQL injection** | rusqlite parameterized queries em **todos** os call sites — verificado via grep |
| **XSS** | tower-http defaults + escape manual em renders SPA + HTML email |
| **CSRF** | Origin allowlist em `csrf_middleware` (CO-205 + 2.4.1) |
| **CORS** | mirror_request com credentials para subdomínios `.artelonga.com.br` + lista fixa de hosts confiáveis |
| **IDOR** (Insecure Direct Object Reference) | Cada endpoint que toca dados de usuário valida `user_id` da sessão contra `owner_id` da entidade — verificado em invitations, universes, chat, dm |
| **Mass assignment** | serde com derive explícito, sem `flatten` em endpoints que aceitam input |
| **Path traversal** | Conteúdo é endereçado por content hash (CO-145 em andamento), não por path |
| **SSRF** (Server-Side Request Forgery) | Webhooks (CO-168) só fazem POST para URL configurada pelo admin; sem fetch de URL arbitrária por usuário |
| **Race conditions / Mutex poisoning** | parking_lot::Mutex (CO-203) — não-poison na panic; storage continua disponível |
| **Authentication** | JWT validation em middleware, antes de qualquer handler stateful |
| **Authorization** | Membership-based (universe_members, chat_room_members); admin tier reservado para Gestao endpoints |
| **Rate limiting** | Token bucket per (user_id, operation_type); bypass `CO_BYPASS_RATE_LIMIT=1` requer também `CO_ENV=test` (CO-208) |

### Camada 4 — Armazenamento (dados em repouso)

| Propriedade | AS IS | TO BE |
|---|---|---|
| Volume cripto | Fly volume encrypted by default (LUKS) | Property: qualquer block storage com encryption at rest (AWS EBS encrypted, GCP Persistent Disk, Cloudflare R2) |
| Backup encryption | CO-143 em andamento — `co-backup-cron` cifra com chave separada | Same |
| Column-level encryption | Só `user_recovery_channels.value_ciphertext` por enquanto (ChaCha20-Poly1305) | TO BE: expandir para mensagens, conteúdo privado, eventual CPF |
| Hash de senhas | Argon2id, parâmetros revisados anualmente | Mantém |
| Key separation | App keys (JWT_SECRET, VAPID, Resend, MaxMind, CO_RECOVERY_KEY) cada um separado, não derivados | Mantém — princípio sólido |

### Camada 5 — Secrets management

| Propriedade | AS IS | TO BE |
|---|---|---|
| Storage | Fly secrets (encrypted at rest, runtime injection only) | Property: qualquer KMS (AWS Secrets Manager, GCP Secret Manager, HashiCorp Vault, doppler.com, env-vars-via-1password) |
| Acesso | `flyctl secrets list` mostra digest, não valor | Mantém |
| Rotação | Manual ad-hoc | Roadmap: política anual + automação para VAPID, JWT, Resend |
| Distribuição | Single secret store, app reads via env | Mantém |

### Camada 6 — Audit e observabilidade

| Propriedade | AS IS |
|---|---|
| Structured logs | `tracing` crate com JSON output em prod |
| PII em logs | Redacted via `redact_email` helper |
| Activity log | `activity_log` tabela para mudanças de task; admin actions ainda não auditados separadamente (TO BE) |
| Login attempts | Logged via tracing; soft-lockout via `recovery_verifications.attempts` |
| Failed auth | Logged at INFO level com email redacted |
| Telemetry isolada | `telemetry_events` é própria tabela, não vaza PII para terceiros |

### Camada 7 — Recovery e disaster

| Propriedade | AS IS | TO BE |
|---|---|---|
| Backup | CO-143 em andamento (daily snapshot) | Same; criptografar com chave separada do app |
| Restore drill | Não testado regularmente | TO BE: drill trimestral |
| Multi-region | Single region (gru) | TO BE quando justifique custo |
| Data export (LGPD) | Não implementado | TO BE: endpoint `GET /api/v1/me/export` retorna ZIP com tudo do usuário |
| Right to be forgotten | Manual via SSH | TO BE: endpoint `DELETE /api/v1/me/account` com confirmação |

---

## Lacunas conhecidas (AS IS → TO BE)

Lista honesta de o que CO **NÃO** faz bem hoje. Prioridade reflete
impacto + viabilidade técnica.

### Alta prioridade (próximas 4 semanas)

| Lacuna | TO BE | Ticket |
|---|---|---|
| Mensagens não-criptografadas at rest | Adicionar coluna-level encryption em `chat_messages.body` (ChaCha20-Poly1305 com per-room key derivado de master) | CO-N futuro |
| Backup automatizado quebrado | Deploy CO-143 backup-cron | CO-143 |
| Sem endpoint LGPD de export / delete | `GET /me/export` retorna ZIP; `DELETE /me/account` com fuse | CO-N futuro |
| Sem 2FA | Adicionar TOTP opcional (`totp_secret_encrypted` em users) | CO-N futuro |
| Audit log de admin não estruturado | Tabela `admin_audit_log` com action, actor, target, timestamp, IP | CO-N futuro |

### Média prioridade

| Lacuna | TO BE | Ticket |
|---|---|---|
| Mutex outros que Storage ainda std::sync | Migrar AuthStore, RateLimiter, etc. para parking_lot (extensão de CO-203) | CO-N futuro |
| CSP nonce-based | Atualizar middleware para gerar nonce per-request | CO-N futuro |
| WAF / Cloudflare na frente | CO-111 Phase 0 | CO-111 |
| Rate limit por IP + user combinado | Adicionar bucket por IP hash além do per-user | CO-N futuro |
| Per-tier rate limiting | Player vs Pro vs Admin com quotas diferentes | CO-80 |
| Webhook payload audit | Append-only tabela com hash signed | CO-N futuro |

### Baixa prioridade (estratégia, não bug)

| Lacuna | TO BE |
|---|---|
| Zona de compute criptografada (operator-cannot-read) | CO-115 Phase 4 — separar app em zona com k-anonimização DLP |
| E2E encryption para chat | Requer protocolo de chaves entre clientes; Signal-style ratcheting; trade-off com recurso de "ver últimas N mensagens em novo dispositivo" |
| Hardware security module (HSM) | Quando volume justificar custo |
| SOC 2 audit | Quando comercializar para enterprise |

---

## Portabilidade entre provedores

CO é deployable hoje em Fly.io, mas a arquitetura é provider-agnostic.
Cada propriedade de segurança é definida em termos abstratos.

### Mapeamento de propriedades

| Propriedade abstrata | Fly.io (atual) | AWS Fargate | Cloudflare | Auto-hospedado |
|---|---|---|---|---|
| **TLS termination** | Fly LB | ALB / NLB com ACM cert | CF SSL/TLS Universal | Caddy / Traefik / nginx |
| **Encrypted block storage** | Fly Volumes (LUKS) | EBS encrypted (default since 2020) | R2 SSE | dm-crypt / LUKS / ZFS native encryption |
| **Secret store** | Fly secrets | AWS Secrets Manager | CF Workers Secrets | HashiCorp Vault / 1Password CLI / SOPS+age |
| **Container isolation** | Firecracker microVM | Fargate ECS task | Workers V8 isolate (different model — see notes) | Docker / Podman / systemd-nspawn |
| **DDoS / WAF** | Fly limitado | AWS Shield + WAF | CF Universal (free tier) | Cloudflare em frente / standalone WAF |
| **Geo-IP database** | MaxMind .mmdb em volume | Same | CF Geo headers (no DB needed) | MaxMind .mmdb em disk |
| **Logs aggregation** | Fly logs CLI / external | CloudWatch | CF Logpush | Loki / Promtail / journalctl |
| **Health checks** | Fly nativos | ALB / Route53 | CF Health Checks | systemd / external probe |

### Garantias que sobrevivem qualquer provedor

Estas propriedades são **propriedades do código + dados**, não do
provedor:

1. **Senhas nunca em plaintext** — Argon2id é uma decisão de código
2. **JWT_SECRET separado de outros secrets** — secret-store-agnostic
3. **Recovery channel values criptografados** — algoritmo + chave são
   propriedade da aplicação, lê-se de qualquer KMS
4. **IP hashes daily-salted** — função pura, independente de provedor
5. **Rate limit token-bucket** — parking_lot::Mutex em processo,
   independente de runtime
6. **CSRF / CORS allowlists** — listas em código, independente de
   reverse proxy

### Propriedades que dependem do provedor

Estas precisam ser re-validadas ao migrar:

1. **Volume encryption at rest** — verificar que o provedor encrypts
   por padrão (AWS EBS sim, GCP sim, Cloudflare R2 sim)
2. **TLS cert auto-renewal** — Fly faz nativo, outros precisam
   Let's Encrypt + cert-manager
3. **DDoS absorbing capacity** — Fly absorve até ~10 Gbps; AWS Shield
   Advanced absorve mais; CF absorve quase qualquer coisa
4. **Latency / availability SLOs** — provedor specific
5. **Logs encryption in transit to aggregator** — depends on what
   aggregator you ship to

### Plano de migração se Fly se torna inviável

Documentado para futuro-você ou sucessor:

1. **Provisionar nuevo provedor** com encrypted block storage, secret
   store, e container runtime
2. **Restaurar último backup** (criptografado) no novo volume
3. **Re-injetar secrets** a partir do password manager
4. **Apontar DNS** para novo endpoint
5. **Verificar** via E2E checklist (`work/co/E2E-RELEASE-CHECKLIST.md`)

Não há lock-in de provedor por design. Volume é SQLite (binário portável),
secrets são strings, código é Rust + JS estáticos.

---

## Como reportar uma vulnerabilidade

Email para **yuri@artelonga.com.br** com:

1. Descrição do problema
2. Passos para reproduzir
3. Impacto observado
4. (Opcional) sugestão de mitigação

Resposta dentro de 72h. Vulnerabilidades críticas (RCE, vazamento massivo
de dados) recebem hotfix prioritário. Não há programa formal de
bug bounty hoje; reconhecimento público (com permissão) e crédito no
CHANGELOG do hotfix são oferecidos.

**Não divulgar publicamente antes** do fix estar deployed em produção —
prática responsible disclosure padrão.
