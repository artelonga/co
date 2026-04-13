---
titulo: Conteudo hibrido (git + web)
status: done
prioridade: alta
prazo: 2026-03-30
etiquetas: [plataforma, conteudo]
criado: 2026-03-20
---

Sistema de conteudo runtime que le markdown do filesystem. Editavel via Obsidian/git e via web editor.

## Entregue

- Runtime loader (gray-matter + marked) em conteudo.ts
- Leitura de posts e paginas sem rebuild
- CONTENT_DIR configuravel via env
- Dockerfile sincroniza paginas institucionais a cada deploy
- Posts preservados no volume persistente
