#!/usr/bin/env python3
"""Bulk-upload .md and binary assets from local project folders into a CO universe.

Two-pass:
  1. Walk tree, POST every binary (image/video/audio/pdf/etc) to /assets, build
     local-path → sha256 map. Hashes locally first to skip already-uploaded
     blobs. Runs with ThreadPoolExecutor (8 workers) + retry-on-429/timeout.
  2. Walk tree, PUT every .md to vault — rewriting any markdown image refs
     `![alt](relative/path.jpg)` whose target is in the map to `![alt](sha256:<hex>)`.

Usage:
    python3 scripts/bulk-upload-binary.py <slug> <root-dir> [base-url]

Reads session cookie from /tmp/c.txt — get one first via:
    curl -sc /tmp/c.txt -X POST <base>/api/v1/auth/password-login \\
      -H 'Content-Type: application/json' \\
      -d '{"email":"yuri@artelonga.com.br","password":"<your-password>"}'

Sends `X-Admin-Override-Quota: true` so authenticated callers bypass the
per-min rate cap (CO-145 / 1.37.1). Anonymous requests are still throttled.
"""
import os, sys, json, re, hashlib, mimetypes, urllib.parse, urllib.request, time
from concurrent.futures import ThreadPoolExecutor, as_completed
from threading import Lock

if len(sys.argv) < 3:
    sys.exit(__doc__)

SLUG    = sys.argv[1]
ROOT    = os.path.abspath(sys.argv[2])
BASE    = sys.argv[3] if len(sys.argv) > 3 else "https://co-artelonga.fly.dev"
COOKIES = "/tmp/c.txt"
WORKERS = 8
MAX_RETRIES = 5

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
print_lock = Lock()

def safe_print(*a, **kw):
    with print_lock:
        print(*a, **kw, flush=True)

def http(method, url, body=None, content_type="application/octet-stream", retry=True):
    headers = {
        "Content-Type": content_type,
        "Cookie": f"session={session}",
        "X-Admin-Override-Quota": "true",
    }
    req = urllib.request.Request(
        url, method=method,
        data=body if isinstance(body, (bytes, bytearray)) else (body.encode("utf-8") if body else None),
        headers=headers,
    )
    last_err = None
    for attempt in range(MAX_RETRIES):
        try:
            resp = opener.open(req, timeout=120)
            return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            code = e.code
            payload = e.read()
            if retry and code in (429, 500, 502, 503, 504) and attempt + 1 < MAX_RETRIES:
                # Honor Retry-After if present, else exponential backoff
                ra = e.headers.get("Retry-After")
                wait = float(ra) if ra and ra.isdigit() else (2 ** attempt) + 0.5
                time.sleep(wait)
                last_err = (code, payload)
                continue
            return code, payload
        except Exception as e:
            if retry and attempt + 1 < MAX_RETRIES:
                time.sleep(2 ** attempt + 0.5)
                last_err = (0, str(e).encode())
                continue
            return 0, str(e).encode()
    code, payload = last_err if last_err else (0, b"unknown")
    return code, payload

def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        while chunk := fh.read(1024 * 1024):
            h.update(chunk)
    return h.hexdigest()

def asset_exists(sha):
    """HEAD-equivalent via GET — server short-circuits on If-None-Match=etag.
    Returns True if already in the universe's assets index."""
    url = f"{BASE}/api/v1/universes/{SLUG}/assets/{sha}"
    req = urllib.request.Request(
        url, method="GET",
        headers={"Cookie": f"session={session}", "If-None-Match": f"\"{sha}\""},
    )
    try:
        resp = opener.open(req, timeout=20)
        return resp.status in (200, 304)
    except urllib.error.HTTPError as e:
        return e.code in (200, 304)
    except Exception:
        return False

def upload_binary(path):
    sha = sha256_of(path)
    rel = os.path.relpath(path, ROOT)

    if asset_exists(sha):
        return rel, sha, None, "skipped"

    encoded_q = urllib.parse.urlencode({"filename": os.path.basename(path)})
    url = f"{BASE}/api/v1/universes/{SLUG}/assets?{encoded_q}"
    mime, _ = mimetypes.guess_type(path)
    mime = mime or "application/octet-stream"
    with open(path, "rb") as fh:
        body = fh.read()
    if len(body) > 50 * 1024 * 1024:
        return rel, None, f"too large ({len(body)} bytes; cap is 50 MB Phase 1)", "fail"
    code, payload = http("POST", url, body=body, content_type=mime)
    if code != 200:
        return rel, None, f"HTTP {code}: {payload[:200].decode('utf-8', 'replace')}", "fail"
    try:
        j = json.loads(payload)
        return rel, j["sha256"], None, "uploaded"
    except Exception as e:
        return rel, None, f"parse: {e}; raw={payload[:120]!r}", "fail"

