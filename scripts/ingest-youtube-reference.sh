#!/usr/bin/env bash
# ingest-youtube-reference — turn a YouTube URL into a CO reference card.
#
# The reusable "yt-summarizer" pipeline (CO-478). Given a YouTube URL, it:
#   1. yt-dlp  → metadata (title, channel, date, tags), full description, top
#                comments, and bestaudio (no ffmpeg post-processing on download).
#   2. whisper → transcription of the audio (model configurable; uses ffmpeg).
#   3. assembles a `reference` card (.md) in the mbya/refs YouTube format, with
#      the description's links lifted into a `connections:` list (the graph seed)
#      and the transcript embedded as searchable text (CO-154 references_fts).
#   4. (optional) a pt-BR summary via Ollama (`--summary-ollama <model>`); the
#      higher-quality curated synthesis is authored separately by an LLM
#      (Claude/Sonnet) from the transcript — see docs/pipelines/youtube-reference.md.
#
# Markdown stays canonical: the card is a plain .md the universe serves/indexes;
# the raw audio is NOT committed (it's an external URL, per the reference model).
#
# Usage:
#   ingest-youtube-reference.sh <youtube-url> <out-dir> [--model small]
#                               [--venv <path>] [--lang en] [--summary-ollama <model>]
# Example:
#   ingest-youtube-reference.sh https://youtu.be/uIk-S2zW41U \
#       ~/projects/agroecologia/referencias --model small
set -uo pipefail

URL="${1:-}"; OUT="${2:-}"; shift 2 2>/dev/null || true
MODEL="small"; VENV=""; LANG="en"; OLLAMA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --model) MODEL="$2"; shift 2;;
    --venv) VENV="$2"; shift 2;;
    --lang) LANG="$2"; shift 2;;
    --summary-ollama) OLLAMA="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
[ -z "$URL" ] || [ -z "$OUT" ] && { echo "usage: $0 <youtube-url> <out-dir> [--model M] [--venv P] [--lang L] [--summary-ollama M]" >&2; exit 2; }

command -v yt-dlp >/dev/null || { echo "yt-dlp not installed (brew install yt-dlp)" >&2; exit 3; }
command -v ffmpeg >/dev/null || { echo "ffmpeg not installed (brew install ffmpeg)" >&2; exit 3; }
WHISPER="whisper"; [ -n "$VENV" ] && WHISPER="$VENV/bin/whisper"
command -v "$WHISPER" >/dev/null 2>&1 || [ -x "$WHISPER" ] || { echo "whisper not found ($WHISPER); pip install openai-whisper" >&2; exit 3; }

VID="$(yt-dlp --no-warnings --print '%(id)s' --skip-download "$URL")"
[ -z "$VID" ] && { echo "could not resolve video id" >&2; exit 4; }
mkdir -p "$OUT"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

echo "→ [$VID] metadata + comments"
yt-dlp --skip-download --write-info-json --write-comments --no-warnings \
  --extractor-args "youtube:max_comments=40,all,0,0" -o "$WORK/%(id)s.%(ext)s" "$URL" >/dev/null 2>&1
echo "→ [$VID] audio"
yt-dlp -f bestaudio --no-warnings -o "$WORK/%(id)s.%(ext)s" "$URL" >/dev/null 2>&1
AUDIO="$(ls "$WORK/$VID".* | grep -vE '\.info\.json$' | head -1)"
echo "→ [$VID] transcribing ($MODEL, $LANG) — this is the slow step"
"$WHISPER" "$AUDIO" --model "$MODEL" --language "$LANG" --output_format txt \
  --output_dir "$WORK" --verbose False >/dev/null 2>&1
TRANSCRIPT="$WORK/$(basename "${AUDIO%.*}").txt"

SUMMARY=""
if [ -n "$OLLAMA" ] && command -v ollama >/dev/null; then
  echo "→ [$VID] pt-BR summary via ollama:$OLLAMA"
  SUMMARY="$(ollama run "$OLLAMA" "Resuma em português (pt-BR), 5 frases, para agroecologistas:\n\n$(head -c 6000 "$TRANSCRIPT")" 2>/dev/null)"
fi

echo "→ [$VID] writing reference card → $OUT/youtube-$VID.md"
INFO="$WORK/$VID.info.json" TRANSCRIPT="$TRANSCRIPT" SUMMARY="$SUMMARY" OUT="$OUT" VID="$VID" python3 - <<'PY'
import json, os, re, datetime
info = json.load(open(os.environ["INFO"]))
tr = open(os.environ["TRANSCRIPT"]).read().strip() if os.path.exists(os.environ["TRANSCRIPT"]) else ""
vid, out, summary = os.environ["VID"], os.environ["OUT"], os.environ["SUMMARY"]
desc = info.get("description","") or ""
links = sorted(set(re.findall(r'https?://[^\s)>\]]+', desc)))
# drop tracking/store links from the connections graph; keep substantive refs
SKIP = ("paypal","patreon","teespring","amzn.to","amazon.","linktr.ee","instagram","twitter","x.com","facebook")
conns = [l for l in links if not any(s in l.lower() for s in SKIP)]
def y(s): return (s or "").replace('"','\\"')
fm = [
 "---","type: reference","medium: video","platform: youtube",
 f'title: "{y(info.get("title"))}"', f'video_id: {vid}',
 f'url: https://www.youtube.com/watch?v={vid}',
 f'channel: "{y(info.get("channel"))}"', f'upload_date: "{info.get("upload_date","")}"',
 f'duration_s: {info.get("duration",0)}', f'language: {info.get("language") or "en"}',
 "tags:",]
for t in ["reference","video","youtube"] + (info.get("tags") or [])[:8]:
    fm.append(f"  - {y(str(t))}")
fm.append("connections:")
for c in conns: fm.append(f"  - {c}")
fm += ["transcription: embedded","---",""]
body = [f'# {info.get("title")}', "",
        f'> Vídeo · {info.get("channel")} · {info.get("upload_date","")} · '
        f'{datetime.timedelta(seconds=info.get("duration",0))} · '
        f'[assistir](https://www.youtube.com/watch?v={vid})', ""]
if summary:
    body += ["## Resumo (pt-BR)", "", summary, ""]
body += ["## Descrição (original)", "", desc.strip(), ""]
if conns:
    body += ["## Referências citadas (conexões)", ""] + [f"- {c}" for c in conns] + [""]
body += ["## Transcrição", "", "```", tr, "```", ""]
open(os.path.join(out, f"youtube-{vid}.md"),"w").write("\n".join(fm)+"\n".join(body)+"\n")
print(f"  card: {len(tr)} transcript chars, {len(conns)} connections")
PY
echo "✓ [$VID] done"
