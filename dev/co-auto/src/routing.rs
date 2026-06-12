//! Model routing (CO-427).
//!
//! Picks the executor model **per task** instead of one global `--model` for the
//! whole run. Resolution follows a four-level precedence:
//!
//! 1. `--model` explicit on the CLI (operator override — wins everything).
//! 2. `model:` in the task's frontmatter (per-task override).
//! 3. Priority→model policy (`high→opus`, `medium→sonnet`, `low→haiku`,
//!    configurable) — **only when opted in** via `routing.by_priority: true`
//!    (or `CO_AUTO_ROUTING_BY_PRIORITY=1`). Off by default.
//! 4. Quality-first default (`opus`) — this is the **default** for any task with
//!    no `--model` / no frontmatter, per the binding owner decision of
//!    2026-06-12 ("default = opus, *not* the priority tier; tiers are opt-in").
//!
//! After resolution, a **window downshift** runs (best-effort): co-auto reads
//! CO-426's `GET /api/v1/usage/summary?window=5h`; if the rolling 5h token
//! consumption has crossed the configured soft limit, the model is degraded one
//! tier (`opus→sonnet→haiku`) and the reason is logged + reported. Every failure
//! path (no endpoint, no network, unparseable response, lowest tier already) is
//! **fail-open**: the requested model is kept unchanged.
//!
//! Model names are CLI aliases (`opus`/`sonnet`/`haiku`/…) and are passed through
//! verbatim — never validated against a hardcoded id list, because the `claude`
//! CLI is the authority and that list changes.

use serde::Deserialize;
use std::path::Path;
use std::process::Command;

/// Which precedence level produced a resolved model (for logging / observability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// `--model` on the CLI.
    CliOverride,
    /// `model:` in the task frontmatter.
    Frontmatter,
    /// The priority→model policy.
    PriorityPolicy,
    /// The quality-first default (opus).
    DefaultQualityFirst,
}

impl ModelSource {
    /// A short human label for log lines.
    pub fn label(&self) -> &'static str {
        match self {
            ModelSource::CliOverride => "--model override",
            ModelSource::Frontmatter => "task frontmatter",
            ModelSource::PriorityPolicy => "priority policy",
            ModelSource::DefaultQualityFirst => "quality-first default",
        }
    }
}

/// co-auto model-routing configuration.
///
/// Defaults are quality-first (CO-427 owner decision, 2026-06-12): a task with
/// no explicit signal runs on Opus. The priority→model policy and the soft limit
/// are configurable via `project.yaml` (`routing:` block) and env overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingConfig {
    /// Fallback when no `--model`, frontmatter, or priority match applies.
    pub default_model: String,
    /// Model for `high`/`critical` priority tasks.
    pub high: String,
    /// Model for `medium` priority tasks.
    pub medium: String,
    /// Model for `low` priority tasks.
    pub low: String,
    /// Whether the priority→model policy is consulted at all. **Default `false`**
    /// per the binding CO-427 owner decision: a task with no `--model` and no
    /// frontmatter `model:` runs on the quality-first default (Opus), *not* its
    /// priority tier. Set `routing.by_priority: true` (or
    /// `CO_AUTO_ROUTING_BY_PRIORITY=1`) to opt into tiered routing.
    pub by_priority: bool,
    /// Soft limit on rolling 5h total tokens that triggers a one-tier downshift.
    /// `None` disables window downshift entirely.
    pub usage_soft_limit_5h_tokens: Option<i64>,
    /// CO-426 usage endpoint base URL (e.g. `https://co-artelonga.fly.dev`).
    /// `None` disables the downshift network read (fail-open).
    pub usage_endpoint: Option<String>,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            default_model: "opus".into(),
            high: "opus".into(),
            medium: "sonnet".into(),
            low: "haiku".into(),
            by_priority: false,
            usage_soft_limit_5h_tokens: None,
            usage_endpoint: None,
        }
    }
}

/// Shape of the optional `routing:` block in `project.yaml`. Every field is
/// optional and overrides the corresponding [`RoutingConfig`] default.
#[derive(Debug, Default, Deserialize)]
struct RoutingFile {
    default: Option<String>,
    high: Option<String>,
    medium: Option<String>,
    low: Option<String>,
    by_priority: Option<bool>,
    usage_soft_limit_5h_tokens: Option<i64>,
    usage_endpoint: Option<String>,
}

