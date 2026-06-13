# Writing proof-verifiable acceptance criteria

> How to make the DoD verifier (`co-web/scripts/dod/verify.ts`) score a task —
> including **refactor / structural** tasks that have no e2e test. Added in
> CO-443; see also CO-382 (the original DoD gate).

## TL;DR

The verifier scores each `- [ ]` item under `## Acceptance` two ways:

1. **Test-name matching** (default). It derives keywords from the item text and
   looks for a matching test — a Playwright `test('…')` **or** a Rust
   `#[test]` / `#[tokio::test]` function. Good for `feat` tasks where an item
   maps to a behaviour test.
2. **Structural proofs** (`dod_checks`). You attach a small, deterministic proof
   (a grep, a missing-grep, a named Rust test, a file existence) to an item in
   the spec frontmatter. Good for `refactor` tasks whose acceptance is
   structural — "trait X moved to `core`", "game-core is axum-free", "no raw SQL
   in handlers" — which never mapped to an e2e test and forced `--ignore-dod`.

All proofs are **pure filesystem reads** — no build, no network — so they are
fast and deterministic, and they never *block* a merge: an unmet proof is
advisory (`⚠️ pending`), exactly like an unmatched test.

## The `dod_checks` frontmatter map

Add a `dod_checks:` list to the spec's YAML frontmatter. Each entry is a string:

```
"<match substring>  =>  <proof>[ && <proof> …]"
```

- **`<match substring>`** — matched case-insensitively against the *full* text of
  each acceptance item (wrapped continuation lines are joined). The item that
  contains the substring gets these proofs. Pick a phrase unique to that item.
- **`<proof>`** — one or more proof directives, joined with `&&` (all must pass).

Example (from CO-433):

```yaml
dod_checks:
  - "teste de contenção => rust-test:writes_to_two_universes_do_not_serialize"
  - "trait EntryStore => grep:trait EntryStore@co-web/src/repository/entry_store.rs && grep:impl EntryStore for SqliteEntryStore@co-web/src/repository/entry_store.rs"
  - "Zero SQL cru em handlers => grep-absent:conn\\(\\)\\.(prepare|execute|query_row)@co-web/src/security/routes.rs && grep:impl Storage@co-web/src/storage/security.rs"
```

## Proof directives

| Directive | Passes when | Use for |
|---|---|---|
| `grep:<regex>[@<scope>]` | ≥1 matching line is found | "trait/struct/fn X exists", "RouteDescriptor is used" |
| `grep-absent:<regex>[@<scope>]` | **0** matching lines are found | "axum-free", "no raw SQL in routes", "no `std::env::var`" |
| `rust-test:<fn_name>` | a `#[test]` / `#[tokio::test]` fn of that exact name exists | "a test proves the swap / contention / round-trip" |
| `e2e-test:<regex>` | a Playwright `test('…')` name matches | behaviour items that *do* have an e2e test |
| `file:<path>` | the path (file or dir) exists | "module split into `storage/migrations/`", "`seed.rs` moved to `server/`" |

### Scope (`@<path>`)

`grep` / `grep-absent` take an optional `@<scope>` (relative to repo root):

- a **file** → search just that file (e.g. `@game-core/Cargo.toml`);
- a **directory** → search source files under it (e.g. `@co-web/src/repository`);
- **omitted** → search the Rust source roots (`core/src`, `co-web/src`,
  `game-core/src`, `co-cli/src`, and their `tests/`).

The pattern is a JavaScript regex. Escape regex metacharacters in the proof
string (e.g. `conn\(\)\.prepare`). Because the spec is parsed by the verifier
(not a YAML loader), backslashes are preserved literally — write them once.

## Worked patterns from the Mythos epic (CO-431..436)

| Acceptance shape | Proof |
|---|---|
| "promote `Universo` trait to `core`" | `grep:pub trait Universo@core/src/universo.rs` |
| "`core` leaks no server concerns" | `grep-absent:axum@core/src` |
| "factory swap proven by a test" | `rust-test:fabrica_fake_serve_universo_nao_filesystem_pelos_mesmos_handlers` |
| "layering propagated to references/relations" | `file:co-web/src/repository/reference_repository.rs && file:co-web/src/service/relation_service.rs` |
| "game-core is axum-free + portable descriptor" | `grep-absent:axum@game-core/Cargo.toml && grep:Vec<RouteDescriptor>@game-core/src/plugin.rs` |
| "migrations.rs sliced; fresh DB still reaches latest" | `file:co-web/src/storage/migrations/mod.rs && rust-test:fresh_db_reaches_latest_version` |

## Running the verifier

```bash
cd co-web
npm run dod:verify -- --spec CO-433
# → DoD: 100% (5/5 passed), report at docs/scrum/dod/CO-433.json
```

## Guidelines

- **Prove the assertion, not a proxy.** A `grep`/`rust-test` should point at the
  thing the item claims exists. The verifier checks *existence*, so keep proofs
  specific (scope the grep to the file you mean).
- **One distinctive match phrase per item.** If two items share a phrase they'll
  both pick up the proofs — choose a substring unique to the item.
- **Structural proofs complement tests, they don't replace them.** Where a
  behaviour can be tested, prefer a `rust-test:`/`e2e-test:` over a grep.
- **Proofs are advisory.** They raise the DoD %, but only a *matched test that
  fails when executed* blocks a merge. `dod_checks` is about giving honest credit
  to refactor work, not about gating.
