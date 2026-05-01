-- CO-123: Sample ClickHouse queries for CO analytics.
--
-- Prerequisites:
--   flyctl proxy 8123:8123 -a co-clickhouse   (keep open in another terminal)
--   clickhouse-client --host 127.0.0.1 --user co_admin --password <pass>
--
-- All queries target co_analytics.wae_events.
-- See docs/OPERATIONS.md §ClickHouse for full runbook.

-- ─── 1. Top-10 universes by activity — last 24 hours ──────────────────────────
--
-- Answers: which universes are most active right now?

SELECT
    universe_id,
    count()                                 AS events,
    countIf(event_type = 'pageview')        AS pageviews,
    countIf(event_type = 'error')           AS errors,
    round(avg(duration_ms), 1)              AS avg_duration_ms
FROM co_analytics.wae_events
WHERE timestamp >= now() - INTERVAL 1 DAY
  AND universe_id != ''
GROUP BY universe_id
ORDER BY events DESC
LIMIT 10;


-- ─── 2. Error rate by deploy / status code — last 7 days ─────────────────────
--
-- Answers: did a recent deploy introduce regressions?

SELECT
    toDate(timestamp)                       AS day,
    toUInt16(status)                        AS status_code,
    count()                                 AS hits,
    round(100.0 * count() / sum(count()) OVER (PARTITION BY day), 2) AS pct
FROM co_analytics.wae_events
WHERE timestamp >= now() - INTERVAL 7 DAY
  AND status > 0
GROUP BY day, status_code
ORDER BY day DESC, hits DESC;


-- ─── 3. 7-day and 30-day visitor retention ────────────────────────────────────
--
-- Answers: of visitors who appeared on day 0, how many returned?
-- Note: user_kind is the visitor cohort proxy (anonymous / logged_in).

WITH
    cohort AS (
        SELECT
            universe_id,
            user_kind,
            min(toDate(timestamp)) AS day0
        FROM co_analytics.wae_events
        WHERE timestamp >= now() - INTERVAL 60 DAY
        GROUP BY universe_id, user_kind
    ),
    activity AS (
        SELECT DISTINCT
            universe_id,
            user_kind,
            toDate(timestamp) AS active_day
        FROM co_analytics.wae_events
        WHERE timestamp >= now() - INTERVAL 60 DAY
    )
SELECT
    c.universe_id,
    count(DISTINCT c.user_kind)                                           AS cohort_size,
    countDistinctIf(c.user_kind, a7.active_day IS NOT NULL)               AS returned_7d,
    countDistinctIf(c.user_kind, a30.active_day IS NOT NULL)              AS returned_30d,
    round(100.0 * countDistinctIf(c.user_kind, a7.active_day IS NOT NULL)
          / count(DISTINCT c.user_kind), 1)                               AS retention_7d_pct,
    round(100.0 * countDistinctIf(c.user_kind, a30.active_day IS NOT NULL)
          / count(DISTINCT c.user_kind), 1)                               AS retention_30d_pct
FROM cohort c
LEFT JOIN activity a7
    ON c.universe_id = a7.universe_id
    AND c.user_kind  = a7.user_kind
    AND a7.active_day BETWEEN c.day0 + 1 AND c.day0 + 7
LEFT JOIN activity a30
    ON c.universe_id = a30.universe_id
    AND c.user_kind  = a30.user_kind
    AND a30.active_day BETWEEN c.day0 + 1 AND c.day0 + 30
GROUP BY c.universe_id
ORDER BY cohort_size DESC
LIMIT 20;


-- ─── 4. Phase 3 readiness: Iceberg on R2 via icebergS3() ─────────────────────
--
-- Smoke test — verifies the icebergS3() table function can reach R2.
-- Replace the URL, key, and secret with real R2 credentials.
-- Expects 0 rows on an empty table; any non-error result = PASS.
--
-- See infra/clickhouse/iceberg-smoke-test.sh for the automated version.

SELECT count() AS row_count
FROM icebergS3(
    'https://<ACCOUNT_ID>.r2.cloudflarestorage.com/co-lakehouse/co/universes/',
    '<R2_ACCESS_KEY>',
    '<R2_SECRET_KEY>'
);
