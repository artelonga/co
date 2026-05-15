# E2E Interactions

Atomic user-level interactions with **acceptance criteria** as the executable specification. Mirrors the co-auto pattern: each interaction declares its expected behavior up front, the test body exercises and verifies it.

## What an interaction is

One coherent CRUD chain a user (or agent) would take on the platform:

- *Alter `artelonga::sobre.md` — replace external Instagram links with internal ArteLonga profile wikilinks. This triggers a sub-task: create a profile page for `falcao` (the one wikilink already pointed at a non-existent profile). Both items remain open until completed.*

The reference notation `<universe>::<path>` is the **universal entry identifier**:

| Form | Meaning |
|---|---|
| `artelonga::sobre.md` | Entry at path `sobre.md` in the `artelonga` universe |
| `artelonga::comunidades` | The `comunidades/` folder (no file extension ⇒ directory) |
| `template::content/seguranca.md` | Seeded transparency page |

## File layout

```
e2e/interactions/
├── README.md                                       (this file)
├── registry.yaml                                   (machine-readable index)
├── 01-artelonga-social-to-profiles.spec.ts         (first interaction)
├── 02-...
```

Each interaction is a single Playwright spec file. The leading `NN-` prefix orders them by progression of the platform's capabilities.

### `registry.yaml`

Machine-readable index — every spec file must have a matching entry. Lets `co-auto` (or any other agent) enumerate, filter by tag, and dispatch interactions without parsing TypeScript:

```bash
# List all interactions
yq '.interactions[] | .id + " — " + .title' e2e/interactions/registry.yaml

# Only interactions that mutate the artelonga universe
yq '.interactions[] | select(.universe == "artelonga") | .id' e2e/interactions/registry.yaml

# Spec file for interaction 01
yq '.interactions[] | select(.id == "01") | .spec' e2e/interactions/registry.yaml
```

Fields per interaction:

| Field | Meaning |
|---|---|
| `id` | Two-digit string matching the spec filename prefix |
| `title` | One-line human label |
| `spec` | Relative path under `e2e/interactions/` |
| `universe` | The universe key the interaction primarily touches |
| `refs` | Entries the interaction reads/edits (`universe::path`) |
| `produces` | Entries the interaction creates |
| `requires` | Env vars that must be set; missing → test skips |
| `tags` | Free-form labels for filtering |
| `safety` | `snapshot-restore`, `dry-run`, or `destructive` |

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
