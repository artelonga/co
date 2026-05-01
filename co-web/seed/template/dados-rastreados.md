---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-04-26T00:00:00+00:00
order: 12
slug: dados-rastreados
tags:
- legal
title: Lista completa de dados rastreados
type: page
---

# Lista completa de dados rastreados

Última atualização: abril de 2026

Esta página lista **TODOS** os dados que o Co coleta na instância gerenciada pela Arte Longa (`co.artelonga.com.br`). Nada além disto é armazenado. Para auto-hospedagem, você é o controlador e decide o que rastrear.

> Versão verificável: <https://github.com/artelonga/co/blob/main/data/universes/template/content/dados-rastreados.md>

## 1. Conta (apenas usuários logados)

| Campo | Quando | Por quê |
|-------|--------|---------|
| `id` | Criação da conta | Identificador interno (nanoid) |
| `email` | Cadastro e login | Autenticação, recuperação |
| `display_name` | Cadastro (opcional) | Exibição na UI |
| `password_hash` | Cadastro/troca de senha | Argon2id — senha original nunca armazenada |
| `tier` | Atribuído pelo sistema | Controle de acesso (`anonymous`, `user`, `admin`) |
| `created_at` | Criação da conta | Auditoria |
| `last_login_at` | Cada login bem-sucedido | Segurança |

## 2. Conteúdo (universos)

| Campo | Descrição |
|-------|-----------|
| `path` | Caminho relativo do arquivo no universo |
| `entry_type` | `task`, `page`, `event`, `clip`, etc. |
| `title` | Título extraído do frontmatter ou primeira linha |
| `frontmatter_json` | Metadados YAML do frontmatter |
| `body` | Corpo Markdown — **texto puro hoje** (cifragem em repouso é roadmap v3.0) |
| `body_hash` | xxh3 para detecção de alterações |
| `created_at`, `updated_at` | Timestamps |

## 3. Cookies

| Cookie | Finalidade | Duração |
|--------|-----------|---------|
| `session` | JWT de autenticação | 30 dias |
| `co_lang` | Idioma da interface (pt/en) | 1 ano |
| `co_named_palette` | Tema visual escolhido | 1 ano |
| `co_local_universe` | Universo anônimo local (slug) | Sessão |
| `co_user_palette` | Override de tema do usuário | 1 ano |
| `al_vid` | Token de visitante para analytics — sem papel de autenticação, sem PII; unifica atribuição entre o site de marketing e o Co (ADR-001). Escopo: `.artelonga.com.br`. Legível por JS. | 1 ano |

**Não usamos:** fingerprinting, supercookies, evercookies, cookies de rastreamento de terceiros.

## 4. Logs técnicos

| Campo | Descrição | Anonimização |
|-------|-----------|--------------|
| `path` | Rota acessada | — |
| `method` | GET/POST/PUT/DELETE | — |
| `status` | Código HTTP | — |
| `duration_ms` | Latência | — |
| `ip_hash` | Hash diário do IP | SHA256(IP + sal_diário) — irreversível |
| `ua_device` | desktop/mobile/tablet | Categoria, não fingerprint |
| `ua_browser` | chrome/firefox/safari/etc | Família, não versão exata |
| `ua_os` | windows/macos/linux/ios/android | Família, não versão exata |

Retenção: **90 dias**.

## 5. Telemetria de UI (opt-in, agregada)

Coletada apenas se você não habilitou Do Not Track e aceitou o banner.

| Evento | Payload |
|--------|---------|
| `pageview` | `path`, `referrer_path`, `duration_ms`, `theme`, `lang` |
| `task.create` | `universe`, `project`, `status` (sem título nem corpo) |
| `task.update` | `universe`, `project`, `field_changed` (sem valor) |
| `task.delete` | `universe`, `project` |
| `task.drag` | `from_status`, `to_status` |
| `theme.change` | `from`, `to` |
| `lang.switch` | `from`, `to` |
| `universe.create` | `slug` (do universo criado) |
| `universe.clone` | `source_slug`, `new_slug` |
| `auth.login` | `success`, timestamp |
| `auth.logout` | timestamp |
| `modal.open` | nome do modal (`create-task`, `settings`, etc.) |
| `search.query` | hash do termo (xxh3, não o termo em si) |
| `error` | `error_type`, `error_path`, `error_message` (sanitizada) |

**O que nunca é enviado:** títulos, corpos, IPs brutos, e-mails, conteúdo de busca em texto puro.

## 6. Performance

| Métrica | Descrição |
|---------|-----------|
| `page_load_ms` | Tempo total de carregamento |
| `time_to_interactive_ms` | Tempo até a UI responder |
| `api_call_duration_ms` | Latência por endpoint |
| `ws_connect` | Status da conexão WebSocket |
| `cache_hit` | HIT/MISS de cache |

## 7. Identificadores anônimos

| Campo | Como é gerado | Reverter? |
|-------|---------------|-----------|
| `visitor_token` | nanoid aleatório, salvo em cookie | Não, mas você pode apagar o cookie |
| `session_id` | Aleatório, expira em 30 min | Não |
| `ip_hash` | SHA256(IP + sal diário) | Não — sal rotaciona a cada 24h |

## 8. O que NÃO rastreamos

- ❌ Endereço IP bruto
- ❌ Senhas (apenas hash Argon2id)
- ❌ Conteúdo de notas, tarefas ou mensagens em telemetria
- ❌ Cookies de terceiros
- ❌ Pixels de publicidade
- ❌ Fingerprinting de navegador (canvas, WebGL, fontes, áudio)
- ❌ Dados biométricos
- ❌ Localização precisa (GPS, Wi-Fi triangulation)
- ❌ Histórico fora do Co (referrers cross-site são truncados ao domínio)
- ❌ Identificadores de outras plataformas (sem login social hoje)
- ❌ Microfone, câmera, sensores
- ❌ Lista de contatos, agenda, qualquer dado fora do escopo declarado

## 9. Compartilhamento com terceiros

| Provedor | O que recebe | Por quê | DPA |
|----------|--------------|---------|-----|
| **Fly.io** | Tudo que está no banco (criptografado em trânsito) | Hospedagem | <https://fly.io/legal/dpa/> |

**Apenas isso.** Sem Google Analytics, Mixpanel, Amplitude, Segment, Hotjar, Sentry remoto, FullStory, LogRocket, etc.

## 10. Retenção

| Categoria | Tempo |
|-----------|-------|
| Eventos de telemetria detalhados | 90 dias |
| Logs técnicos | 90 dias |
| Agregados anônimos (sem ligação com identidade) | Indefinido |
| Conta ativa | Enquanto a conta existir |
| Universos anônimos inativos | 90 dias |
| Após exclusão de conta | 30 dias até remoção completa |

## 11. Seus direitos (LGPD)

A qualquer momento você pode:
- Solicitar uma cópia dos seus dados: yuri@artelonga.com.br
- Pedir exclusão completa
- Desativar telemetria (cookie consent ou Do Not Track)
- Exportar todo seu conteúdo (Markdown nativo, sempre disponível)

---

*Este documento é exaustivo. Se acharmos algo não listado, atualizamos imediatamente — abra issue em <https://github.com/artelonga/co/issues>.*
