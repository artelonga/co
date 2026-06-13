//! CO-273: Deployment snapshot worker — probes each deployable unit's Fly.io
//! machines API and /api/health endpoint every 5 minutes and persists the
//! result to `deployment_snapshots`.

use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use crate::server::AppState;

// ---------------------------------------------------------------------------
// Hardcoded unit registry
// ---------------------------------------------------------------------------

pub struct UnitConfig {
    pub id: &'static str,
    pub display: &'static str,
    pub url: &'static str,
    pub health_url: Option<&'static str>,
    pub fly_app: Option<&'static str>,
}

pub static UNITS: &[UnitConfig] = &[
    UnitConfig {
        id: "co",
        display: "co.artelonga",
        url: "https://co.artelonga.com.br",
        health_url: Some("https://co.artelonga.com.br/api/health"),
        fly_app: Some("co-artelonga"),
    },
    UnitConfig {
        id: "artelonga",
        display: "artelonga",
        url: "https://artelonga.com.br",
        health_url: Some("https://artelonga.com.br/api/health"),
        fly_app: None,
    },
    UnitConfig {
        id: "quilombo",
        display: "quilomboaraucaria",
        url: "https://quilomboaraucaria.org",
        health_url: Some("https://quilomboaraucaria.org/api/health"),
        fly_app: Some("quilomboaraucaria"),
    },
    UnitConfig {
        id: "yggdrasil",
        display: "yggdrasil",
        url: "https://yggdrasil.artelonga.com.br",
        health_url: Some("https://yggdrasil.artelonga.com.br/api/health"),
        fly_app: Some("yggdrasil-artelonga"),
    },
    UnitConfig {
        id: "rfq",
        display: "rfq",
        url: "https://rfq.fly.dev",
        health_url: Some("https://rfq.fly.dev/api/health"),
        fly_app: Some("rfq-artelonga"),
    },
    UnitConfig {
        id: "comunicacao",
        display: "comunicacao",
        url: "https://co.artelonga.com.br/co/comunicacao",
        health_url: None,
        fly_app: None,
    },
];

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
    if let Some(app) = unit.fly_app {
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
    if let Some(health_url) = unit.health_url {
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

    // Fan out all 6 units in parallel — one unit's failure never blocks others.
    let (r0, r1, r2, r3, r4, r5) = tokio::join!(
        probe_unit(&client, &fly_token, &UNITS[0]),
        probe_unit(&client, &fly_token, &UNITS[1]),
        probe_unit(&client, &fly_token, &UNITS[2]),
        probe_unit(&client, &fly_token, &UNITS[3]),
        probe_unit(&client, &fly_token, &UNITS[4]),
        probe_unit(&client, &fly_token, &UNITS[5]),
    );
    let results = [r0, r1, r2, r3, r4, r5];

    let now = chrono::Utc::now().timestamp();
    let storage = state.core.storage.lock();
    let conn = storage.conn();

    for (unit, result) in UNITS.iter().zip(results.iter()) {
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
                unit.id,
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

    #[test]
    fn units_list_has_six_entries() {
        assert_eq!(UNITS.len(), 6);
    }

    #[test]
    fn all_units_have_unique_ids() {
        let mut ids: Vec<&str> = UNITS.iter().map(|u| u.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), UNITS.len());
    }

    #[test]
    fn fly_apps_are_only_for_fly_units() {
        let fly_units: Vec<&str> = UNITS
            .iter()
            .filter(|u| u.fly_app.is_some())
            .map(|u| u.id)
            .collect();
        // artelonga (gh-pages) and comunicacao (CO sub-universe) have no fly app
        assert!(!fly_units.contains(&"artelonga"));
        assert!(!fly_units.contains(&"comunicacao"));
    }
}
