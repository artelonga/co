# Markdown renderers — como visualizar conteúdo CO

Universo CO é Markdown-first. Você pode browsear seu universo (e
outros públicos) através de várias ferramentas — todas consomem a
mesma API REST + Vault.

## Opções disponíveis

### 1. Web SPA nativa (incluso)

`https://co.artelonga.com.br/<seu-universo>`

- **O que é**: a SPA principal de CO, escrita em ES modules vanilla
- **Para que serve**: operacional + admin + edição rápida
- **Vantagens**: zero setup, sempre atualizado, integrado com chat
  + notificações + tarefas
- **Limitações**: minimal por design; não é o lugar para reading prolongado

### 2. Cloud markdown viewer (Svelte + TS — CO-212)

**Status: em desenvolvimento.** Será servido em
`https://co.artelonga.com.br/viewer/<universo>` ou subdomínio
separado.

- **O que é**: app Svelte focado em **leitura bonita** de markdown
- **Vantagens**: tipografia cuidada, links cross-universe, citações
  com hover preview, dark mode polished, melhor para leitura longa
- **Limitações**: read-mostly (editing happens em outra ferramenta)

### 3. Obsidian (recomendado para edição local)

[Obsidian](https://obsidian.md/) é um editor Markdown desktop popular.

- **Como conectar**: usa o plugin CO-Obsidian (CO-68, polish em
  CO-213)
- **Setup**:
  1. Instale Obsidian
  2. Em CO settings → "Vault API token" → gere um token
  3. Em Obsidian → instale plugin "CO Vault Sync" (do community plugins
     ou via BRAT)
  4. Configure URL: `https://co.artelonga.com.br`, token, e universo
  5. Sync acontece a cada save, pull-on-open também
- **Vantagens**: full editing power do Obsidian (gráfico, backlinks,
  daily notes, plugins community), offline, local files
- **Limitações**: setup precisa configuração; mudanças não propagam
  em tempo real para outros viewers (apenas no próximo sync)

### 4. CLI (`co` binary)

Para usuários técnicos que querem fluxo terminal.

```bash
# Instalar
cargo install co-cli

# Browse seu universo no terminal
co browse seu-universo

# Servir o universo localmente (renderer próprio web em localhost)
co serve seu-universo
```

- **O que é**: CLI Rust binary que serve seu universo como site
  estático local
- **Vantagens**: pode trabalhar offline com SQLite local; rápido
- **Limitações**: apenas para devs

### 5. Browsers de markdown genéricos

Como o conteúdo é Markdown em formato standard (CommonMark + frontmatter
YAML), qualquer tool pode renderizar:

- **VS Code / Cursor**: open the .md file, preview com Cmd+Shift+V
- **Marked** (macOS): drag .md file
- **MarkText**, **Zettlr**, **Typora**: editores Markdown standalone
- **Glow** (CLI): `glow .` em diretório do universo

Esses não conectam à API; precisam dos arquivos localmente (via Obsidian
sync, git clone do universo, ou `co pull`).

### 6. GitHub web UI

Universos backed por git repo (CO-89, futuro) são navegáveis em
`github.com/<user>/<universo>` diretamente.

Funciona sem nenhum setup. Limitado a renderização padrão do GitHub.

---

## Comparação rápida

| Tool | Setup | Edição | Live updates | Offline | Best for |
|---|---|---|---|---|---|
| Web SPA | zero | rápida | sim (WS) | não | uso diário |
| Cloud viewer (CO-212) | zero | não | sim | não | leitura longa |
| Obsidian + CO plugin | médio | excelente | sync a cada save | sim | edição séria |
| CLI `co serve` | dev setup | via editor | manual | sim | técnicos |
| MarkText etc | mínimo | standalone | não | sim | quick view |
| GitHub web | zero (se git-backed) | via PR | manual | não | visitas casuais |

---

## API contract (CO-211)

Para implementar um novo renderer, consuma estes endpoints:

### Públicos (sem auth para universos públicos)

```
GET /api/v1/universes/<slug>                      → metadata
GET /api/v1/universes/<slug>/entries              → lista todos os arquivos
GET /api/v1/universes/<slug>/entries/<path>       → conteúdo de 1 arquivo
GET /api/v1/universes/<slug>/manifest             → schema do universo
```

### Autenticados (para universos privados)

```
GET /api/v1/universes/<slug>/vault/notes
GET /api/v1/universes/<slug>/vault/tags
GET /api/v1/universes/<slug>/vault/note/<path>
PUT /api/v1/universes/<slug>/vault/note/<path>
```

Auth: Bearer token via `Authorization: Bearer <token>` header
(emitido em settings → API tokens).

OpenAPI spec completo: planejado em CO-211. Versionado v1.

---

## Cloud markdown viewer — telemetria e 404 tracing (CO-210)

O cloud viewer (CO-212) tem **telemetria embarcada** para que possamos
melhorar a experiência sem que você precise relatar problemas
manualmente:

### O que coletamos

- **Page views**: qual universo + qual rota foi visitada
- **404s**: quais URLs retornam não-encontrado (para detectar
  links quebrados, paths incorretos)
- **Errors**: JavaScript errors no renderer (stack trace + browser
  context)
- **Performance**: tempo de load por página

### O que NÃO coletamos

- **Conteúdo lido**: não logamos qual texto específico você viu, só a
  rota
- **Mensagens privadas / DMs**: nunca
- **PII**: emails, nomes pessoais não vão pra telemetry
- **Sessões individuais**: usamos visitor_id opaco (CO-46), não user_id

### Por que coletamos

Cloud viewer está em **fase de teste**. Sem telemetria, problemas que
você experimenta (link quebrado, página lenta, erro de render) ficam
invisíveis para nós até alguém abrir um issue. Telemetria nos permite
detectar e corrigir proativamente.

### Como desativar

Em settings → Privacidade → "Não enviar telemetria de viewer". Stored
em `localStorage["co_viewer_telemetry"] = "0"`. O viewer respeita esse
flag e desativa sender.

---

## Roadmap

- **CO-210**: serve estas páginas (`/seguranca`, `/dependencias`,
  `/licensa`, `/markdown-renderers`) com SPA routing + telemetria
- **CO-211**: formalizar Universe Content API contract v1 + OpenAPI spec
- **CO-212**: build Svelte + TS cloud viewer
- **CO-213**: polish Obsidian plugin (CO-68 follow-up)

Quando esses 4 landam, qualquer Markdown renderer (terceiro, custom,
nosso novo viewer) consome a mesma API uniformemente. CO se torna
uma plataforma multi-frontend.
