"""co-py — attach to a CO universe's DuckDB-readable SQLite from Python."""

import glob
import os
import tempfile

import duckdb


def connect(slug, *, base_url=None, api_token=None, data_dir=None):
    """Open a read-only DuckDB connection to a CO universe's data.db.

    Priority order:
    1. If `data_dir` is given, resolve the 2-level hash path under
       ``{data_dir}/universes/**/{slug}/data.db`` using glob.
    2. Otherwise, download a snapshot from
       ``POST {base_url}/api/v1/universes/{slug}/db-snapshot``
       using `api_token` as Bearer auth and write it to a temp file.

    Returns a ``duckdb.DuckDBPyConnection`` opened read-only.
    Raises ``ConnectionError`` when neither path produces a file.
    """
    db_path = None

    if data_dir is not None:
        pattern = os.path.join(data_dir, "universes", "*", "*", slug, "data.db")
        matches = glob.glob(pattern)
        if matches:
            db_path = matches[0]

    if db_path is None:
        if base_url is None:
            raise ConnectionError(
                f"Universe '{slug}' not found under data_dir={data_dir!r} "
                "and no base_url was given to download a snapshot."
            )
        db_path = _download_snapshot(slug, base_url=base_url, api_token=api_token)

    return duckdb.connect(db_path, read_only=True)


def _download_snapshot(slug, *, base_url, api_token):
    """POST to the db-snapshot endpoint and write the response to a temp file."""
    import urllib.request

    url = f"{base_url.rstrip('/')}/api/v1/universes/{slug}/db-snapshot"
    headers = {"Content-Type": "application/json"}
    if api_token:
        headers["Authorization"] = f"Bearer {api_token}"

    req = urllib.request.Request(url, data=b"{}", headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req) as resp:
            data = resp.read()
    except Exception as exc:
        raise ConnectionError(
            f"Failed to download snapshot for universe '{slug}' from {url}: {exc}"
        ) from exc

    tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
    tmp.write(data)
    tmp.flush()
    tmp.close()
    return tmp.name
