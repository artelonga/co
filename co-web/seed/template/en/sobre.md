---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-05-03T00:00:00+00:00
order: 1
slug: sobre
language: en
tags:
- sobre
- manifesto
title: About Co
type: page
---

# About Co

The short story is on the [home](/en/index) page — **Co**llective **Co**nsciousness, three verbs: **Co**create, **Co**llaborate, **Co**nnect.

This page keeps the technical and governance details.

## Principles

1. **Markdown is the canonical format.** YAML frontmatter for metadata, body in Markdown. Portability above all.
2. **Universes are units of identity.** Private = profiles. Public = showcases. Each universe has an owner, visibility rules and a history of its own.
3. **The network is the product.** Universes connect through links, subscriptions, typed relations. The graph emerges with no central coordination.
4. **Free software, your content.** Code under [AGPL v3](/licensa); owners hold their own universes.

## Stack

- **Backend:** Rust (Axum), SQLite per universe, ChaCha20-Poly1305 for encryption at rest.
- **Frontend:** lightweight SPA, Markdown rendered client-side with `marked` + DOMPurify, CRDTs (Yjs) for collaborative editing.
- **Deploy:** Fly.io for the official instance; anyone can run their own.

## Community

- Code: [github.com/artelonga/co](https://github.com/artelonga/co)
- Issues and roadmap: public board at [/co/co](/co/co)
- License: [AGPL v3](/licensa) — network copyleft that protects the community against private enclosure

---

*Collective consciousness is not a technology — it is a practice. Co is the infrastructure.*
