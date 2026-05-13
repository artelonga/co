---
created: 2026-05-13T00:00:00+00:00
modified: 2026-05-13T00:00:00+00:00
order: 20
slug: licensa
tags:
- licenca
- agpl
- transparencia
- soberania
title: Licença
type: page
---

# Licença

Co é software livre licenciado sob **GNU Affero General Public License v3 (AGPL v3)** — proposta atual (aguardando finalização).

Esta página explica o que isso significa **para usuários**, **operadores que hospedam Co**, e **desenvolvedores que modificam o código**.

## TL;DR

Para a maioria dos usuários:

- Você pode **usar Co grátis**, fazer **fork**, **modificar**, e **redistribuir**
- Se você **hospeda uma versão modificada como serviço para outros**, precisa **publicar o código modificado**
- Não há garantia. Use por sua conta e risco

Para empresas: contate `yuri@artelonga.com.br` para licença comercial alternativa se necessário.

## O que é AGPL v3

A **GNU Affero General Public License v3** é uma licença *copyleft* publicada pela [Free Software Foundation](https://www.fsf.org/).

**Copyleft** significa: as liberdades que você recebe são **passadas adiante** para quem você redistribui o software. Não é "domínio público com créditos" — é "compartilhe nas mesmas condições".

### Quatro liberdades garantidas

1. **Liberdade 0** — usar o programa, com qualquer propósito
2. **Liberdade 1** — estudar como o programa funciona (código fonte é precondição)
3. **Liberdade 2** — redistribuir cópias
4. **Liberdade 3** — distribuir versões modificadas

### A "cláusula AGPL" — uso de rede

A AGPL adiciona uma cláusula que **GPL v3 não tem**:

> Se você roda uma versão modificada do software como **serviço de rede**, você deve fornecer o código fonte modificado aos usuários daquele serviço.

Em GPL v3, você pode pegar o código, modificar privadamente, e rodar como SaaS sem nunca compartilhar de volta. A AGPL fecha essa brecha ("ASP loophole").

### Por que Co escolheu AGPL e não MIT

Co existe para promover **soberania digital** — a ideia de que seus dados, suas conversas, e suas comunidades pertencem a você.

| Licença | Permite SaaS modificado sem compartilhar fonte? |
|---|---|
| MIT | Sim — incoerente com a missão |
| Apache 2.0 | Sim — mesma issue |
| GPL v3 | Sim (cobre redistribuição mas não uso de rede) |
| **AGPL v3** | **Não — coerente com a missão** |

## O que isso significa para você

### Se você apenas usa Co em `co.artelonga.com.br`

Sem implicações. AGPL é entre a equipe Co e o público — você é um usuário normal e seus dados / sua conta são seus.

### Se você quer self-host Co para você ou sua comunidade

Você pode:

- Clonar o repositório
- Modificar como quiser
- Rodar em seu próprio servidor
- **Não publicar suas modificações se ninguém mais usa esse servidor**

Você precisa publicar suas modificações se:

- Outros usuários acessam seu servidor (uso de rede triggera AGPL)
- Você distribui o binário compilado

Como publicar:

1. Manter o repositório git acessível (GitHub, GitLab, próprio servidor)
2. Adicionar link "Source: <https://...>" no rodapé do site
3. Garantir que o link funciona

### Se você modifica Co e contribui de volta

Toda contribuição (PR) ao repositório `artelonga/co` é considerada licenciada sob AGPL v3 — mesmo termo do código existente. Você mantém o copyright de suas contribuições.

Co **não exige CLA** (Contributor License Agreement). Sua contribuição permanece sua sob AGPL.

### Se você é empresa querendo usar Co comercialmente

Duas opções:

1. **Self-host sob AGPL**: livre, mas suas modificações ficam públicas se o serviço é usado por outros
2. **Licença comercial separada**: contate `yuri@artelonga.com.br`. Termos negociáveis para casos específicos

## Comparação de licenças

| | MIT | Apache 2.0 | GPL v3 | **AGPL v3 (Co)** |
|---|---|---|---|---|
| Usar comercialmente | ✅ | ✅ | ✅ | ✅ |
| Modificar | ✅ | ✅ | ✅ | ✅ |
| Distribuir | ✅ | ✅ | ✅ (compartilhar fonte) | ✅ (compartilhar fonte) |
| SaaS modificado sem compartilhar | ✅ | ✅ | ✅ | ❌ |
| Patent grant | ❌ | ✅ | ✅ | ✅ |
| Atribuição obrigatória | ✅ | ✅ | ✅ | ✅ |

## Por que não MIT — "sou simples, gosto de simples"

MIT é simples porque desiste de proteger a comunidade.

Co é uma plataforma que promete soberania digital. Para essa promessa ser **verificável**, qualquer versão pública precisa ter código auditável. AGPL é o mecanismo que garante isso.

Se você quer escrever uma biblioteca embebível, MIT está certo. Se você quer construir uma plataforma de soberania, AGPL é mais coerente.

## "AGPL é tóxica para empresas"

Algumas empresas têm policies anti-AGPL — geralmente por medo de "contaminação" (usar AGPL dep em código proprietário forçaria liberação).

Para Co: você está rodando Co como serviço, não embebendo Co em seu produto. Policies anti-AGPL para dependências não se aplicam.

Se sua empresa precisa "rodar Co modificado sem publicar mudanças", existe a [licença comercial](#se-você-é-empresa-querendo-usar-co-comercialmente) acima.

## O que NÃO é coberto

A licença cobre **código**, não:

- **Marcas e logos** "Co", "Co.artelonga", "ArteLonga" — uso comercial requer permissão separada
- **Dados de usuários** — pertencem aos usuários, não à licença
- **Conteúdo de cada universo** — pertence ao criador/dono do universo
- **Contribuições não-incorporadas** — comments em issues, discussions: fair use

## Recursos

- Texto canônico: <https://www.gnu.org/licenses/agpl-3.0.html>
- FSF FAQ: <https://www.gnu.org/licenses/gpl-faq.html>
- Comparação: <https://choosealicense.com/licenses/agpl-3.0/>
- Para questões específicas: `yuri@artelonga.com.br`

---

**Lembrete**: este documento é uma explicação, não um substituto do texto legal. O texto canônico está em [LICENSE](https://github.com/artelonga/co/blob/main/LICENSE) no repositório.

Voltar para [Segurança](/co/template?page=seguranca).
