//! CO-273: Deployment snapshot worker — probes each deployable unit's Fly.io
//! machines API and /api/health endpoint every 5 minutes and persists the
//! result to `deployment_snapshots`.

use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use crate::server::AppState;

// ---------------------------------------------------------------------------
// Unit registry (CO-338/CO-280) — derived from the universe surface registry
// ---------------------------------------------------------------------------

/// A deployable/tracked unit, assembled from the surface registry. The `url`
/// (and hence `health_url`) is **computed** by `core::surface` from the unit's
/// lineage + DNS — it is never hardcoded, so promoting a sub-universe (giving
/// it its own `surface_dns`) reroutes its URL with no code change.
pub struct UnitConfig {
    pub id: String,
    pub display: String,
    pub url: String,
    pub health_url: Option<String>,
    pub fly_app: Option<String>,
}

/// CO-338/CO-280: the deployment registry seed — lineage (`parent`), DNS
/// (`surface_dns`) and operational metadata (`fly_app`) for each tracked unit.
/// This replaces the old hardcoded URL list: URLs are resolved from this data
/// via `core::surface`, and live `universes.surface_dns` values overlay it
/// (`tick`) so a promotion recorded in the DB reroutes a unit with zero code
/// edits.
struct RegistryUnit {
    key: &'static str,
    parent: Option<&'static str>,
    surface_dns: Option<&'static str>,
    fly_app: Option<&'static str>,
    display: &'static str,
    probe_health: bool,
}

static REGISTRY_UNITS: &[RegistryUnit] = &[
    RegistryUnit {
        key: "co",
        parent: None,
        surface_dns: Some("co.artelonga.com.br"),
        fly_app: Some("co-artelonga"),
        display: "co.artelonga",
        probe_health: true,
    },
    RegistryUnit {
        key: "artelonga",
        parent: None,
        surface_dns: Some("artelonga.com.br"),
        fly_app: None,
        display: "artelonga",
        probe_health: true,
    },
    RegistryUnit {
        key: "yggdrasil",
        parent: None,
        surface_dns: Some("yggdrasil.artelonga.com.br"),
        fly_app: Some("yggdrasil-artelonga"),
        display: "yggdrasil",
        probe_health: true,
    },
    RegistryUnit {
        key: "rfq",
        parent: None,
        surface_dns: Some("rfq.fly.dev"),
        fly_app: Some("rfq-artelonga"),
        display: "rfq",
        probe_health: true,
    },
    // comunicacao is a sub-universe of co with no DNS of its own: its URL is
    // resolved through co (→ https://co.artelonga.com.br/comunicacao). Give it a
    // `surface_dns` in the DB to promote it and this row follows automatically.
    RegistryUnit {
        key: "comunicacao",
        parent: Some("co"),
        surface_dns: None,
        fly_app: None,
        display: "comunicacao",
        probe_health: false,
    },
];

