//! CO-457: OTLP metrics receiver — ingest Claude Code native telemetry.
//!
//! Claude Code emits token usage as the native OpenTelemetry metric
//! `claude_code.token.usage` (and cost as `claude_code.cost.usage`). co-auto now
//! enables that exporter (CO-457 producer half) instead of hand-parsing
//! stream-json (the deleted CO-425 `usage.rs`). This endpoint is the **ingest**
//! direction of the same OTLP stack CO-291 stood up for export: it decodes an
//! OTLP/HTTP protobuf `ExportMetricsServiceRequest` and folds each session into
//! the existing `usage_sessions` ledger (CO-426) — so `GET /usage/summary`, the
//! dashboard, and the CO-427 downshift keep reading the same table, unchanged.
//!
//! Aggregation temporality: co-auto requests **delta** temporality
//! (`OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE=delta`), so each export
//! carries the token increment since the last flush. Every export becomes one
//! `usage_sessions` row and `usage_summary` SUMs them — totals stay correct with
//! no per-session upsert and **no schema migration** (reuses CO-275/426 columns).
//!
//! Auth reuses the producer's bearer scheme: co-auto sets
//! `OTEL_EXPORTER_OTLP_HEADERS=Authorization=Bearer <token>`, validated here by
//! the same `auth_provider` the usage write-path uses.

use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{KeyValue, any_value};
use opentelemetry_proto::tonic::metrics::v1::{metric, number_data_point};
use prost::Message;

use crate::platform::error::AppError;
use crate::server::AppState;
use crate::storage::NewUsageSession;

/// The native Claude Code metric carrying per-`type` token counts.
const METRIC_TOKEN_USAGE: &str = "claude_code.token.usage";
/// The native Claude Code metric carrying USD cost (0 under keychain auth).
const METRIC_COST_USAGE: &str = "claude_code.cost.usage";

// ---------------------------------------------------------------------------
// Attribute helpers
// ---------------------------------------------------------------------------

