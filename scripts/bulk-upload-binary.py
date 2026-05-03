#!/usr/bin/env python3
"""Bulk-upload .md and binary assets from local project folders into a CO universe.

Two-pass:
  1. Walk tree, POST every binary (image/video/audio/pdf/etc) to /assets, build
     local-path → sha256 map.
  2. Walk tree, PUT every .md to vault — rewriting any markdown image refs
     `![alt](relative/path.jpg)` whose target is in the map to `![alt](sha256:<hex>)`.

Usage:
    python3 scripts/bulk-upload-binary.py <slug> <root-dir> [base-url]

Reads session cookie from /tmp/c.txt — get one first via:
    curl -sc /tmp/c.txt -X POST <base>/api/v1/auth/password-login \\
      -H 'Content-Type: application/json' \\
      -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}'

The cookie file's domain field is matched loosely — use the same fly.dev host
in both the login curl and the [base-url] arg here.
"""
import os, sys, json, re, mimetypes, urllib.parse, urllib.request, time

if len(sys.argv) < 3:
    sys.exit(__doc__)

SLUG    = sys.argv[1]
ROOT    = os.path.abspath(sys.argv[2])
BASE    = sys.argv[3] if len(sys.argv) > 3 else "https://co-artelonga.fly.dev"
COOKIES = "/tmp/c.txt"

EXCLUDE_DIRS = {".git", ".jj", "node_modules", "target", "build", "dist",
                ".next", ".svelte-kit", ".cache", ".vercel", ".claude", ".co"}

BINARY_EXTS = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg",
    ".mp4", ".mov", ".webm", ".avi",
    ".mp3", ".wav", ".ogg", ".m4a",
    ".pdf",
}

# ---------- session ----------
session = None
with open(COOKIES) as fh:
    for line in fh:
        if "\tsession\t" in line:
            session = line.strip().split("\t")[-1]
            break
if not session:
    sys.exit("No session cookie in /tmp/c.txt — run login curl first")

opener = urllib.request.build_opener()

def http(method, url, body=None, content_type="application/octet-stream"):
    req = urllib.request.Request(
        url, method=method,
        data=body if isinstance(body, (bytes, bytearray)) else (body.encode("utf-8") if body else None),
        headers={
            "Content-Type": content_type,
            "Cookie": f"session={session}",
        },
    )
    try:
        resp = opener.open(req, timeout=60)
        return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except Exception as e:
        return 0, str(e).encode()

def upload_binary(path):
    rel = os.path.relpath(path, ROOT)
    encoded_q = urllib.parse.urlencode({"filename": os.path.basename(path)})
    url = f"{BASE}/api/v1/universes/{SLUG}/assets?{encoded_q}"
    mime, _ = mimetypes.guess_type(path)
    mime = mime or "application/octet-stream"
    with open(path, "rb") as fh:
        body = fh.read()
    if len(body) > 50 * 1024 * 1024:
        return None, f"too large ({len(body)} bytes; cap is 50 MB Phase 1)"
    code, payload = http("POST", url, body=body, content_type=mime)
    if code != 200:
        return None, f"HTTP {code}: {payload[:200].decode('utf-8', 'replace')}"
    try:
        j = json.loads(payload)
        return j["sha256"], None
    except Exception as e:
        return None, f"parse: {e}; raw={payload[:120]!r}"

def put_markdown(rel_path, body):
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel_path.split(os.sep))
    url = f"{BASE}/api/v1/universes/{SLUG}/vault/{encoded}"
    code, payload = http("PUT", url, body=body, content_type="text/markdown")
    if code in (200, 201):
        return True, None
    return False, f"HTTP {code}: {payload[:200].decode('utf-8', 'replace')}"

# ---------- walk ----------
binaries = []   # (full_path, rel_path)
markdowns = []  # (full_path, rel_path)
for dirpath, dirnames, filenames in os.walk(ROOT):
    dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS]
    for f in filenames:
        if f.startswith("."):
            continue
        full = os.path.join(dirpath, f)
        rel  = os.path.relpath(full, ROOT)
        ext  = os.path.splitext(f)[1].lower()
        if ext == ".md":
            markdowns.append((full, rel))
        elif ext in BINARY_EXTS:
            binaries.append((full, rel))

print(f"[{SLUG}] {len(binaries)} binaries, {len(markdowns)} markdown files under {ROOT}")
print(f"        target: {BASE}")

# ---------- pass 1: upload binaries, build sha256 map ----------
sha_by_relpath = {}
ok = fail = skipped = 0
for i, (full, rel) in enumerate(binaries, 1):
    sha, err = upload_binary(full)
    if sha:
        sha_by_relpath[rel] = sha
        # also map basename for refs that don't include the parent path
        sha_by_relpath[os.path.basename(rel)] = sha
        ok += 1
    else:
        fail += 1
        print(f"  BIN-FAIL {rel}: {err}")
    if i % 50 == 0 or i == len(binaries):
        print(f"  [bin] {i}/{len(binaries)}  ok={ok} fail={fail}")

print(f"[{SLUG}] binaries: {ok} ok, {fail} fail")

# ---------- pass 2: upload markdown, rewrite refs ----------
img_ref_re = re.compile(r"!\[([^\]]*)\]\(([^)]+)\)")

def rewrite(body, md_dir_rel):
    def sub(m):
        alt, src = m.group(1), m.group(2).strip()
        if src.startswith("http://") or src.startswith("https://") or src.startswith("sha256:"):
            return m.group(0)
        # try resolved path relative to md file
        resolved = os.path.normpath(os.path.join(md_dir_rel, src))
        sha = sha_by_relpath.get(resolved) or sha_by_relpath.get(src) or sha_by_relpath.get(os.path.basename(src))
        if sha:
            return f"![{alt}](sha256:{sha})"
        return m.group(0)
    return img_ref_re.sub(sub, body)

ok = fail = 0
for i, (full, rel) in enumerate(markdowns, 1):
    try:
        with open(full, "r", encoding="utf-8", errors="replace") as fh:
            body = fh.read()
    except Exception as e:
        print(f"  MD-READ-ERR {rel}: {e}"); fail += 1; continue
    body = rewrite(body, os.path.dirname(rel))
    success, err = put_markdown(rel, body)
    if success:
        ok += 1
    else:
        fail += 1
        print(f"  MD-FAIL {rel}: {err}")
    if i % 25 == 0 or i == len(markdowns):
        print(f"  [md] {i}/{len(markdowns)}  ok={ok} fail={fail}")

print(f"[{SLUG}] markdown: {ok} ok, {fail} fail")
print(f"[{SLUG}] DONE")
