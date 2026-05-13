# Dependências — B: decisões e por quê

Companheiro narrativo de [dependencies.md](dependencies.md). Aqui é a
explicação das **decisões de segurança** sobre quais dependências
escolher e por quê — útil quando alguém pergunta "por que argon2 e
não bcrypt?" ou "por que parking_lot e não std::sync?".

Cada seção referencia a entrada correspondente no catálogo A.

---

## 1. Argon2id em vez de bcrypt / scrypt / PBKDF2

Hashing de senhas + códigos de recuperação.

### Decisão

**Argon2id** com parâmetros `m=19 MiB, t=2, p=1` (recommended baseline
do [Password Hashing Competition](https://www.password-hashing.net/)).

### Por quê não bcrypt

bcrypt é o "ainda funciona" da indústria, mas:

- **Tem 56-byte input limit** (bcrypt internamente trunca para 72 bytes).
  Senhas longas e passphrases ficam parcialmente perdidas — péssimo
  pra "use uma frase longa em vez de senha curta complicada".
- **Não é memory-hard.** GPUs e ASICs aceleram bcrypt linearmente.
- **Parâmetro "cost" é uma escala linear**; Argon2 tem três dimensões
  (memória, tempo, paralelismo) — mais difícil de otimizar para
  hardware específico.

### Por quê não scrypt

scrypt é memory-hard mas:

- **Não tem paralelismo configurável** explicitamente.
- **Parâmetros são acoplados** (N, r, p inter-dependentes) — mais fácil
  errar a calibração.

### Por quê Argon2id (não 2d nem 2i)

- **2i** resiste a ataques side-channel (timing, cache) mas é mais
  fraco contra GPU.
- **2d** é mais forte contra GPU mas vulnerável a side-channel.
- **2id** combina: primeira passada em 2i, depois 2d. Recomendação
  oficial do PHC para password hashing.

### Validação de parâmetros

`m=19 MiB` foi escolhido em 2024 baseado em ASUS commodity laptops.
Em 2026+ esperamos bumpar para `m=64 MiB`. Verificação anual:
medir `argon2id` em uma máquina típica de usuário; alvo é ~500 ms
para hash (suficiente para frear ataque offline; tolerável em login).

### Catálogo: dependencies.md §4 → `argon2`

---

## 2. ChaCha20-Poly1305 + BLAKE3 em vez de AES-GCM + SHA-256

Cifragem em colunas: `user_recovery_channels.value_ciphertext`.

### Decisão

**ChaCha20-Poly1305** (AEAD) para encrypt, **BLAKE3** para lookup hashes.

### Por quê ChaCha em vez de AES

AES-GCM exige hardware AES-NI para ter performance decente. Em VMs
sem AES-NI (alguns runtimes serverless, embedded, ARM antigo):

- ChaCha20 mantém performance constante (~1 GB/s em qualquer CPU).
- AES-GCM sem AES-NI: ~200 MB/s (5x mais lento).

Side-channel:

- AES-GCM tem histórico de ataques timing em implementações sem AES-NI.
- ChaCha20 é constant-time por design (sem table lookups).

Padding oracle:

- AES-GCM já é AEAD (não tem padding oracle classic).
- Mesma garantia para ChaCha20-Poly1305.

Para CO especificamente: rodamos em Firecracker microVM (com AES-NI)
mas a arquitetura deveria sobreviver migração para qualquer ambiente.
ChaCha20 é a escolha portável.

### Por quê BLAKE3 em vez de SHA-256

Lookup hashes (`value_lookup_hash` em recovery channels) precisam:

- Determinístico ✓ (ambos)
- Resistente a colisão ✓ (ambos)
- Rápido ✓ (BLAKE3 é ~3-5x mais rápido que SHA-256 em hardware moderno)

SHA-256 fica para `ip_hash` e tokens de convite por compatibilidade
com padrões da Internet (RFC 6920, expectativa de outros sistemas).

### Catálogo: dependencies.md §4 → `chacha20poly1305`, `blake3`

---

## 3. parking_lot::Mutex em vez de std::sync::Mutex

Para o `Mutex<Storage>` compartilhado.

### Decisão

**parking_lot::Mutex** desde CO-203 (2.3.4).

### Por quê

`std::sync::Mutex` é envenenado quando um thread panica enquanto
segura o lock. Toda aquisição posterior falha com `PoisonError`.

Em ambiente de servidor de longa duração:

- Workers (notif email, push) seguram o lock periodicamente.
- Qualquer panic dentro do lock envenena o Mutex para a vida do processo.
- Todos os handlers retornam 500 "storage lock" → cascata de falha
  site-wide.
- O incidente 2026-05-12 (que produziu 2.3.1, 2.3.2 hotfixes) foi
  exatamente isso.

parking_lot::Mutex:

- Não tem semântica de poison.
- Um panic dentro do lock libera o lock normalmente (RAII via drop).
- A próxima aquisição vê um lock saudável.
- Resultado: panic mata 1 request, não 1 app.

Trade-off: parking_lot não está em std (mais um dep externo).
Justificativa: 1 dep externo « 12 dias de incidente reativo.

### Outras propriedades

- parking_lot é mais rápido em workloads não-contestados (~10% mais rápido)
- Tem fairness melhor (FIFO instead of best-effort)

### Catálogo: dependencies.md §3 → `parking_lot`

---

## 4. ES256 (P-256 ECDSA) para handover JWT, HS256 para sessão

JWT signing em duas variantes.

### Decisão

- **Handover tokens (cross-domain SSO)**: ES256 com JWKS público
- **Session tokens (cookie + Authorization)**: HS256 com `JWT_SECRET`

### Por quê ES256 para handover

Cross-domain SSO precisa que:

- Quilombo, yggdrasil, artelonga verifiquem tokens emitidos pelo CO.
- Sem compartilhar segredos entre apps.

Solução padrão: **JWKS** (RFC 7517) — CO publica a chave pública em
`/.well-known/jwks.json`. Receivers fetch + cache. Validação é com a
pública.

HS256 (HMAC) exigiria compartilhar `JWT_SECRET` entre apps — operacional
pesadelo de rotação.

ES256 vs RS256:

- ES256: chave de 32 bytes + assinatura curta (64 bytes)
- RS256: chave de 256+ bytes + assinatura ~256 bytes
- ES256 cabe em URL como query param (`?co_token=...`); RS256 não

### Por quê HS256 para sessão

Session cookie é validado **pelo mesmo servidor** que emitiu. Não há
benefício de ter chave pública / privada separada — só overhead.

HMAC com `JWT_SECRET` (Fly secret, ~256 bits) é:

- Mais rápido (no public-key ops)
- Mais simples (uma chave em vez de keypair)
- Suficiente para o caso de uso

Roadmap: migrar sessão também para ES256 alinha tudo no mesmo modelo,
mas trade-off é micro-benchmark performance vs operational uniformity.

### Catálogo: dependencies.md §4 → `jsonwebtoken`, `p256`

---

## 5. SQLite + parking_lot Mutex em vez de Postgres

Banco de dados principal.

### Decisão

**SQLite single file** em `/data/co.db`, WAL mode, exclusivo via
parking_lot::Mutex.

### Por quê não Postgres

Postgres seria adequado para CO se:

- Múltiplos servidores precisarem do mesmo dataset
- Replication / hot standby fosse necessária
- Concurrent writes de múltiplas máquinas

Nenhum é verdade hoje:

- CO roda em **uma** máquina Fly (single-region, single-instance)
- Backup via volume snapshot (CO-143)
- Single-writer concurrency é suficiente

Custos Postgres:

- Operacional: migrations, conexões, pool, vacuum
- Latência: round-trip rede mesmo localhost
- Setup: separar processo, configurar persistência, monitorar

SQLite benefícios:

- Latência: ~1µs por query (in-process)
- Backup: copy /data/co.db
- Migrations: simples + idempotentes (CO usa `ensure_table` / `ensure_column`)
- Sem operacional separado

### Quando virar Postgres

Trigger CO-76 (scalability infrastructure) sinaliza:

- > 100k usuários ativos simultâneos
- Necessidade de multi-region
- Concurrent writes saturando o WAL

Não estamos lá. Pode levar anos.

### Catálogo: dependencies.md §3 → `rusqlite`, `parking_lot`

---

## 6. lettre + Resend cascade em vez de SDK proprietário

Email transactional.

### Decisão

Cascade: **Resend HTTP API** primária → **SMTP** fallback (lettre) → **log** fallback.

### Por quê não Resend SDK exclusivo

Resend tem um Rust SDK oficial, mas:

- Bloca em uma única vendor
- Se Resend ficar fora do ar, todo o app fica sem email
- SDK não fala SMTP — quando o servidor está em região onde Resend tem
  problemas, não temos fallback

Cascade resolve:

- Primário Resend: rápido, tracking, dashboards
- Fallback SMTP via lettre: funciona com qualquer provedor (Gmail SMTP,
  Postfix self-hosted, AWS SES)
- Fallback log: dev mode + emergências (operador grep nos logs)

Trade-off: mais código pra manter. Justificativa: email é primary
recovery channel — não pode estar em vendor lock-in.

### Por quê lettre

- Pure Rust, sem OpenSSL
- Suporte STARTTLS + implicit TLS
- Maintained ativamente
- Simples API

### Catálogo: dependencies.md §5 → `lettre`

---

## 7. Vanilla JS frontend em vez de React / Vue / Svelte

SPA do co-web.

### Decisão

ES modules nativos, sem bundler, sem framework. Em produção (2026-05-13):
~6000 linhas de JS hand-written em `static/variants/a/modules/`.

### Por quê

CO é uma plataforma sobre soberania digital. Frontend é a superfície
mais auditável — usuários técnicos podem ver exatamente o que roda no
browser.

Vanilla ES modules:

- **Source = what runs.** Sem minificação, sem bundling, sem source maps.
- **Zero deps npm em runtime.** Não há "atualização do esbuild quebra a build".
- **Hot reload nativo.** Cada módulo é cacheado independentemente.
- **Aprendível.** Qualquer dev com JS sabe ler.

Frameworks adicionam:

- Build complexity
- Surface of attack (npm supply chain)
- Reactivity model que esconde quando re-render acontece
- Tooling lock-in

CO escolheu trade-off oposto: mais código manual em troca de menos
opacity.

### Quando reconsiderar

Para o **viewer / read-pretty** experience (CO-212), Svelte + TS faz
sentido — pequeno escopo, novo código, foco em rendering. Mantém o
operational SPA atual.

### Catálogo: dependencies.md §7

---

## 8. AGPL v3 em vez de MIT / GPL v3

Licença.

### Decisão

**AGPL v3** (default proposed, sujeito a confirmação por todos os
contribuidores).

### Por quê não MIT

CO foi MIT até 2026-05. MIT permite que alguém:

1. Forke o código
2. Adicione features proprietárias
3. Rode como SaaS sem retornar nada à comunidade

Para uma plataforma de **soberania digital**, isso é incoerente: o
projeto promete "seus dados são seus" mas a licença permite que
alguém construa uma versão que mente sobre essa promessa.

### Por quê AGPL em vez de GPL

GPL v3 cobre **redistribuição** mas não **uso de rede**. Se alguém
roda uma fork modificada como SaaS sem distribuir o binário, GPL
não obriga liberação de fonte.

AGPL v3 fecha essa brecha: "se você roda como serviço, deve liberar
fonte mesmo sem distribuir binário".

AGPL é controverso porque:

- Alguns empregadores proíbem AGPL deps por medo de "contaminar" code
- Algumas plataformas (Google Play, AWS) têm policies anti-AGPL

Trade-off aceitável: CO é serviço, não biblioteca embebida. Empresas
que querem usar CO podem fazer self-host ou pagar licença comercial
separada (futuro).

Ver [/licensa](../licensa.md) para explicação completa em PT.

### Catálogo: N/A — licença, não dep

---

## Política de revisão dessas decisões

Cada decisão acima foi calibrada para o estado em 2026-05. Revisar:

- **Anualmente**: parâmetros de Argon2id (memory-hard), tamanho de chaves
- **Em CVE crítica**: reavaliar qualquer crate cripto
- **Em mudança de provedor**: reavaliar AES-NI assumption (ChaCha continua bom)
- **Em mudança de escala**: SQLite vs Postgres (CO-76 trigger)

Mudanças significativas precisam:

1. RFC interno (issue / PR de discussão)
2. Confirmação que migration path está claro
3. CHANGELOG entry explicando o porquê (não só o quê)
