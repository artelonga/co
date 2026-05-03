# CO Telemetry Envelope — CO-156

Every server-side state change emits a single `telemetry_events` row with `event_type = "crud"`.

## Envelope shape (JSON stored in `properties` column)

```json
{
  "list": "reference",
  "deployment_version": "1.39.0",
  "timestamp_ns": 1714750000000000000,
  "extra": { "…kind-specific data…" }
}
```

Main columns in `telemetry_events`:

| Column        | CO-156 meaning                                     |
|---------------|----------------------------------------------------|
| `event_type`  | `"crud"`                                           |
| `event_name`  | Event kind (see table below)                       |
| `universe_key`| Universe the event targets                         |
| `path`        | Entry path or asset sha256                         |
| `user_id`     | Authenticated actor; `NULL` for anonymous          |
| `session_id`  | xxh3 hash of JWT session cookie or anon visitante  |
| `properties`  | JSON with `list`, `deployment_version`, `timestamp_ns`, `extra` |

## Event kinds

| Kind              | Hook                                              | `extra` fields                          |
|-------------------|---------------------------------------------------|-----------------------------------------|
| `entry.upsert`    | `entry_routes::create_entry` / `update_entry`<br>`vault_routes::put_vault_file`<br>`processos::approve_alterar_pagina`<br>`reference_routes::create/update_reference` | — |
| `entry.delete`    | `entry_routes::delete_entry`<br>`vault_routes::delete_vault_file`<br>`processos::revert_alterar_pagina`<br>`reference_routes::delete_reference` | — |
| `asset.upload`    | `asset_routes::upload_asset`                      | `mime`, `size_bytes`                    |
| `asset.delete`    | `asset_routes::delete_asset`                      | —                                       |
| `relation.create` | After `sync_entry_relations` when count > 0       | `count`                                 |
| `relation.delete` | After `relation_index::delete_for_entry`          | —                                       |
| `ws.connect`      | `sync_ws::handle_sync_socket` (post-auth)         | `conn_id`                               |
| `ws.disconnect`   | `sync_ws::handle_sync_socket` (loop exit)         | `conn_id`                               |
| `ws.lag`          | broadcast `Lagged` branch                         | `lagged` (dropped message count)        |
| `auth.login`      | `verify_handler` (magic-link)<br>`password_login_handler`<br>`uat_login_handler` | `list: "magic-link"` or `"password"` |
| `auth.logout`     | `logout_handler`                                  | —                                       |

## `list` field

For entry/relation events: the `entry_type` (e.g. `"reference"`, `"task"`, `"page"`).
For asset events: not set.
For WS events: not set.
For auth events: the flow name (`"magic-link"`, `"password"`).

## `deployment_version`

Set at compile time from `CARGO_PKG_VERSION` (e.g. `"1.39.0"`).

## `session_id`

- Authenticated callers: `ses_<xxh3(co_session cookie)>` (hex)
- Anonymous callers: `anon_<xxh3(visitante_id cookie)>` (hex)
- `NULL` when no session cookie is present (e.g. background jobs)

## Retention

CRUD events are stored in `telemetry_events` alongside page views.
The existing 90-day retention policy (CO-46) applies.

## Admin dashboard

`GET /api/v1/admin/telemetry/crud-summary` returns CRUD events aggregated
over the last 24 hours by kind.

The `/co/co/telemetria` admin page renders this as a live table.
