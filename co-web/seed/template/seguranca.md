---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 10
slug: seguranca
tags:
- seguranca
- privacidade
- soberania
title: Segurança
type: page
---

# Segurança em Co

**Princípio:** seus dados são seus. Co é uma plataforma de **soberania digital** — onde seus dados, mensagens e comunidades pertencem a você, não ao operador.

Este documento é vivo: descreve o estado atual + o caminho para fechar as lacunas conhecidas.

## O que essa página garante

- **Transparência radical**: você lê aqui exatamente como seus dados são protegidos, e onde as lacunas ainda estão
- **Modelo de ameaças explícito**: definimos contra o quê protegemos e o quê **não** protegemos (ainda)
- **Caminho de melhoria contínua**: cada lacuna conhecida vira ticket público

## Sumário

- [Modelo de ameaças](#modelo-de-ameaças)
- [O que protegemos](#o-que-protegemos)
- [O que NÃO protegemos (ainda)](#o-que-não-protegemos-ainda)
- [Camadas de defesa](#camadas-de-defesa)
- [Páginas relacionadas](#páginas-relacionadas)

## Modelo de ameaças

| Persona | Vetor principal |
|---|---|
| **Usuário comum** | Phishing, conta tomada, mensagens vazadas |
| **Comunidade / universo** | Vazamento de membro privado para fora do universo |
| **Operador** | Comprometimento de chaves, perda de backup, ransomware |

### Contra quê protegemos

- Tomada de conta (credential stuffing, phishing, sessão sequestrada)
- Vazamento de mensagens privadas e DMs
- Acesso não autorizado a universo privado
- Vazamento de email, e eventualmente CPF/nome quando armazenado
- Adulteração de conteúdo (sabotagem)

### Contra quê NÃO protegemos (ainda — TO BE)

Honestidade explícita:

- **End-to-end encryption** para chat: ainda não. Mensagens são cifradas em trânsito (TLS) e o volume é cifrado pelo provedor, mas o servidor lê o texto puro. Operador honesto pode ler qualquer mensagem se quiser. Caminho: zona de computação criptografada (operator-cannot-read).
- **Zero-knowledge**: ainda não. Co é "trust the operator" hoje.
- **Anonimato perfeito**: não. Co sabe o IP do navegador (hashed diariamente, descartado). Em ambientes hostis, use Tor.
- **Defesa contra estado-nação**: não. Co é defesa contra ataque oportunista, não contra adversário com recursos arbitrários.

## O que protegemos

### Camadas de defesa

Co usa **defense in depth** — uma falha em uma camada não causa comprometimento total.

| Camada | Propriedade principal |
|---|---|
| **Transporte** | TLS 1.2+ obrigatório. HSTS, X-Frame-Options DENY, Referrer-Policy strict-origin-when-cross-origin |
| **Aplicação** | SQL injection mitigado via parameterized queries; CSRF via origin allowlist; CORS via mirror-request controlled |
| **Autenticação** | JWT HS256 para sessão, ES256 via JWKS para handover cross-domain |
| **Senhas** | Argon2id (m=19 MiB, t=2, p=1) — nunca em texto |
| **Canais de recuperação** | ChaCha20-Poly1305 cifrado em coluna |
| **Volume em disco** | Cripto pelo provedor (LUKS) |
| **Secrets** | Encrypted at rest, runtime injection only — provider-agnostic |

### Tipos de dados protegidos

| Categoria | Como |
|---|---|
| Email | Plaintext em SQLite no volume cripto. Único identificador primário. |
| Senha | **Nunca armazenada.** Só hash Argon2id em `users.password_hash` |
| Mensagens de chat | Plaintext (lido só por membros do universo). E2E roadmap. |
| Mensagens privadas (DM) | Plaintext (lido só pelas 2 partes). E2E roadmap. |
| Tokens de sessão | JWT assinado, expira em 7 dias, SameSite=Lax cookie |
| Códigos de recuperação | Argon2id hash, válido 15 min, lockout após 5 tentativas |
| IP do visitante | SHA-256 com salt diário, IP raw nunca persistido |

### Como reportar uma vulnerabilidade

Email para **yuri@artelonga.com.br** com descrição, passos para reproduzir, e impacto observado. Resposta dentro de 72h. Vulnerabilidades críticas recebem hotfix prioritário.

**Não divulgar publicamente antes** do fix estar em produção — prática responsible disclosure padrão.

## Páginas relacionadas

- [Dependências](/co/template?page=seguranca-dependencias) — quais bibliotecas e por quê escolhemos cada uma
- [Cenários de Red Team](/co/template?page=seguranca-cenarios) — ataques considerados + playbook de resposta
- [VAPID e notificações push](/co/template?page=seguranca-vapid) — modelo de ameaça específico
- [Licença](/co/template?page=licensa) — AGPL v3 e o que isso significa
- [Visualizadores de markdown](/co/template?page=renderers) — opções para ler conteúdo Co

---

**Documento vivo.** Atualizado: 2026-05-13. Versão canônica + roadmap completo no repositório: `docs/security/SECURITY.md`.
