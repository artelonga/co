# Yggdrasil UX — visão centralizada (epics → user stories), timeline real e tempo mediano por universo

> **Origem:** CO-420 (parent CO-414). Centraliza a narrativa de UX de
> `~/projects/yggdrasil/docs/experiencia-usuario-exemplo.md` (persona Marina,
> produto v2.14.0) em uma visão de **alto nível orientada a user-stories**, plota
> as **datas reais de release** dos três universos do ecossistema (co · artelonga ·
> yggdrasil) numa timeline, e reporta o **tempo mediano de conclusão por universo**
> a partir de fontes reais (frontmatter de work items + CHANGELOG/git).
>
> Datas: extraídas de CHANGELOG.md e do frontmatter `work/<repo>/*.md`. **Nada
> inventado** — onde não há par de datas, está dito explicitamente.

---

## 1. A experiência como epics → user stories

A narrativa da Marina não é uma lista de features; é uma sequência de **intenções
de usuário**, cada uma sustentada por um princípio de design e entregue por um
release datado. Reestruturada como backlog de produto:

### EPIC A — Explorar sem pertencer (visitante anônimo)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como visitante, quero ver universos vivos, placares e estatísticas **sem cadastro**, para decidir se vale a pena. | Explorar antes de pertencer | v2.7.0 (sessão na landing) |
| Como visitante, quero que **todo clique tenha destino** — slug sem página cai no catálogo, nunca em 404 — para nunca bater num beco. | Nenhum beco | v2.7.1 (slugs do catálogo → redirect) |
| Como visitante, quero ver um item ainda-não-portado (ex.: Tagmar) como 🟡 *planejado* com "quero portar", em vez de erro. | Catálogo é convite, não muro | v2.6.0 (catálogo) → v2.7.1 |

### EPIC B — Entrar a serviço da intenção (autenticação)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como pessoa que clicou "criar", quero login **só com email** (código) e voltar **direto à criação**, não a um lobby. | Login serve à intenção (`?next=`) | v2.8.0 |
| Como usuária logada, quero meu estado de sessão (email, "Sair") **visível em toda página**. | Sessão visível | v2.7.0 |
| Como plataforma, quero delegar a autenticação ao **CO** (um request de distância), não reimplementá-la. | Federação de identidade | v2.7.0 (bridge go-live, CO-384) |

### EPIC C — Criar é uma decisão, não cinco (onboarding de universo)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como criadora, quero **um único modelo "Universo"** (nota + pasta + evento), sem escolher "jardim/timeline/branco" antes de escrever. | Forma é lente, não compromisso | v2.14.0 (YG-126) |
| Como criadora, quero cair **já em modo edição** num universo vazio, porque tela morta parece quebrada. | Vazio editável > vazio inerte | v2.9.0 |
| Como criadora, quero ver **"meus universos"** e criar de verdade (página de criação). | Criação real | v2.8.0 (YG-122) |

### EPIC D — Escrever com rascunho-como-branch (edição segura)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como escritora, quero editar nota em **popup com preview ao vivo** (markdown + wikilinks `[[…]]`). | Edição direta | v2.11.0 |
| Como escritora, quero que **digitar nunca toque o canônico**: rascunho é branch (local + servidor), salvar é o único commit. | Rascunho = branch, salvar = commit | v2.11.0 → v2.12.0 |
| Como escritora cross-device, quero retomar no celular um rascunho aberto no laptop ("rascunho de outro dispositivo — Continuar/Descartar"). | Nada se perde | v2.12.0 (YG-125, DraftStore) |
| Como editora, quero que o rascunho viva **fora do caminho do bridge** — privado por construção, não por configuração. | Privado por construção | v2.12.0 (drafts fora de `notes/`) |
| Como dona, quero editar por **manipulação direta** no grid (composer inline, sem botão "editar"). | O grid é o editor | v2.17.0 (YG-129) |

