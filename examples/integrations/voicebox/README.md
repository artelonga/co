# voicebox — voice as a predictive HTTP CO tool

**What it demonstrates.** Voice (speech-to-text + text-to-speech) plugs into CO
**unchanged, behind the manifest**, as a CO-503 add-on (**CO-531**, landing the
voice goal of **CO-522**). It is the `predictive` / HTTP shape of the CO-503
contract: the manifest carries a `url`, CO's HTTP invoker (injected from `co-web`,
which has `reqwest`) POSTs the JSON args and reads JSON back. The agent calls
`transcribe` / `synthesize` exactly like any other tool and never knows a
self-hosted voicebox (jamiepine/voicebox) STT/TTS engine runs behind the URL —
same adapter shape as `tools.d/yt-summarizer.example.yaml` and the `rag-service`
sample.

**Local-first, no API bill.** The default path is a **self-hosted** voicebox
backend: audio never leaves the instance and **no cloud STT/TTS is ever called**
(aligns with the CO-497 data-residency invariant). `service.py` is a stdlib-only
stub with canned demo output so a reviewer can run and curl it offline. A real
deployment replaces `transcribe()` / `synthesize()` with the injected/configured
voicebox backend; **the HTTP contract and the CO manifest stay identical.**

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | Voice is handled ad-hoc inside the WhatsApp bot's bridge (`bridge/voice.py`) — a single caller wired by hand, not reachable by the agent runtime or any other surface, and easy to accidentally route through a cloud STT/TTS. |
| **TO-BE** | `voicebox.yaml` registers one `predictive` tool. Any surface (CLI, co-web, WhatsApp) and the agent runtime (CO-504) call `POST /voice` with `action: transcribe \| synthesize` and get `{text}` or `{audio_b64}` back in-loop. Swapping the engine is a service-side change; CO is untouched. Local-first by construction — no API bill. |

## How an agent calls it

Transcribe (STT) — input matches `input_schema`:
```json
{ "action": "transcribe", "audio_b64": "<base64 audio>" }
```
Output:
```json
{ "text": "Olá, isto é uma transcrição de demonstração gerada localmente..." }
```

Synthesize (TTS):
```json
{ "action": "synthesize", "text": "Bom dia!", "voice": "pt-BR" }
```
Output:
```json
{ "audio_b64": "Vk9JQ0VCT1gtU1RVQjo6cHQtQlI6OkJvbSBkaWEh", "note": "stub" }
```

## Try it

```bash
# 1. start the stub (foreground; Ctrl-C to stop)
python3 examples/integrations/voicebox/service.py

# 2. in another shell:
curl -s localhost:9200/health
curl -s -X POST localhost:9200/voice \
  -H 'Content-Type: application/json' \
  -d '{"action":"transcribe","audio_b64":"ZGVtbw=="}' | python3 -m json.tool
curl -s -X POST localhost:9200/voice \
  -H 'Content-Type: application/json' \
  -d '{"action":"synthesize","text":"Bom dia!","voice":"pt-BR"}' | python3 -m json.tool
```
