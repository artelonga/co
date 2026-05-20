---
created: 2026-05-18T00:00:00+00:00
modified: 2026-05-18T00:00:00+00:00
order: 11
slug: seguranca-criptografia
tags:
- seguranca
- criptografia
- privacidade
- senhas
title: Criptografia e armazenamento
type: page
---

# Criptografia e armazenamento

Como cada tipo de dado é guardado, hasheado, criptografado ou apenas indexado. Esta página é o **espelho fiel do código** — cada parágrafo aponta para o arquivo e linha real. Quando o código mudar, a página muda junto.

## Sumário

- [Senhas](#senhas)
- [Sessões e JWT](#sessões-e-jwt)
- [Cookies de sessão](#cookies-de-sessão)
- [Tokens de API (longos)](#tokens-de-api-longos)
- [Códigos de verificação](#códigos-de-verificação)
- [Canais de recuperação](#canais-de-recuperação)
- [Conteúdo em repouso](#conteúdo-em-repouso)
- [Anexos e arquivos binários](#anexos-e-arquivos-binários)
- [O que é registrado em logs](#o-que-é-registrado-em-logs)
- [Lacunas conhecidas](#lacunas-conhecidas)

## Senhas

**Algoritmo:** Argon2id — recomendação OWASP atual para hashing de senha.

**Parâmetros:** `Argon2::default()` da crate `argon2` (`co-web/src/recovery_routes.rs:172-181`). Padrões da biblioteca: `t=2, m=19MB, p=1` (memory-hard, resistente a ataque com GPU/ASIC). Cada senha tem **salt aleatório de 16 bytes** gerado via `OsRng`.

**Verificação:** `PasswordVerifier::verify_password` em tempo constante (resistente a timing-attack).

**Onde está:** mesma função usada para:
- Senhas de usuário (`co-web/src/auth.rs:840-870`, login)
- Códigos de verificação curtos (forgot-password, recovery channels)
- Hashes de seed-admin via env `CO_SEED_ADMIN_PASSWORD_HASH` (gerado localmente com `argon2 -id -t 3 -m 16 -p 1`)

**Não armazenamos a senha em si — apenas o hash + salt no banco.** Mesmo o operador (Co Platform) **não consegue** recuperar a senha original.

## Sessões e JWT

**Algoritmo:** ES256 (ECDSA P-256) — assinatura assimétrica.

**Chave:** par EC P-256 gerado uma única vez por instalação, persistido em volume Fly. A chave pública é exposta via JWKS para SSO cross-domain (CO-166).

**TTL:** 7 dias (`JWT_EXPIRY_SECS = 604_800` em `co-web/src/auth.rs:31`).

**Compatibilidade reversa:** tokens HS256 antigos ainda são aceitos durante a decodificação (`co-web/src/auth.rs:245-260`) para não invalidar sessões existentes durante upgrades — mas todos os tokens **novos** já são ES256.

**Por que ES256 em vez de HS256?** ES256 permite que serviços externos (oferecidos como sub-universos abertos) **verifiquem** tokens sem precisar de uma chave compartilhada. A chave privada nunca sai do servidor; a pública é distribuída via JWKS.

## Cookies de sessão

**Atributos do cookie de sessão:**

| Atributo | Valor | Por quê |
|---|---|---|
| `HttpOnly` | sim | JavaScript não pode ler — protege contra XSS roubando o JWT |
| `SameSite=Lax` | sim | CSRF protection: cookie só é enviado em navegação de mesmo site (exceto requests GET top-level) |
| `Path=/` | sim | Disponível em toda a aplicação |
| `Max-Age` | 604800 (7 dias) | Equivalente ao TTL do JWT |
| `Secure` | em produção (TLS) | Cookie só viaja por HTTPS |
| `Domain` | `.artelonga.com.br` (quando configurado) | Cross-subdomínio para SSO |

**Implementação:** `build_session_cookie()` em `co-web/src/auth.rs:270-285`.

**Implicação prática:** o JWT na sessão **não pode ser lido por JavaScript no navegador**. Bugs anteriores que testavam `document.cookie` para detectar login (ex.: CO-234) foram corrigidos para usar o objeto `me` que vem da API — esse é o sinal correto de autenticação no client.

## Tokens de API (longos)

**Formato:** `co_<40 caracteres nanoid>` (~240 bits de entropia).

**TTL:** 90 dias (`co-web/src/storage/api_tokens.rs:56`).

**Uso:** `Authorization: Bearer co_<token>` em qualquer endpoint que aceita JWT — substitui o cookie em scripts/CLI.

**Onde armazenamos:** tabela `api_tokens` no `meta.db`. Colunas: `id`, `user_id`, `name`, `token_hash`, `token_prefix`, `created_at`, `expires_at`, `last_used_at`.

**Hashing:** o token bruto nunca é persistido. No momento da criação, calculamos `SHA-256(token)` e armazenamos apenas o hash hexadecimal na coluna `token_hash`. No lookup, fazemos o mesmo: `SHA-256(bearer_value)` é comparado contra `token_hash` via query SQL — nenhum texto puro toca o banco. A coluna `token_prefix` guarda os primeiros 11 caracteres (ex.: `co_abc12345`) para exibição na listagem de tokens.

**Migração CO-237 (2.11.7):** todos os tokens anteriores à versão 2.11.7 foram invalidados. Usuários com tokens antigos precisam gerar um novo via `POST /api/v1/auth/token`. Essa é uma breaking change documentada para portadores de tokens CLI/agente.

**Proteções:**
- O banco está em volume Fly criptografado-at-rest (camada de infraestrutura, não do app)
- Tokens são revogáveis individualmente (`DELETE /api/v1/auth/tokens/:id`)
- Cada token tem um nome (não anônimo — facilita auditoria via `GET /api/v1/auth/tokens`)
- Acesso ao banco em produção exige SSH via Fly + chave do operador
- Um dump do banco expõe apenas hashes SHA-256 irreversíveis — não os tokens em si

**Recomendação para usuários:** trate tokens como senhas. Não comite em git, não cole em chat público, rotacione a cada 90 dias.

## Códigos de verificação

**Casos de uso:** confirmação de email/telefone, forgot-password, magic-link, recovery channel verify.

**Formato:** 6 dígitos numéricos (ou 8 alfanuméricos em alguns fluxos), gerados via `OsRng`.

**TTL:** 5 minutos (`CODE_EXPIRY_SECS = 300` em `co-web/src/auth.rs:28`).

**Armazenamento:** **Argon2id-hashed** no banco, igual a senhas. O código em texto puro nunca é persistido — só o hash. Mesmo o operador não pode ler o código de outra pessoa.

**Resgate em texto puro:** apenas no momento do envio para o canal verificado (email/SMS). Após envio, o servidor mantém só o hash.

## Canais de recuperação

**O que é:** email ou telefone marcado como "verificado" para reset de senha (CO-165).

**Por que precisam de cuidado:** se um operador puder ler todos os emails de recovery, ele pode forçar reset em qualquer conta.

**Solução:** o valor do canal (email/telefone) é **criptografado** com ChaCha20-Poly1305 antes de persistir (`co-web/src/recovery_crypto.rs`). A chave de criptografia vem de um KEK (Key Encryption Key) por instalação, não comitada no código.

**Lookup:** para encontrar uma conta por email, o servidor calcula um **identificador determinístico** (HMAC-SHA256 com sub-chave de busca) do email — isso permite "buscar pelo email" sem desencriptar nada.

## Conteúdo em repouso

**Entradas de texto (.md, frontmatter, board, chat):** armazenadas **em texto puro** no SQLite per-universo, com a flag `encrypted = 0`.

**Por quê em texto puro?** Performance (busca textual full-text, indexação) e simplicidade (backup, migração, debug).

**Mitigação:** o banco em si está em volume Fly com criptografia at-rest (LUKS-style no nível de bloco). Operadores Fly não conseguem ler diretamente; apenas o app — após boot, com chave de descriptografia em memória — vê o conteúdo.

**Universos com criptografia opt-in:** linhas com `encrypted = 1` são ChaCha20-Poly1305 com `nonce` por linha (`co-web/src/universe_pool.rs:359+`). Esse modo é usado para conteúdo sensível (mensagens DM, drafts privados — CO-148 Phase 3+).

## Anexos e arquivos binários

**Algoritmo:** ChaCha20-Poly1305 com chave por-universo (CO-148).

**Nonce:** 12 bytes aleatórios por blob, persistido junto.

**AAD:** `universe_key || sha256(filename)` — vincula a descriptografia ao universo correto (impossível "renomear" um anexo para outro universo).

**Onde:** `co-web/src/asset_crypto.rs:39-115`. Cada anexo (foto, PDF, áudio) é cifrado antes de gravar em disco; descifrado on-the-fly no GET (com verificação AEAD — qualquer mexida no ciphertext rejeita a leitura).

**Chave por universo:** derivada da chave-mestre do servidor + slug do universo via HKDF.

**Cenário de comprometimento:** se um operador rouba o disco do Fly, ele tem **apenas ciphertext** — sem a chave-mestre (que mora em variável de ambiente fora do disco) não há como ler os anexos.

## O que é registrado em logs

**Logs estruturados (`tracing`):**
- Eventos de auth: `login_success`, `login_failed`, `password_reset_requested` — incluem `user_id` mas **não** o IP nem o email (apenas o hash determinístico de busca).
- Erros do servidor: traços de execução + IP de origem (necessário para diagnóstico de abuso).
- Acessos à Vault API: rota + método + status, **sem corpo**.

**Logs NÃO incluem:**
- Senhas, códigos, tokens em texto puro
- Corpo de mensagens DM ou de chat
- Conteúdo de entradas privadas

**Retenção de logs:** 14 dias (Fly default). Após esse período, descartados sem backup.

## Lacunas conhecidas

Cada lacuna tem um ticket público correspondente — você pode acompanhar o progresso no board:

| Lacuna | Ticket | Status |
|---|---|---|
| Sessões expiram em 7 dias (sem refresh token automático) | parte de CO-235 ("co clone") + melhorias futuras | em design |
| Sem 2FA / TOTP ainda (recuperação é apenas via canais verificados) | a planejar | pendente |
| Logs de erro podem incluir IP por 14 dias | aceito (necessário para diagnóstico) | documentado |

## Páginas relacionadas

- [Segurança — visão geral](./seguranca)
- [Cenários de ataque e defesa](./seguranca-cenarios)
- [Dependências e auditoria](./seguranca-dependencias)
- [Push notifications (VAPID)](./seguranca-vapid)
- [Conta e mensagens — como funciona](./conta-e-mensagens)
- [Privacidade](./privacidade)
- [Termos](./termos)

---

**Esta página é viva.** Se você encontrar algo aqui que não bate com o código, abra um ticket — a página segue o código, não o contrário.
