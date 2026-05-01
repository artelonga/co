-- CO-123: ClickHouse initial schema for CO analytics.
-- Mirrors the WAE telemetry data points written by the co-wae-proxy Cloudflare Worker.
--
-- WAE data layout (from workers/wae-proxy/src/index.ts):
--   indexes[0] = universe_id
--   blobs[0]   = event_type
--   blobs[1]   = user_kind
--   blobs[2]   = flag_key   (A/B test key; empty string when absent)
--   blobs[3]   = variant    (A/B test variant; empty string when absent)
--   doubles[0] = duration_ms
--   doubles[1] = status     (HTTP status code; 0 when absent)

CREATE DATABASE IF NOT EXISTS co_analytics;

CREATE TABLE IF NOT EXISTS co_analytics.wae_events
(
    timestamp    DateTime      CODEC(DoubleDelta, ZSTD(1)),
    universe_id  LowCardinality(String),
    event_type   LowCardinality(String),
    user_kind    LowCardinality(String),
    flag_key     String        DEFAULT '',
    variant      String        DEFAULT '',
    duration_ms  Float64       DEFAULT 0,
    status       Float64       DEFAULT 0
)
ENGINE = MergeTree()
PARTITION BY toYYYYMMDD(timestamp)
ORDER BY (universe_id, event_type, timestamp)
TTL timestamp + INTERVAL 90 DAY DELETE
SETTINGS index_granularity = 8192;

-- Daily aggregate materialized view for fast dashboard queries.
-- Filled incrementally as wae_events receives data.
CREATE MATERIALIZED VIEW IF NOT EXISTS co_analytics.wae_daily_agg
ENGINE = SummingMergeTree()
PARTITION BY toYYYYMMDD(day)
ORDER BY (day, universe_id, event_type)
AS
SELECT
    toStartOfDay(timestamp)                        AS day,
    universe_id,
    event_type,
    count()                                        AS events,
    countIf(status >= 400 AND status < 600)        AS errors,
    avg(duration_ms)                               AS avg_duration_ms,
    uniq(user_kind)                                AS unique_user_kinds
FROM co_analytics.wae_events
GROUP BY day, universe_id, event_type;
