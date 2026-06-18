## workspace-universes — ~/projects as a universe workspace (folder = universe)

Local dev can now treat every top-level folder under `CO_LOCAL_REPOS_DIR`
(`~/projects`) that carries a `_universe.yaml` as a CO universe — **no code change,
no deploy, no git required**. Move/drop a folder in (e.g. `~/projects/yuri`) and it
registers as a universe (key = folder name) on the next `co serve`, with content
ingested for localhost and full CRUD via the web editor + Vault API + `co sync`.

### What changed
- `Storage::register_universes_from_local_dir(dir)` — scans top-level folders,
  registers each `_universe.yaml`-bearing one (`INSERT OR IGNORE`, idempotent;
  name/parent from the manifest; indexes `content/` if present, else root).
- Wired into `run_sister_repo_seeds`, **gated on `CO_LOCAL_REPOS_DIR`** — so it runs
  only in local dev. Prod never sets that env, so prod is untouched (inert).
- Replaces the need to hand-edit the hardcoded key→path bootstrap list for new
  content universes (DB-driven; aligns with the no-hardcoded-mappings rule).
- `scripts/co-local.sh` — serve the whole workspace on localhost in one command.
- `scripts/co-deploy.sh` — one-word CO-app deploy to prod (gate + deploy + smoke).
  (Universe *content* deploys CO-natively via `co sync push <key>`.)

### Verified
Booting locally with `CO_LOCAL_REPOS_DIR=~/projects` auto-registered miguel, mse,
nlp, grcsamazonia from their manifests (others already seeded). Unit test
`test_register_universes_from_local_dir` covers register + idempotency.
