---
created: 2026-05-09T00:00:00+00:00
modified: 2026-05-09T00:00:00+00:00
order: 2
slug: guia
tags:
- featured
- guia
- onboarding
- yggdrasil
title: Guia do Co
type: page
---

# Guia do Co

> Bem-vindo ao Co — gestão de conteúdo em grafo, livre e aberto.

Este guia cobre tudo o que você precisa saber para começar: criar seu universo, organizar ideias, colaborar com outras pessoas — e explorar o **Yggdrasil**, o mundo fantasia do Co para apaixonados por jogos.

---

## O que é o Co?

O Co é uma plataforma para **organizar ideias, projetos e presença online** em *universos* — espaços pessoais ou coletivos onde cada cartão, nota e página é um arquivo `.md` com metadados YAML. Tudo que você escreve é seu, em formato aberto, portável para qualquer editor.

Três verbos definem o que você pode fazer:

| Verbo | O que significa |
|-------|-----------------|
| **Cocriar** | Escreva, arraste e conecte ideias em quadros kanban |
| **Colaborar** | Convide pessoas; editem juntos em tempo real |
| **Conectar** | Seus universos formam uma rede — cada link é um nó na consciência coletiva |

---

## Primeiros passos

### 1. Explore o template

Você já está no **universo template** — um espaço público de leitura com tutoriais, linhas do tempo e exemplos reais. Navegue pelas visualizações no topo da tela: **Quadro**, **Tabela**, **Conteúdo**, **Linha do Tempo**.

### 2. Crie seu universo

Faça login (clique em **Entrar** ou **Criar conta**) e depois em **+ Novo universo** na barra lateral. Em segundos você terá um espaço só seu.

### 3. Adicione ideias

Clique em **+ Nova tarefa** no quadro. Cada cartão tem título, descrição em Markdown, status, prioridade, etiquetas e data de entrega. Arraste entre colunas para avançar o status.

### 4. Convide colaboradores

Em **Configurações do universo**, adicione membros por e-mail. Edições em tempo real via sincronização CRDT — cada pessoa vê as mudanças aparecerem ao vivo.

---

## As visualizações

O Co oferece múltiplas formas de ver o mesmo conteúdo:

- **Quadro** — kanban clássico: colunas por status, arraste entre elas.
- **Tabela** — lista ordenável com filtros, ideal para triagem e bulk-edit.
- **Linha do tempo** — barras de Gantt para planejar datas e dependências.
- **Calendário** — eventos organizados por mês, com entradas de data semântica.
- **Conteúdo** — navegador de arquivos e leitura corrida de todas as páginas.
- **Dashboard** — gráficos de velocidade, burndown e distribuição de etiquetas.

Cada universo lembra a última visualização que você usou.

---

## Universos públicos

Qualquer universo pode ser tornado **público**. Visitantes anônimos podem lê-lo; membros convidados podem editar. Para aparecer na busca pública, defina visibilidade como *public-subscribable* nas configurações.

Universos **template** (como este) são somente-leitura para visitantes. Ao fazer login e clicar em **Criar universo**, você recebe uma cópia sua — editável, privada, com todo o conteúdo inicial.

---

## Vault e Obsidian

O Co é compatível com o **Obsidian Local REST API**. Se você usa o Obsidian, pode sincronizar seu cofre com um universo Co via plugin:

- `GET /api/v1/universes/{slug}/vault/notes` — lista arquivos
- `GET /api/v1/universes/{slug}/vault/notes/{path}` — lê conteúdo
- `PUT /api/v1/universes/{slug}/vault/notes/{path}` — escreve conteúdo
- Tags, árvore e busca full-text também disponíveis

Gere um token de API em **Configurações → API** para autenticar o plugin.

---

## Linhas do tempo

O Co vem com três universos curados que ilustram a escala do tempo — do Big Bang ao presente, com a história humana sobreposta:

