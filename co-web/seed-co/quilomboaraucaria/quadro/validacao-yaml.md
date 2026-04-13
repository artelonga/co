---
titulo: Validacao de frontmatter YAML
status: todo
prioridade: alta
prazo: 2026-04-15
etiquetas: [plataforma, qualidade]
criado: 2026-04-01
---

Validar frontmatter YAML de todos os arquivos markdown antes de publicar.

## O que precisa ser feito

- Script CLI que valida todos os .md em content/ contra schema.yaml
- Validacoes: campos obrigatorios, tipos corretos, datas validas, slugs unicos
- Rodar como pre-commit hook ou comando manual (`npm run validate`)
- Reportar erros claros: arquivo, campo, problema
- Integrar com pipeline de publicacao local

## Regras de validacao

| Campo | Regra |
|-------|-------|
| title | obrigatorio, nao vazio |
| date | formato YYYY-MM-DD valido |
| slug | lowercase, sem espacos, unico entre posts |
| draft | booleano |
| tags | array de strings |
| type | um dos tipos definidos em schema.yaml |

## Subtarefas

- [ ] Criar script validate-content.ts
- [ ] Integrar como npm script
- [ ] Adicionar pre-commit hook (opcional)
- [ ] Validar conteudo existente e corrigir erros
