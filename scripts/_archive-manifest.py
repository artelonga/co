#!/usr/bin/env python3
"""CO-108: Generate the global manifest.json for a co-archive run.

Reads all sidecar <key>.manifest.json files from the run directory and
produces a combined manifest.json with SHA256 digests for verify support.

Usage: _archive-manifest.py <out_dir> <mode> <date>
"""
import json
import os
import sys
from datetime import datetime, timezone


def main():
    if len(sys.argv) < 4:
        print("usage: _archive-manifest.py <out_dir> <mode> <date>", file=sys.stderr)
        sys.exit(1)

    out_dir = sys.argv[1]
    mode = sys.argv[2]
    date = sys.argv[3]

    archives = []
    for fname in sorted(os.listdir(out_dir)):
        if not fname.endswith(".manifest.json"):
            continue
        path = os.path.join(out_dir, fname)
        try:
            with open(path) as f:
                meta = json.load(f)
            archives.append({
                "universe_key": meta.get("universe_key"),
                "filename": meta.get("filename"),
                "sha256": meta.get("tar_zst_sha256"),
                "entry_count": meta.get("entry_count"),
                "source_repo": meta.get("source_repo"),
                "source_commit": meta.get("source_commit"),
                "archived_at": meta.get("archived_at"),
                "archive_size_bytes": meta.get("archive_size_bytes"),
            })
        except Exception as e:
            print(f"warn: could not read {fname}: {e}", file=sys.stderr)

    global_manifest = {
        "format_version": 1,
        "co_archive_run": True,
        "date": date,
        "mode": mode,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "archive_count": len(archives),
        "archives": archives,
    }

    manifest_path = os.path.join(out_dir, "manifest.json")
    with open(manifest_path, "w") as f:
        json.dump(global_manifest, f, indent=2)
        f.write("\n")

    total_bytes = sum(a.get("archive_size_bytes") or 0 for a in archives)
    total_entries = sum(a.get("entry_count") or 0 for a in archives)
    print(f"[manifest] {len(archives)} archives, {total_entries} entries total, "
          f"{total_bytes // 1024} KB compressed")
    print(f"[manifest] written → {manifest_path}")


if __name__ == "__main__":
    main()
