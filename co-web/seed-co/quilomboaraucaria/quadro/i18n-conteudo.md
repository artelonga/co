---
titulo: Internacionalizacao de conteudo
status: todo
prioridade: alta
prazo: 2026-05-15
etiquetas: [plataforma, i18n]
criado: 2026-04-01
---

Permitir conteudo em multiplos idiomas com pt-BR como padrao.

## O que precisa ser feito

- Estrutura de arquivos com sufixo de locale: `sobre.md` (pt-BR), `sobre.en.md` (en)
- Fallback: se traducao nao existe, mostra pt-BR
- Seletor de idioma no frontend (bandeiras ou dropdown)
- UI strings extraidas para arquivo de mensagens (labels, botoes, navegacao)
- Frontmatter `lang` para indicar idioma do conteudo
- Hreflang tags no HTML para SEO

## Arquitetura

```
content/
  pages/
    sobre.md          # pt-BR (padrao)
    sobre.en.md       # English
    privacidade.md
    privacidade.en.md
  posts/
    reuniao.md        # pt-BR only (sem traducao)
```

## Subtarefas

- [ ] Extrair UI strings para arquivo de mensagens
- [ ] Loader de conteudo com fallback por locale
- [ ] Seletor de idioma no layout
- [ ] Traduzir paginas institucionais para ingles
