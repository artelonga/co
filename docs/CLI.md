# CO CLI — Referência de Comandos

> Gerada e revisada contra a saída real de `co --help` em **3.2.x** (CO-411).
> Domínio em português, técnica em inglês — como o resto do CO.
> Instalação: `cargo install --path co-cli` a partir do repo (binário `co`).

## Os cinco verbos do dia a dia

### `co auth` — autenticação e tokens

Tudo com input oculto; credenciais em `~/.config/co/credentials` (mode 600).
Servidor default: `https://co.artelonga.com.br`.

```bash
co auth login --email you@example.com --save-token   # login com senha
co auth reset-password --email you@example.com       # esqueceu? código por e-mail → nova senha → login
co auth token create --save                          # token de API (90 dias) p/ co push e Vault API
co auth status                                       # exit 0/1 — bom para scripts
co auth logout --revoke-token
```

> Desde a 3.2.x o cliente envia `User-Agent: co-cli/<versão>` — versões
> anteriores eram rejeitadas pelo gate anti-abuso do servidor
> (`400 missing_user_agent`). Se vir esse erro, atualize o binário.

### `co push` — universo local → servidor remoto

```bash
co push --remote https://co.example.com --token mytoken
CO_REMOTE=… CO_TOKEN=… co push        # via env (token de co auth token create)
co push --dry-run                     # plano sem escrever
co push --delete-missing              # remove no servidor o que sumiu localmente
```

Idempotente: `POST /api/v1/universes` + `PUT …/vault/{path}` por arquivo.

### `co construir` — universo → site estático (Quartz)

```bash
co construir [<key>] [--out <dir>] [--redearte <path>]
```

### `co updates` — notas de release no terminal

```bash
co updates           # a release mais recente
co updates -n 3      # as três últimas
co updates --all     # histórico completo desde 0.1.0
```

Changelog embutido no binário — offline, sempre na versão instalada.

### `co serve` / `co board` — rodar localmente

```bash
co serve                     # servidor web localhost-first
co board [--port 8080]       # quadro de projetos (web UI)
co launch                    # bootstrap do diretório atual como universo no CO local
```

## Conteúdo e grafo

| Comando | Função |
|---|---|
| `co init <name>` | novo espaço |
| `co new` / `co create` | criar conteúdo (task, definition, …) |
| `co show` / `co update` / `co delete` | CRUD de arquivos de conteúdo |
| `co locate [--type task]` | busca e filtros (+ gestão do índice) |
| `co query` | consultas DSL sobre o grafo |
| `co validate all` | validação de erros/avisos |
| `co status` | estatísticas do grafo |
| `co index` | reconstrói o índice |
| `co archive` (`ar`) | arquiva conteúdo (desindexado) |

## Línguas e esquemas

| Comando | Função |
|---|---|
| `co define` / `co translate` | definições e traduções |
| `co lang` (`languages`) | línguas e traduções de UI |
| `co schema` | esquemas de tipos de conteúdo |

## Integração e operação

| Comando | Função |
|---|---|
| `co deploy` | deploy de universo para plataforma alvo |
| `co repo` / `co space` | repositórios federados e espaços multi-repo |
| `co gh` / `co collab` / `co conduct` | fluxo GitHub (issues, sync, objetivos) |
| `co tool` / `co tools` / `co agents` | ferramentas git-backed e agents |
| `co write` / `co analyze` / `co lead` | escrita assistida, análise, exploração |
| `co config` | configuração |
| `co help` (`h`) | conceitos, fluxos e comandos |

---

Para a história e o primeiro CRUD guiado: [`WELCOME.md`](./WELCOME.md).
Para o pipeline de entrega (git × jj): [`delivery-pipeline.md`](./delivery-pipeline.md).