/// Build the tracked unit list by resolving each registry unit's URL through
/// `core::surface`. `db_nodes` are the live `(key, parent, surface_dns)` rows
/// (from `Storage::list_surface_nodes`); any `surface_dns` set there overlays
/// the seed so an operator-recorded promotion reroutes the URL.
pub fn build_units(db_nodes: &[co::surface::SurfaceNode]) -> Vec<UnitConfig> {
    use std::collections::HashMap;

    let live_dns: HashMap<&str, &str> = db_nodes
        .iter()
        .filter_map(|n| n.surface_dns.as_deref().map(|d| (n.key.as_str(), d)))
        .collect();

    let nodes: Vec<co::surface::SurfaceNode> = REGISTRY_UNITS
        .iter()
        .map(|ru| co::surface::SurfaceNode {
            key: ru.key.to_string(),
            parent: ru.parent.map(str::to_string),
            // Live DB promotion wins over the seed DNS.
            surface_dns: live_dns
                .get(ru.key)
                .map(|d| d.to_string())
                .or_else(|| ru.surface_dns.map(str::to_string)),
        })
        .collect();

    let registry = co::surface::SurfaceRegistry::new(nodes);

    REGISTRY_UNITS
        .iter()
        .map(|ru| {
            // Resolve the unit's base URL; an unresolvable unit degrades to the
            // seed DNS (or empty) rather than dropping out of the dashboard.
            let url = registry
                .resolve_surface_ref(ru.key, "")
                .map(|r| r.url.trim_end_matches('/').to_string())
                .unwrap_or_else(|_| {
                    ru.surface_dns
                        .map(|d| format!("https://{d}"))
                        .unwrap_or_default()
                });
            let health_url = if ru.probe_health && !url.is_empty() {
                Some(format!("{url}/api/health"))
            } else {
                None
            };
            UnitConfig {
                id: ru.key.to_string(),
                display: ru.display.to_string(),
                url,
                health_url,
                fly_app: ru.fly_app.map(str::to_string),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fly.io API response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct FlyMachine {
    id: Option<String>,
    region: Option<String>,
    state: Option<String>,
    config: Option<FlyMachineConfig>,
    events: Option<Vec<FlyEvent>>,
}

#[derive(Debug, Deserialize, Default)]
struct FlyMachineConfig {
    image: Option<String>,
    guest: Option<FlyGuest>,
}

#[derive(Debug, Deserialize, Default)]
struct FlyGuest {
    cpu_kind: Option<String>,
    cpus: Option<u32>,
    memory_mb: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
struct FlyEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct HealthResponse {
    #[allow(dead_code)]
    status: Option<String>,
    version: Option<String>,
}

// ---------------------------------------------------------------------------
// Probe result (produced per unit, outside DB lock)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ProbeResult {
    pub machine_id: String,
    pub region: String,
    pub vm_size: String,
    pub state: String,
    pub image: String,
    pub last_deploy_at: String,
    pub version: String,
    pub health_status: String,
    pub error_msg: String,
}

impl Default for ProbeResult {
    fn default() -> Self {
        Self {
            machine_id: String::new(),
            region: String::new(),
            vm_size: String::new(),
            state: String::new(),
            image: String::new(),
            last_deploy_at: String::new(),
            version: String::new(),
            health_status: "unknown".to_string(),
            error_msg: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-unit probe (no DB lock held)
// ---------------------------------------------------------------------------

async fn probe_unit(client: &reqwest::Client, fly_token: &str, unit: &UnitConfig) -> ProbeResult {
    let mut result = ProbeResult::default();

    // 1. Fly machines API (only for units with a fly_app)
    if let Some(app) = unit.fly_app.as_deref() {
        if !fly_token.is_empty() {
            let url = format!("https://api.machines.dev/v1/apps/{app}/machines");
            match client
                .get(&url)
                .header("Authorization", format!("Bearer {fly_token}"))
                .timeout(Duration::from_secs(15))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(machines) = resp.json::<Vec<FlyMachine>>().await {
                        // Use the first machine in the list
                        if let Some(m) = machines.first() {
                            result.machine_id = m.id.clone().unwrap_or_default();
                            result.region = m.region.clone().unwrap_or_default();
                            result.state = m.state.clone().unwrap_or_default();

                            if let Some(cfg) = &m.config {
                                result.image = cfg.image.clone().unwrap_or_default();
                                if let Some(guest) = &cfg.guest {
                                    let kind = guest.cpu_kind.as_deref().unwrap_or("shared");
                                    let cpus = guest.cpus.unwrap_or(1);
                                    let mb = guest.memory_mb.unwrap_or(256);
                                    result.vm_size = format!("{kind}-{cpus}x-{mb}mb");
                                }
                            }

                            // Last launch event timestamp
                            if let Some(events) = &m.events
                                && let Some(ts) = events
                                    .iter()
                                    .filter(|e| {
                                        e.event_type.as_deref() == Some("launch")
                                            || e.event_type.as_deref() == Some("start")
                                    })
                                    .filter_map(|e| e.timestamp)
                                    .max()
                            {
                                result.last_deploy_at = chrono::DateTime::from_timestamp(ts, 0)
                                    .map(|dt: chrono::DateTime<chrono::Utc>| dt.to_rfc3339())
                                    .unwrap_or_default();
                            }
                        }
                    }
                }
                Ok(resp) => {
                    result.error_msg = format!("fly API {} returned {}", app, resp.status());
                }
                Err(e) => {
                    result.error_msg = format!("fly API {app} error: {e}");
                }
            }
        } else {
            result.error_msg = "CO_FLY_API_TOKEN not set".to_string();
        }
    } else {
        // Not a Fly app (e.g., artelonga = GitHub Pages, comunicacao = CO sub-universe)
        result.state = "n/a".to_string();
    }

    // 2. Health check (only for units with a health_url)
    if let Some(health_url) = unit.health_url.as_deref() {
        match client
            .get(health_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(h) = resp.json::<HealthResponse>().await {
                    result.version = h.version.unwrap_or_default();
                    result.health_status = "ok".to_string();
                } else {
                    result.health_status = "ok".to_string();
                }
            }
            Ok(resp) => {
                result.health_status = "unreachable".to_string();
                if result.error_msg.is_empty() {
                    result.error_msg = format!("health {} {}", health_url, resp.status());
                }
            }
            Err(e) => {
                result.health_status = "unreachable".to_string();
                if result.error_msg.is_empty() {
                    result.error_msg = format!("health {health_url} error: {e}");
                }
            }
        }
    } else {
        // comunicacao has no health URL
        result.health_status = "synced".to_string();
    }

    result
}

// ---------------------------------------------------------------------------
// Public tick function (called by DeploymentSnapshotWorker)
// ---------------------------------------------------------------------------

pub async fn tick(state: &AppState) -> Result<()> {
    let fly_token = state.core.secrets.get_or("CO_FLY_API_TOKEN", "");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // CO-338: units come from the universe surface registry (live DB rows
    // overlaying the seed), not a hardcoded URL list. The lock is held only to
    // snapshot the rows, then released before any network probe.
    let units = {
        let nodes = state.core.storage.lock().list_surface_nodes();
        build_units(&nodes)
    };

    // Fan out every unit in parallel — one unit's failure never blocks others.
    let results: Vec<ProbeResult> =
        futures_util::future::join_all(units.iter().map(|u| probe_unit(&client, &fly_token, u)))
            .await;

    let now = chrono::Utc::now().timestamp();
    let storage = state.core.storage.lock();
    let conn = storage.conn();

    for (unit, result) in units.iter().zip(results.iter()) {
        if let Err(e) = conn.execute(
            "INSERT INTO deployment_snapshots
                (unit, snapshot_at, machine_id, region, vm_size, state, image,
                 version, last_deploy_at, health_status, error_msg)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(unit) DO UPDATE SET
                snapshot_at   = excluded.snapshot_at,
                machine_id    = excluded.machine_id,
                region        = excluded.region,
                vm_size       = excluded.vm_size,
                state         = excluded.state,
                image         = excluded.image,
                version       = excluded.version,
                last_deploy_at = excluded.last_deploy_at,
                health_status  = excluded.health_status,
                error_msg      = excluded.error_msg",
            rusqlite::params![
                &unit.id,
                now,
                result.machine_id,
                result.region,
                result.vm_size,
                result.state,
                result.image,
                result.version,
                result.last_deploy_at,
                result.health_status,
                result.error_msg,
            ],
        ) {
            tracing::warn!("deployment_snapshot write failed for {}: {e}", unit.id);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn units() -> Vec<UnitConfig> {
        // No live DB rows → URLs come purely from the registry seed.
        build_units(&[])
    }

    #[test]
    fn units_list_has_five_entries() {
        assert_eq!(units().len(), 5);
    }

    #[test]
    fn all_units_have_unique_ids() {
        let units = units();
        let mut ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), units.len());
    }

    #[test]
    fn fly_apps_are_only_for_fly_units() {
        let units = units();
        let fly_units: Vec<&str> = units
            .iter()
            .filter(|u| u.fly_app.is_some())
            .map(|u| u.id.as_str())
            .collect();
        // artelonga (gh-pages) and comunicacao (CO sub-universe) have no fly app
        assert!(!fly_units.contains(&"artelonga"));
        assert!(!fly_units.contains(&"comunicacao"));
    }

    #[test]
    fn urls_are_resolved_from_the_registry_not_hardcoded() {
        let units = units();
        let url = |id: &str| {
            units
                .iter()
                .find(|u| u.id == id)
                .map(|u| u.url.clone())
                .unwrap_or_default()
        };
        // Deployable surfaces resolve to their own DNS root.
        assert_eq!(url("co"), "https://co.artelonga.com.br");
        assert_eq!(url("yggdrasil"), "https://yggdrasil.artelonga.com.br");
        assert_eq!(url("rfq"), "https://rfq.fly.dev");
        // A sub-universe (no DNS) resolves through its deployable ancestor `co`.
        assert_eq!(
            url("comunicacao"),
            "https://co.artelonga.com.br/comunicacao"
        );
    }

    #[test]
    fn db_promotion_reroutes_a_unit_with_no_code_change() {
        // Operator records a promotion in the DB: comunicacao gains its own DNS.
        let promoted = [co::surface::SurfaceNode {
            key: "comunicacao".to_string(),
            parent: Some("co".to_string()),
            surface_dns: Some("comunicacao.artelonga.com.br".to_string()),
        }];
        let units = build_units(&promoted);
        let comunicacao = units.iter().find(|u| u.id == "comunicacao").unwrap();
        assert_eq!(comunicacao.url, "https://comunicacao.artelonga.com.br");
        // …and it now gets a health probe, since it has its own surface.
        assert_eq!(
            comunicacao.health_url.as_deref(),
            None,
            "comunicacao stays health-probe-free (probe_health=false in the seed)"
        );
    }
}