- **[Tempo](/co/tempo)** — 21 eventos atravessando toda a escala.
- **[Universo](/co/universo)** — 28 eventos cósmicos até a morte térmica.
- **[Humanidade](/co/humanity)** — 26 eventos centrados na espécie humana.

[Abrir linha do tempo interativa →](/shared/timeline.html?u=tempo,universo,humanity)

---

## ⚔ Yggdrasil — o mundo fantasia do Co

> *Na mitologia nórdica, Yggdrasil é a Grande Árvore do Mundo — o eixo que conecta os nove mundos, raízes fincadas no abismo e galhos tocando o céu.*

No Co, **Yggdrasil** é o universo dos jogos — um **hub de minigames** para quem quer transformar tempo livre em pontos, rankings e conquistas. É o lado lúdico da consciência coletiva: onde estratégia, reflexo e sorte se encontram.

### Os mundos de Yggdrasil

Cada jogo é um "mundo" independente na árvore. Faça login e explore:

| Jogo | Ícone | Descrição |
|------|-------|-----------|
| **Tetris** | 🧱 | Peças clássicas em queda livre — encaixe, pontue, sobreviva |
| **Snake** | 🐍 | A cobra faminta cresce a cada presa — até engolir a si mesma |
| **Invaders** | 🚀 | Defenda a Terra de ondas de invasores espaciais |
| **PointSet** | 🔲 | Encontre os pares antes que o tempo acabe |
| **Poker** | 🃏 | Video poker — monte a melhor mão com cinco cartas |

### Perfil e ranking

Cada partida conta. Ao fazer login, o Co registra suas **pontuações máximas**, o **número de partidas** e calcula seu **nível de jogador** (uma partida a cada cinco jogos). Tudo visível no seu perfil dentro de Yggdrasil.

No topo da página há o **ranking global** — os dez maiores pontuadores do servidor. Seu nome pode aparecer lá.

### Como chegar ao Yggdrasil

1. Faça login no Co.
2. Na barra lateral, clique em **Yggdrasil** (ou acesse `/co/yggdrasil`).
3. Escolha um jogo e comece a jogar.

> **Login obrigatório** — o Yggdrasil exige conta para registrar pontuações. Visitantes anônimos veem a tela de boas-vindas mas não podem jogar.

### Por que o nome?

A árvore nórdica Yggdrasil conecta mundos distintos por uma raiz comum. No Co, os jogos são mundos independentes — cada um com sua física, seus recordes, sua atmosfera — mas todos enraizados na mesma plataforma, no mesmo perfil, na mesma rede de pessoas. Quando você bate seu recorde em Tetris e o colega bate em Snake, vocês dois aparecem no mesmo ranking. A árvore une.

---

## Temas

O Co oferece doze temas visuais. Alguns são exclusivos para usuários com conta:

| Tema | Disponível para |
|------|-----------------|
| Modern | Todos |
| Scholarly Light / Dark | Todos |
| Relic Light / Dark | Todos |
| Medieval, Steampunk, Cyberpunk | Usuários com conta |
| Matrix, Garden, Terminal, Retro | Usuários com conta |

Troque o tema no seletor no cabeçalho da página — sem recarregar, instantâneo.

---

## Privacidade e portabilidade

- Todo o conteúdo é armazenado como arquivos `.md` + metadados SQLite. **Nunca fica preso.**
- A instância oficial coleta apenas o necessário (e-mail para login, interações para melhorar a plataforma). Veja a [política de privacidade](/privacidade).
- O Co é **software livre** (MIT). Você pode rodar sua própria instância, migrar dados, auditar o código.

---

## Atalhos de teclado

| Tecla | Ação |
|-------|------|
| `N` | Nova tarefa (no quadro) |
| `F` | Buscar conteúdo |
| `1–6` | Trocar visualização (Quadro, Tabela, Conteúdo, Linha do tempo, Calendário, Dashboard) |
| `Esc` | Fechar modal aberto |

---

*Co — cada nota é um nó. Cada nó, um mundo.*
