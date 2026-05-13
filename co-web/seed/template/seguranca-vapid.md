---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 13
slug: seguranca-vapid
tags:
- seguranca
- vapid
- push-notifications
- transparencia
title: VAPID e Notificações Push
type: page
---

# VAPID e Notificações Push

O que um vazamento da chave VAPID privada do Co realmente permite, o que **não** permite, e como mitigamos.

Voltar para [Segurança](/co/template?page=seguranca).

## O que é VAPID

VAPID (Voluntary Application Server Identification) é um esquema de **auto-autenticação do servidor de push** — RFC 8292. Co mantém um par de chaves ES256 (P-256 ECDSA):

- **Chave pública** — exposta em `GET /api/v1/notifications/vapid-public-key`. Browsers a usam durante `PushManager.subscribe()` e a registram na endpoint do push service. **Pública de propósito** — não tem valor de segurança em si.
- **Chave privada** — Fly secret. Usada pelo push worker para assinar um JWT curto (`Authorization: vapid t=<jwt>, k=<public_key>`) que acompanha cada requisição de push para o serviço do browser (Mozilla autopush, Apple, FCM).

Push services validam o JWT contra a chave pública registrada na subscription. **Sem JWT VAPID válido, a requisição é rejeitada.**

## O que VAPID NÃO faz

| O que **não** garante | Por quê importa |
|---|---|
| **Não criptografa o payload** | Criptografia da mensagem usa as chaves do **subscriber** (`p256dh` + `auth`), RFC 8291. VAPID é só autenticação do sender. |
| **Não autentica o destinatário** | Qualquer um com `endpoint` + `p256dh` + `auth` de um usuário consegue criptografar uma payload para ele. VAPID só diz "este sender é legítimo." |
| **Não dá acesso a contas de usuário** | VAPID é canal de push apenas. Comprometimento não rende tokens de sessão, senhas, ou dados do Co. |
| **Não dá leitura de histórico de push** | Push service não retém mensagens entregues. Co registra timestamps de delivery, não corpos (só `tag` + `url` no fio). |

## Threat model — o que aconteceria se vazasse

### Cenário A: Apenas `VAPID_PRIVATE_KEY` vaza

**Quase nada útil.** Sem dados de subscription, attacker pode assinar JWTs VAPID válidos mas não tem `endpoint` / `p256dh` / `auth` para atacar. Uma push request precisa de todos os três.

**Severidade: BAIXA.**

### Cenário B: `VAPID_PRIVATE_KEY` + `push_subscriptions` table vazam

**Spoofing de push notifications.**

Concretamente:

1. Attacker fetcha `endpoint`, `p256dh`, `auth` do DB leaked
2. Constrói payload (título, corpo, url) de sua escolha
3. Criptografa com `p256dh` + `auth` (RFC 8291 subscriber-keyed encryption)
4. Assina JWT VAPID com a chave privada vazada
5. POST para o `endpoint` do usuário
6. Push service valida o JWT, **aceita como legítimo do Co**
7. Browser do usuário mostra notificação que parece vir do Co

**Ataques realistas**: phishing ("sua senha expira em 1h, clique aqui"); brand damage; engenharia social direcionada.

**Severidade: MÉDIA.**

### O que VAPID compromise NÃO permite

- Ler dados do usuário
- Decriptar push messages passadas
- Adicionar/remover subscriptions
- Modificar preferências de notificação
- Impersonar o usuário em direção ao servidor

## Armazenamento e rotação

### Onde a chave privada VIVE

| Local | Status |
|---|---|
| Fly secrets (`VAPID_PRIVATE_KEY`) | ✅ Principal — encrypted at rest, runtime injection |
| Password manager (1Password) | ✅ Backup necessário para rotação |

### Onde NÃO pode viver

| Local | Risco |
|---|---|
| Git repo (qualquer branch) | Risco público via clone / history |
| Source code mesmo comentado | Mesmo problema |
| Plaintext em disco | Forensics, backup acidental |
| Chat com IA / Slack / email | Logado em servidor terceiro |
| Bash history | Outros processos podem ler |
| Env vars expostos fora do momento de set | `/proc/<pid>/environ` |

### Procedimento de rotação

Quando rotacionar:

- **Imediato**: vazamento suspeito (chave em log, screenshot, lost laptop)
- **Anual**: defesa em profundidade
- **Em troca de operador**: alguém com acesso ao secret store sai

Como rotacionar:

1. Gerar par novo localmente (`/tmp/vapidgen.sh`)
2. `flyctl secrets set VAPID_PUBLIC_KEY=... VAPID_PRIVATE_KEY=...`
3. Restart Fly machine
4. Atualizar password manager (deletar entry antigo após confirmação)
5. **Todas as subscriptions atuais ficam inválidas** — usuários re-subscrevem na próxima visita

## Detecção

Como notaríamos um compromise:

- Relatos de usuários: "recebi uma notificação estranha"
- Padrões anômalos nos logs (`flyctl logs | grep "push delivered"`) — bursts para muitos usuários em horários incomuns, URLs apontando para fora de `co.artelonga.com.br`
- Reclamações sobre conteúdo phishing-shaped

**TO BE**: tabela de audit de push payloads (signed hash, append-only, alarme em anomalia) — não implementado hoje.

## TL;DR

- **VAPID private key alone é baixo-valor** para attacker
- **Combined com subscriber DB, é toolkit de phishing**
- **Tratamos como MÉDIO-sensitive** — mesmo nível que Resend API key
- **Armazenamento: Fly secrets + password manager apenas**
- **Rotação anual ou em suspeita**
- **Confidencialidade do payload é garantia separada** via RFC 8291

---

Voltar para [Segurança](/co/template?page=seguranca) · [Cenários de Red Team](/co/template?page=seguranca-cenarios) · [Dependências](/co/template?page=seguranca-dependencias).
