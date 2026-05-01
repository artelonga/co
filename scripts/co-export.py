#!/usr/bin/env python3
"""
co-export: Export CO SQLite data to Iceberg tables on MinIO.

Each run appends a new Iceberg snapshot — full table refresh by default.
Existing snapshots are preserved for time-travel queries.

Tables exported:
  co.universes  — universe metadata
  co.entries    — all entries (.md files indexed in SQLite)
  co.tasks      — task entries with their frontmatter fields unpacked

Usage:
  python3 scripts/co-export.py

Environment (defaults shown):
  CO_DB              data/co.db
  S3_ENDPOINT        http://localhost:9000
  S3_BUCKET          co-lakehouse
  AWS_ACCESS_KEY_ID  co-admin
  AWS_SECRET_ACCESS_KEY  co-secret-local
  ICEBERG_CATALOG_DB data/iceberg-catalog.db
"""

import os
import duckdb
import pyarrow as pa
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import (
    NestedField, StringType, LongType, BooleanType, IntegerType,
)
from pyiceberg.partitioning import PartitionSpec, PartitionField
from pyiceberg.transforms import IdentityTransform
from pyiceberg.exceptions import NoSuchTableError

DB_PATH    = os.environ.get("CO_DB",                  "data/co.db")
ENDPOINT   = os.environ.get("S3_ENDPOINT",             "http://localhost:9000")
BUCKET     = os.environ.get("S3_BUCKET",               "co-lakehouse")
KEY_ID     = os.environ.get("AWS_ACCESS_KEY_ID",       "co-admin")
SECRET     = os.environ.get("AWS_SECRET_ACCESS_KEY",   "co-secret-local")
CATALOG_DB = os.environ.get("ICEBERG_CATALOG_DB",      "data/iceberg-catalog.db")

_host   = ENDPOINT.replace("https://", "").replace("http://", "")
_ssl    = str(ENDPOINT.startswith("https://")).lower()

# ── Iceberg catalog (SQLite-backed — no REST server required) ─────────────────

catalog = SqlCatalog(
    "co",
    **{
        "uri":                  f"sqlite:///{CATALOG_DB}",
        "warehouse":            f"s3://{BUCKET}",
        "s3.endpoint":          ENDPOINT,
        "s3.access-key-id":     KEY_ID,
        "s3.secret-access-key": SECRET,
        "s3.region":            "us-east-1",
        "s3.path-style-access": "true",
    },
)

# ── Schemas ───────────────────────────────────────────────────────────────────

UNIVERSES_SCHEMA = Schema(
    NestedField(1,  "key",           StringType(),  required=False),
    NestedField(2,  "name",          StringType(),  required=False),
    NestedField(3,  "description",   StringType(),  required=False),
    NestedField(4,  "owner_id",      StringType(),  required=False),
    NestedField(5,  "created_at",    StringType(),  required=False),
    NestedField(6,  "is_template",   BooleanType(), required=False),
    NestedField(7,  "is_public",     BooleanType(), required=False),
    NestedField(8,  "requires_login",BooleanType(), required=False),
    NestedField(9,  "content_count", LongType(),    required=False),
    NestedField(10, "theme_preset",  StringType(),  required=False),
    NestedField(11, "layout",        StringType(),  required=False),
)

ENTRIES_SCHEMA = Schema(
    NestedField(1,  "universe_key",  StringType(),  required=False),
    NestedField(2,  "path",          StringType(),  required=False),
    NestedField(3,  "entry_type",    StringType(),  required=False),
    NestedField(4,  "title",         StringType(),  required=False),
    NestedField(5,  "created_at",    StringType(),  required=False),
    NestedField(6,  "updated_at",    StringType(),  required=False),
    NestedField(7,  "tags",          StringType(),  required=False),   # JSON array string
    NestedField(8,  "status",        StringType(),  required=False),
    NestedField(9,  "priority",      StringType(),  required=False),
    NestedField(10, "assignee",      StringType(),  required=False),
)

TASKS_SCHEMA = Schema(
    NestedField(1,  "universe_key",  StringType(),  required=False),
    NestedField(2,  "path",          StringType(),  required=False),
    NestedField(3,  "project_key",   StringType(),  required=False),
    NestedField(4,  "title",         StringType(),  required=False),
    NestedField(5,  "status",        StringType(),  required=False),
    NestedField(6,  "priority",      StringType(),  required=False),
    NestedField(7,  "assignee",      StringType(),  required=False),
    NestedField(8,  "tags",          StringType(),  required=False),
    NestedField(9,  "created_at",    StringType(),  required=False),
    NestedField(10, "updated_at",    StringType(),  required=False),
    NestedField(11, "archived",      BooleanType(), required=False),
)

# ── Helpers ───────────────────────────────────────────────────────────────────

