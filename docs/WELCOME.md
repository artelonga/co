# Welcome to CO — the story so far

> You just arrived. This document is told the way a place is shown to a guest:
> first the name on the door, then the rooms, then the history of the house,
> then the neighbors — and at the end, you get the keys.
>
> _PT-BR primary applies to docs; an EN original is offered here because new
> arrivals come from anywhere. Tradução bem-vinda._

---

## 1. The name on the door

**CO** is not an acronym. It is the smallest piece of language that means
*together*. The Latin prefix *co-* is the load-bearing element of an entire
family of words, and CO claims the whole family as its feature list:

| co-word | what it becomes in CO |
|---|---|
| **connect** | wikilinks, typed relations, the graph |
| **collect** | entradas — every markdown file you bring in |
| **communicate** | chat, DMs, notifications, the event bus |
| **construct** | `co construir` — build a static site from a universe |
| **configure** | `_universe.yaml`, modelos, themes — everything is declared, nothing is hardcoded |
| **coordinate** | quadros, tarefas, sprints, the delivery pipeline |
| **collaborate** | proposals, merges, suggest/review, shared universes |
| **cocreate** | anonymous visitors can clone, edit, and submit — creation before identity |
| **coexist** | many universes on one server, many servers in one federation |
| **conscience** | the op log and atividades — the system remembers what was done, and by whom |

Stack those words and you get the project's full name: **Collective
Consciousness**. Not as mysticism — as architecture. A collective consciousness
is exactly what you build when many people's notes, tasks, and conversations
become one queryable, navigable, *shared* graph that no single member holds in
their head.

There is one more word, borrowed from Guaraní, that explains the posture:
**ñandé**, the inclusive *we* — the "we" that includes the person being spoken
to — as opposed to *oré*, the "we" that excludes them. CO is ñandé software.
The platform is free and open-source; what can be charged for is bounded
intelligence *services* on top of it, never the brain itself. You are not the
audience of this system. You are in the "we."

---

## 2. The rooms — CO's abstractions, in arrival order