impl RoutingConfig {
    /// Load routing config from `<data_dir>/project.yaml` (`routing:` block),
    /// then apply env overrides:
    ///
    /// - `CO_AUTO_SOFT_LIMIT_5H_TOKENS` → `usage_soft_limit_5h_tokens`
    /// - `CO_USAGE_ENDPOINT`            → `usage_endpoint`
    ///
    /// Missing file / missing block / parse error → silently keep defaults.
    pub fn load(data_dir: &Path) -> Self {
        let mut cfg = RoutingConfig::default();

        let project_yaml = data_dir.join("project.yaml");
        if let Ok(content) = std::fs::read_to_string(&project_yaml)
            && let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(&content)
            && let Some(routing) = root.get("routing")
            && let Ok(rf) = serde_yaml::from_value::<RoutingFile>(routing.clone())
        {
            if let Some(v) = rf.default {
                cfg.default_model = v;
            }
            if let Some(v) = rf.high {
                cfg.high = v;
            }
            if let Some(v) = rf.medium {
                cfg.medium = v;
            }
            if let Some(v) = rf.low {
                cfg.low = v;
            }
            if let Some(v) = rf.by_priority {
                cfg.by_priority = v;
            }
            if rf.usage_soft_limit_5h_tokens.is_some() {
                cfg.usage_soft_limit_5h_tokens = rf.usage_soft_limit_5h_tokens;
            }
            if rf.usage_endpoint.is_some() {
                cfg.usage_endpoint = rf.usage_endpoint;
            }
        }

        if let Ok(v) = std::env::var("CO_AUTO_SOFT_LIMIT_5H_TOKENS")
            && let Ok(n) = v.trim().parse::<i64>()
        {
            cfg.usage_soft_limit_5h_tokens = Some(n);
        }
        if let Ok(v) = std::env::var("CO_USAGE_ENDPOINT")
            && !v.is_empty()
        {
            cfg.usage_endpoint = Some(v);
        }
        if let Ok(v) = std::env::var("CO_AUTO_ROUTING_BY_PRIORITY") {
            let v = v.trim();
            cfg.by_priority = v == "1" || v.eq_ignore_ascii_case("true");
        }

        cfg
    }

    /// Map a priority label to its policy model, if the label is recognized.
    fn for_priority(&self, priority: &str) -> Option<&str> {
        match priority {
            "high" | "critical" => Some(&self.high),
            "medium" => Some(&self.medium),
            "low" => Some(&self.low),
            _ => None,
        }
    }
}

/// Resolve the executor model for a task following the four-level precedence.
///
/// Returns the chosen model alias plus the [`ModelSource`] that produced it.
/// Empty CLI/frontmatter strings are treated as "not set" and fall through.
pub fn resolve_model(
    cli_override: Option<&str>,
    task_model: Option<&str>,
    priority: &str,
    cfg: &RoutingConfig,
) -> (String, ModelSource) {
    if let Some(m) = cli_override.filter(|s| !s.trim().is_empty()) {
        return (m.trim().to_string(), ModelSource::CliOverride);
    }
    if let Some(m) = task_model.filter(|s| !s.trim().is_empty()) {
        return (m.trim().to_string(), ModelSource::Frontmatter);
    }
    if cfg.by_priority
        && let Some(m) = cfg.for_priority(priority)
    {
        return (m.to_string(), ModelSource::PriorityPolicy);
    }
    (cfg.default_model.clone(), ModelSource::DefaultQualityFirst)
}

/// Downshift one quality tier: `opus→sonnet→haiku`. Returns `None` for the
/// lowest tier (`haiku`) or an unrecognized alias — we never downshift a model
/// we can't rank.
pub fn downshift_one(model: &str) -> Option<&'static str> {
    match model {
        "opus" => Some("sonnet"),
        "sonnet" => Some("haiku"),
        _ => None,
    }
}

/// A recorded downshift decision (for logging + the usage report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downshift {
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// Decide whether to downshift, purely (no I/O) so it is deterministically
/// testable. Fail-open: returns `None` (keep the requested model) when there is
/// no soft limit, no usage reading, consumption is under the limit, or the model
/// is already at the lowest tier.
pub fn decide_downshift(
    requested: &str,
    total_tokens_5h: Option<i64>,
    soft_limit: Option<i64>,
) -> Option<Downshift> {
    let limit = soft_limit?;
    let total = total_tokens_5h?;
    if total < limit {
        return None;
    }
    let to = downshift_one(requested)?;
    Some(Downshift {
        from: requested.to_string(),
        to: to.to_string(),
        reason: format!(
            "5h usage {total} tokens ≥ soft limit {limit} — degraded one tier ({requested}→{to})"
        ),
    })
}

/// Fetch the rolling 5h total-token count (and server-reported soft limit, if
/// any) from CO-426's `GET /api/v1/usage/summary?window=5h`.
///
/// Best-effort via `curl`; returns `None` on any failure so the caller stays
/// fail-open. Reuses `CO_SESSION_TOKEN` for the admin-gated read when set.
pub fn fetch_5h_usage(endpoint: &str) -> Option<(i64, Option<i64>)> {
    let url = format!(
        "{}/api/v1/usage/summary?window=5h",
        endpoint.trim_end_matches('/')
    );
    let mut args: Vec<String> = vec!["-s".into(), "--max-time".into(), "10".into()];
    if let Ok(token) = std::env::var("CO_SESSION_TOKEN")
        && !token.is_empty()
    {
        args.push("-H".into());
        args.push(format!("Authorization: Bearer {token}"));
    }
    args.push(url);

    let out = Command::new("curl").args(&args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&out.stdout);
    parse_5h_usage(body.trim())
}