### EPIC E — Compartilhar sem vazar segredo (permissão)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como dona, quero um **link por fragmento `#`** (não aparece em log/Referer, não carrega credencial); exclusividade vem do modelo de permissão (JWT owner-only), não da URL. | Link é endereço, nunca chave | v2.11.0 (links seguros) |

### EPIC F — Organizar com hierarquia que não esconde (estrutura)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como organizadora, quero **pastas no grid** e **arrastar** notas para dentro (toast de confirmação). | Pastas são lugares | v2.9.0 |
| Como organizadora, quero **ligações tipadas** com cor/estilo/toggle: pai/filho (dourada), referência (ciano), wikilink (tracejada roxa), irmãos (mesmo pai). | Hierarquia sem esconder | v2.9.0 (drag-to-link) → v2.17.0 (ligações entre irmãs) |
| Como organizadora, quero uma **árvore TUI** na sidebar que abre/fecha filhos no canvas. | Navegação terminal | v2.17.0 (YG-129) |

### EPIC G — Mudar de lente: o tempo que já estava lá (views de runtime)

| User story | Princípio | Entregue por (YG) |
|---|---|---|
| Como exploradora, quero alternar 🗺 Mapa · 🕐 Timeline · 🕸 Grafo sobre a **mesma instância**, sem reconfigurar nem migrar. | Conteúdo não sabe da forma | v2.13.0 (YG-123) → v2.14.0 (YG-126) |
| Como exploradora, quero a **Timeline read-only derivada**: cada nota no dia em que nasceu (`created_at`), eventos por `at_iso`, e "universo criado" como marco. | Datas já são dado canônico | v2.13.0 / v2.14.0 |
| Como ecossistema, quero que **novas lentes** (placares, sessões de jogo via bridge) aterrissem sem migração. | Extensível sem migração | v2.13.0 (Projection::Timeline aditivo) |

### EPIC H — Federação invisível (o que ela não viu)

| User story | Princípio | Entregue por (YG / CO) |
|---|---|---|
| Como usuária, quero que cada nota salva vire **markdown canônico + evento assinado** e aterrisse no CO (`co.artelonga.com.br`) **sem eu configurar nada**. | Federação é infraestrutura | YG v2.7.0 (bridge wire+dial) ↔ CO-384 |
| Como ecossistema, quero telemetria de site e jogo fluindo ao **hub de analytics do CO**, privacy-first. | Observabilidade federada | YG v2.15.0–v2.16.0 (YG-127/128) ↔ CO-335/CO analytics |

> **Checklist de regressão de UX** (do doc-fonte): qualquer mudança que quebre uma
> destas cenas quebra a experiência. Os 8 princípios são os critérios de aceitação
> de cada epic acima.

---

## 2. Timeline real de releases (co · artelonga · yggdrasil)

**Fonte:** linhas `## [versão] — AAAA-MM-DD — …` de cada `CHANGELOG.md` (datas
reais, Keep a Changelog). Tags git (`git tag --sort=creatordate`) confirmam os
marcos principais mas são esparsas — o CHANGELOG é a fonte canônica de datas.

Marcos do ecossistema, ordenados no tempo (um recorte; cada repo tem dezenas de
releases — ver §3 para contagem completa):

