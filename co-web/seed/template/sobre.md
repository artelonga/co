---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-05-03T00:00:00+00:00
order: 1
slug: sobre
tags:
- sobre
- manifesto
title: Sobre o Co
type: page
---

# Sobre o Co

A história curta está em [home](/index) — **Co**nsciência **Co**letiva, três verbos: **Co**criar, **Co**laborar, **Co**nectar.

Esta página guarda os detalhes técnicos e de governança.

## Princípios

1. **Markdown é o formato canônico.** Frontmatter YAML para metadados, corpo em Markdown. Portabilidade antes de tudo.
2. **Universos são unidades de identidade.** Privados = perfis. Públicos = vitrines. Cada universo tem dono, regras de visibilidade e um histórico próprio.
3. **A rede é o produto.** Universos se conectam por links, subscrições, relações tipadas. O grafo emerge sem coordenação central.
4. **Software livre, conteúdo seu.** Licença MIT no código; donos detêm seus próprios universos.

## Stack

- **Backend:** Rust (Axum), SQLite por universo, ChaCha20-Poly1305 para criptografia em repouso.
- **Frontend:** SPA leve, Markdown renderizado client-side com `marked` + DOMPurify, CRDTs (Yjs) para edição colaborativa.
- **Deploy:** Fly.io para a instância oficial; qualquer pessoa pode rodar a sua.

## Comunidade

- Código: [github.com/artelonga/co](https://github.com/artelonga/co)
- Issues e roadmap: board público em [/co/co](/co/co)
- Licença: MIT

---

*A consciência coletiva não é uma tecnologia — é uma prática. O Co é a infraestrutura.*