Everything in CO reduces to a small vocabulary. Domain words are Portuguese
(the project's home is Brazil); technical words stay English.

**Conteúdo (content).** A markdown file with YAML frontmatter. This is the
atom, and it is *canonical*: the files are the truth, everything else —
database, index, site — is derived and disposable. You can leave at any time
with a folder of plain text.

**Entrada (entry).** What a content file becomes once CO indexes it: a node in
the graph, with a type, tags, dates, and relations. The same entrada can appear
as a card on a board, a page in a wiki, an event on a calendar.

**Universo (universe).** A folder of content with a name. That is the whole
definition. Any folder on your disk can become a universe; a git repo behind it
is optional backing, not a requirement. A universe is the unit of ownership,
visibility (privado / público), subscription, versioning, and scaling.

**Sub-universo.** A universe whose `_universe.yaml` declares a `parent:`. This
is how a person can be the hub of their projects (see Miguel, §5), and how a
folder inside a universe can one day be *promoted* into a universe of its own.
The ladder is: **file → folder → universo → sub-universo → federation.** You
climb it only when you need to.

**Quadro (board), jardim (garden), sala (canvas).** Three lenses over the same
entradas. The quadro shows entries with status as a kanban of tarefas; the
jardim shows entries without status as a wiki of notas with backlinks; the sala
(since 3.0) is a free spatial canvas — the same graph, arranged by hand.

**Tarefa / nota.** A tarefa is an entrada with a status; a nota is an entrada
without one. That single distinction is the entire difference between "task
management" and "knowledge management" in CO — which is to say, there isn't one.

**Modelo (template).** A content-type schema declared in YAML. Universes define
their own types; the validator enforces them. No deploy needed to add a type.

**Assinatura (subscription).** Following someone else's universe. Subscriptions
are how content composes across universes without being copied — the seed of
federation.

---

## 3. The layers you may choose — requirements as options

CO's mission, stated plainly: **be an easy wrapper around content management,
where every traditional "requirement" is abstracted into an optional layer the
user chooses.** Most platforms make you accept their storage, their server,
their identity system, and their pricing on day one. CO inverts this. Layer
zero is a folder of markdown; everything above it is opt-in.

```
  Layer 0 — Content      markdown + frontmatter. Always. The only requirement.
  Layer 1 — Storage      SQLite index (per-universe shard); git backing optional;
                         assets content-addressed and optionally encrypted.
  Layer 2 — Serving      none (just files) · static (`co construir` → Quartz site)
                         · dynamic (`co serve` locally, self-host, or hosted).
  Layer 3 — Identity     none (anonymous, up to 100 entradas) · account ·
                         SSO across deployments (ES256 + JWKS) · OAuth.
  Layer 4 — Sync         none · `co push` over the Vault API · real-time
                         WebSocket deltas · federated event bus across servers.
  Layer 5 — Intelligence embeddings, semantic search, knowledge-base ingest —
                         bounded services on a free substrate (ñandé, §1).
```

The scaling story follows from the same shape. Since 1.25.0, every universe is
its own SQLite shard (xxHash fanout, LiteFS replicas). There is no central
table that grows with the user base — **the system scales horizontally because
the universe is the shard**, and users arrive bringing their own. A deployment
with ten universes and a deployment with ten thousand run the same code; the
federation layer (2.43.0) lets separate deployments exchange events instead of
forcing everyone onto one server. Frameworks built on CO inherit this: they
scale *with* their user base, not against it.

---

## 4. A history in five acts — the timeline

CO is young and moved fast. The changelog is the primary source; dates below
are its own.

**Act I — the CLI (0.1.0, 2026-01-02 → 1.0.0, 2026-04-07).**
CO was born as a Rust command-line tool for graph-based content: `co init`,
`co new`, `co show board`. Markdown in, kanban out. Three months of
foundations: entry storage, querying, validation, the first auth flows, and
the decision that would define everything after — files are canonical, the
database is a cache.

**Act II — the multi-tenant turn (1.25.0 → 1.45.0, late April–early May 2026).**
The web server grew up. Per-universe SQLite sharding (CO-77) replaced the
monolithic database; typed relations (CO-74) turned wikilinks into a real
graph; the temporal model (CO-73) gave entradas dates, calendars, and Gantt
views; binary assets arrived, then encryption, then real-time WebSocket delta
sync replacing polling. The permission model collapsed to a single tier — every
authenticated user is a full citizen.

**Act III — versioning, or replacing git (1.47.0 → 1.64.0, early May 2026).**
In four days CO grew states (commits), branches, proposals, and merges — as
*native universe operations*, not a git wrapper. `?as_of=` rewind, reference
editions, a universal CRUD envelope. This is the era when CO stopped being a
tool that lives beside your version control and started becoming the version
control. (House rule ever since: CO replaces git; never bolt git workflows
back on.)

**Act IV — identity and the social fabric (1.73.0 → 2.43.0, mid-May–early June 2026).**
2.0.0 (2026-05-10) completed the identity arc: SSO across deployments, signup
bridges from sister sites, OAuth, recovery channels. Then the social organs:
chat, DMs, moderation, invitations, the notification engine. Then the nervous
system: a universal event-driven bus (CO-380), a live timeline (/agora), audit
trails, and — closing the act — the **federated event bus** (2.43.0), letting
separate CO deployments publish and subscribe to each other.

**Act V — the public door (3.0.0, 2026-06-10 → 3.1.0, 2026-06-11, today).**
3.0.0 was the public launch — "brain on any device": the sala spatial canvas,
mobile-first interaction, a PWA shell that works offline, suggest/review so
strangers can contribute before they have accounts, and rate limits so the
door can stay open. 3.1.0, released today, added the delivery pipeline (status
driven by deploys, not by dragging cards) and a universal knowledge base —
every entrada written anywhere becomes searchable everywhere downstream.

Five months from `0.1.0` to a federated, offline-capable, publicly writable
collective consciousness. The timeline *is* the argument: each act removed a
requirement (a database, a git host, an identity provider, a single server, a
desktop) and turned it into a layer you may choose.

---

## 5. The neighbors — who lives in CO today

CO is not a demo platform; it is inhabited. Every universe below is real and
current, and together they exercise every layer of §3:

- **`co`** — CO's own development universe. The platform manages itself:
  its tasks, sprints, docs, and delivery pipeline are CO entradas on a CO
  quadro. Dogfooding is the oldest resident.
- **`artelonga`** — the public content universe and agency site
  (artelonga.com.br), including the published `yuri/` garden. The original
  "content × form" demonstration: one markdown corpus, many surfaces.
- **`quilomboaraucaria`** — a private community platform for Quilombo
  Araucária, with its own Rust module (processos, permissões). The proof that a
  universe can be an *application*, not just a site.
- **`rfq`** — the RFQ Gateway's trading-API documentation, private. CO as the
  documentation layer of an entirely separate financial system.
- **`comunicacao`** — cross-linguistic meaning topology (it absorbed the
  mbya/Guaraní and topologia universes). Where the ñandé in §1 comes from.
- **`time`** — a universe of nothing but time-stamped events, astronomical and
  telemetric. The temporal model, used as the whole point.
- **`grcsamazonia`** — Escola de Samba Amazônia: began as a *mission folder*
  inside artelonga and was **promoted** to its own universe (board at
  co.artelonga.com.br/grcsamazonia, public garden at its own subdomain). The
  folder→universe ladder of §2, climbed in production.
- **[`miguel`](https://github.com/artelonga/miguel) → `mse`** — the
  **stakeholder ↔ project pattern**. `miguel` is a *person* universe
  (type: person, the hub — `~/projects/miguel`, remote at
  `github.com/artelonga/miguel`); `mse` is a *project* universe declaring
  `parent: miguel` in its `_universe.yaml`. The person owns the hub; the
  project is the public face. Any consultancy, agency, or research group can
  replicate this shape with two folders and two YAML files.

---

## 6. Your first gesture — CRUD, by example

Theory ends here. This is the actual API, demonstrated with a real change:
giving **miguel** a new sub-universe-to-be called **`scholars`** — a folder of
the academics and researchers whose work grounds his projects.

Everything content-shaped goes through the **Vault API**, which speaks plain
files at `/{universe}/vault/{path}` with the full verb set —
`GET · PUT · POST · PATCH · DELETE` (binary-safe up to 50 MB):

```bash
# One-time: get a token (login first, then POST /api/v1/auth/token)
TOKEN="..."
BASE="https://co.artelonga.com.br/api/v1/universes"

# CREATE — PUT a new folder into miguel by writing its index note.
# In CO, a folder begins to exist the moment its first file does.
curl -X PUT "$BASE/miguel/vault/scholars/index.md" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/markdown" \
  --data-binary @- <<'MD'
---
title: "Scholars"
type: page
tags: [pasta, pesquisa]
---
# Scholars
Pasta de acadêmicos e pesquisadores que fundamentam os projetos de Miguel.
MD

# READ — the folder is now in the tree, the note is in the graph
curl -H "Authorization: Bearer $TOKEN" "$BASE/miguel/vault/tree"
curl -H "Authorization: Bearer $TOKEN" "$BASE/miguel/vault/scholars/index.md"

# UPDATE — PATCH edits in place (or PUT to replace wholesale)
curl -X PATCH "$BASE/miguel/vault/scholars/index.md" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/markdown" \
  --data-binary $'\n- Primeira entrada de scholar a caminho.'

# DELETE — and the graph forgets it (the op log does not)
curl -X DELETE "$BASE/miguel/vault/scholars/index.md" \
  -H "Authorization: Bearer $TOKEN"
```

Two notes a newly arrived person should hear:

1. **The PUT above is the whole content-management story.** No migration, no
   schema change, no deploy. A write to a path *is* creation; the index, the
   board, the garden, and the knowledge base all observe it through the event
   bus.
2. **When `scholars/` outgrows folderhood**, it gets promoted exactly the way
   `mse` was: it receives its own `_universe.yaml` with `parent: miguel`, and
   becomes a sub-universe — its own visibility, its own subscriptions, its own
   shard. (Universes themselves are managed at `POST /api/v1/universes` /
   `PUT /api/v1/universes/{slug}`; the parent link lives in the universe's own
   config, where it belongs — with the content, not in someone's database.)

The `scholars` folder is not hypothetical: it exists in the
[miguel](https://github.com/artelonga/miguel) universe as of today, created as
this document was written — on the canonical markdown at `~/projects/miguel`.
(miguel is not on the hosted server yet; `co push` below is the gesture that
puts it there, after which the curl example runs against it verbatim.)

### Bringing a universe with you — two doors

The example above edited a universe already on a server. The other half of the
story is getting a universe *onto* one — or taking one home.

**Door one — add a local folder.** Any folder becomes a universe the moment it
has content. Say you start **`offscholars`**:

```bash
mkdir -p ~/projects/offscholars/content
cat > ~/projects/offscholars/content/index.md <<'MD'
---
title: "Off Scholars"
type: page
---
# Off Scholars
Assuntos que gostaria de abordar: assistir vídeos, escrever textos.
MD

cd ~/projects/offscholars
co push --remote http://127.0.0.1:54321 --token $CO_TOKEN      # your own `co serve`
co push --remote https://co.artelonga.com.br --token $CO_TOKEN  # or the hosted one
```

`co push` creates the universe if it is absent and uploads `content/**/*.md`;
re-running converges (no duplicates). The folder on your disk stays canonical —
the server holds a copy, not the truth.

**Door two — clone from a remote.** A universe with a git remote can be taken
whole. miguel lives at `github.com/artelonga/miguel` (private — the GitHub
login from §6's study case is the key):

```bash
git clone git@github.com:artelonga/miguel.git
```

The clone is the *current* miguel — the calendar (the `content/diario/` daily
notes, e.g. the 2026-06-11 meeting) together with the original entries. From
there the loop closes: edit locally, `co push` to publish.

Git is one door, not a requirement (§3, layer 1): a universe with no remote is
still a universe, and `rsync` or a tarball moves it just as well.

---

## 7. Where to go next

| You want to… | Read / run |
|---|---|
| Run CO in two commands | [`../README.md`](../README.md) — `curl … install.sh \| sh && co serve --open` |
| Understand the components | [`ARCHITECTURE.md`](./ARCHITECTURE.md) |
| Set up from source | [`ONBOARDING.md`](./ONBOARDING.md) |
| Operate a deployment | [`OPERATIONS.md`](./OPERATIONS.md) |
| Understand how work ships | [`delivery-pipeline.md`](./delivery-pipeline.md) — review on localhost → approve → merge, in git and in jj |
| Read the primary source | [`../CHANGELOG.md`](../CHANGELOG.md) — newest at top, every act of §4 in full. *(We will walk through the changelog together next week.)* |
| Try it without installing anything | [artelonga.com.br/co](https://artelonga.com.br/co) — anonymous, up to 100 entradas |

You arrived at a platform whose name is a prefix waiting for your verb.
Connect, collect, construct — *co*-anything. Welcome.