| Data | co | artelonga | yggdrasil |
|---|---|---|---|
| 2026-01-02 | primeiro release datado (1.x) | — | — |
| 2026-03-24 | … | AL 0.1.0 (primeiro release) | — |
| 2026-05-09 | … | … | YG 0.0.1 (primeiro release) |
| 2026-05-13 | 2.6.0–2.7.2 (analytics, conversas) | … | … |
| 2026-05-14 | 2.7.4–2.7.11 (Conteúdo, seed yggdrasil) | … | … |
| 2026-05-20 | 2.11.x–2.13.0 (event bus, branching, métricas) | AL 0.14.0 | **YG 1.0.0** (plataforma v1.0, YG-54) |
| 2026-05-29 | 2.32.0 (localhost-first) | AL 0.15.x (telemetria) | YG 1.1.0 (editor data-driven) |
| 2026-06-01 | 2.35.0–2.38.0 (yuri vision Waves) | … | YG 1.3.0 / 2.1.0 (nav unificada) |
| 2026-06-05 | 2.40.0 (substrate stable) | AL 0.17–0.21 (geo, scrum, analytics) | … |
| 2026-06-06 | 2.41.0 (brain interlink + EDA) | AL 0.22.0 (IaaS) | YG 2.2.0 (Content+Messaging GA) |
| 2026-06-07 | … | **AL 0.24.0** (último release) | YG 2.5.0–2.6.0 (léxico, corpus) |
| 2026-06-08 | 2.42.0 (unified gestão + live timeline) | … | YG 2.6.0 |
| 2026-06-10 | **3.0.0** (public launch) | … | … |
| 2026-06-11 | 3.1.0–3.3.1 (delivery pipeline, sala paisagem) | … | YG 2.7.0–2.11.0 (bridge, criação, pastas, editor popup) |
| 2026-06-12 | **3.4.0** (source:github + lente de tempo) | … | YG 2.12.0–2.17.0 (rascunho cross-device, Timeline, views, /analytics, grid-editor) |

**Leitura:** o ecossistema converge em 2026-06-11/12 — a maior parte da jornada da
Marina (EPICs C–H) foi entregue numa janela de **2 dias**, em paralelo nos três
repos: o CO abre o public launch (3.0.0) e a lente de tempo (3.4.0, CO-387);
yggdrasil entrega as views de runtime e o rascunho-branch (2.12.0–2.17.0); a
federação (bridge) liga os dois (YG 2.7.0 ↔ CO-384).

### Como isto renderiza no quadro (lente CO-387 `<co-time-grid>`)

Cada linha acima é um **evento datado** — exatamente o que a Timeline read-only
do CO-387 consome. No grid de tempo:

- **eixo X = data** (a régua `x_for`/`lane_rows` da spec draft CO-387);
- **uma faixa (lane) por universo** — co, artelonga, yggdrasil — em vez de por
  família de `kind`;
- cada release é um bloco posicionado no dia do seu changelog; a **adição** de um
  item = `created_at` do work item correspondente, a **conclusão** = release que o
  entregou (status `done` + `updated_at`).
- O mesmo conteúdo serve de Mapa (dependências entre CO-N e YG-N) ou Grafo
  (wikilinks entre os work items) — *conteúdo não sabe da forma* (princípio 7).

O doc é o entregável; a renderização no quadro é a projeção natural desta tabela
quando alimentada à lente CO-387.

---

## 3. Tempo mediano de conclusão por universo

### 3.1 Fonte e método (auditável)

Dois métodos complementares, porque **nenhum dos três repos popula
`completed_at` de forma consistente** (co: 1 item; artelonga: 0; yggdrasil: 0).
Em vez de fabricar a data de conclusão, usamos os pares de datas que existem de
fato:

- **Método A — work items (`created_at` → `updated_at`):** para cada item com
  `status: done`, o tempo-até-conclusão é `updated_at − created_at`. `updated_at`
  é o timestamp do commit que virou o status para `done` (a conclusão efetiva).
  Fonte: frontmatter de `work/co/CO-*.md`, `work/artelonga/AL-*.md`,
  `work/yggdrasil/YG-*.md`. Itens sem ambos os campos, ou com delta negativo, são
  descartados.
- **Método B — cadência de release (CHANGELOG):** mediana do intervalo em dias
  entre **dias de release consecutivos** de cada repo. Captura "tempo para enviar
  uma unidade de trabalho concluída", robusto ao ruído de `T00:00:00Z` do Método A.

### 3.2 Método A — mediana de (updated_at − created_at) em itens `done`

