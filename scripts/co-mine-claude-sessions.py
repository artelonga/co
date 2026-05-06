#!/usr/bin/env python3
"""
co-mine-claude-sessions — import Claude Code session transcripts as CO entries.

Walks ~/.claude/projects/*/ for .jsonl session files, renders each as
markdown with title + per-turn blocks, and PUTs them into a target CO
universe at `sessions/<project-dir>/<session-id-short>.md`.

Idempotent: vault PUT is upsert. Re-runs refresh content; the cap on
sessions imported is the total under ~/.claude/projects/.

Auth: long-lived API token from macOS Keychain
      (service=co-sync-token, account=prod).

Usage:
    python3 scripts/co-mine-claude-sessions.py [TARGET_UNIVERSE] [--limit N]

Defaults: TARGET_UNIVERSE=co (the platform's dogfood universe).

Mempalace inspiration: equivalent of `mempalace mine ~/.claude/projects --mode convos`,
but writing into CO's vault instead of mempalace's local backend. CO becomes
the durable substrate; future mempalace runs can read from CO via a backend
shim (planned interop).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

BASE = "https://co.artelonga.com.br"
SESSIONS_DIR = Path.home() / ".claude" / "projects"
DEFAULT_TARGET_UNIVERSE = "co"


def get_token() -> str:
    return subprocess.check_output(
        ["security", "find-generic-password", "-a", "prod", "-s", "co-sync-token", "-w"]
    ).decode().strip()


def extract_content(message: dict) -> str:
    """Pull text content from a message (string or list of blocks)."""
    content = message.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for c in content:
            if isinstance(c, dict):
                if c.get("type") == "text":
                    parts.append(c.get("text", ""))
                # Skip tool_use / tool_result blocks for compactness.
                # They're recoverable from the raw .jsonl if needed.
        return "\n".join(p for p in parts if p)
    return ""


def parse_session(jsonl_path: Path) -> dict | None:
    """Parse a .jsonl session. Returns metadata + rendered markdown,
    or None if the session has no user/assistant content (boilerplate-only)."""
    title = None
    started = None
    ended = None
    user_count = 0
    assistant_count = 0
    blocks: list[tuple[str, str | None, str]] = []

    with open(jsonl_path) as f:
        for raw in f:
            try:
                d = json.loads(raw)
            except json.JSONDecodeError:
                continue
            t = d.get("type")
            ts = d.get("timestamp")
            if ts:
                if started is None:
                    started = ts
                ended = ts
            if t == "custom-title":
                title = d.get("customTitle")
            elif t == "user":
                user_count += 1
                content = extract_content(d.get("message", {}))
                if content.strip():
                    blocks.append(("user", ts, content))
            elif t == "assistant":
                assistant_count += 1
                content = extract_content(d.get("message", {}))
                if content.strip():
                    blocks.append(("assistant", ts, content))

    if user_count == 0 and assistant_count == 0:
        return None

    if not title:
        first_user = next((c for r, _, c in blocks if r == "user"), "")
        title = first_user.split("\n")[0][:80] if first_user else jsonl_path.stem[:8]

    md = render_markdown(title, blocks)
    return {
        "title": title,
        "session_id": jsonl_path.stem,
        "project": jsonl_path.parent.name,
        "user_messages": user_count,
        "assistant_messages": assistant_count,
        "started_at": started,
        "ended_at": ended,
        "body": md,
    }


def render_markdown(title: str, blocks: list[tuple[str, str | None, str]]) -> str:
    out = [f"# {title}\n"]
    for role, ts, content in blocks:
        ts_short = (ts or "")[:19]
        out.append(f"\n## {role.title()} · {ts_short}\n\n{content}\n")
    return "".join(out)


def put_entry(token: str, slug: str, path: str, frontmatter: dict, body: str) -> int:
    """PUT a vault entry with markdown-frontmatter wrapper.
    Returns the HTTP status code."""
    fm_yaml = yaml_dump(frontmatter)
    full = f"---\n{fm_yaml}---\n\n{body}"
    url = f"{BASE}/api/v1/universes/{slug}/vault/{urllib.parse.quote(path)}"
    req = urllib.request.Request(
        url,
        data=full.encode("utf-8"),
        method="PUT",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "text/plain; charset=utf-8",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


def yaml_dump(d: dict) -> str:
    """Minimal YAML emitter for flat scalar dicts. Quotes strings."""
    lines = []
    for k, v in d.items():
        if v is None:
            lines.append(f"{k}: ")
        elif isinstance(v, bool):
            lines.append(f"{k}: {'true' if v else 'false'}")
        elif isinstance(v, (int, float)):
            lines.append(f"{k}: {v}")
        else:
            s = str(v).replace("\\", "\\\\").replace('"', '\\"').replace("\n", " ")
            lines.append(f'{k}: "{s}"')
    return "\n".join(lines) + "\n"


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    p.add_argument("universe", nargs="?", default=DEFAULT_TARGET_UNIVERSE,
                   help=f"target universe slug (default: {DEFAULT_TARGET_UNIVERSE})")
    p.add_argument("--limit", type=int, default=0,
                   help="cap on the number of sessions to import (0 = no cap)")
    p.add_argument("--project", default="",
                   help="only import sessions whose project-dir name contains this substring")
    p.add_argument("--dry-run", action="store_true", help="parse but don't PUT")
    args = p.parse_args()

    if not SESSIONS_DIR.is_dir():
        print(f"ERROR: {SESSIONS_DIR} does not exist — Claude Code sessions not available")
        return 1

    sessions: list[Path] = []
    for proj_dir in sorted(SESSIONS_DIR.iterdir()):
        if not proj_dir.is_dir():
            continue
        if args.project and args.project not in proj_dir.name:
            continue
        sessions.extend(sorted(proj_dir.glob("*.jsonl")))

    if args.limit:
        sessions = sessions[: args.limit]

    print(f"Found {len(sessions)} session(s)")
    print(f"Target: {args.universe}{' [DRY RUN]' if args.dry_run else ''}")

    if args.dry_run:
        # Parse all but don't write — useful for previewing volume.
        ok = 0
        for jsonl in sessions:
            r = parse_session(jsonl)
            if r:
                ok += 1
        print(f"\nDry run: {ok}/{len(sessions)} would be imported (rest are boilerplate-only)")
        return 0

    token = get_token()
    ok = fail = skipped = 0
    for i, jsonl in enumerate(sessions, 1):
        meta = parse_session(jsonl)
        if meta is None:
            skipped += 1
            continue
        proj_short = meta["project"].lstrip("-").replace("--", "-")[:60]
        sess_short = meta["session_id"][:8]
        path = f"sessions/{proj_short}/{sess_short}.md"
        fm = {
            "type": "claude-session",
            "title": meta["title"][:120],
            "session_id": meta["session_id"],
            "project": meta["project"],
            "user_messages": meta["user_messages"],
            "assistant_messages": meta["assistant_messages"],
            "started_at": meta["started_at"],
            "ended_at": meta["ended_at"],
        }
        status = put_entry(token, args.universe, path, fm, meta["body"])
        if status in (200, 201):
            ok += 1
            if ok % 25 == 0:
                print(f"  imported {ok}/{len(sessions)}…")
        else:
            fail += 1
            if fail <= 5:
                print(f"  FAIL {path} → HTTP {status}")

    print(f"\nDone: {ok} imported, {fail} failed, {skipped} skipped (boilerplate)")
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
