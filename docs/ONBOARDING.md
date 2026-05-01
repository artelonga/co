# Onboarding — CO Platform

Guia de 5 minutos para rodar o CO localmente a partir do código-fonte. Siga este documento se você for um novo contribuidor ou quiser inspecionar o servidor antes de fazer deploy. Para entender o que cada componente faz, veja [ARCHITECTURE.md](ARCHITECTURE.md). Para operar em produção, veja [OPERATIONS.md](OPERATIONS.md).

> _English translation is welcome — open a PR._

---

## 1. Pré-requisitos

| Ferramenta | Versão mínima | Para que serve |
|-----------|--------------|----------------|
| Rust toolchain | stable (≥ 1.83) | compilar `co-web` e utilitários |
| sqlite3 | qualquer | inspecionar o banco localmente (opcional) |
| protobuf-compiler | qualquer | dependência de build do `co-web` |

Instale o Rust via [rustup.rs](https://rustup.rs). No macOS, `protobuf-compiler` vem via `brew install protobuf`.

---

## 2. Clone e entre no diretório

```bash
git clone https://github.com/artelonga/co
cd co
```

---

## 3. Compile o servidor

```bash
cargo build -p co-web --release
```

A primeira compilação baixa dependências e leva ~3–5 min. Compilações seguintes são incrementais.

---

## 4. Gere o hash da senha admin

O servidor semeia um usuário admin na inicialização a partir de variáveis de ambiente. Use `co-pwhash` para gerar o hash Argon2id:

```bash
cargo run -p co-pwhash -- 'minha-senha-segura'
# saída: $argon2id$v=19$m=65536,t=3,p=1$...$...
```

Copie o hash completo (incluindo `$argon2id$...`).

---

## 5. Configure as variáveis de ambiente

Crie um arquivo `.env` (não commitado) ou exporte manualmente:

```bash
export JWT_SECRET=$(openssl rand -base64 48)
export CO_SEED_ADMIN_EMAIL=admin@localhost
export CO_SEED_ADMIN_PASSWORD_HASH='$argon2id$...'   # hash do passo 4
```

Variáveis opcionais (valores padrão funcionam para desenvolvimento local):

| Variável | Padrão | Descrição |
|----------|--------|-----------|
| `CO_WEB_PORT` | `8742` | Porta do servidor |
| `CO_WEB_DATA` | `./data` | Diretório de dados (SQLite + arquivos) |
| `CO_WEB_STATIC` | `co-web/static` | Diretório de arquivos estáticos |

---

## 6. Execute o servidor

```bash
cargo run -p co-web
```

Saída esperada:

```
  Project Board
  http://localhost:8742
```

Para o binário compilado: `./target/release/co-web`.

---

## 7. Abra no navegador

Acesse `http://localhost:8742`. A página inicial carrega o universo template com o board de tutorial.

---

## 8. Faça login com as credenciais admin

```bash
curl -sc cookies.txt -X POST http://localhost:8742/api/v1/auth/password-login \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@localhost","password":"minha-senha-segura"}'
# → 200, Set-Cookie: session=<JWT>
```

No navegador: clique em **Entrar**, informe e-mail e senha do passo 5.

---

## 9. Crie seu primeiro universo

Via interface: clique em **Novo universo** no sidebar.

Via API:

```bash
curl -sb cookies.txt -X POST http://localhost:8742/api/v1/universes \
  -H 'Content-Type: application/json' \
  -d '{"name":"Meu universo","key":"meu-universo"}'
```

---

## Onde as coisas ficam

| Caminho | O que contém |
|---------|-------------|
| `co-web/src/` | Servidor Axum (rotas, storage, temas, auth) |
| `co-web/static/` | SPA, SW, CSS |
| `core/src/` | Tipos compartilhados, parser Markdown, validação |
| `co-cli/src/` | Binário CLI (`co`) |
| `dev/co-pwhash/` | Utilitário para gerar hash de senha |
| `dev/co-token/` | Utilitário para gerar API tokens |
| `data/` | Banco SQLite + arquivos de universos (runtime, não commitado) |
| `scripts/` | Smoke tests, backup, restore |
| `docs/` | Documentação (você está aqui) |

---

## Testes

```bash
cargo test                          # todos os crates
cargo test -p co-web                # só o servidor
cargo clippy -- -D warnings         # linting
cargo fmt                           # formatação
```

Todos os testes rodam sem abrir portas de rede reais. Ver `DEV-TESTING.md` para padrões de teste do projeto.
