#!/usr/bin/env python3
"""Migrate /Users/artelonga/projects/comunicacao → CO universe `comunicacao`.

Goal: comunicacao stops being a git repo and becomes a live CO universe.
Every write thereafter flows through CO's Vault PUT → `entry_events`
append-only log (Iceberg-compatible per `co::public/transaction-log.md`).

Usage:
    # 1. Log in to prod and save cookie
    curl -sc /tmp/c.txt -X POST https://co.artelonga.com.br/api/v1/auth/password-login \\
      -H 'Content-Type: application/json' \\
      -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}'

    # 2. Dry-run first
    python3 scripts/migrate-comunicacao.py --dry-run

    # 3. Execute
    python3 scripts/migrate-comunicacao.py

    # Optional: target UAT
    python3 scripts/migrate-comunicacao.py --base https://co-artelonga-uat.fly.dev

Excludes:
    - mbya/         (4619-file mirror — per CO-141 / CO-155, Arandu wires this at runtime)
    - .git/         (vcs metadata; CO IS the audit log now)
    - .obsidian/    (editor metadata)
    - docs/architecture/  (this is the audit subuniverse, registered separately)
"""
from __future__ import annotations
import argparse
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_BASE = "https://co.artelonga.com.br"
DEFAULT_SOURCE = "/Users/artelonga/projects/comunicacao"
DEFAULT_SLUG = "comunicacao"
DEFAULT_COOKIE = "/tmp/c.txt"

EXCLUDE_DIRS = {
    ".git", "mbya", ".obsidian", "node_modules", "target",
    "build", "dist", ".svelte-kit", ".cache", ".vercel",
}
EXCLUDE_PATH_PREFIXES = (
    "docs/architecture",  # the audit subuniverse — registered separately
)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--base", default=DEFAULT_BASE, help=f"CO base URL (default {DEFAULT_BASE})")
    p.add_argument("--source", default=DEFAULT_SOURCE, help=f"Source dir (default {DEFAULT_SOURCE})")
    p.add_argument("--slug", default=DEFAULT_SLUG, help=f"Target universe slug (default {DEFAULT_SLUG})")
    p.add_argument("--cookie", default=DEFAULT_COOKIE, help=f"Path to curl cookies file (default {DEFAULT_COOKIE})")
    p.add_argument("--dry-run", action="store_true", help="List what would be uploaded; don't write")
    return p.parse_args()


def read_session(path: str) -> str:
    if not os.path.isfile(path):
        sys.exit(f"Cookies file not found at {path}. Log in first (see top of script).")
    with open(path) as fh:
        for line in fh:
            if "\tsession\t" in line:
                return line.strip().split("\t")[-1]
    sys.exit(f"No 'session' cookie found in {path}.")


def is_excluded(rel: str) -> bool:
    rel_norm = rel.replace(os.sep, "/")
    if any(part in EXCLUDE_DIRS for part in rel_norm.split("/")):
        return True
    if any(rel_norm.startswith(p) for p in EXCLUDE_PATH_PREFIXES):
        return True
    return False


def walk_md(root: str):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS and not d.startswith(".")]
        for f in filenames:
            if not f.endswith(".md"):
                continue
            full = os.path.join(dirpath, f)
            rel = os.path.relpath(full, root)
            if is_excluded(rel):
                continue
            yield full, rel


def ensure_universe(base: str, slug: str, session: str, dry: bool) -> None:
    """Create the universe row if it doesn't exist. Idempotent."""
    url = f"{base}/api/v1/universes"
    body = (
        f'{{"key":"{slug}","name":"Comunicacao","description":"Cross-language dictionary + concept topology",'
        f'"is_public":false}}'
    ).encode("utf-8")
    if dry:
        print(f"[dry-run] POST {url}  body={body!r}")
        return
    req = urllib.request.Request(
        url, method="POST", data=body,
        headers={"Content-Type": "application/json", "Cookie": f"session={session}"},
    )
    try:
        resp = urllib.request.urlopen(req, timeout=30)
        print(f"[universe] POST {url} → {resp.status}")
    except urllib.error.HTTPError as e:
        # 409 / 422 → already exists, fine. Anything else is fatal.
        body_excerpt = e.read().decode("utf-8", "replace")[:200]
        if e.code in (409, 422):
            print(f"[universe] exists already ({e.code}): {body_excerpt}")
        else:
            sys.exit(f"[universe] POST failed ({e.code}): {body_excerpt}")


def put_file(base: str, slug: str, rel: str, body: str, session: str) -> tuple[int, str | None]:
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel.split(os.sep))
    url = f"{base}/api/v1/universes/{slug}/vault/{encoded}"
    req = urllib.request.Request(
        url, method="PUT",
        data=body.encode("utf-8"),
        headers={"Content-Type": "text/markdown", "Cookie": f"session={session}"},
    )
    try:
        resp = urllib.request.urlopen(req, timeout=30)
        return resp.status, None
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:200]
    except Exception as e:
        return 0, str(e)[:200]


def main() -> int:
    args = parse_args()
    session = read_session(args.cookie) if not args.dry_run else "DRY"
    if not os.path.isdir(args.source):
        sys.exit(f"Source not found: {args.source}")

    files = sorted(list(walk_md(args.source)))
    print(f"[plan] {len(files)} files from {args.source} → {args.base}/api/v1/universes/{args.slug}/vault/")
    if args.dry_run:
        for _, rel in files[:20]:
            print(f"  PUT  {rel}")
        if len(files) > 20:
            print(f"  … and {len(files) - 20} more")
        return 0

    ensure_universe(args.base, args.slug, session, dry=False)
    ok = fail = 0
    for full, rel in files:
        try:
            with open(full, "r", encoding="utf-8", errors="replace") as fh:
                body = fh.read()
        except OSError as e:
            print(f"  READ-ERR {rel}: {e}")
            fail += 1
            continue
        code, err = put_file(args.base, args.slug, rel, body, session)
        if code in (200, 201):
            ok += 1
            if ok % 5 == 0:
                print(f"  … {ok} ok, {fail} fail")
        else:
            fail += 1
            print(f"  HTTP {code} {rel}  {err or ''}")

    print(f"\n[done] {ok} ok, {fail} fail")
    print(f"[verify] curl -s {args.base}/api/v1/universes/{args.slug}/entries | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get(\"total\"))'")
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