| Universo | n (itens done com par de datas) | **mediana** | média | máx | mesmo-dia (<1d) |
|---|---|---|---|---|---|
| **co** | 254 | **0,00 d** | 1,05 d | 37,65 d | 209 (82%) |
| **artelonga** | 40 | **0,00 d** | 0,53 d | 2,49 d | 30 (75%) |
| **yggdrasil** | 107 | **0,00 d** | 0,92 d | 14,00 d | 82% |

**Interpretação honesta:** a mediana de 0 d **não** é um artefato de bug — é o
modo de trabalho real. ~80% dos work items nos três universos são *mintados e
concluídos no mesmo dia* (muitos `created_at` são `AAAA-MM-DDT00:00:00Z`,
data-only de mint em lote, e o `updated_at` cai no mesmo dia útil). O sinal útil
está na **cauda** (a média e o máx): co arrasta os itens mais longos (até ~38
dias — épicos de plataforma), artelonga é o mais enxuto (máx 2,5 d), yggdrasil
fica no meio (máx 14 d). A mediana mede que **o trabalho é fatiado em tarefas de
um dia**; a média mede quanto custam as exceções.

### 3.3 Método B — cadência de release (gap mediano entre dias de release)

| Universo | releases datados | janela | dias com release | **gap mediano entre dias** | releases/dia (média) |
|---|---|---|---|---|---|
| **co** | 298 | 2026-01-02 → 06-12 (161 d) | 46 | **1 d** | 6,5 |
| **artelonga** | 25 | 2026-03-24 → 06-07 (75 d) | 12 | **2 d** | 2,1 |
| **yggdrasil** | 48 | 2026-05-09 → 06-12 (34 d) | 13 | **3 d** | 3,7 |

**Interpretação:** quando há trabalho ativo, o **co** envia ~diariamente
(mediana 1 d entre dias de release, 6,5 releases/dia), o **yggdrasil** a cada ~3
dias, o **artelonga** a cada ~2 dias. Isto reflete o foco: co é o substrato em
desenvolvimento contínuo; yggdrasil e artelonga avançam em ondas.

### 3.4 Veredito por universo (resposta da métrica pedida)

| Universo | Tempo mediano de conclusão | n | Fonte |
|---|---|---|---|
| **co** | **0 d por item** (work item mediano fecha no dia em que abre); **1 d** de cadência de release | A: 254 itens · B: 298 releases | frontmatter `work/co/*.md` + `CHANGELOG.md` |
| **artelonga** | **0 d por item**; **2 d** de cadência de release | A: 40 itens · B: 25 releases | frontmatter `work/artelonga/*.md` + `CHANGELOG.md` |
| **yggdrasil** | **0 d por item**; **3 d** de cadência de release | A: 107 itens · B: 48 releases | frontmatter `work/yggdrasil/*.md` + `CHANGELOG.md` |

> **Por que duas medidas:** o item-mediano de 0 d responde literalmente o pedido
> (mediana de `concluído − criado`), mas é dominado pelo padrão "tarefa de um
> dia". A cadência de release é a métrica de fato comparável entre universos —
> **co (1 d) < artelonga (2 d) < yggdrasil (3 d)**. Ambas estão acima porque a
> primeira é auditável-por-item e a segunda é robusta-e-comparável.

### 3.5 Reprodutibilidade

- **Método A:** para cada `work/<repo>/<PREFIX>-*.md` com `status: done`,
  `delta = updated_at − created_at` (descartando ausentes/negativos); `mediana`,
  `média`, `máx` sobre o conjunto. (1 item do co tem `completed_at` explícito —
  usado quando presente; senão `updated_at`.)
- **Método B:** datas das linhas `^## \[versão\] — (\d{4}-\d{2}-\d{2})` do
  `CHANGELOG.md`; ordenar ascendente; `gap = mediana dos intervalos > 0 entre dias
  de release distintos`.
- As contagens (`n`, janelas, máximos) acima são a evidência; não é um número
  isolado.
