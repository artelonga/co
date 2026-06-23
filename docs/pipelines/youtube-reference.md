# E2E pipeline: universe CRUD + connections + YouTube-reference ingestion → Quartz

A documented, repeatable pipeline (CO-478) for: **create a universe → add a
`referencias/` folder → ingest YouTube videos as reference cards (transcript +
metadata + connection links) → author a synthesis document → publish a Quartz
static site** (the grcsamazonia/miguel pattern). Markdown stays canonical; the
raw audio/metadata are working artifacts, never served.

Worked example: the **agroecologia** universe (`~/projects/agroecologia`), title
*"Raízes Pensantes"*, built from two videos on plant cognition + distributed
intelligence.

## Prerequisites (one-time)

```bash
brew install yt-dlp ffmpeg                 # download + audio decode
uv venv ~/projects/agroecologia/_ingest/.venv
uv pip install --python <venv>/bin/python openai-whisper   # pulls torch
# redearte (Quartz template) must exist with node_modules — see redearte repo
```

## 1. Create the universe (CRUD)

```yaml
# ~/projects/<u>/_universe.yaml
name: <Name>
schema_version: 1
handle: <u>
type: project
visibility: public-subscribable
content_types:
  - { name: page,      schema: { title: { type: string, required: true } } }
  - { name: note,      schema: { title: { type: string, required: true } } }
  - { name: reference, schema: { title: { type: string, required: true } } }
```

`mkdir -p ~/projects/<u>/{content,referencias,_ingest}`. The folder is the
universe (workspace-scan auto-registers it; no API call needed). The `reference`
content type backs the `referencias/` folder — a candidate to ship in the CO
template so every universe gets it by default.

## 2. Ingest references (the "yt-summarizer")

```bash
co/scripts/ingest-youtube-reference.sh <youtube-url> ~/projects/<u>/referencias \
    --model small --venv ~/projects/<u>/_ingest/.venv --lang en
```

For each URL it: (a) `yt-dlp` → metadata + full description + top-40 comments +
bestaudio; (b) `whisper` → transcript; (c) writes
`referencias/youtube-<id>.md` — a `reference` card in the mbya/refs format with
the transcript embedded (searchable, CO-154 FTS) and the **substantive
description links lifted into `connections:`** (store/tracking links filtered
out). Those connections are the graph seed (CO-74) linking the reference into the
universe and across universes.

Optional pt-BR summary inline: `--summary-ollama qwen2.5-coder:14b`.

## 3. Author the synthesis (the deliverable)

The cards are evidence; the **document** is the product — a quality-constrained
essay with a creative title for a defined audience, grounded in the transcripts +
the cited literature. This is an editorial/LLM step (Claude/Sonnet preferred for
pt-BR over the local coder models), written to `content/index.md`, wikilinking
the reference cards (`[[referencias/youtube-<id>]]`) so connections resolve.

## 4. Publish (Quartz, gated deploy)

```bash
~/projects/<u>/rebuild.sh            # build → public/ (sets pageTitle + baseUrl, restores redearte)
~/projects/<u>/rebuild.sh --deploy   # build + flyctl deploy (needs Fly app + prod approval)
```

Mirrors miguel/grcsamazonia: assemble `content/` (+`referencias/`) → `npx quartz
build` with the shared redearte template → `public/`. The template's
`pageTitle`/`baseUrl` leak between universes, so the script sets this universe's
values for the build and restores redearte afterward.

## Hygiene

- `_ingest/` (audio + `.venv` with torch — hundreds of MB) and `public/` (built)
  are working artifacts: gitignore them; they are not canonical content.
- Whisper transcription is the slow step (CPU, `small` ≈ real-time-ish). Audio is
  never committed — a reference is an external URL, per the reference model.
