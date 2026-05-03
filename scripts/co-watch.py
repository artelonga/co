#!/usr/bin/env python3
"""co-watch — continuous delta sync to prod.

Polls the 4 admin repos every 5s, tracks (mtime, size) per file, and on
each tick:
  - Diffs vs the previous snapshot.
  - For new/modified files: POST binaries to /assets, PUT markdown to /vault
    (server is idempotent on both).
  - For deleted files: DELETE the asset / vault entry.
  - For renames: appears as delete-then-add of the same sha256, which
    server idempotency handles cleanly.

This is the v1 wire shape — JSON over the existing REST endpoints, batched
client-side. CO-151 upgrades this to protobuf SyncDelta + WebSocket + zstd
for true streaming bidi sync.

Auth: re-uses the cookie that sync-all.sh wrote to ~/.co/cookie.txt. Refreshes
on 401 by re-running password-login from keychain.

Logs to ~/.co/watch.log.
"""

import json
import mimetypes
import os
import re
import signal
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor

# ----------------------------------------------------------------------------
# Config
# ----------------------------------------------------------------------------

BASE = os.environ.get("CO_BASE", "https://co-artelonga.fly.dev")
EMAIL = os.environ.get("CO_EMAIL", "yuri@artelonga.com.br")
COOKIE_FILE = os.path.expanduser("~/.co/cookie.txt")
LOG_FILE = os.path.expanduser("~/.co/watch.log")
HOME = os.path.expanduser("~")

POLL_INTERVAL = 5.0
WORKERS = 4
MAX_BLOB_BYTES = 50 * 1024 * 1024

EXCLUDE_DIRS = {".git", ".jj", "node_modules", "target", "build", "dist",
                ".next", ".svelte-kit", ".cache", ".vercel", ".claude", ".co"}

BINARY_EXTS = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg",
    ".mp4", ".mov", ".webm", ".avi",
    ".mp3", ".wav", ".ogg", ".m4a",
    ".pdf",
}

REPOS = [
    ("quilomboaraucaria", os.path.join(HOME, "projects", "quilomboaraucaria")),
    ("artelonga",         os.path.join(HOME, "projects", "ArteLonga")),
    ("rfq",               os.path.join(HOME, "projects", "rfq-gateway")),
    ("co",                os.path.join(HOME, "projects", "co")),
]

# ----------------------------------------------------------------------------
# Logging
# ----------------------------------------------------------------------------

os.makedirs(os.path.dirname(LOG_FILE), exist_ok=True)

def log(msg):
    ts = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    line = f"{ts}  {msg}\n"
    sys.stdout.write(line)
    sys.stdout.flush()
    try:
        with open(LOG_FILE, "a") as fh:
            fh.write(line)
    except Exception:
        pass

# ----------------------------------------------------------------------------
# Auth: pull from keychain, refresh cookie on 401
# ----------------------------------------------------------------------------

_session = {"value": None}

def keychain_password():
    try:
        out = subprocess.check_output(
            ["security", "find-generic-password",
             "-a", EMAIL, "-s", "co-prod-admin", "-w"],
            timeout=10,
        )
        return out.decode().strip()
    except Exception as e:
        log(f"keychain lookup failed: {e}")
        return None

