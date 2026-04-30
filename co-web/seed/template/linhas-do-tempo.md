---
created: 2026-04-29T00:00:00+00:00
modified: 2026-04-29T00:00:00+00:00
order: 5
slug: linhas-do-tempo
tags:
- featured
- timeline
- categoria
title: Linhas do tempo
type: page
---

# Linhas do tempo

> Onde tudo se encaixa. Três universos públicos, uma única visualização.

O Co inclui um conjunto de universos curados — uma "categoria" sob o template — que ilustram a visão de **linha do tempo**: do Big Bang ao fim do universo, com a história humana sobreposta na escala certa.

## Os três universos

### [Tempo](/co/tempo)

> Onde tudo se encaixa.

Universo-meta com 21 eventos atravessando toda a escala — Big Bang, formação do Sistema Solar, primeira vida, cambriano, dinossauros, humanos, agora, e o que vem depois das estrelas. Bom ponto de partida.

### [Universo](/co/universo)

> Linha do tempo cósmica completa, do Big Bang à morte térmica.

28 eventos com ênfase no comportamento de longuíssimo prazo: era da inflação, recombinação, era estelífera, era degenerada, era dos buracos negros, era sombria. Inclui a colisão Via Láctea + Andrômeda em 4,5 bilhões de anos.

### [Humanidade](/co/humanity)

> Onde nos colocamos no tempo.

26 eventos centrados na espécie humana: Homo sapiens, agricultura, escrita, imprensa, Iluminismo, computador, Web, smartphone, LLMs. Termina em "agora" para você se localizar.

## Visualizar

Abra a [linha do tempo interativa](/shared/timeline.html?u=tempo,universo,humanity) — você pode ativar/desativar cada universo independentemente, ou ver todos sobrepostos.

- `←` `→` viaja entre eventos com animação suave
- Arraste para navegar manualmente
- Cada universo tem sua própria cor

## Inspiração

A visualização foi inspirada em [scaleofuniverse.com/pt](https://scaleofuniverse.com/pt) — uma ferramenta extraordinária que mostra a escala de **distância** do universo. Aqui invertemos a ênfase: mostramos a escala de **tempo**, do Big Bang ao colapso térmico final, com humanos como uma linha estreita perto do "agora".

## Construa o seu

Cada evento é um arquivo Markdown com `type: event` e `date_year` no frontmatter. Você pode criar seu próprio universo com sua própria linha do tempo — pessoal, organizacional, histórica, especulativa. O Co simplesmente lê os arquivos e os coloca na escala.

```yaml
---
type: event
title: Meu evento
date_year: 1987
description: Algo que aconteceu nessa data.
---
```

---

*Esta página faz parte do conjunto curado do template. A linha do tempo é uma das visões fundamentais do Co — testada e refinada para uso público.*
