#!/usr/bin/env python3
"""CO-108: Verify co-archive integrity by re-hashing .tar.zst files.

Reads the global manifest.json, re-computes SHA256 of each archive file,
and compares against the stored digest. Exits 1 on any mismatch.

Usage: _archive-verify.py <verify_dir> <manifest_path>
"""
import hashlib
import json
import os
import sys


def sha256_file(path: str, chunk: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            buf = f.read(chunk)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()


def main():
    if len(sys.argv) < 3:
        print("usage: _archive-verify.py <verify_dir> <manifest_path>", file=sys.stderr)
        sys.exit(1)

    verify_dir = sys.argv[1]
    manifest_path = sys.argv[2]

    with open(manifest_path) as f:
        global_manifest = json.load(f)

    archives = global_manifest.get("archives", [])
    if not archives:
        print("ERROR: no archives listed in manifest", file=sys.stderr)
        sys.exit(1)

    passed = 0
    failed = 0

    for entry in archives:
        filename = entry.get("filename")
        expected_sha256 = entry.get("sha256")
        key = entry.get("universe_key", "?")

        if not filename:
            print(f"  [{key}] SKIP — no filename in manifest entry")
            continue
        if not expected_sha256:
            print(f"  [{key}] SKIP — no sha256 in manifest entry")
            continue

        archive_path = os.path.join(verify_dir, filename)
        if not os.path.exists(archive_path):
            print(f"  [{key}] MISSING — {filename}")
            failed += 1
            continue

        actual = sha256_file(archive_path)
        if actual == expected_sha256:
            size = os.path.getsize(archive_path)
            print(f"  [{key}] OK — {filename} ({size // 1024} KB)")
            passed += 1
        else:
            print(f"  [{key}] FAIL — hash mismatch for {filename}")
            print(f"         expected: {expected_sha256}")
            print(f"         actual:   {actual}")
            failed += 1

    print()
    print(f"Verify result: {passed} OK, {failed} FAILED")

    if failed > 0:
        print("BIT-ROT DETECTED — re-run backup or restore from a clean copy.", file=sys.stderr)
        sys.exit(1)
    else:
        print("All archives intact.")


if __name__ == "__main__":
    main()