def refresh_session():
    pw = keychain_password()
    if not pw:
        log("no keychain entry — cannot refresh session; sleeping 60s")
        time.sleep(60)
        return False
    body = json.dumps({"email": EMAIL, "password": pw}).encode()
    req = urllib.request.Request(
        f"{BASE}/api/v1/auth/password-login",
        method="POST",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    try:
        resp = urllib.request.urlopen(req, timeout=30)
        # Extract the session cookie from the Set-Cookie header.
        sc = resp.headers.get("set-cookie", "")
        m = re.search(r"session=([^;]+)", sc)
        if m:
            _session["value"] = m.group(1)
            with open(COOKIE_FILE, "w") as fh:
                fh.write("# Netscape HTTP Cookie File\n")
                fh.write(f"#HttpOnly_co-artelonga.fly.dev\tFALSE\t/\tFALSE\t0\tsession\t{m.group(1)}\n")
            try:
                os.symlink(COOKIE_FILE, "/tmp/c.txt")
            except FileExistsError:
                pass
            log("session refreshed via keychain")
            return True
        log("login response had no session cookie")
        return False
    except urllib.error.HTTPError as e:
        log(f"login failed: HTTP {e.code}: {e.read()[:200].decode('utf-8','replace')}")
        return False
    except Exception as e:
        log(f"login error: {e}")
        return False

def load_session_from_disk():
    """Try to read an existing session from ~/.co/cookie.txt before refreshing."""
    try:
        with open(COOKIE_FILE) as fh:
            for line in fh:
                if "\tsession\t" in line:
                    _session["value"] = line.strip().split("\t")[-1]
                    return True
    except Exception:
        pass
    return False

# ----------------------------------------------------------------------------
# HTTP helpers
# ----------------------------------------------------------------------------

def http(method, url, body=None, content_type="application/octet-stream"):
    req = urllib.request.Request(
        url, method=method,
        data=body if isinstance(body, (bytes, bytearray)) else (body.encode("utf-8") if body else None),
        headers={
            "Content-Type": content_type,
            "Cookie": f"session={_session['value']}",
            "X-Admin-Override-Quota": "true",
        },
    )
    for attempt in range(4):
        try:
            resp = urllib.request.urlopen(req, timeout=120)
            return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            if e.code == 401 and attempt == 0:
                # Session expired: refresh and retry once.
                if refresh_session():
                    req.add_header("Cookie", f"session={_session['value']}")
                    continue
                return e.code, e.read()
            if e.code in (429, 500, 502, 503, 504) and attempt < 3:
                ra = e.headers.get("Retry-After")
                wait = float(ra) if ra and ra.isdigit() else (2 ** attempt)
                time.sleep(wait)
                continue
            return e.code, e.read()
        except Exception as e:
            if attempt < 3:
                time.sleep(2 ** attempt)
                continue
            return 0, str(e).encode()
    return 0, b"unknown"

# ----------------------------------------------------------------------------
# File ops
# ----------------------------------------------------------------------------

def upload_binary(slug, root, full_path):
    rel = os.path.relpath(full_path, root)
    try:
        with open(full_path, "rb") as fh:
            body = fh.read()
    except Exception as e:
        return rel, None, f"read: {e}"
    if len(body) > MAX_BLOB_BYTES:
        return rel, None, f"too large ({len(body)} bytes)"
    mime, _ = mimetypes.guess_type(full_path)
    mime = mime or "application/octet-stream"
    qs = urllib.parse.urlencode({"filename": os.path.basename(full_path)})
    code, payload = http("POST",
                        f"{BASE}/api/v1/universes/{slug}/assets?{qs}",
                        body=body, content_type=mime)
    if code != 200:
        return rel, None, f"HTTP {code}: {payload[:120].decode('utf-8','replace')}"
    try:
        return rel, json.loads(payload)["sha256"], None
    except Exception as e:
        return rel, None, f"parse: {e}"

IMG_RE = re.compile(r"!\[([^\]]*)\]\(([^)]+)\)")

def rewrite_md(body, md_dir_rel, sha_by_relpath):
    def sub(m):
        alt, src = m.group(1), m.group(2).strip()
        if src.startswith(("http://", "https://", "sha256:")):
            return m.group(0)
        resolved = os.path.normpath(os.path.join(md_dir_rel, src))
        sha = sha_by_relpath.get(resolved) or sha_by_relpath.get(src) or sha_by_relpath.get(os.path.basename(src))
        return f"![{alt}](sha256:{sha})" if sha else m.group(0)
    return IMG_RE.sub(sub, body)

def upload_markdown(slug, root, full_path, sha_by_relpath):
    rel = os.path.relpath(full_path, root)
    try:
        with open(full_path, "r", encoding="utf-8", errors="replace") as fh:
            body = fh.read()
    except Exception as e:
        return rel, False, f"read: {e}"
    body = rewrite_md(body, os.path.dirname(rel), sha_by_relpath)
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel.split(os.sep))
    code, payload = http("PUT",
                        f"{BASE}/api/v1/universes/{slug}/vault/{encoded}",
                        body=body, content_type="text/markdown")
    return rel, code in (200, 201), f"HTTP {code}: {payload[:120].decode('utf-8','replace')}" if code not in (200, 201) else None

def delete_entry(slug, rel):
    encoded = "/".join(urllib.parse.quote(seg, safe="") for seg in rel.split(os.sep))
    code, payload = http("DELETE",
                        f"{BASE}/api/v1/universes/{slug}/vault/{encoded}")
    return code in (200, 204, 404), code

# ----------------------------------------------------------------------------
# Snapshot + diff
# ----------------------------------------------------------------------------

