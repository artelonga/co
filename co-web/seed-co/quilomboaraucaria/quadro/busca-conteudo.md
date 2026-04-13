---
titulo: Busca de conteudo
status: todo
prioridade: media
prazo: 2026-05-30
etiquetas: [plataforma, ux]
criado: 2026-04-01
---

Busca full-text nos relatos, paginas e eventos.

## O que precisa ser feito

- [ ] Indice de busca gerado a partir dos arquivos markdown
- [ ] API endpoint GET /api/busca?q=termo
- [ ] Componente de busca no frontend (barra no header ou pagina dedicada)
- [ ] Highlight de termos encontrados nos resultados
- [ ] Filtro por tipo (relato, evento, pagina)

## Opcoes tecnicas

- SQLite FTS5 (full-text search) — ja temos SQLite
- MiniSearch client-side — indice JSON gerado no build
- Pagefind — indice estatico (funciona bem com SSR)
