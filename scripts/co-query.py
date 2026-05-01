#!/usr/bin/env python3
"""
co-query: DuckDB queries against the CO Iceberg lakehouse on MinIO.

Shows example queries — time-travel, cross-universe aggregation,
entry type breakdown. Edit freely.

Usage:
  python3 scripts/co-query.py

Requires:
  MinIO running + data exported via co-export.py first.
"""

import os
import duckdb

ENDPOINT    = os.environ.get("S3_ENDPOINT",             "http://localhost:9000")
BUCKET      = os.environ.get("S3_BUCKET",               "co-lakehouse")
KEY_ID      = os.environ.get("AWS_ACCESS_KEY_ID",       "co-admin")
SECRET      = os.environ.get("AWS_SECRET_ACCESS_KEY",   "co-secret-local")
CATALOG_DB  = os.environ.get("ICEBERG_CATALOG_DB",      "data/iceberg-catalog.db")

_endpoint_host = ENDPOINT.replace("https://", "").replace("http://", "")
_use_ssl = ENDPOINT.startswith("https://")

con = duckdb.connect()
con.execute("INSTALL iceberg; LOAD iceberg;")
con.execute("INSTALL httpfs; LOAD httpfs;")
con.execute(f"""
    CREATE SECRET minio (
        TYPE s3,
        ENDPOINT '{_endpoint_host}',
        KEY_ID '{KEY_ID}',
        SECRET '{SECRET}',
        USE_SSL {str(_use_ssl).lower()},
        URL_STYLE path,
        REGION 'us-east-1'
    )
""")

# Locate Iceberg table metadata via the catalog DB
import sqlite3
_cat = sqlite3.connect(CATALOG_DB)

def iceberg_location(namespace: str, table: str) -> str:
    row = _cat.execute(
        "SELECT metadata_location FROM iceberg_tables WHERE catalog_name='co' AND table_namespace=? AND table_name=?",
        (namespace, table),
    ).fetchone()
    if not row:
        raise RuntimeError(f"Table {namespace}.{table} not found in catalog. Run co-export.py first.")
    return row[0]

def scan(namespace: str, table: str) -> duckdb.DuckDBPyRelation:
    loc = iceberg_location(namespace, table)
    return con.sql(f"SELECT * FROM iceberg_scan('{loc}')")


# ── Queries ───────────────────────────────────────────────────────────────────

print("=" * 60)
print("CO Lakehouse — example queries")
print("=" * 60)

print("\n1. Universes")
print("-" * 40)
scan("co", "universes").select("key, name, content_count, is_public, is_template, layout").show()

print("\n2. Entry type breakdown by universe")
print("-" * 40)
entries = scan("co", "entries")
con.register("entries_view", entries)
con.sql("""
    SELECT universe_key, entry_type, COUNT(*) AS n
    FROM entries_view
    GROUP BY universe_key, entry_type
    ORDER BY universe_key, n DESC
""").show()

print("\n3. Tasks by status")
print("-" * 40)
tasks = scan("co", "tasks")
con.register("tasks_view", tasks)
con.sql("""
    SELECT project_key, status, COUNT(*) AS n
    FROM tasks_view
    GROUP BY project_key, status
    ORDER BY project_key, n DESC
""").show()

print("\n4. Most recently updated entries (any universe)")
print("-" * 40)
con.sql("""
    SELECT universe_key, entry_type, title, updated_at
    FROM entries_view
    ORDER BY updated_at DESC
    LIMIT 10
""").show()

print("\n5. Iceberg snapshot history (entries table)")
print("-" * 40)
loc = iceberg_location("co", "entries")
con.sql(f"SELECT snapshot_id, timestamp_ms, sequence_number FROM iceberg_snapshots('{loc}')").show()
