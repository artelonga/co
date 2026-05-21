---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-05-21T00:00:00+00:00
order: 11
slug: privacidade
tags:
- legal
title: Política de Privacidade
type: page
---

# Política de Privacidade

Atualizada em 21 de maio de 2026.

## Resumo em uma linha

Você é dono do seu conteúdo, ele fica no servidor que **você** escolhe, e a Arte Longa não vende, aluga, nem repassa nada a terceiros.

## 1. Dois cenários

| Cenário | Quem é o controlador | Esta política se aplica? |
|---|---|---|
| **Auto-hospedagem** (você roda o Co num servidor seu) | Você | Não — você é o controlador |
| **Instância gerenciada** (`co.artelonga.com.br`) | Arte Longa | Sim |

O Co é software livre sob [AGPL v3](/co/public/licensa). Em qualquer cenário, o código é o mesmo e auditável.

## 2. O que coletamos

Só o estritamente necessário:

| Categoria | Dados | Por quê |
|---|---|---|
| Conta | e-mail, nome, hash Argon2id da senha | autenticação |
| Conteúdo | seus textos, tarefas, arquivos | é o serviço |
| Cookies essenciais | sessão JWT, idioma, tema | funcionamento |
| Logs técnicos | hash diário do IP, rota, código de erro | segurança/debug |
| Telemetria | eventos de UI agregados, anônimos, **opt-out via Do Not Track** | melhorar UX |

**Não coletamos:** IP bruto, biometria, localização precisa, identificadores cross-site, conteúdo de mensagens via terceiros, nem nada que permita perfilamento publicitário.

Detalhe exaustivo em [Dados Rastreados](/template/dados-rastreados).

## 3. Proteção

- **TLS 1.3** em todas as requisições.
- **Argon2id** para senhas — texto puro nunca toca o banco.
- **SHA-256** para tokens de API em repouso (v2.12.1+).
- **Universos privados retornam 404** para quem não tem permissão (CO-49).
- **Sem terceiros embedados** — sem CDN, sem Google Analytics, sem Facebook Pixel, sem Hotjar, sem Sentry remoto.

**Honestidade sobre cifragem em repouso:** corpos de notas/tarefas estão em texto puro no SQLite hoje. O operador da instância tecnicamente consegue ler conteúdo se acessar o servidor. Roadmap [CO-86](/co/CO-86) implementa cifragem ChaCha20-Poly1305 derivada da sua senha — até lá, conteúdo sensível deve ir em auto-hospedagem.

## 4. Cookies

Apenas essenciais — não precisamos do seu consentimento para usá-los (LGPD Art. 7, IX).

| Cookie | Finalidade | Duração |
|---|---|---|
| `session` | autenticação JWT | 30 dias |
| `co_lang` | idioma da interface | 1 ano |
| `co_named_palette` | tema visual | 1 ano |
| `co_local_universe` | universo anônimo local | sessão |

Nenhum cookie de terceiro. Nenhum cookie de tracking. Por isso não há banner "Aceitar cookies".

## 5. LGPD — Lei 13.709/2018

A Arte Longa atua como **controladora** dos dados na instância gerenciada. Bases legais aplicáveis:

| Tratamento | Base legal | Art. |
|---|---|---|
| Sua conta + conteúdo | Execução de contrato | 7º, V |
| Cookies essenciais | Legítimo interesse + exercício regular de direitos | 7º, IX |
| Telemetria agregada anônima | Anonimizada → fora do escopo da LGPD | Art. 12 |
| Logs técnicos | Cumprimento de obrigação legal (segurança) | 7º, II |

### Seus direitos (LGPD Art. 18)

- **Acessar** seus dados pessoais — solicite por email, resposta em até 15 dias
- **Corrigir** dados incompletos ou inexatos
- **Excluir** sua conta e todo o conteúdo associado — purga em até 30 dias, backups em até 90 dias
- **Exportar** seu conteúdo — Markdown nativo, sempre disponível via API ou UI
- **Revogar** consentimento para telemetria — Do Not Track ou Configurações → Privacidade
- **Saber** com quem seus dados foram compartilhados (resposta: ninguém, exceto Fly.io como sub-operador)

### Controladora e DPO

- Controlador: Yuri Felipe Hild — `yuri@artelonga.com.br`
- Encarregado (DPO): mesmo contato
- Incidentes de segurança comunicados à ANPD em até 48h (LGPD Art. 48)

## 6. Compartilhamento

A Arte Longa **não compartilha dados com terceiros**, exceto:

- **Fly.io** (sub-operador de infraestrutura, região São Paulo): hospeda o servidor e o volume. Sujeito ao [DPA Fly.io](https://fly.io/legal/privacy-policy/), disponível mediante solicitação.
- **Ordem judicial** fundamentada conforme legislação brasileira.

## 7. Retenção

- Conta ativa: enquanto você usar
- Conta excluída: 30 dias (90 dias para backups)
- Universos anônimos sem login: removíveis após 90 dias de inatividade
- Logs técnicos: 90 dias

## 8. Mudanças

Alterações relevantes serão comunicadas com **15 dias** de antecedência. Histórico de versões: <https://github.com/artelonga/co/commits/main/co-web/seed/template/privacidade.md>.

---

*[Arte Longa](https://artelonga.com.br) — São Paulo, Brasil.*  
*Licença do código: [AGPL v3](/co/public/licensa).*
