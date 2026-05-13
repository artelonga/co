# Licença — CO

CO é software livre licenciado sob **GNU Affero General Public License
v3 (AGPL v3)**.

Texto completo: [LICENSE](../LICENSE) (em inglês — versão canônica do
texto da GNU).

Esta página explica o que essa licença significa **para usuários**,
para **operadores que hospedam CO**, e para **desenvolvedores que
modificam o código**.

---

## TL;DR

Para a maioria dos usuários:

- Você pode **usar CO grátis**, fazer **fork**, **modificar**, e
  **redistribuir**.
- Se você **hospeda uma versão modificada** como serviço para outros,
  precisa **publicar o código modificado**.
- Não há garantia. Use por sua conta e risco.

Para empresas: contate `yuri@artelonga.com.br` se você precisa de
licença comercial alternativa.

---

## O que é AGPL v3

A **GNU Affero General Public License versão 3** é uma licença
*copyleft* publicada pela [Free Software Foundation](https://www.fsf.org/).

Copyleft significa: as liberdades que você recebe são **passadas adiante**
para quem você redistribui o software. Não é "domínio público com
créditos". É "compartilhe nas mesmas condições".

### Quatro liberdades garantidas

1. **Liberdade 0** — usar o programa, com qualquer propósito
2. **Liberdade 1** — estudar como o programa funciona; o **código fonte
   é precondição**
3. **Liberdade 2** — redistribuir cópias
4. **Liberdade 3** — distribuir versões modificadas (em condições
   AGPL)

### A "cláusula AGPL" — uso de rede

A AGPL adiciona uma cláusula que **GPL v3 não tem**:

> Se você roda uma versão modificada do software como **serviço de
> rede**, você deve fornecer o código fonte modificado aos usuários
> daquele serviço.

Em GPL v3, você pode pegar o código, modificar privadamente, e rodar
como SaaS sem nunca compartilhar de volta. A AGPL fecha essa brecha
("ASP loophole").

### Por que CO escolheu AGPL e não MIT / GPL

CO existe para promover **soberania digital** — a ideia de que seus
dados, suas conversas, e suas comunidades pertencem a você.

- **MIT seria incoerente**: permite que alguém pegue CO, adicione
  back-doors ou rastreamento, e rode como SaaS sem dizer nada à
  comunidade.
- **GPL v3 seria parcial**: cobre redistribuição do binário mas não uso
  como serviço.
- **AGPL v3 é coerente**: se você roda uma versão modificada, a
  comunidade tem direito de ver o que mudou.

---

## O que isso significa para você

### Se você apenas usa CO em `co.artelonga.com.br`

Sem implicações para você. AGPL é entre a equipe CO e o público — você
é um usuário normal e seus dados / sua conta são seus.

### Se você quer self-host CO para você ou sua comunidade

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
2. Adicionar um link "Source: <https://...>" no rodapé do site
3. Garantir que o link funciona

### Se você modifica CO e contribui de volta

Toda contribuição (PR) ao repositório artelonga/co é considerada
licenciada sob AGPL v3 — mesmo termo do código existente. Você mantém
o copyright de suas contribuições.

CO **não exige CLA** (Contributor License Agreement). Sua contribuição
permanece sua sob AGPL.

### Se você é empresa querendo usar CO comercialmente

Duas opções:

1. **Self-host sob AGPL**: livre, mas suas modificações ficam públicas se
   o serviço é usado por outros (mesmo internamente para uma empresa
   grande, se múltiplas pessoas acessam).
2. **Licença comercial separada**: contate `yuri@artelonga.com.br`.
   Termos negociáveis para casos específicos (white-label, removal of
   copyleft requirement, etc.).

---

## Diferenças entre AGPL e outras opções

| | MIT | Apache 2.0 | GPL v3 | **AGPL v3 (CO)** |
|---|---|---|---|---|
| Usar comercialmente | ✅ | ✅ | ✅ | ✅ |
| Modificar | ✅ | ✅ | ✅ | ✅ |
| Distribuir | ✅ | ✅ | ✅ (compartilhar fonte) | ✅ (compartilhar fonte) |
| Sublicenciar | ✅ | ✅ | ❌ | ❌ |
| Rodar como SaaS modificado sem compartilhar fonte | ✅ | ✅ | ✅ | ❌ — **compartilhar obrigatório** |
| Patent grant | ❌ | ✅ | ✅ | ✅ |
| Atribuição obrigatória | ✅ | ✅ | ✅ | ✅ |

---

## "Por quê não MIT — sou simples, gosto de simples"

MIT é simples porque desiste de proteger a comunidade.

CO é uma plataforma que promete soberania digital. Para essa promessa
ser **verificável**, qualquer versão pública precisa ter código
auditável. AGPL é o mecanismo que garante isso.

Se você quer escrever uma biblioteca embebível, MIT está certo. Se
você quer construir uma plataforma de soberania, AGPL é mais coerente.

---

## "AGPL é tóxica para empresas"

É verdade que algumas empresas têm policies anti-AGPL — geralmente por
medo de "contaminação" (usar uma AGPL dep em código proprietário
forçaria liberação).

Para CO: você está rodando CO como serviço, não embebendo CO em seu
produto. As policies anti-AGPL para dependências não se aplicam.

Se sua empresa precisa específicamente "rodar CO modificado sem
publicar mudanças", existe a [licença comercial](#se-você-é-empresa-querendo-usar-co-comercialmente)
acima.

---

## O que NÃO é coberto

A licença cobre **código**, não:

- **Marcas e logos** "CO", "Co.artelonga", "ArteLonga" — uso comercial
  precisa de permissão separada
- **Dados de usuários** — esses pertencem aos usuários, não à licença
- **Conteúdo de cada universo** — pertence ao criador / dono do universo
- **Contribuições não-incorporadas** — issue comments, discussions:
  uso permitido sob princípios de fair use

---

## Como ler o texto completo

A licença canônica está em [LICENSE](../LICENSE) (texto da GNU, em
inglês). Tradução não-oficial em português: <https://www.gnu.org/licenses/translations.html#agpl3>.

Se houver conflito entre o texto inglês e qualquer tradução, **o texto
inglês prevalece** (per termos da GNU FSF).

---

## Histórico de licença

| Período | Licença |
|---|---|
| 2025 — 2026-05-13 | MIT |
| 2026-05-13 — agora | AGPL v3 |

A migração de MIT → AGPL v3 foi feita após decisão consciente de
alinhar a licença com a missão do projeto. Versões anteriores
continuam disponíveis sob MIT (irrevogável); o código atual (`main`
branch + tags >= v2.7.0) é AGPL v3.

---

## Recursos

- Texto canônico: <https://www.gnu.org/licenses/agpl-3.0.html>
- FSF perguntas frequentes: <https://www.gnu.org/licenses/gpl-faq.html>
- Comparação de licenças: <https://choosealicense.com/licenses/agpl-3.0/>
- Para questões específicas: `yuri@artelonga.com.br`

---

**Lembrete**: este documento é uma explicação, não um substituto do
texto legal. O texto canônico em [LICENSE](../LICENSE) é o que vale
em caso de disputa.
