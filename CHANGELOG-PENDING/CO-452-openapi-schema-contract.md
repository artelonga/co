## CO-452 — OpenAPI spec becomes a real contract (schemas, version, auth)

The generated `co-web/openapi.yaml` (served at `/api/docs` and `/api/openapi.json`)
was a bare endpoint inventory: every operation emitted only `200 OK`, the ~34
component schemas in `openapi-components.yaml` were defined but never referenced,
auth was hardcoded to `sessionCookie`, and `info.version` was pinned at a stale
`2.40.0`. The generator now produces a usable contract:

- **`info.version`** is read from the workspace `Cargo.toml` (now 3.15.0), not hardcoded.
- **Security schemes** are mapped correctly per catalog auth tag (`bearerJWT`,
  `apiToken`, `sessionCookie`, `sharedSecret`) with OR-semantics, instead of
  labelling everything `sessionCookie`.
- **Request/response schemas** are wired via a sidecar `SCHEMA_MAP` in the
  generator (the catalog markdown and the catalog↔code drift check are untouched):
  mapped operations emit a `requestBody` + typed success response + a `default`
  `Error` response, all `$ref`-ing existing component schemas. 24 high-confidence
  endpoints wired (auth, tasks, gestão eventos/validar/publicar/manifesto,
  quilombo auth/perfil/mensagens/missões/eventos/comentários); the rest stay bare
  and can be annotated incrementally.

`npm run openapi:check` stays green (no drift); the spec validates as OpenAPI 3.1
with zero dangling `$ref`s.

### Why
A schema-less spec gives Swagger UI no "try it out" payloads and no documented
errors. Wiring the already-defined component schemas turns the served spec into a
real, explorable contract without touching the source-of-truth catalog.
