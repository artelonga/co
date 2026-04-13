---
titulo: Pipeline de publicacao local
status: todo
prioridade: alta
prazo: 2026-04-20
etiquetas: [plataforma, devops]
criado: 2026-04-01
---

Fluxo de trabalho para editar conteudo localmente (Obsidian/editor) e publicar com validacao.

## Fluxo desejado

```
1. Editar markdown no Obsidian ou editor local
2. npm run validate   → validar YAML + conteudo
3. npm run preview    → ver resultado local
4. git commit + push  → conteudo chega ao repo
5. fly deploy         → publicar em producao
```

## O que precisa ser feito

- [ ] Symlink ou git submodule entre quilomboaraucaria/ e quilombo-blog/content/
- [ ] Script `npm run validate` que roda validacao-yaml
- [ ] Script `npm run sync-content` que copia conteudo validado para content/
- [ ] Dockerfile atualizado para sincronizar conteudo do repo no deploy
- [ ] Documentar fluxo no README

## Integracao Obsidian

O diretorio quilomboaraucaria/ ja tem .obsidian/ configurado.
Editar localmente e rodar `npm run validate` antes de commitar.

## Integracao co-web

Futuro: co-web pode servir como backend unico, eliminando a necessidade de
copiar arquivos. O quilombo-blog consumiria a API do co-web para conteudo.
