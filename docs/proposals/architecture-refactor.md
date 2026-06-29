# CO Architecture Refactor — AS IS → TO BE

> Capstone strategic proposal. This is not a "add some libraries" memo; it is the
> full refactor / reorganization plan that turns CO from a monolith-with-embedded-
> tenants into a **generalist, composable, self-hostable agent platform**.
>
> Status: proposal for owner review. Backlog already cut as **CO-503 … CO-530**;
> new adopt-now add-on tasks proposed as **CO-531 … CO-534** (§4 + owner decision #7).
> The full estate-level tool registry (every candidate, mode + ponytail verdict) lives in
> [`ArteLonga/docs/TOOL-REVIEW.md`](../../../ArteLonga/docs/TOOL-REVIEW.md).
> Baseline version at time of writing: `3.22.0`.

---

## 0. Executive summary — the thesis in five lines

1. **AS IS:** CO is a Rust monolith that *embeds a private tenant* (`co-web/src/universes/quilombo/`) inside the open repo, ships a **hand-rolled vanilla-TS SPA** (no component library, no Tailwind, ~7 deps), exposes only **3 in-binary tools** (github/claude/obsidian), and runs its **agentic loop in a separate, non-git Python service** (`tools/whatsapp-bot`).
2. **TO BE:** a **generalist** platform with no tenant hardcoding, built on **three primitives** — **Agents** (model-agnostic `Backend` trait), **Tools** (one canonical contract, in-binary *or* external-by-manifest, NOT MCP), **Vectorizers** (representation-agnostic trait).
3. **The agent runtime moves INTO `co`** (CO-504); the WhatsApp bot becomes one *thin client* — an "agente", not a "bot" (CO-507/508).
4. **Self-host-first**: everything runs on one box (Ollama-local default, sqlite-vec in the canonical DB, the security kit + CO-497 data-residency invariant); nothing in the recommended path makes a network call.
5. **Composability + a contract-test tier** are the connective tissue: tools/agents/vectorizers compose into workflows (CO-506), `co-auto` and the Workflow tool unify on one worktree-isolated runner (CO-516), and a contract-test layer (CO-520) closes the lib-vs-integration gap the StaaS wave review exposed.

---

## 1. AS IS — current architecture and its concrete pain points

### 1.1 What exists today

**`co` (Rust workspace)**
- `co-web/` — Axum server (the product surface). Contains `src/universes/quilombo/` — **a private tenant's backend embedded in the open repo**, plus `src/universes/game`.
- `co/` (core lib) — content model, storage (SQLite, `meta.db` + per-universe `data.db`), op_log, relations (CO-74), branching (CO-95).
- `co-cli/` — the `co` binary.
- `co-agent/` — agent definitions (personas as markdown).
- `dev/co-auto` — the autonomous wave runner (worktrees, FF-on-success).
- **Frontend:** `co-web/static/variants/a/` — a **hand-rolled TypeScript SPA**. ~7 npm deps, **no component library, no Tailwind, no design system**. Each surface (`admin.html`, `analytics.html`, `gestao.html`, `leads.html`, `index.html`, …) is bespoke.
- **Tools:** only **3 in-binary Rust tools** — github, claude, obsidian. Adding one meant a binary recompile (until CO-503).
- **Agentic loop:** lives **outside the repo** in `tools/whatsapp-bot` (a non-git Python service). The tool-calling brain is therefore *not* part of the product, not versioned with it, and not reusable by other clients.

**`yggdrasil`**
- Godot 4 (native client) + HTML `<canvas>`/WASM (web), path-dep on `co/game-core`.
- **`game-core` is server-authoritative game *logic*, not a web client engine.** Its renderer is `crossterm` (terminal); deps are `redb`, `prost`, `redis`/`tokio`, `chacha20poly1305`/`argon2`. It is **not** compiled to WASM. The browser is a *thin view* fed by the server.

### 1.2 Concrete pain points (the "why")

| # | Pain | Concrete symptom |
|---|------|------------------|
| P1 | **Embedded tenant in open repo** | `co-web/src/universes/quilombo/` couples a private customer to the open-sourceable shell; can't open the repo cleanly, can't add a tenant without a deploy, content and code intermingled. |
| P2 | **Hardcoded universe→repo mappings** | Adding content requires a Rust change + deploy (already flagged in memory: never hardcode mappings in Rust). |
| P3 | **Brain lives outside the product** | The agentic loop is a separate non-git Python service. No other client can reuse it; it is unversioned and un-tested with the platform. |
| P4 | **Tools require recompile** | Only 3 in-binary tools; every new capability was a Rust build + redeploy. |
| P5 | **Bespoke frontend, zero leverage** | No component library / design system → every surface is hand-rolled, a11y/dark-mode/keyboard are re-implemented ad hoc, slow to ship, inconsistent. |
| P6 | **No vector / retrieval layer** | No embeddings, no RAG, no semantic search; sacred multilingual corpora (Mbyá, Yoruba) have no sovereign representation. |
| P7 | **lib-vs-integration test gap** | The StaaS wave review showed library tests pass while integration boundaries break (migrations, route wiring). No contract-test tier. |
| P8 | **Two orchestration substrates** | `co-auto` (waves) and any future workflow engine risk diverging; co-auto isolation is already flaky (duplicate agents in one worktree). |
| P9 | **game-core can't be a web engine** | The web client is a thin canvas view; no plan for richer 2D/3D without either rewriting game-core or demoting it. |

---

## 2. TO BE — the target architecture

### 2.1 The three primitives

Everything in the platform is expressed as a composition of three model-agnostic, representation-agnostic, self-hostable primitives.

**(A) AGENTS — a model-agnostic `Backend` trait** (CO-504)
- One trait abstracts the model provider. **Ollama-local is the default** (sovereignty + cost), **Claude** and any OpenAI-compatible endpoint are drop-ins.
- The agent *runtime* (the tool-calling loop) is promoted **into `co`**. Personas (`agents/*.md`) become capability packs (CO-505).
- Because both Ollama and vLLM expose OpenAI-compatible APIs, scaling serving is a *config swap behind the trait*, not a rewrite.

**(B) TOOLS — one canonical contract** (CO-503, already built, PR #324)
- Contract: `name / description / JSON input_schema / run(args) → result`, with `tool_type: deterministic | predictive` and `command` / `url` / `dependencies` fields.
- A tool can be **in-binary OR any external OSS by manifest** (`tools/schema.yaml`). Adding a tool needs **no binary recompile** (CO-503 adapter). Reference adapters: yt-summarizer (predictive), CO-521 deterministic batch.
- **Explicitly NOT MCP.** External frameworks plug in as one manifest row (local service/CLI), keeping everything on one box.

**(C) VECTORIZERS — a representation-agnostic trait** (CO-517)
- Returns one of `{ Dense(Vec<f32>), Sparse(Vec<(u32,f32)>), Numeric(Vec<f32>) }`.
- Three families are first-class: **neural** (BGE-M3, nomic-v2-moe), **statistical** (BM25/FTS5, PPMI+SVD), **raw-numeric** (hand-built feature vectors). The trait makes the non-neural fallback a *citizen, not a toy* — essential for low-resource sacred corpora.

### 2.2 Cross-cutting properties

- **Generalist core:** de-embed quilombo (CO-509), DB-driven mappings (no tenant hardcoding), scrub private-repo refs (CO-510). **Private repos = data**, not code.
- **Self-host-first:** the security kit + **CO-497 data-residency invariant**; local-first defaults; nothing in the recommended path phones home.
- **Composability:** tools + agents + vectorizers compose into **workflows** (CO-506); workflows and `co-auto` share **one worktree-isolated runner** (CO-516).
- **Contract-tested boundaries:** a dedicated contract-test tier (CO-520) is the safety net for every primitive seam.
- **Template self-containment (CO-530):** the **template IS the software, universes ARE use-cases** — the binary builds, distributes, and e2e-tests with **zero universe examples**; content is a runtime add-on (subscription / manifest / artifact), never a build/test/distribution prerequisite. A CI guard builds + e2e's the template in a **clean environment with no content repos present** — so the property holds by construction. This is the acceptance test for the whole composability direction.

### 2.3 Text diagram

```
                         ┌──────────────────────── CLIENTS (thin) ─────────────────────────┐
                         │  WhatsApp "agente"   Web SPA      CLI        Yggdrasil web view  │
                         │  (CO-507/508)         (CO-526 UI)  (co)       (<canvas>)          │
                         └───────────────┬──────────────────────────────────────────────────┘
                                         │  one HTTP/contract surface
            ┌────────────────────────────▼─────────────────────────────────────────────┐
            │                         co-web (Axum)  —  generalist core                  │
            │                                                                            │
            │   ┌── AGENT RUNTIME (CO-504) ───────────────────────────────────────────┐ │
            │   │  Backend trait:  Ollama(local default) | Claude | OpenAI-compatible  │ │
            │   │  tool-calling loop (was the Python bot) · capability packs (CO-505)  │ │
            │   └──────────────┬──────────────────────────────┬───────────────────────┘ │
            │                  │ calls TOOLS                   │ uses VECTORIZERS          │
            │   ┌──────────────▼───────────────┐  ┌───────────▼─────────────────────────┐│
            │   │ TOOL CONTRACT (CO-503)        │  │ VECTORIZER trait (CO-517)           ││
            │   │ name/desc/input_schema/run    │  │ Dense | Sparse | Numeric            ││
            │   │ deterministic | predictive    │  │ neural(BGE-M3/nomic) ·              ││
            │   │ in-binary OR external manifest│  │ statistical(FTS5-BM25/PPMI) · numeric││
            │   │ (NOT MCP). add tool = no build │  └───────────┬─────────────────────────┘│
            │   └──────────────┬────────────────┘              │                          │
            │                  │                                │                          │
            │   ┌── WORKFLOWS (CO-506) ── compose agents+tools+vectorizers ──┐            │
            │   │  shared worktree-isolated runner with co-auto (CO-516)     │            │
            │   └────────────────────────────────────────────────────────────┘           │
            │                                                                            │
            │   STORAGE (SQLite canonical):  meta.db  +  per-universe data.db            │
            │       entries · relations(CO-74) · op_log · vec0(sqlite-vec) · fts5(BM25)  │
            └───────────────┬────────────────────────────────────────────────────────────┘
                            │ path-dep (authoritative brain, unchanged)
                   ┌────────▼─────────┐
                   │  game-core (Rust)│  terminal · native(Godot) · web(Pixi/Three render-only)
                   └──────────────────┘

   CONTRACT-TEST TIER (CO-520) ── verifies every ║ boundary above (lib↔integration gap)
   EXTERNAL ADD-ONS via manifest (NOT MCP): LlamaIndex(rag) · DSPy(prompt-compile) · RAGAS(eval) · voicebox(voice) · last30days(research) · R(stats) · vLLM(serving swap)
   ABSORBED TECHNIQUES (no dep): headroom(context-compression) · FTS5/BM25 · PPMI/SVD · deer-flow/LangGraph(checkpoint/HITL ideas)   |   full registry → ArteLonga/docs/TOOL-REVIEW.md
```

---

## 3. Per-subsystem AS IS → TO BE (with the alternative that justifies the pick)

For each subsystem: the AS IS, the **recommended pick**, and the **one advantage** that justifies it over the surveyed alternatives.

### 3.1 UI component layer (CO-526)

- **AS IS:** hand-rolled vanilla-TS SPA, no component library, no Tailwind, a11y/dark-mode/keyboard re-implemented per surface.
- **Decision fork:** *web components* (framework-agnostic, drop into the current SPA) **vs** *Svelte kits* (only pay off if migrating to SvelteKit).
- **Recommended pick — Web Awesome (Shoelace v3, v3.9.0).**
  - **The justifying advantage:** it is the **only mature framework-agnostic option** — real custom elements drop into the existing vanilla-TS SPA with **no framework rewrite**, CSS-custom-property theming means **no Tailwind required**, and a11y + light/dark are inherited from years of Shoelace hardening. Start on the free core; Pro only for the theme builder/Figma kit.
  - Alternatives considered and why not (now): every Svelte kit (shadcn-svelte, Bits UI + Melt UI, Skeleton v3, Flowbite-Svelte) is Svelte-only and demands a SvelteKit commitment + Tailwind. **If** we deliberately move to SvelteKit later, the pick is **shadcn-svelte** (copy-paste *ownership* of component source on Bits UI accessibility) — see §5 for the rewrite-vs-refactor call.

### 3.2 Web game engine (CO-527)

- **AS IS:** `game-core` is server-authoritative Rust logic (terminal renderer); browser is a thin `<canvas>` view. game-core is **not** WASM and isn't structured to be.
- **The governing principle:** prefer **render-only libraries** over full engines, so the Rust `game-core` stays the single authoritative brain across terminal/native/web.
- **Recommended pick — PixiJS v8 (2D now), Three.js (3D north-star).**
  - **The justifying advantage (Pixi v8):** a pure GPU **2D renderer**, not an engine — it imposes no game loop/physics/ECS, so it matches the *actual* architecture (server draws, client renders) and reuses `game-core` **100%, zero Rust rewrite**; WebGPU+WebGL2 in one package, TS-native, small tree-shaken bundle.
  - **The justifying advantage (Three.js):** a 3D **scene-graph/renderer** (not engine) → "engine-neutral" stays literally true and game-core remains the brain; largest ecosystem, near-zero-config WebGPU with WebGL fallback.
  - Alternatives and why not: Phaser 4 / Excalibur / Babylon.js are **full engines** that want to own the loop and gameplay → they demote or duplicate game-core. **Godot→HTML5** reuses Godot work but gives **no extra game-core reuse** (game-core is a separate crate, not Godot code), and carries the heaviest payload (≈40 MB wasm, 25 MB host single-file limit, WASM memory ceiling, weak mobile-web). Keep Godot as an optional native thick client; **do not** bet the web experience on its export.

### 3.3 Agentic workflow add-ons (CO-525)

- **AS IS:** no retrieval/ingestion, no prompt optimization, no eval gate; the loop is the external Python bot.
- **The seam:** external frameworks plug in via the **CO-503 manifest** as a local service/CLI = one `predictive` tool row (`command`/`url`/`dependencies`). **Not MCP**, stays on one box (satisfies CO-497). Each must *earn its place* by filling a gap the native loop (CO-504/505/506) does not.
- **Recommended picks (wire first, in order):**
  1. **LlamaIndex** (`rag-ingest` + `rag-query`). **Advantage:** CO's Vectorizer is only the vector *primitive* — LlamaIndex brings the missing **ingestion pipeline**: 150+ connectors, **LlamaParse** agentic OCR over 130+ formats (directly serves the mbya PDF pipeline, NotebookLM exports, grcsamazonia `_source` docs), chunking, hybrid retrieval/rerank. Local FastAPI service, embeddings via local Ollama, index on disk under the universe.
  2. **DSPy** (`prompt-compile` CLI, offline). **Advantage:** nothing in CO self-optimizes prompts; **MIPROv2** jointly optimizes instructions + few-shot exemplars for a reported **10–40% lift** on structured tasks and **never touches weights** — output is a better prompt string you commit. Zero runtime coupling, ~$5–10 per 200-example run.
  3. **RAGAS + faithfulness judge** (`eval-faithfulness` tool). **Advantage:** the **measurement layer CO lacks** — an objective quality gate that tells you which retrieval/prompt change actually helped, *and* gives DSPy the metric to optimize toward. Judge runs on local Ollama.
- **Foundational but NOT a manifest tool — vLLM.** **Advantage:** 6–9× aggregate throughput at 50–64 concurrent users (continuous batching + PagedAttention) when multi-user load justifies a GPU. It's the *serving endpoint the Backend trait points at*, a config swap — keep **Ollama as the local default**.
- **Defer / skip:** CrewAI (overlaps native workflows — adopt only if role-crews prove their worth), LangGraph (most overlap with CO-504/506 — mine its **checkpoint-per-stage + interrupt-for-approval** *ideas* into CO-506 rather than run a second engine), AutoGen (maintenance mode — skip).

### 3.4 Vectorizers + vector store (CO-517)

- **AS IS:** none. No embeddings, no semantic search, no sovereign representation for sacred low-resource corpora.
- **Constraints that decide it:** SQLite is canonical (a second daemon is a tax); sacred corpora (Mbyá "Arandu", Yoruba/Ogunté) must **never leave the box**; Guarani/Yoruba are genuinely low-resource (multilingual coverage dominates English benchmarks).
- **Recommended store — sqlite-vec.** **Advantage:** it lives *inside the canonical store* — vectors become rows next to `entries`/`entry_relations` in the same `data.db`: **one file, one backup, one transaction boundary, one source of truth**, and the sovereignty gate is satisfied for free. Actively maintained successor to the deprecated sqlite-vss. Limitation: brute-force (no ANN) → fine to ~100K–500K vectors *per universe*, which suits CO's many-small-universes model; reserve **LanceDB** as a per-universe ANN accelerator only when a universe outgrows brute-force. Avoid Qdrant/pgvector/FAISS (daemon / re-platform / no-persistence respectively).
- **Recommended embedders (two-tier, both fully local):**
  - **BGE-M3** for sacred + low-resource + quality-critical multilingual. **Advantage:** the only widely-available local model with *measured* low-resource results on these languages (beats mE5-large and LaBSE on MIRACL Yorùbá), and it is **hybrid-native** — emits dense **and** learned-sparse from one pass, mapping straight onto the trait's neural+statistical split (the sparse signal is a real safety net where dense is shaky).
  - **nomic-embed-text-v2-moe @ 256-dim Matryoshka** as the fast default for PT/general/public content. **Advantage:** MoE (305M active), Apache-licensed, first-class `ollama pull`, and 256-dim truncation = **3× smaller blobs / 3× faster brute-force scan** in sqlite-vec. (E5-large = drop-in fallback; **avoid mxbai / nomic-v1** — English-only, near-zero cross-lingual.)
- **Non-neural, first-class:** **BM25 via SQLite FTS5** as the always-on lexical channel (free, in-store, robust on languages no model covers) + **PPMI + truncated-SVD** as the **model-free sovereign representation** for sacred corpora (the model *is* the corpus — no pretrained bias, deterministic, reproducible). This is the defensible answer to "we don't trust any neural model on this language."

### 3.5 Frontend framework decision

- **AS IS:** hand-rolled vanilla-TS SPA.
- **Recommendation: incremental, not a rewrite (now).** Adopt **Web Awesome custom elements inside the existing SPA** (§3.1) to get a component layer, a11y, theming and dark mode **without a framework migration**. Treat a SvelteKit migration as a *separate, deliberate* future decision; if/when taken, the pick is **shadcn-svelte** (source ownership + Bits UI a11y, Svelte-5 native). Rationale in §5.

### 3.6 Deploy / hosting

- **AS IS:** Fly-centric (prod-direct to `co-artelonga` in `gru`), content and code intermingled.
- **TO BE — self-host-first, Fly as one target not the only one.** **Advantage:** the whole recommended stack (sqlite-vec, FTS5, BGE-M3/nomic via Ollama, PPMI, LlamaIndex/DSPy/RAGAS local services) runs **entirely on one box with no network call**, so the same artifact runs on a laptop, a self-hosted server, or Fly. The security kit (pf firewall, kill-switch, exposure-scan e2e guard) + **CO-497 data-residency invariant** make self-host the *default posture*. Fly remains the canonical *public* deploy; GPU serving (vLLM/L40S) is an optional add-on behind the Backend trait. (Fly GPUs deprecated after 2026-07-31 → Modal/RunPod/Lambda or the CPU `gru` scale-to-zero variant for the LLM path.)

### 3.7 comunicacao / meaning-topology (CO-528, CO-529)

- **AS IS:** `comunicacao` is a content universe in its own repo (`artelonga/comunicacao`, markdown-canonical: `languages/`·`concepts/`·`corpus/`), but it is **consumed only by yggdrasil** — the topology *logic* lives in `yggdrasil-core/src/comunicacao/` and reads the lexicon via `COMUNICACAO_DIR` (the `../comunicacao` sibling locally; a baked seed → `/data` on Fly). The `mbya_lexicon.db` corpus is local-first and un-shipped; the topology is yggdrasil-locked; PR #140 runs only on `:8175`. The deploy blocker is exactly this sibling-dir / un-shipped-corpus coupling.
- **TO BE — comunicacao stays the canonical content universe; its *topology* becomes a composable tool + a portable artifact.** **Advantage:** decouples the topology from yggdrasil-core into a CO-503 tool that *any* surface (yggdrasil/topologia, co agents, the bot) calls via the canonical contract — no sibling-dir coupling — and the portable artifact is what ships to prod.
  - **Build:** `build-topologia-data.sh` compiles `comunicacao` lexicon + `mbya_lexicon.db` → `data/topologia.json` (gloss/def/examples/verses/co-occ per term). This artifact, not the sibling dirs, is what deploys — **unblocking the topologia (PR #140) prod data-sourcing**.
  - **Tool:** a `topology` tool (manifest + service) over the artifact, backed by the **CO-517 Vectorizer** — neural (nomic via Ollama) *or* statistical (PPMI/co-occurrence); swap is a flag. (See the runnable sample at `examples/integrations/topology/`.)
  - **Promotion:** `comunicacao` → a **subscribable co universe** (not yggdrasil-internal only); content stays canonical markdown in its own repo, the write-back contribution flow is preserved (CO-529).
  - **Sovereignty gate (non-negotiable):** for sacred corpora (Ayvu Rapytã / Ifá Odù) the artifact is **custodian-gated, never auto-shipped** — CARE principles govern access, not a build script.

### 3.8 External tool add-ons & techniques — the composed estate

> **Governing lens — "ponytail" (`DietrichGebert/ponytail`): the most efficient code is the
> one you didn't write.** A "full refactor using all of these tools" is **not** cramming
> them into the binary — it is making them all **composable** and wiring only the high-value
> few. The full estate-level registry (every candidate, its mode + verdict) lives in
> [`ArteLonga/docs/TOOL-REVIEW.md`](../../../ArteLonga/docs/TOOL-REVIEW.md); this subsection
> is the CO-side incorporation.

Every external tool enters by one of six **integration modes**, ordered by code cost —
**almost none add binary code:**

| Mode | How it enters | Tools (examples) |
|---|---|---|
| **`co503-addon`** | one manifest row (`tools.d/*.yaml`), local service/CLI, **NOT MCP** | LlamaIndex, RAGAS, voicebox, last30days, daily_stock_analysis (freeze), R/RStudio, OpenMontage |
| **`technique`** | absorb the *method*, no new dep | headroom (context-compression), FTS5/BM25, PPMI+SVD, DSPy (also addon), deer-flow/LangGraph *ideas*, vLLM (config swap) |
| **`skill`** | `.claude/skills` layer (dev agent, not product) | last30days, mattpocock/skills |
| **`frontend-dep`** | co-web / yggdrasil front only, never the core | Web Awesome (Shoelace v3), PixiJS v8, Three.js |
| **`ours-not-import`** | build our own; the external is a concept | codebase-memory-mcp → universe-atlas + CO-74 (**no MCP**) |
| **`freeze` / `reference`** | catalogued, not wired | daily_stock_analysis (trading dormant), odysseus, CrewAI/LangGraph/AutoGen, full game engines |

The discipline: **manifest > new dep > new binary code**; **absorb a technique > import a
framework**; **reuse a primitive (agent/tool/vectorizer/workflow) > add a fourth**. The
five runnable adapters in [`examples/integrations/`](../../examples/integrations/) are the
reference proof that any OSS becomes a CO tool by manifest with **zero recompile**. "Using
all of these" means **all are available as add-ons** — only the high-value few are wired
(see §4 phase notes and the TOOL-REVIEW shortlist).

---

## 4. Refactor plan — sequenced phases mapping CO-503 … CO-534

Discipline applied throughout:
- **Additive vs invasive** is called per item.
- **Manifest add-on vs real stack change** is called per item (manifest = no recompile, external OSS via CO-503; stack change = Rust/frontend work).
- **One-runner discipline:** workflows and co-auto converge on a single worktree-isolated substrate (CO-516) — no second orchestration engine.
- **Contract-test tier (CO-520) is the safety net** and is sequenced *early* so every later boundary is verified.

### Phase 0 — Safety net & isolation (do first; unblocks everything)
| Item | Nature | Note |
|------|--------|------|
| **CO-519** flaky-test fix | additive | stop the bleeding before adding surface. |
| **CO-520** contract-test tier | **invasive (foundational)** | closes the lib-vs-integration gap; gate for all later phases. |
| **CO-516** co-auto worktree isolation | invasive | one-runner substrate; fixes duplicate-agents-in-one-worktree. |

### Phase 1 — Generalist core (de-tenant the open repo)
| Item | Nature | Note |
|------|--------|------|
| **CO-509** de-embed quilombo | **invasive** | remove `co-web/src/universes/quilombo/`; private repo = data. |
| **CO-510** scrub private-repo refs | invasive | finish the de-tenanting; DB-driven mappings, no hardcoding. |

*Depends on Phase 0 contract tests to prove the seam holds.*

### Phase 2 — Tool contract (the plug-in seam)
| Item | Nature | Note |
|------|--------|------|
| **CO-503** tool contract + adapter | **built (PR #324)** | merge first; everything below registers through it. |
| **CO-521** extract deterministic tools | additive | reference deterministic adapters out of the binary. |
| **CO-523** external-tool adapter | additive | the manifest path for any external OSS (NOT MCP). |

### Phase 3 — Agent runtime promoted into co
| Item | Nature | Note |
|------|--------|------|
| **CO-504** agent runtime (`Backend` trait) | **invasive** | the loop moves into `co`; Ollama default, Claude/OpenAI-compatible; vLLM is a config swap behind the trait. |
| **CO-505** capability packs | additive | personas/`agents/*.md` become packs. |
| **CO-507** WhatsApp as thin client | invasive (external) | bot calls the in-co runtime. |
| **CO-508** bot → "agente" rename | additive | terminology + client framing. |
| **CO-532** context-compression in runtime | **technique (absorbed)** | absorb the **headroom** method into the agent's context window — fewer tokens, same task; no new dep. |

*Depends on Phase 2 (tools) — the runtime calls tools.*

### Phase 4 — Vectorizers & retrieval
| Item | Nature | Note |
|------|--------|------|
| **CO-517** Vectorizer trait + sqlite-vec + FTS5/BM25 + nomic/BGE-M3 + PPMI | **invasive (new subsystem)** | the representation layer; in-store, sovereign. FTS5/PPMI are absorbed techniques (no dep). |
| **CO-525** production-agentic-RAG add-on | **manifest add-on** | LlamaIndex(rag) + DSPy(prompt-compile) + RAGAS(eval), local services. |
| **CO-534** wire LlamaIndex/DSPy/RAGAS | **manifest add-on** | land CO-525's three add-ons in order (ingest → prompt-compile → eval-gate) via `tools.d/`. |
| **CO-533** last30days research add-on | **manifest add-on + skill** | research/digest as a CO-503 tool *and* a `.claude` skill; feeds leads/digests, no product code. |

*Depends on Phase 3 (agents use vectorizers) and Phase 2 (RAG tools register via manifest).*

### Phase 5 — Composition
| Item | Nature | Note |
|------|--------|------|
| **CO-506** workflows compose agents/tools/vectorizers | **invasive** | on the Phase 0 one-runner substrate; mine LangGraph **and deer-flow** checkpoint/HITL *ideas* (reference, not a second engine). |
| **CO-522 / CO-531** voice tool (voicebox) | manifest add-on | predictive tool — **voicebox** STT/TTS as a thin client of the runtime; OpenMontage (video) catalogued for later. |
| **CO-524** R/RStudio-web REPL (session/kernel tool) | manifest add-on | session/kernel tool via CO-503 (sample `r-stats/`). |

### Phase 6 — Surfaces (parallelizable once core is stable)
| Item | Nature | Note |
|------|--------|------|
| **CO-526** UI component layer (Web Awesome) | **stack change (frontend), incremental** | drop custom elements into existing SPA; no framework rewrite. |
| **CO-527** web game engine (PixiJS v8 → Three.js) | stack change (frontend), additive | render-only client; game-core unchanged. |

### Dependency order (one line)
`0 (safety+isolation) → 1 (de-tenant) → 2 (tool contract) → 3 (agent runtime + headroom compression) → 4 (vectors/RAG + LlamaIndex/DSPy/RAGAS + last30days) → 5 (workflows + voicebox/R) → 6 (UI/game, parallel)`.

> **Where the rest of the estate's tools land** (full registry + verdicts in
> [`ArteLonga/docs/TOOL-REVIEW.md`](../../../ArteLonga/docs/TOOL-REVIEW.md)): **voicebox**→P5
> (CO-531), **headroom** context-compression→P3 (CO-532), **last30days**→P4 (CO-533),
> **LlamaIndex/DSPy/RAGAS**→P4 (CO-534, lands CO-525). Catalogued / not wired now:
> daily_stock_analysis (`freeze` — trading dormant), OpenMontage (`co503-addon`, later),
> codebase-memory-mcp (`ours-not-import` — it's MCP; the concept is universe-atlas + CO-74),
> odysseus + mattpocock/skills (`reference`/`skill`), CrewAI/LangGraph/AutoGen and full game
> engines (`reference` — one-runner / render-only discipline). Net: ~5 wired (all
> manifest/front-dep), ~6 absorbed as techniques, the rest catalogued — **composition, not
> binary growth.**

---

## 5. Risks + rewrite-vs-refactor call per subsystem

| Subsystem | Call | Rationale / risk |
|-----------|------|------------------|
| **Frontend (UI)** | **REFACTOR (incremental web components), not rewrite** | A SvelteKit rewrite is high-cost, high-risk, and pulls in Tailwind. Web Awesome custom elements give a11y/theming/dark-mode *inside the current SPA today*. Risk if rewriting: long migration freeze, regression across every bespoke surface. Defer the SvelteKit/shadcn-svelte rewrite to a deliberate, separately-funded decision. |
| **Game client** | **REFACTOR (add render-only client), keep game-core** | Risk of a full engine: demoting/rewriting the authoritative Rust brain. PixiJS/Three keep game-core as the single source of truth. Risk to watch: Godot-native and web client drift — keep Godot optional. |
| **Tenant embedding** | **REWRITE the boundary (de-embed)** | Genuine invasive change; mitigated by the Phase 0 contract-test tier proving the seam before/after. Risk: hidden coupling in quilombo code — contract tests + DB-driven mappings de-risk. |
| **Agent runtime** | **REWRITE location (Python bot → in-co Rust runtime)** | The brain belongs in the product. Risk: feature parity with the existing bot loop; mitigate by keeping the bot as a thin client (CO-507) so behavior is observable side-by-side during cutover. |
| **Tools** | **REFACTOR (already done in CO-503)** | Low risk; additive. Main risk is security of external manifest tools — they run local, bounded by CO-497, never MCP. |
| **Vectors** | **NEW subsystem (additive)** | Risk: sqlite-vec brute-force ceiling (~100K–500K/universe) — mitigated by 256-dim Matryoshka + FTS5 pre-filter + LanceDB escape hatch. Risk: trusting a 100-language model on sacred corpora — mitigated by PPMI/SVD model-free path. |
| **Agentic add-ons** | **MANIFEST add-ons (no stack change)** | Lowest risk; purely additive, local-first. Risk: orchestration sprawl — enforced away by the one-runner discipline (don't adopt CrewAI/LangGraph as second engines). |
| **Deploy** | **REFACTOR posture (self-host-first), keep Fly** | Risk: GPU path (Fly GPU deprecation 2026-07-31) — mitigate with Modal/RunPod/Lambda or CPU scale-to-zero behind the Backend trait. |

**Top architectural risk overall:** doing Phases 1/3/4 (invasive) *before* the Phase 0 contract-test tier — the StaaS wave proved lib tests pass while integration boundaries break. Phase 0 is non-negotiably first.

---

## 6. Decisions needed from the owner

1. **Frontend path — confirm incremental.** Adopt **Web Awesome inside the current vanilla-TS SPA now** (recommended), and treat a **SvelteKit + shadcn-svelte** migration as a *separate future decision* — not now? (Recommendation: yes, incremental now.)
2. **Web Awesome Pro?** Free core is sufficient to start; approve Pro only if the theme builder / Figma kit is wanted. Budget call.
3. **3D north-star engine — Three.js vs Babylon.js.** Recommendation **Three.js** (render-only, engine-neutral, keeps game-core authoritative). Choose Babylon only if built-in XR/physics/editor tooling is a hard requirement (accept heavier/more opinionated). Decide now or defer to CO-527.
4. **Godot's role.** Confirm Godot stays an *optional native thick client* and the **web target is Pixi/Three, not Godot→HTML5** (recommended), given the 25 MB host limit + WASM ceiling + no extra game-core reuse.
5. **GPU serving trigger.** Approve the policy: **Ollama-local default**, reach for **vLLM** (config swap behind the Backend trait) only when multi-user concurrency justifies the GPU + ops cost — and pick the post-Fly-GPU provider (Modal/RunPod/Lambda vs CPU scale-to-zero).
6. **De-embed quilombo cutover (CO-509/510).** Approve removing the embedded tenant from the open repo and moving it to data/private-repo, gated by the contract-test tier — and the timing relative to the next public-repo milestone.
7. **External add-on order (CO-525).** Confirm wiring **LlamaIndex → DSPy → RAGAS** first (and *deferring* CrewAI, *mining-not-adopting* LangGraph, *skipping* AutoGen).
