#!/usr/bin/env python3
"""Bulk-upload .md files from local project folders into CO universes via Vault API.

Usage:
    python3 scripts/bulk-upload.py [base-url]

Defaults to UAT (co-artelonga-uat.fly.dev). Reads session cookie from /tmp/c.txt.

Auth — get a session cookie first:
    curl -sc /tmp/c.txt -X POST <base>/api/v1/auth/uat-login \\
      -H 'Content-Type: application/json' \\
      -d '{"email":"yuri@uat.local","password":"uat"}'
"""
import os, sys, json, http.cookiejar, urllib.parse, urllib.request, time

BASE = sys.argv[1] if len(sys.argv) > 1 else "https://co-artelonga-uat.fly.dev"
COOKIES_PATH = "/tmp/c.txt"
EXCLUDE_DIRS = {".git", "node_modules", "target", "build", "dist", ".next", ".svelte-kit", ".cache", ".vercel"}

JOBS = [
    ("artelonga",         "/Users/artelonga/projects/ArteLonga"),
    ("quilomboaraucaria", "/Users/artelonga/projects/quilomboaraucaria"),
    ("rfq",               "/Users/artelonga/projects/rfq-gateway"),
]

# curl writes "#HttpOnly_" prefix that MozillaCookieJar won't parse — read manually
session = None
with open(COOKIES_PATH) as fh:
    for line in fh:
        if "\tsession\t" in line:
            session = line.strip().split("\t")[-1]
            break
if not session:
    sys.exit("No session cookie in /tmp/c.txt — run curl login first")

opener = urllib.request.build_opener()

def put_file(slug, rel_path, body):
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel_path.split(os.sep))
    url = f"{BASE}/api/v1/universes/{slug}/vault/{encoded}"
    req = urllib.request.Request(
        url, method="PUT",
        data=body.encode("utf-8"),
        headers={"Content-Type": "text/markdown", "Cookie": f"session={session}"},
    )
    try:
        resp = opener.open(req, timeout=30)
        return resp.status, None
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:200]
    except Exception as e:
        return 0, str(e)[:200]

def walk_md(root):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS and not d.startswith(".")]
        for f in filenames:
            if f.endswith(".md"):
                full = os.path.join(dirpath, f)
                rel = os.path.relpath(full, root)
                yield full, rel

for slug, root in JOBS:
    if not os.path.isdir(root):
        print(f"[{slug}] SKIP: {root} not found")
        continue
    files = list(walk_md(root))
    print(f"\n[{slug}] {len(files)} files from {root}")
    ok = fail = 0
    for full, rel in files:
        try:
            with open(full, "r", encoding="utf-8", errors="replace") as fh:
                body = fh.read()
        except Exception as e:
            print(f"  READ-ERR {rel}: {e}"); fail += 1; continue
        code, err = put_file(slug, rel, body)
        if code in (200, 201):
            ok += 1
        else:
            fail += 1
            print(f"  HTTP {code} {rel}  {err or ''}")
    print(f"[{slug}] done: {ok} ok, {fail} fail")