def put_markdown(rel_path, body):
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel_path.split(os.sep))
    url = f"{BASE}/api/v1/universes/{SLUG}/vault/{encoded}"
    code, payload = http("PUT", url, body=body, content_type="text/markdown")
    if code in (200, 201):
        return True, None
    return False, f"HTTP {code}: {payload[:200].decode('utf-8', 'replace')}"

# ---------- walk ----------
binaries = []
markdowns = []
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

safe_print(f"[{SLUG}] {len(binaries)} binaries, {len(markdowns)} markdown files under {ROOT}")
safe_print(f"        target: {BASE}  workers: {WORKERS}")

# ---------- pass 1: upload binaries (parallel) ----------
sha_by_relpath = {}
ok = fail = skipped = 0
counter = {"n": 0}
counter_lock = Lock()
total = len(binaries)
t0 = time.time()

with ThreadPoolExecutor(max_workers=WORKERS) as pool:
    futures = {pool.submit(upload_binary, full): rel for full, rel in binaries}
    for fut in as_completed(futures):
        rel_arg = futures[fut]
        try:
            rel, sha, err, status = fut.result()
        except Exception as e:
            rel, sha, err, status = rel_arg, None, str(e), "fail"
        with counter_lock:
            counter["n"] += 1
            n = counter["n"]
        if sha:
            sha_by_relpath[rel] = sha
            sha_by_relpath[os.path.basename(rel)] = sha
            if status == "skipped":
                skipped += 1
            else:
                ok += 1
        else:
            fail += 1
            safe_print(f"  BIN-FAIL {rel}: {err}")
        if n % 50 == 0 or n == total:
            elapsed = time.time() - t0
            rate = n / elapsed if elapsed else 0
            safe_print(f"  [bin] {n}/{total}  uploaded={ok} skipped={skipped} fail={fail}  ({rate:.1f}/s)")

safe_print(f"[{SLUG}] binaries: {ok} uploaded, {skipped} skipped, {fail} fail in {time.time()-t0:.1f}s")

# ---------- pass 2: upload markdown (parallel) ----------
img_ref_re = re.compile(r"!\[([^\]]*)\]\(([^)]+)\)")

def rewrite(body, md_dir_rel):
    def sub(m):
        alt, src = m.group(1), m.group(2).strip()
        if src.startswith("http://") or src.startswith("https://") or src.startswith("sha256:"):
            return m.group(0)
        resolved = os.path.normpath(os.path.join(md_dir_rel, src))
        sha = sha_by_relpath.get(resolved) or sha_by_relpath.get(src) or sha_by_relpath.get(os.path.basename(src))
        if sha:
            return f"![{alt}](sha256:{sha})"
        return m.group(0)
    return img_ref_re.sub(sub, body)

def md_job(full, rel):
    try:
        with open(full, "r", encoding="utf-8", errors="replace") as fh:
            body = fh.read()
    except Exception as e:
        return rel, False, f"READ-ERR: {e}"
    body = rewrite(body, os.path.dirname(rel))
    success, err = put_markdown(rel, body)
    return rel, success, err

ok = fail = 0
counter["n"] = 0
total = len(markdowns)
t0 = time.time()

with ThreadPoolExecutor(max_workers=WORKERS) as pool:
    futures = {pool.submit(md_job, full, rel): rel for full, rel in markdowns}
    for fut in as_completed(futures):
        rel_arg = futures[fut]
        try:
            rel, success, err = fut.result()
        except Exception as e:
            rel, success, err = rel_arg, False, str(e)
        with counter_lock:
            counter["n"] += 1
            n = counter["n"]
        if success:
            ok += 1
        else:
            fail += 1
            safe_print(f"  MD-FAIL {rel}: {err}")
        if n % 25 == 0 or n == total:
            elapsed = time.time() - t0
            rate = n / elapsed if elapsed else 0
            safe_print(f"  [md] {n}/{total}  ok={ok} fail={fail}  ({rate:.1f}/s)")

safe_print(f"[{SLUG}] markdown: {ok} ok, {fail} fail in {time.time()-t0:.1f}s")
safe_print(f"[{SLUG}] DONE")
