---
created: 2026-05-14T00:00:00+00:00
modified: 2026-05-14T00:00:00+00:00
order: 0
slug: index
tags:
- home
- yggdrasil
- jogos
title: Yggdrasil — Hub de Jogos
type: page
---

# **Y**ggdrasil

Yggdrasil é o hub de **minijogos** da Arte Longa — perfis de jogadores, rankings globais, partidas casuais.

A árvore-mundo da mitologia nórdica conecta nove reinos. Aqui ela conecta jogos: cada nó é uma experiência curta, mas o caminhar por todos eles deixa rastro — XP, sementes (moeda interna), conquistas.

## Jogos disponíveis

Os jogos rodam direto no navegador, sem instalação. A engine 2D é compartilhada com [CO](/co/template?page=co-plataforma) (mesmo `game-core` em Rust → WASM).

- **Tetris** — clássico
- **Snake** — clássico
- **2048** — clássico
- *(mais por vir — em produção)*

## Sementes

Cada partida concluída deposita **sementes** na sua carteira. Sementes são a moeda interna do universo Yggdrasil — usadas para desbloquear temas visuais, avatares, e (no futuro) habilidades dentro dos jogos.

A carteira é **portátil entre jogos** — você acumula em qualquer um e gasta em qualquer outro.

## Como entrar

Se você está logado no Co, sua conta já tem acesso. Sem cadastro adicional, sem download.

Visite o hub:

- [yggdrasil-artelonga.fly.dev](https://yggdrasil-artelonga.fly.dev) — o app standalone
- ou a partir do menu lateral aqui mesmo no Co, alterne para o universo Yggdrasil

## Relação com Co

Yggdrasil **reusa o engine 2D** do Co em build-time (Rust path-dep com `co/game-core`). Em runtime, os dois sistemas não conversam — Yggdrasil tem seu próprio backend, lobby e DB.

Detalhes da infraestrutura: [infra-yggdrasil](/co/template?page=infra-yggdrasil).

## Status

Em desenvolvimento ativo. Bugs e ideias podem ser reportados em [github.com/artelonga/co](https://github.com/artelonga/co).
