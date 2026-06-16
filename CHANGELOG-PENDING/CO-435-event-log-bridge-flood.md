## CO-435 — Stop persisting bridge transport events to event_log (prod disk flood)

The EDA bridge emits a `bridge.event_sent` / `bridge.event_received` event for
*every* event it relays (~73/sec in prod). `AtividadesPersistor` persisted
**every** EDA event to `event_log`, so these transport events flooded the table
to **38M rows / 18 GB in 6 days — 99.1% of all rows** — which filled the prod
`/data` volume to 88%+ (deploy-blocking, near the SQLITE_FULL crash-loop).

`AtividadesPersistor` now skips the high-frequency transport events
(`bridge.event_sent`, `bridge.event_received`) before persisting — they are pure
bus-transport telemetry, not domain events worth durable replay; the in-memory
observability ring buffer / live layer still sees them. Low-frequency bridge
lifecycle events (`bridge.connected`/`disconnected`) and all domain events are
unaffected. With transport events suppressed, `event_log` drops to ~58K rows/day,
so the existing 30-day retention is comfortably sufficient.

### Why
Durable, indexed persistence of a 73/sec internal relay signal is never the right
default. This stops the growth at the source; the pre-existing bloat is reclaimed
separately (30-day retention prune, or a one-time meta.db compaction).
