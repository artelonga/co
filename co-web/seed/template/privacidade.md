---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-04-26T00:00:00+00:00
order: 11
slug: privacidade
tags:
- legal
title: Política de Privacidade
type: page
---

# Política de Privacidade

Última atualização: abril de 2026

## Resumo

- **Você é dono do seu conteúdo.** Markdown nativo, exportável a qualquer momento.
- **Hospedagem flexível:** rode o Co no seu próprio servidor (código MIT) ou use a instância gerenciada pela Arte Longa.
- **Privado por padrão:** universos novos são privados; só você e quem você convidar enxerga.
- **Sem rastreadores de terceiros:** nada de Google Analytics, Facebook Pixel, ads ou fingerprinting.
- **Transparência total:** lista exaustiva do que coletamos em [dados rastreados](/co/template?path=content/dados-rastreados.md).

## 1. Dois modelos de hospedagem

O Co é software livre (MIT). Você escolhe onde seus dados moram:

### Auto-hospedagem
Você roda o Co no seu próprio servidor (laptop, VPS, Raspberry Pi, qualquer coisa). **A Arte Longa não tem acesso aos seus dados nessa modalidade.** Esta política não se aplica — você é o controlador.

### Instância Arte Longa (`co.artelonga.com.br`)
Hospedada na Fly.io, região São Paulo (GRU). A Arte Longa é a controladora dos dados. Esta política descreve essa modalidade.

## 2. Dados coletados

O mínimo necessário para o serviço funcionar:

| Categoria | Dados | Por quê |
|-----------|-------|---------|
| **Conta** | e-mail, nome de exibição (opcional), hash Argon2id da senha | Autenticação |
| **Conteúdo** | textos, tarefas, arquivos que você cria nos seus universos | É o serviço |
| **Cookies** | sessão JWT, idioma, tema, universo anônimo local | Funcionamento |
| **Logs técnicos** | hash diário do IP, rotas acessadas, código de erro | Segurança e debug |
| **Telemetria opt-in** | eventos de UI agregados e anônimos | Melhorar a UX |

**Não coletamos:** IP bruto, conteúdo de mensagens via terceiros, dados biométricos, localização precisa, histórico fora do Co, identificadores cross-site.

## 3. Como protegemos seus dados

### Hoje (implementado)

- **Em trânsito:** TLS 1.3 em todas as requisições (HTTPS obrigatório).
- **Senhas:** hash Argon2id, nunca armazenadas em texto puro.
- **Tokens:** JWT com expiração; tokens de API armazenáveis no keychain do SO via `co-token`.
- **Controle de acesso:** modelo determinístico por universo (CO-49). Universos privados retornam 404 para quem não tem permissão.
- **Isolamento de banco:** SQLite no volume da Fly.io, montado apenas na máquina autorizada, acessível apenas via SSH com a chave do operador.
- **Sem terceiros:** sem CDN externa para JS, sem rastreador, sem analytics de terceiros.

### Honestidade sobre cifragem em repouso

**Os corpos das notas e tarefas são armazenados em texto puro no SQLite hoje.** Isto significa que o operador da instância (Arte Longa, no caso de `co.artelonga.com.br`) tecnicamente consegue ler conteúdo se acessar o servidor. Não fazemos isso, mas a possibilidade técnica existe.

**Roadmap (v3.0 — CO-86):** cifragem dos corpos com envelope ChaCha20-Poly1305 e chave derivada da senha do usuário, de modo que nem o operador consiga ler. Até lá:
- Para conteúdo sensível, **use auto-hospedagem** ou aguarde a v3.0.
- Para conteúdo público ou semi-privado (notas pessoais, kanban de projeto), o modelo atual é equivalente ao de qualquer SaaS comum (Notion, Trello, etc.).

Esta honestidade é deliberada — preferimos descrever o que está pronto hoje a vender uma promessa que ainda não cumprimos.

## 4. Uso dos dados

Seus dados são usados exclusivamente para:
- Autenticar sua conta e manter sua sessão
- Armazenar e exibir seu conteúdo nos seus universos
- Melhorar o produto (telemetria agregada e anônima, opt-in)
- Cumprir obrigações legais quando aplicável

**Não vendemos, alugamos, cedemos ou monetizamos seus dados.** Sem ads, sem retargeting, sem data brokers.

## 5. Cookies

Apenas estritamente necessários:

| Cookie | Finalidade | Duração |
|--------|-----------|---------|
| `session` | Autenticação (JWT) | 30 dias |
| `co_lang` | Idioma da interface | 1 ano |
| `co_named_palette` | Tema visual | 1 ano |
| `co_local_universe` | Universo anônimo local | Sessão |

## 6. Telemetria

Eventos agregados e anonimizados, com **opt-out automático** se você habilitou Do Not Track no navegador.

- IP convertido em hash diário (irreversível)
- Identificador de visitante é nanoid aleatório, sem ligação com identidade real
- Conteúdo de mensagens, notas ou tarefas **nunca é enviado**
- Lista exaustiva: [dados rastreados](/co/template?path=content/dados-rastreados.md)

Para desativar: opt-out em Configurações → Privacidade (em breve), ou habilite Do Not Track no navegador.

## 7. Seus direitos (LGPD)

Conforme a Lei 13.709/2018 (LGPD), você tem direito a:
- **Acessar** seus dados pessoais
- **Corrigir** dados incompletos ou inexatos
- **Excluir** sua conta e todo o conteúdo associado
- **Exportar** seu conteúdo (Markdown nativo, sempre disponível)
- **Revogar** consentimento de telemetria

Para exercer: yuri@artelonga.com.br — resposta em até 15 dias.

## 8. Retenção

- Conta ativa: dados mantidos enquanto você usar o serviço
- Universos anônimos (sem login): podem ser removidos após 90 dias de inatividade
- Conta excluída: dados removidos em até 30 dias (incluindo backups)
- Logs técnicos: 90 dias
- Agregados anônimos: indefinidamente (sem ligação com identidade)

## 9. Compartilhamento

A Arte Longa **não compartilha dados com terceiros**, exceto:
- **Provedor de infraestrutura:** Fly.io (hospedagem). Sujeito à [política de privacidade da Fly.io](https://fly.io/legal/privacy-policy/) e ao DPA correspondente, disponível mediante solicitação por se tratar de cliente corporativo.
- **Obrigação legal:** mediante ordem judicial fundamentada, conforme legislação brasileira.

Não usamos: Google Analytics, Facebook Pixel, Hotjar, Sentry remoto, intercom de chat, nem qualquer outro tracker de terceiros.

## 10. Crianças

O Co não é direcionado a menores de 13 anos. Não coletamos conscientemente dados de crianças.

## 11. Mudanças nesta política

Alterações relevantes serão comunicadas via banner na Plataforma com pelo menos 15 dias de antecedência. O histórico de versões está em <https://github.com/artelonga/co/commits/main/co-web/seed/template/privacidade.md>.

## 12. Contato

Controlador (instância gerenciada): Yuri Felipe Hild — yuri@artelonga.com.br
Encarregado (DPO): mesmo contato

---

*Arte Longa — Curitiba, PR, Brasil*