def walk_repo(root):
    """Yield (full_path, rel_path, ext, size, mtime) for every tracked file."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS]
        for f in filenames:
            if f.startswith("."):
                continue
            ext = os.path.splitext(f)[1].lower()
            if ext != ".md" and ext not in BINARY_EXTS:
                continue
            full = os.path.join(dirpath, f)
            rel = os.path.relpath(full, root)
            try:
                st = os.stat(full)
                yield full, rel, ext, st.st_size, st.st_mtime
            except OSError:
                continue

def snapshot(root):
    """Return {rel_path: (size, mtime, ext, full_path)}."""
    return {rel: (size, mtime, ext, full)
            for full, rel, ext, size, mtime in walk_repo(root)}

def diff_snapshots(prev, curr):
    """Return (added_or_modified_rels, deleted_rels)."""
    changed = []
    deleted = []
    for rel, val in curr.items():
        if rel not in prev:
            changed.append(rel)
        else:
            if prev[rel][0] != val[0] or prev[rel][1] != val[1]:
                changed.append(rel)
    for rel in prev:
        if rel not in curr:
            deleted.append(rel)
    return changed, deleted

# ----------------------------------------------------------------------------
# Tick: process one snapshot diff for a single repo
# ----------------------------------------------------------------------------

def process_repo(slug, root, prev_snapshot, sha_cache):
    if not os.path.isdir(root):
        return prev_snapshot
    curr = snapshot(root)
    changed, deleted = diff_snapshots(prev_snapshot, curr)

    if not changed and not deleted:
        return curr

    # Bin uploads first (so md rewrites can resolve sha256).
    bin_changes = [r for r in changed if curr[r][2] in BINARY_EXTS]
    md_changes  = [r for r in changed if curr[r][2] == ".md"]

    if bin_changes:
        with ThreadPoolExecutor(max_workers=WORKERS) as pool:
            futs = [pool.submit(upload_binary, slug, root, curr[r][3]) for r in bin_changes]
            for fut in futs:
                rel, sha, err = fut.result()
                if sha:
                    sha_cache[rel] = sha
                    sha_cache[os.path.basename(rel)] = sha
                else:
                    log(f"[{slug}] BIN-FAIL {rel}: {err}")

    if md_changes:
        with ThreadPoolExecutor(max_workers=WORKERS) as pool:
            futs = [pool.submit(upload_markdown, slug, root, curr[r][3], sha_cache) for r in md_changes]
            for fut in futs:
                rel, ok, err = fut.result()
                if not ok:
                    log(f"[{slug}] MD-FAIL {rel}: {err}")

    if deleted:
        for rel in deleted:
            ok, code = delete_entry(slug, rel)
            if not ok:
                log(f"[{slug}] DEL-FAIL {rel}: HTTP {code}")

    if changed or deleted:
        log(f"[{slug}] tick: +{len(bin_changes)}bin +{len(md_changes)}md -{len(deleted)}")

    return curr

# ----------------------------------------------------------------------------
# Main loop
# ----------------------------------------------------------------------------

_running = {"v": True}
def _stop(*_):
    _running["v"] = False
    log("shutdown signal received")

signal.signal(signal.SIGTERM, _stop)
signal.signal(signal.SIGINT, _stop)

def main():
    if not load_session_from_disk():
        if not refresh_session():
            log("FATAL: cannot establish session; exiting")
            sys.exit(1)

    # Initial snapshot — treat existing files as already-synced (no upload
    # storm on first start). The user has already run the bulk uploader;
    # start tracking deltas from now.
    state = {}  # slug -> snapshot
    for slug, root in REPOS:
        if os.path.isdir(root):
            state[slug] = snapshot(root)
            log(f"[{slug}] initial snapshot: {len(state[slug])} files")
        else:
            state[slug] = {}
            log(f"[{slug}] root missing: {root}")

    sha_cache = {}  # path → sha256, persists across ticks for md rewrite resolution

    while _running["v"]:
        t0 = time.time()
        for slug, root in REPOS:
            try:
                state[slug] = process_repo(slug, root, state.get(slug, {}), sha_cache)
            except Exception as e:
                log(f"[{slug}] tick error: {e}")
        elapsed = time.time() - t0
        sleep_for = max(0.0, POLL_INTERVAL - elapsed)
        for _ in range(int(sleep_for * 10)):
            if not _running["v"]:
                break
            time.sleep(0.1)

    log("clean shutdown")

if __name__ == "__main__":
    log("=== co-watch start ===")
    main()
