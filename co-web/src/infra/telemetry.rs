//! Distributed tracing — CO-291.
//!
//! Wires a `tracing` subscriber that emits spans to stderr (dev default) or
//! exports them via OTLP gRPC to a collector such as Jaeger or Honeycomb when
//! `CO_TELEMETRY_OTLP_ENDPOINT` is set.
//!
//! See `docs/observability.md` for a local Jaeger quickstart.

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Telemetry backend selection.  Resolved from environment at server startup.
pub enum TelemetryConfig {
    /// Emit structured logs to stderr — the default when OTLP env var is absent.
    Stderr,
    /// Export traces via OTLP gRPC to a collector (Jaeger, Honeycomb, Grafana…).
    Otlp {
        endpoint: String,
        service_name: String,
        sampling_ratio: f64,
    },
}

impl TelemetryConfig {
    /// Resolve config from environment variables.
    ///
    /// | Variable                        | Default   | Description                              |
    /// |---------------------------------|-----------|------------------------------------------|
    /// | `CO_TELEMETRY_OTLP_ENDPOINT`    | —         | gRPC endpoint; enables OTLP when set    |
    /// | `CO_TELEMETRY_SERVICE_NAME`     | `co-web`  | Service name shown in the collector UI   |
    /// | `CO_TELEMETRY_SAMPLING_RATIO`   | `1.0`     | Fraction of traces to sample (0.0–1.0)  |
    pub fn from_env() -> Self {
        match std::env::var("CO_TELEMETRY_OTLP_ENDPOINT") {
            Ok(endpoint) if !endpoint.is_empty() => {
                let service_name = std::env::var("CO_TELEMETRY_SERVICE_NAME")
                    .unwrap_or_else(|_| "co-web".to_string());
                let sampling_ratio = std::env::var("CO_TELEMETRY_SAMPLING_RATIO")
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0);
                Self::Otlp {
                    endpoint,
                    service_name,
                    sampling_ratio,
                }
            }
            _ => Self::Stderr,
        }
    }
}

// ---------------------------------------------------------------------------
// Shutdown guard
// ---------------------------------------------------------------------------

/// Returned by [`init_subscriber`].  When dropped, flushes any pending OTLP spans.
///
/// Must be held for the lifetime of the server process — dropping it early will
/// cause in-flight spans to be lost.
pub struct TelemetryGuard {
    otlp_active: bool,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if self.otlp_active {
            opentelemetry::global::shutdown_tracer_provider();
        }
    }
}

// ---------------------------------------------------------------------------
// Subscriber initialisation
// ---------------------------------------------------------------------------

/// Initialise the global `tracing` subscriber.
///
/// Call exactly once at server startup, before any spans are created.
/// Hold the returned [`TelemetryGuard`] until the server exits.
pub fn init_subscriber(config: TelemetryConfig) -> TelemetryGuard {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "co_web=info,tower_http=info".parse().unwrap());

    match config {
        TelemetryConfig::Stderr => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
            TelemetryGuard { otlp_active: false }
        }

        TelemetryConfig::Otlp {
            endpoint,
            service_name,
            sampling_ratio,
        } => {
            // install_batch registers the provider globally and returns a Tracer.
            let tracer = build_otlp_tracer(&endpoint, &service_name, sampling_ratio);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer())
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();

            // Log after init so the message itself is captured by the subscriber.
            tracing::info!(
                endpoint = %endpoint,
                service = %service_name,
                sampling = %sampling_ratio,
                "OTLP telemetry enabled",
            );

            TelemetryGuard { otlp_active: true }
        }
    }
}

// ---------------------------------------------------------------------------
// DB span helper
// ---------------------------------------------------------------------------

/// Create a tracing span for a SQLite query.
///
/// The span becomes a child of the current HTTP request span automatically
/// (tracing propagates context through the task).  Enter it around any storage
/// call you want to appear in the OTLP collector:
///
/// ```ignore
/// let _guard = db_span("entries", "select", universe_key).entered();
/// let rows = entry_index.query(...)?;
/// ```
pub fn db_span(table: &str, operation: &str, universe: &str) -> tracing::Span {
    tracing::info_span!(
        "db.query",
        db.system = "sqlite",
        db.operation = %operation,
        db.table = %table,
        co.universe = %universe,
    )
}

// ---------------------------------------------------------------------------
// OTLP tracer builder
// ---------------------------------------------------------------------------

/// Build an OTLP tracer via the pipeline API.
///
/// Creates a `TracerProvider` with a `BatchSpanProcessor`, registers it as the
/// global provider, and returns a `Tracer` from it.
/// Must be called from within a Tokio runtime context.
fn build_otlp_tracer(
    endpoint: &str,
    service_name: &str,
    sampling_ratio: f64,
) -> opentelemetry_sdk::trace::Tracer {
    use opentelemetry::KeyValue;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::{
        Resource, runtime,
        trace::{Config, Sampler},
    };

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            Config::default()
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
                    sampling_ratio,
                ))))
                .with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    service_name.to_string(),
                )])),
        )
        .install_batch(runtime::Tokio)
        .expect("OTLP pipeline init failed");

    opentelemetry::global::set_tracer_provider(provider.clone());
    provider.tracer("co-web")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify `from_env` doesn't panic regardless of env state.
    #[test]
    fn from_env_does_not_panic() {
        // We intentionally do NOT manipulate env vars here: `set_var`/`remove_var`
        // are `unsafe` in Rust 2024 (they can cause UB when called concurrently)
        // and the test runner can run tests in parallel.  We just confirm the
        // function is callable without panicking.
        let _config = TelemetryConfig::from_env();
    }

    /// Sampling ratio is clamped to [0.0, 1.0] regardless of the input value.
    #[test]
    fn sampling_ratio_clamp_logic() {
        assert_eq!(42.0_f64.clamp(0.0, 1.0), 1.0);
        assert_eq!((-1.0_f64).clamp(0.0, 1.0), 0.0);
        assert_eq!(0.5_f64.clamp(0.0, 1.0), 0.5);
    }

    /// `db_span` must not panic and must return a valid (possibly disabled) span.
    #[test]
    fn db_span_does_not_panic() {
        let _span = db_span("entries", "select", "my-universe");
    }
}