def ensure_ns(ns: str):
    try:
        catalog.create_namespace(ns)
    except Exception:
        pass

def get_or_create(ns: str, name: str, schema: Schema, partition_cols: list = []):
    full = (ns, name)
    try:
        return catalog.load_table(full)
    except NoSuchTableError:
        spec = PartitionSpec(*[
            PartitionField(
                source_id=schema.find_field(col).field_id,
                field_id=1000 + i,
                transform=IdentityTransform(),
                name=col,
            )
            for i, col in enumerate(partition_cols)
        ])
        return catalog.create_table(full, schema=schema, partition_spec=spec)

def overwrite(table, arrow_table: pa.Table, label: str):
    # DuckDB .arrow() returns a RecordBatchReader; materialise to Table for pyiceberg.
    if hasattr(arrow_table, "read_all"):
        arrow_table = arrow_table.read_all()
    table.overwrite(arrow_table)
    snap = table.current_snapshot()
    print(f"  ✓ {label}: {arrow_table.num_rows} rows → snapshot {snap.snapshot_id}")

# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    print(f"Source  : {DB_PATH}")
    print(f"Dest    : s3://{BUCKET}  ({ENDPOINT})")
    print(f"Catalog : {CATALOG_DB}")
    print()

    con = duckdb.connect()
    con.execute("INSTALL sqlite; LOAD sqlite;")
    con.execute("INSTALL httpfs; LOAD httpfs;")
    con.execute(f"""
        CREATE SECRET minio (
            TYPE s3,
            ENDPOINT '{_host}',
            KEY_ID '{KEY_ID}',
            SECRET '{SECRET}',
            USE_SSL {_ssl},
            URL_STYLE path,
            REGION 'us-east-1'
        )
    """)
    con.execute(f"ATTACH '{DB_PATH}' AS co (TYPE sqlite, READ_ONLY)")

    ensure_ns("co")

    # ── universes ─────────────────────────────────────────────────────────────
    t = get_or_create("co", "universes", UNIVERSES_SCHEMA)
    arrow = con.execute("""
        SELECT
            key,
            name,
            description,
            owner_id,
            CAST(created_at AS VARCHAR)       AS created_at,
            CAST(is_template    AS BOOLEAN)   AS is_template,
            CAST(is_public      AS BOOLEAN)   AS is_public,
            CAST(requires_login AS BOOLEAN)   AS requires_login,
            CAST(content_count  AS BIGINT)    AS content_count,
            theme_preset,
            layout
        FROM co.universes
    """).arrow()
    overwrite(t, arrow, "universes")

    # ── entries (all types, tags + task fields extracted from frontmatter) ────
    t = get_or_create("co", "entries", ENTRIES_SCHEMA, ["universe_key"])
    arrow = con.execute("""
        SELECT
            universe_key,
            path,
            entry_type,
            title,
            CAST(created_at AS VARCHAR)  AS created_at,
            CAST(updated_at AS VARCHAR)  AS updated_at,
            CAST(json_extract(frontmatter_json, '$.tags')     AS VARCHAR) AS tags,
            CAST(json_extract(frontmatter_json, '$.status')   AS VARCHAR) AS status,
            CAST(json_extract(frontmatter_json, '$.priority') AS VARCHAR) AS priority,
            CAST(json_extract(frontmatter_json, '$.assignee') AS VARCHAR) AS assignee
        FROM co.entries
    """).arrow()
    overwrite(t, arrow, "entries")

    # ── tasks (entry_type='task', project path segment extracted) ─────────────
    t = get_or_create("co", "tasks", TASKS_SCHEMA, ["universe_key"])
    arrow = con.execute("""
        SELECT
            universe_key,
            path,
            -- project key = second segment: projects/<KEY>/N.md
            SPLIT_PART(path, '/', 2)                                        AS project_key,
            title,
            CAST(json_extract(frontmatter_json, '$.status')   AS VARCHAR)  AS status,
            CAST(json_extract(frontmatter_json, '$.priority') AS VARCHAR)  AS priority,
            CAST(json_extract(frontmatter_json, '$.assignee') AS VARCHAR)  AS assignee,
            CAST(json_extract(frontmatter_json, '$.tags')     AS VARCHAR)  AS tags,
            CAST(created_at AS VARCHAR)                                      AS created_at,
            CAST(updated_at AS VARCHAR)                                      AS updated_at,
            CAST(json_extract(frontmatter_json, '$.archived') AS BOOLEAN)  AS archived
        FROM co.entries
        WHERE entry_type = 'task'
    """).arrow()
    overwrite(t, arrow, "tasks")

    print()
    print("Done. Query the lakehouse:")
    print("  python3 scripts/co-query.py")

if __name__ == "__main__":
    main()
