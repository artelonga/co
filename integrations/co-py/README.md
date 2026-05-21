# co-py

Query CO universe data from Python via DuckDB.

## Install

```bash
pip install co-py
```

## Usage

### Local data directory (no network needed)

```python
import co_py

con = co_py.connect("my-universe", data_dir="/data")
df = con.execute("SELECT path, title FROM entries WHERE entry_type = 'task'").df()
```

### Hosted API (Bearer token required)

```python
import co_py

con = co_py.connect(
    "my-universe",
    base_url="https://co.example.com",
    api_token="your-api-token",
)
df = con.execute("SELECT entry_type, COUNT(*) FROM entries GROUP BY entry_type").df()
```

## API

### `co_py.connect(slug, *, base_url=None, api_token=None, data_dir=None)`

Returns a read-only `duckdb.DuckDBPyConnection`.

| Parameter | Description |
|-----------|-------------|
| `slug` | Universe key (e.g. `"template"`) |
| `data_dir` | Root data directory (resolves `{data_dir}/universes/**/{slug}/data.db`) |
| `base_url` | CO server base URL (used when `data_dir` is absent or misses) |
| `api_token` | Bearer token for authenticated download |