/// Read a string-ish attribute by key from an OTLP `KeyValue` list. Numbers and
/// bools are stringified; absent/array/kvlist values yield `None`.
fn attr_str(attrs: &[KeyValue], key: &str) -> Option<String> {
    let kv = attrs.iter().find(|kv| kv.key == key)?;
    match kv.value.as_ref()?.value.as_ref()? {
        any_value::Value::StringValue(s) => Some(s.clone()),
        any_value::Value::IntValue(i) => Some(i.to_string()),
        any_value::Value::DoubleValue(d) => Some(d.to_string()),
        any_value::Value::BoolValue(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The integer value of a number data point (`AsInt`, or a truncated `AsDouble`).
fn point_int(value: &Option<number_data_point::Value>) -> i64 {
    match value {
        Some(number_data_point::Value::AsInt(i)) => *i,
        Some(number_data_point::Value::AsDouble(d)) => *d as i64,
        None => 0,
    }
}

/// The float value of a number data point (`AsDouble`, or a widened `AsInt`).
fn point_f64(value: &Option<number_data_point::Value>) -> f64 {
    match value {
        Some(number_data_point::Value::AsInt(i)) => *i as f64,
        Some(number_data_point::Value::AsDouble(d)) => *d,
        None => 0.0,
    }
}

/// Convert OTLP unix-nanos to epoch seconds (0 when unset).
fn nanos_to_secs(nanos: u64) -> i64 {
    (nanos / 1_000_000_000) as i64
}

/// Shorten a model id to its family (`claude-opus-4-…` → `opus`), mirroring the
/// producer + the JSON-ingest path so OTLP and POST rows group together.
fn short_model(model: &str) -> String {
    for family in ["opus", "sonnet", "haiku", "fable", "mythos"] {
        if model.contains(family) {
            return family.to_string();
        }
    }
    model.to_string()
}

// ---------------------------------------------------------------------------
// Pure mapping: ExportMetricsServiceRequest → usage rows
// ---------------------------------------------------------------------------

/// One accumulating session, keyed by `(task, universe, machine, model, session)`.
#[derive(Default)]
struct Acc {
    task_key: String,
    universe_key: String,
    machine: String,
    model: String,
    pr_url: Option<String>,
    tokens_in: i64,
    tokens_out: i64,
    tokens_cache_read: i64,
    tokens_cache_write: i64,
    cost_usd: f64,
    has_cost: bool,
    started_at: i64,
    ended_at: i64,
}

/// Fold an OTLP metrics export into zero or more `usage_sessions` rows.
///
/// Resource attributes (`task.key`, `universe.key`, `machine`, `pr.url`) scope
/// each `ResourceMetrics`; the per-point `type`/`model`/`session.id` attributes
/// split a resource into one accumulator per distinct session+model. Only
/// `claude_code.token.usage` and `claude_code.cost.usage` are read — every other
/// metric is ignored. Best-effort by construction: missing fields default to
/// zero, never an error. `reported_at` is stamped by the caller.
pub fn ingest_metrics(req: &ExportMetricsServiceRequest, reported_at: i64) -> Vec<NewUsageSession> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<(String, String, String, String, String), Acc> = BTreeMap::new();

    for rm in &req.resource_metrics {
        let res_attrs = rm
            .resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or(&[]);
        let task_key = attr_str(res_attrs, "task.key")
            .or_else(|| attr_str(res_attrs, "task_id"))
            .unwrap_or_default();
        let universe_key = attr_str(res_attrs, "universe.key")
            .or_else(|| attr_str(res_attrs, "universe_key"))
            .unwrap_or_default();
        let machine = attr_str(res_attrs, "machine")
            .or_else(|| attr_str(res_attrs, "host.name"))
            .unwrap_or_else(|| "unknown".to_string());
        let pr_url = attr_str(res_attrs, "pr.url").or_else(|| attr_str(res_attrs, "pr_url"));
        let res_session = attr_str(res_attrs, "session.id");

        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                let is_tokens = m.name == METRIC_TOKEN_USAGE;
                let is_cost = m.name == METRIC_COST_USAGE;
                if !is_tokens && !is_cost {
                    continue;
                }
                // Both metrics are monotonic Sums in Claude Code's exporter.
                let points = match &m.data {
                    Some(metric::Data::Sum(sum)) => &sum.data_points,
                    Some(metric::Data::Gauge(g)) => &g.data_points,
                    _ => continue,
                };
                for p in points {
                    let model = attr_str(&p.attributes, "model").unwrap_or_default();
                    let session = attr_str(&p.attributes, "session.id")
                        .or_else(|| res_session.clone())
                        .unwrap_or_else(|| task_key.clone());
                    let key = (
                        task_key.clone(),
                        universe_key.clone(),
                        machine.clone(),
                        short_model(&model),
                        session,
                    );
                    let entry = acc.entry(key).or_default();
                    if entry.task_key.is_empty() && !task_key.is_empty() {
                        entry.task_key = task_key.clone();
                    }
                    entry.universe_key = universe_key.clone();
                    entry.machine = machine.clone();
                    if entry.model.is_empty() {
                        entry.model = short_model(&model);
                    }
                    if entry.pr_url.is_none() {
                        entry.pr_url = pr_url.clone();
                    }
                    let start = nanos_to_secs(p.start_time_unix_nano);
                    let end = nanos_to_secs(p.time_unix_nano);
                    if start > 0 && (entry.started_at == 0 || start < entry.started_at) {
                        entry.started_at = start;
                    }
                    if end > entry.ended_at {
                        entry.ended_at = end;
                    }

                    if is_cost {
                        entry.cost_usd += point_f64(&p.value);
                        entry.has_cost = true;
                        continue;
                    }
                    // token.usage: split by the `type` attribute.
                    let n = point_int(&p.value);
                    match attr_str(&p.attributes, "type").as_deref() {
                        Some("input") => entry.tokens_in += n,
                        Some("output") => entry.tokens_out += n,
                        Some("cacheRead") => entry.tokens_cache_read += n,
                        Some("cacheCreation") => entry.tokens_cache_write += n,
                        _ => {}
                    }
                }
            }
        }
    }

    acc.into_values()
        .filter(|a| {
            // Drop empty accumulators (a cost-only point with no tokens still
            // counts — it carries a real session).
            a.tokens_in != 0
                || a.tokens_out != 0
                || a.tokens_cache_read != 0
                || a.tokens_cache_write != 0
                || a.has_cost
        })
        .map(|a| NewUsageSession {
            task_key: if a.task_key.is_empty() {
                "unknown".to_string()
            } else {
                a.task_key
            },
            universe_key: if a.universe_key.is_empty() {
                "unknown".to_string()
            } else {
                a.universe_key
            },
            machine: a.machine,
            model: if a.model.is_empty() {
                "?".to_string()
            } else {
                a.model
            },
            tokens_in: a.tokens_in,
            tokens_out: a.tokens_out,
            tokens_cache_read: a.tokens_cache_read,
            tokens_cache_write: a.tokens_cache_write,
            // Subscription/keychain auth reports no USD → leave NULL, not 0.0.
            cost_usd: if a.has_cost && a.cost_usd > 0.0 {
                Some(a.cost_usd)
            } else {
                None
            },
            started_at: if a.started_at > 0 {
                a.started_at
            } else {
                reported_at
            },
            ended_at: if a.ended_at > 0 {
                a.ended_at
            } else {
                reported_at
            },
            outcome: "telemetry".to_string(),
            reported_at,
            // OTel metric stream carries neither tool counts nor turns — accepted
            // loss (the stream-json producer's enrichment is gone with CO-425).
            tool_uses: None,
            pr_url: a.pr_url,
            turns: 0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP receiver
// ---------------------------------------------------------------------------

/// Validate the bearer token the OTLP exporter attaches via
/// `OTEL_EXPORTER_OTLP_HEADERS`. Any principal the `auth_provider` accepts is
/// allowed (parity with the usage write-path); no admin tier required.
async fn require_token(state: &AppState, headers: &axum::http::HeaderMap) -> Result<(), AppError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .ok_or_else(|| AppError::Unauthorized("missing or malformed authorization".into()))?;
    state
        .core
        .auth_provider
        .verify_token(&crate::infra::auth::Token(token))
        .await
        .map_err(|_| AppError::Unauthorized("invalid or expired token".into()))?;
    Ok(())
}

/// `POST /v1/metrics` — OTLP/HTTP metrics receiver (protobuf).
///
/// Decodes the export, folds `claude_code.*` into `usage_sessions`, and returns
/// 200. Best-effort: a row insert that fails is logged and skipped, never 500s —
/// telemetry must not push back on the producer. A body that is not valid OTLP
/// protobuf is a 400.
async fn post_metrics(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    require_token(&state, &headers).await?;

    let req = ExportMetricsServiceRequest::decode(body)
        .map_err(|e| AppError::BadRequest(format!("invalid OTLP metrics protobuf: {e}")))?;

    let now = chrono::Utc::now().timestamp();
    let sessions = ingest_metrics(&req, now);

    if !sessions.is_empty() {
        let storage = state.core.storage.lock();
        for s in &sessions {
            if let Err(e) = storage.insert_usage_session(s) {
                tracing::warn!(error = %e, task = %s.task_key, "OTLP usage row insert failed");
            }
        }
    }

    // OTLP/HTTP accepts an empty 200 as full success (partial-success body optional).
    Ok(StatusCode::OK)
}

/// The OTLP receiver router. Mounted at the server root so the standard
/// `OTEL_EXPORTER_OTLP_ENDPOINT=<base>` + signal path `/v1/metrics` resolves
/// without a prefix. Self-authenticates per request (bearer), so no auth layer.
pub fn router() -> Router<AppState> {
    Router::new().route("/v1/metrics", post(post_metrics))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
    use opentelemetry_proto::tonic::metrics::v1::{
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric,
    };
    use opentelemetry_proto::tonic::resource::v1::Resource;

    fn kv(key: &str, val: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(val.to_string())),
            }),
        }
    }

    fn token_point(type_attr: &str, model: &str, n: i64) -> NumberDataPoint {
        NumberDataPoint {
            attributes: vec![kv("type", type_attr), kv("model", model)],
            start_time_unix_nano: 1_000 * 1_000_000_000,
            time_unix_nano: 1_060 * 1_000_000_000,
            value: Some(number_data_point::Value::AsInt(n)),
            ..Default::default()
        }
    }

    /// Build a representative Claude Code export: one session, four token types.
    fn sample_request() -> ExportMetricsServiceRequest {
        let token_metric = Metric {
            name: METRIC_TOKEN_USAGE.to_string(),
            data: Some(metric::Data::Sum(Sum {
                data_points: vec![
                    token_point("input", "claude-opus-4-8", 1200),
                    token_point("output", "claude-opus-4-8", 340),
                    token_point("cacheRead", "claude-opus-4-8", 8000),
                    token_point("cacheCreation", "claude-opus-4-8", 500),
                ],
                aggregation_temporality: 1, // delta
                is_monotonic: true,
            })),
            ..Default::default()
        };
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![
                        kv("task.key", "CO-457"),
                        kv("universe.key", "co"),
                        kv("machine", "macbook"),
                        kv("pr.url", "https://github.com/artelonga/co/pull/457"),
                    ],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![token_metric],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn ingest_maps_token_types_to_one_row() {
        let rows = ingest_metrics(&sample_request(), 9999);
        assert_eq!(rows.len(), 1, "one session → one row");
        let r = &rows[0];
        assert_eq!(r.task_key, "CO-457");
        assert_eq!(r.universe_key, "co");
        assert_eq!(r.machine, "macbook");
        assert_eq!(r.model, "opus", "model id shortened to family");
        assert_eq!(r.tokens_in, 1200);
        assert_eq!(r.tokens_out, 340);
        assert_eq!(r.tokens_cache_read, 8000);
        assert_eq!(r.tokens_cache_write, 500);
        assert_eq!(
            r.pr_url.as_deref(),
            Some("https://github.com/artelonga/co/pull/457")
        );
        // Resource start/end timestamps map to epoch seconds.
        assert_eq!(r.started_at, 1000);
        assert_eq!(r.ended_at, 1060);
        // Keychain auth: no cost metric → NULL, not 0.
        assert_eq!(r.cost_usd, None);
        assert_eq!(r.reported_at, 9999);
    }

    #[test]
    fn ingest_reads_cost_metric_when_present() {
        let mut req = sample_request();
        let cost_metric = Metric {
            name: METRIC_COST_USAGE.to_string(),
            data: Some(metric::Data::Sum(Sum {
                data_points: vec![NumberDataPoint {
                    attributes: vec![kv("model", "claude-opus-4-8")],
                    value: Some(number_data_point::Value::AsDouble(0.0421)),
                    ..Default::default()
                }],
                aggregation_temporality: 1,
                is_monotonic: true,
            })),
            ..Default::default()
        };
        req.resource_metrics[0].scope_metrics[0]
            .metrics
            .push(cost_metric);
        let rows = ingest_metrics(&req, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cost_usd, Some(0.0421));
    }

    #[test]
    fn ingest_ignores_unrelated_metrics() {
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![kv("task.key", "CO-1"), kv("universe.key", "co")],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: "claude_code.lines_of_code.count".to_string(),
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![token_point("added", "x", 99)],
                            aggregation_temporality: 1,
                            is_monotonic: true,
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        assert!(ingest_metrics(&req, 1).is_empty());
    }

    #[test]
    fn ingest_splits_distinct_sessions() {
        let mut req = sample_request();
        // Add a second resource for a different task/session.
        req.resource_metrics.push(ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![kv("task.key", "CO-999"), kv("universe.key", "ygg")],
                ..Default::default()
            }),
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: METRIC_TOKEN_USAGE.to_string(),
                    data: Some(metric::Data::Sum(Sum {
                        data_points: vec![token_point("input", "claude-sonnet-4-5", 50)],
                        aggregation_temporality: 1,
                        is_monotonic: true,
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        });
        let rows = ingest_metrics(&req, 1);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .any(|r| r.task_key == "CO-457" && r.model == "opus")
        );
        assert!(
            rows.iter()
                .any(|r| r.task_key == "CO-999" && r.model == "sonnet")
        );
    }

    /// The protobuf round-trips through prost decode exactly as the HTTP handler
    /// will receive it (encode in a fake exporter, decode in the receiver).
    #[test]
    fn export_request_roundtrips_through_prost() {
        let req = sample_request();
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let decoded = ExportMetricsServiceRequest::decode(&buf[..]).unwrap();
        let rows = ingest_metrics(&decoded, 7);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tokens_in, 1200);
    }
}
