# CO — Feedback Checklist (Teste Público Inicial)

Versão: **1.20.2** — abril de 2026
URL: <https://co.artelonga.com.br> (após DNS) ou <https://co-artelonga.fly.dev>

Obrigado por testar o CO. Este documento lista o que avaliar e como reportar.

## Como reportar

| Canal | Quando usar |
|-------|-------------|
| **GitHub Issues** — <https://github.com/artelonga/co/issues> | Bugs reproduzíveis, propostas de feature |
| **E-mail** — yuri@artelonga.com.br | Privacidade, segurança, dados de conta |
| **Bate-papo no app** (em breve) | Feedback rápido, dúvidas de UX |

Para bugs, inclua: navegador, sistema, passos para reproduzir, captura de tela, console output (F12 → Console).

---

## 1. Primeiro contato (anônimo)

- [ ] Página carrega em < 3s na primeira visita
- [ ] Idioma padrão é português (pt-BR)
- [ ] Tema padrão é "Modern" (claro, contraste OK)
- [ ] Seção "Aprenda CO" mostra 7 tarefas tutoriais na coluna "A fazer"
- [ ] Banner "CO — Gestão de conteúdo em grafo" visível
- [ ] É possível arrastar tarefa entre colunas sem login
- [ ] É possível criar nova tarefa via "+ Nova Tarefa"
- [ ] Refresh mantém o estado (mesmo universo anônimo carregado)
- [ ] Após 100 entradas: aparece modal "Crie uma conta para continuar"

## 2. Cadastro e login

- [ ] "Criar conta" abre fluxo de cadastro
- [ ] Senha exige mínimo razoável (não aceita "1234")
- [ ] Login com e-mail+senha funciona
- [ ] Após login, universo anônimo é "reivindicado" (vira do usuário)
- [ ] Badge do usuário aparece no header
- [ ] Logout funciona e volta para estado anônimo
- [ ] "Esqueci minha senha" — funciona ou está claramente "em breve"

## 3. Universos

- [ ] Sidebar lista universos do usuário
- [ ] É possível criar novo universo (`+`)
- [ ] Universo novo é privado por padrão
- [ ] É possível alternar entre universos sem perder estado
- [ ] É possível tornar universo público / privado
- [ ] É possível duplicar universo (botão "Duplicar")
- [ ] Universos públicos compartilhados via link funcionam

## 4. Edição de conteúdo

- [ ] Criação de tarefa: título, status, tags, due_date salvam
- [ ] Edição inline funciona (clique no título → editar → enter)
- [ ] Markdown renderiza corretamente (negrito, listas, links)
- [ ] Mermaid diagrams renderizam (cole `\`\`\`mermaid graph LR; A-->B \`\`\``)
- [ ] Imagens via `![alt](url)` renderizam
- [ ] Tabelas markdown renderizam
- [ ] Wiki-links `[[Outra Nota]]` funcionam (se aplicável)

## 5. Visões

- [ ] **Quadro (Kanban):** colunas, drag-and-drop, contagem por coluna
- [ ] **Tabela:** ordenação por coluna, filtro por tag/status
- [ ] **Conteúdo:** lista de páginas/notas, abre no editor
- [ ] **Linha do tempo:** eventos com `type: event` aparecem ordenados
- [ ] **Jardim/grafo** (se disponível): nós e arestas renderizam

## 6. Temas

- [ ] Dropdown de temas no header lista 12 temas
- [ ] Cada tema aplica instantaneamente (sem reload)
- [ ] Tema escolhido persiste após refresh
- [ ] "Modern" é padrão para usuários novos
- [ ] Contraste de cada tema é legível (sem texto invisível)

## 7. Idioma

- [ ] Toggle pt/en alterna interface
- [ ] Idioma persiste após refresh (cookie)
- [ ] Tradução de "A fazer" / "Em andamento" / "Concluído" correta

## 8. Performance

- [ ] Quadro com 50+ tarefas continua fluido
- [ ] Trocar de universo: < 500ms
- [ ] Salvar tarefa: < 300ms (sem sensação de lag)
- [ ] Mobile (Safari iOS / Chrome Android): scroll suave, drag funciona

## 9. Privacidade e legal

- [ ] [Política de Privacidade](https://co.artelonga.com.br/co/template?path=content/privacidade.md) acessível
- [ ] [Termos de Uso](https://co.artelonga.com.br/co/template?path=content/termos.md) acessível
- [ ] [Lista completa de dados rastreados](https://co.artelonga.com.br/co/template?path=content/dados-rastreados.md) acessível
- [ ] DNT (Do Not Track) é respeitado: telemetria não envia eventos
- [ ] Exportação de dados funciona (download Markdown)
- [ ] Exclusão de conta funciona (e remove dados em 30d — verificar manual)

## 10. Console (DevTools)

Abra F12 → Console e Network durante uso normal. Reporte:

- [ ] Sem erros 4xx/5xx em chamadas de API durante uso normal
- [ ] Sem stack traces JS no console
- [ ] Sem warnings de mixed content (HTTP em HTTPS)
- [ ] Beacon de telemetria envia 200 (ou 204), não 415

## 11. Sugestões abertas

- O que faltou? (Feature mais pedida)
- O que confundiu? (UX que precisou de explicação)
- O que travou? (Bug, lentidão, crash)
- O que encantou? (Feature que vale destacar)

---

## Limitações conhecidas (v1.20.x)

Não são bugs — estão no roadmap. Não precisam ser reportados:

- Cifragem dos corpos em repouso ainda não implementada (CO-86, v3.0)
- App desktop nativo não disponível (Electron — v3.0)
- App mobile não disponível (Capacitor — v3.0)
- Sync CRDT entre dispositivos é experimental (CO-77)
- Login social (Google/GitHub) não implementado
- Recuperação de senha por e-mail é simplificada
- Editor markdown não tem WYSIWYG ainda

---

*Obrigado por participar do teste público inicial. Cada feedback ajuda a moldar a v3.0.*