/// Parse the `/usage/summary` JSON body into `(total_tokens_5h, soft_limit_5h)`.
/// Split out from the network call so it is unit-testable. Returns `None` when
/// the body is not the expected shape (fail-open).
fn parse_5h_usage(body: &str) -> Option<(i64, Option<i64>)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let total = v.get("window_5h")?.get("total_tokens")?.as_i64()?;
    let soft = v.get("soft_limit_5h").and_then(|x| x.as_i64());
    Some((total, soft))
}

/// Apply the window downshift policy to a `requested` model. Performs the
/// network read and is **fail-open** on every error. Returns the (possibly
/// unchanged) model plus an optional [`Downshift`] record for logging/reporting.
///
/// The effective soft limit is the configured `usage_soft_limit_5h_tokens` when
/// set, otherwise the server-reported `soft_limit_5h`.
pub fn apply_downshift(requested: &str, cfg: &RoutingConfig) -> (String, Option<Downshift>) {
    let Some(endpoint) = cfg.usage_endpoint.as_deref() else {
        return (requested.to_string(), None);
    };
    let Some((total, server_soft)) = fetch_5h_usage(endpoint) else {
        return (requested.to_string(), None);
    };
    let soft = cfg.usage_soft_limit_5h_tokens.or(server_soft);
    match decide_downshift(requested, Some(total), soft) {
        Some(d) => (d.to.clone(), Some(d)),
        None => (requested.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RoutingConfig {
        RoutingConfig::default()
    }

    /// Config with the priority→model policy opted in (the non-default mode).
    fn cfg_by_priority() -> RoutingConfig {
        RoutingConfig {
            by_priority: true,
            ..RoutingConfig::default()
        }
    }

    // ---- precedence (4 levels) ----

    #[test]
    fn cli_override_wins_everything() {
        let (m, src) = resolve_model(Some("haiku"), Some("opus"), "high", &cfg());
        assert_eq!(m, "haiku");
        assert_eq!(src, ModelSource::CliOverride);
    }

    #[test]
    fn frontmatter_wins_over_priority_policy() {
        let (m, src) = resolve_model(None, Some("opus"), "low", &cfg());
        assert_eq!(m, "opus");
        assert_eq!(src, ModelSource::Frontmatter);
    }

    #[test]
    fn default_is_opus_for_any_priority_when_not_opted_in() {
        // Binding owner decision (2026-06-12): with no --model and no frontmatter,
        // the DEFAULT is opus regardless of the task's priority tier. The
        // priority policy is off unless `by_priority` is set.
        let c = cfg();
        for priority in ["high", "critical", "medium", "low", "weird"] {
            let (m, src) = resolve_model(None, None, priority, &c);
            assert_eq!(m, "opus", "priority={priority} must default to opus");
            assert_eq!(src, ModelSource::DefaultQualityFirst);
        }
    }

    #[test]
    fn priority_policy_applies_only_when_opted_in() {
        let c = cfg_by_priority();
        assert_eq!(
            resolve_model(None, None, "high", &c),
            ("opus".to_string(), ModelSource::PriorityPolicy)
        );
        assert_eq!(
            resolve_model(None, None, "medium", &c),
            ("sonnet".to_string(), ModelSource::PriorityPolicy)
        );
        assert_eq!(
            resolve_model(None, None, "low", &c),
            ("haiku".to_string(), ModelSource::PriorityPolicy)
        );
        // critical maps to the high tier.
        assert_eq!(
            resolve_model(None, None, "critical", &c).0,
            "opus".to_string()
        );
    }

    #[test]
    fn unknown_priority_falls_to_quality_first_default() {
        // Even with the policy opted in, an unrecognized priority → opus default.
        let (m, src) = resolve_model(None, None, "weird", &cfg_by_priority());
        assert_eq!(m, "opus");
        assert_eq!(src, ModelSource::DefaultQualityFirst);
    }

    #[test]
    fn empty_strings_are_treated_as_unset() {
        // Empty CLI + empty frontmatter → falls through to the priority policy
        // (opted in here so we exercise the fall-through, not the default).
        let (m, src) = resolve_model(Some("  "), Some(""), "low", &cfg_by_priority());
        assert_eq!(m, "haiku");
        assert_eq!(src, ModelSource::PriorityPolicy);
    }

    // ---- per-task resolution (as used in --cycle) ----

    #[test]
    fn resolves_per_task_across_a_cycle() {
        // Opted into tiered routing so per-task resolution visibly differs.
        let c = cfg_by_priority();
        // (priority, frontmatter) → expected model, simulating a mixed batch.
        let tasks: Vec<(&str, Option<&str>, &str)> = vec![
            ("high", None, "opus"),
            ("low", None, "haiku"),
            ("medium", Some("opus"), "opus"), // frontmatter override
            ("medium", None, "sonnet"),
        ];
        for (priority, fm, expected) in tasks {
            // No CLI override → per-task resolution differs across the run.
            let (m, _) = resolve_model(None, fm, priority, &c);
            assert_eq!(m, expected, "priority={priority} fm={fm:?}");
        }
    }

    #[test]
    fn cli_override_pins_every_task_in_a_cycle() {
        let c = cfg();
        for priority in ["high", "medium", "low"] {
            let (m, src) = resolve_model(Some("sonnet"), None, priority, &c);
            assert_eq!(m, "sonnet");
            assert_eq!(src, ModelSource::CliOverride);
        }
    }

    // ---- downshift decision ----

    #[test]
    fn downshift_one_tier_each_step() {
        assert_eq!(downshift_one("opus"), Some("sonnet"));
        assert_eq!(downshift_one("sonnet"), Some("haiku"));
        assert_eq!(downshift_one("haiku"), None);
        assert_eq!(downshift_one("fable"), None);
    }

    #[test]
    fn decide_downshift_triggers_at_or_above_soft_limit() {
        let d = decide_downshift("opus", Some(1000), Some(1000)).unwrap();
        assert_eq!(d.from, "opus");
        assert_eq!(d.to, "sonnet");
        assert!(d.reason.contains("soft limit"));
    }

    #[test]
    fn decide_downshift_under_limit_keeps_model() {
        assert_eq!(decide_downshift("opus", Some(999), Some(1000)), None);
    }

    #[test]
    fn decide_downshift_fail_open_without_limit_or_reading() {
        // No soft limit configured → never downshift.
        assert_eq!(decide_downshift("opus", Some(10_000), None), None);
        // No usage reading → never downshift.
        assert_eq!(decide_downshift("opus", None, Some(1000)), None);
    }

    #[test]
    fn decide_downshift_at_lowest_tier_is_noop() {
        // Over limit but already at the floor — nothing to degrade to.
        assert_eq!(decide_downshift("haiku", Some(10_000), Some(1000)), None);
    }

    // ---- fail-open without network ----

    #[test]
    fn apply_downshift_fail_open_when_no_endpoint() {
        // No usage_endpoint → no network call, model unchanged.
        let c = RoutingConfig {
            usage_soft_limit_5h_tokens: Some(1),
            usage_endpoint: None,
            ..RoutingConfig::default()
        };
        let (m, d) = apply_downshift("opus", &c);
        assert_eq!(m, "opus");
        assert!(d.is_none());
    }

    // ---- usage summary parsing ----

    #[test]
    fn parse_5h_usage_extracts_total_and_soft_limit() {
        let body = r#"{"window":"5h","window_5h":{"total_tokens":42000},"soft_limit_5h":50000}"#;
        assert_eq!(parse_5h_usage(body), Some((42000, Some(50000))));
    }

    #[test]
    fn parse_5h_usage_soft_limit_optional() {
        let body = r#"{"window_5h":{"total_tokens":7}}"#;
        assert_eq!(parse_5h_usage(body), Some((7, None)));
    }

    #[test]
    fn parse_5h_usage_fail_open_on_garbage() {
        assert_eq!(parse_5h_usage("not json"), None);
        assert_eq!(parse_5h_usage("{}"), None);
    }

    // ---- config loading ----

    #[test]
    fn load_defaults_when_no_project_yaml() {
        let tmp =
            std::env::temp_dir().join(format!("co-auto-routing-empty-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let c = RoutingConfig::load(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(c.default_model, "opus");
        assert_eq!(c.high, "opus");
        assert_eq!(c.medium, "sonnet");
        assert_eq!(c.low, "haiku");
        // Priority routing is OFF by default — unspecified tasks default to opus.
        assert!(!c.by_priority);
    }

    #[test]
    fn load_reads_routing_block_from_project_yaml() {
        let tmp =
            std::env::temp_dir().join(format!("co-auto-routing-block-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("project.yaml"),
            "key: CO\nrouting:\n  default: sonnet\n  high: opus\n  medium: haiku\n  low: haiku\n  usage_soft_limit_5h_tokens: 1234\n",
        )
        .unwrap();
        let c = RoutingConfig::load(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(c.default_model, "sonnet");
        assert_eq!(c.medium, "haiku");
        assert_eq!(c.usage_soft_limit_5h_tokens, Some(1234));
    }
}
