# E2E Interactions

Atomic platform **primitives** for content, exposed as typed RPCs with machine-checkable pre/postconditions. Content (paths, bodies, frontmatter) is **runtime data** — the primitives are the contract; specific business operations like "switch IG links to wikilinks" are clients of those primitives, not part of the contract itself.

## The atomic action

One resource: **`entry`**. Four operations defined by HTTP verb. `{universe}` is a path parameter — the same primitives work for every universe the caller can access.

| Operation     | HTTP     | Path |
|---|---|---|
| `getEntry`    | `GET`    | `/api/v1/universes/{universe}/entries/{path}` |
| `putEntry`    | `PUT`    | `/api/v1/universes/{universe}/entries/{path}` |
| `deleteEntry` | `DELETE` | `/api/v1/universes/{universe}/entries/{path}` |
| `listEntries` | `GET`    | `/api/v1/universes/{universe}/entries` |

These are the contract. Anything else is a composition of these.

The reference notation `<universe>::<path>` is the universal entry identifier — useful for documenting fixtures and for agents talking about entries without parsing URLs:

| Form | Meaning |
|---|---|
| `artelonga::sobre.md` | Entry at path `sobre.md` in the `artelonga` universe |
| `artelonga::comunidades` | The `comunidades/` folder (no file extension ⇒ directory) |
| `template::content/seguranca.md` | Seeded transparency page |

## File layout

```
e2e/interactions/
├── README.md            (this file)
├── registry.yaml        (machine-readable index; OpenAPI source)
├── 01-content-crud.spec.ts  (exercises all four CRUD primitives)
```

One spec exercises all four primitives back-to-back so a failure points at the broken primitive by name. Adding a new primitive means a new entry in `registry.yaml` plus a new assertion in the spec (or a new spec file if the operation is too distinct to fit the cycle).

### `registry.yaml`

The registry **is** a canonical OpenAPI 3.1 document — `paths` × `methods`, exactly the shape REST clients already understand. No flat operation list, no custom schema. Pre/postconditions live in `x-preconditions` / `x-postconditions` extensions per operation; safety classification in `x-safety`.

```bash
# Every operationId in the doc
yq '.paths[][] | select(.operationId) | .operationId' e2e/interactions/registry.yaml

# Find the path + method for getEntry
yq '.paths | to_entries | .[] | .key as $p | .value | to_entries | .[] | select(.value.operationId == "getEntry") | {path: $p, method: .key}' e2e/interactions/registry.yaml

# All destructive operations
yq '.paths[][] | select(."x-safety" == "destructive") | .operationId' e2e/interactions/registry.yaml
```

OpenAPI fields used per operation:

| Field | Meaning |
|---|---|
| `operationId` | Stable RPC name — URL slug at `/api/v1/interactions/{operationId}` |
| `summary` | One-line human label |
| `parameters` | OpenAPI parameters (path / query / header) |
| `requestBody` | For PUT/POST — JSON-Schema |
| `responses` | Status code → description + schema |
| `tags` | OpenAPI tag grouping (currently just `entry`) |
| `x-preconditions` | Rules that must hold before the call |
| `x-postconditions` | Rules that must hold after the call |
| `x-safety` | `snapshot-restore` / `dry-run` / `destructive` |

### Served endpoints

The registry is parsed once at server startup and exposed under `/api/v1/interactions/`:

```
GET  /api/v1/interactions/                  list interactions (id + summary)
GET  /api/v1/interactions/openapi.json      derived OpenAPI 3.1 paths
GET  /api/v1/interactions/{operationId}     single interaction spec
POST /api/v1/interactions/{operationId}     execute (reserved — 501 today)
```

The OpenAPI doc is generated from the YAML at request time — no
build step, no codegen lag. Plug it into Swagger UI, generate a
client with `openapi-generator`, or hand it to an agent SDK.

The POST runtime is reserved (returns `501 Not Implemented` with a
message pointing to the Playwright command). Adding it is the next
step: a Rust handler that authenticates the caller, executes the
WHEN logic via existing entry API calls, then returns
`{ operationId, criteria: [{id, rule, passed, evidence}], produced: [...] }`.

Once the runtime is wired, calling an interaction becomes:

```bash
curl -X POST https://co.artelonga.com.br/api/v1/interactions/artelongaSwitchSocialToProfiles \
  -H 'Content-Type: application/json' -b cookies.txt \
  -d '{"universe":"artelonga","targetEntry":"sobre.md"}'
```

Equivalent to running the Playwright spec — same pre/postcondition
contract, same produced entries.

## Required spec shape

Each spec opens with a JSDoc comment block carrying the acceptance criteria. The body is a normal Playwright `test()` that exercises the flow and asserts each criterion.

```typescript
/**
 * INTERACTION-NN: <one-line title>
 *
 * REF: <universe::path entries this interaction touches>
 *
 * GIVEN:
 *   - <preconditions — what must be true for the interaction to start>
 *
 * WHEN:
 *   - <the user-level action(s)>
 *
 * THEN:
 *   - <acceptance criterion 1>
 *   - <acceptance criterion 2>
 *   - <sub-tasks created and their expected state>
 *
 * SAFETY:
 *   - <snapshot-restore or dry-run mode if mutating shared data>
 */
```

The acceptance criteria become the test's assertions one-to-one. Each criterion is a `expect()` call so a CI failure points at the violated criterion by name.

## Running

```bash
# All interactions against prod
BASE_URL=https://co-artelonga.fly.dev \
CO_TEST_USER_EMAIL=yuri@artelonga.com.br \
CO_TEST_USER_PASSWORD=*** \
npx playwright test e2e/interactions/

# A single interaction
BASE_URL=https://co-artelonga.fly.dev \
CO_TEST_USER_EMAIL=... \
CO_TEST_USER_PASSWORD=... \
npx playwright test e2e/interactions/01-artelonga-social-to-profiles.spec.ts
```

Specs **must** snapshot the original state before mutating and restore it in `afterEach` so a re-run is idempotent and a failed run doesn't leave prod in a broken state. If credentials are missing, the spec **must** skip rather than fail — a CI without secrets shouldn't go red on every interaction.

### Idempotency contract

Specs are expected to be safely re-runnable. Three rules:

1. **Detect the post-state.** Before asserting preconditions, the test checks whether the WHEN action has already happened (e.g. "are the Instagram links already gone?"). If so:
   - Clear the snapshot variables (do **not** "restore" garbage into the canonical entry).
   - Call `test.skip(true, "<explicit message — manual restore needed>")`.
   - Return.
2. **Don't overwrite human-authored content created out-of-band.** When producing an entry that may already exist (e.g. a profile stub a contributor wrote between runs), GET it first; if it returns 200, leave it alone and don't mark it for cleanup.
3. **Cleanup is conditional on creation.** `afterEach` deletes only entries the spec itself created in *this* run, tracked via boolean flags. Restoring a snapshot uses the snapshotted body only if it was actually captured (precondition succeeded).

Together: a stuck run, a successful run, and a partially-applied run all converge — the test never silently corrupts shared data.

## Why interactions instead of just spec files

Three properties together:

1. **Atomic**: each interaction is one user-level operation with a clear before/after — not a unit test of an API endpoint, not a full user journey across the app.
2. **Acceptance criteria as the contract**: GIVEN/WHEN/THEN drives both the test assertions and the design conversation. A new platform feature gets a new interaction file with new criteria — not a buried PR comment.
3. **Sub-tasks tracked through completion**: an interaction that triggers downstream work (create a profile, file a follow-up task) asserts that work exists and is open. Forgetting the follow-up fails the test.
