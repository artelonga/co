//! CO-365: Backup worker tick — build snapshot, put to backend, prune old copies.
//!
//! CO-405 (2026-06-11 double outage): the tick is defensive by construction —
//! it debounces restart bursts, refuses to snapshot without disk headroom,
//! retains by count/size (days secondary), and builds the tarball on a
//! blocking thread so it can never starve the async runtime. The worker also
//! declares an `initial_delay` (see `platform::workers::BackupWorker`) so the
//! snapshot never sits in the boot path.

use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::atividade::{Acao, Atividade, Tipo, log_atividade};
use crate::eda::{Event, Visibility};
use crate::server::AppState;

use super::{SnapshotId, SnapshotMeta};

const DEFAULT_MIN_INTERVAL_HOURS: u64 = 6;
const DEFAULT_FREE_FLOOR_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB
const DEFAULT_RETAIN_COUNT: u64 = 3;
const DEFAULT_RETENTION_DAYS: u64 = 30;

fn env_u64(name: &str, default: u64) -> u64 {
    use crate::infra::secrets::SecretsProviderExt;
    crate::infra::secrets::global().get_parsed(name, default)
}

/// Executes one backup cycle:
/// 1. Debounce: skip when the newest snapshot is younger than
///    `CO_BACKUP_MIN_INTERVAL_HOURS` (restart bursts must not multiply
///    snapshots — 4 accumulated in one day re-filled the volume, CO-405).
/// 2. Free-space guard: skip (never fail the app) when available space is
///    below max(2× last snapshot, `CO_BACKUP_MIN_FREE_BYTES`).
/// 3. Build snapshot tarball on a blocking thread.
/// 4. Upload via the configured backend.
/// 5. Prune by count (`CO_BACKUP_RETAIN_COUNT`), cumulative size
///    (`CO_BACKUP_RETAIN_MAX_BYTES`, 0 = unlimited), and age
///    (`CO_BACKUP_RETENTION_DAYS`, secondary).
/// 6. Log success/failure/skips to atividades.
pub async fn run_backup_tick(state: &AppState) -> anyhow::Result<()> {
    let data_dir = {
        let storage = state.core.storage.lock();
        storage.data_dir.clone()
    };

    let backend = match super::backend_from_env(&data_dir) {
        Some(b) => b,
        None => {
            tracing::debug!("backup: CO_BACKUP_BACKEND=disabled — skipping");
            return Ok(());
        }
    };

    let backend_name = backend.name();

    let metas = match backend.list().await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("backup: could not list existing snapshots: {e}");
            Vec::new()
        }
    };

    // CO-405 debounce — one snapshot per window, no matter how many restarts.
    let min_interval = ChronoDuration::hours(env_u64(
        "CO_BACKUP_MIN_INTERVAL_HOURS",
        DEFAULT_MIN_INTERVAL_HOURS,
    ) as i64);
    if let Some(newest) = metas.first() {
        let age = Utc::now() - newest.created_at;
        if debounce_skip(age, min_interval) {
            tracing::info!(
                "backup: skipped — newest snapshot is {}min old (< {}h debounce)",
                age.num_minutes(),
                min_interval.num_hours()
            );
            return Ok(());
        }
    }

    // CO-405 free-space guard — a backup must never take production down.
    let floor = env_u64("CO_BACKUP_MIN_FREE_BYTES", DEFAULT_FREE_FLOOR_BYTES);
    let last_bytes = metas.first().map(|m| m.bytes).unwrap_or(0);
    let avail = available_bytes(&data_dir);
    if let Some(required) = low_disk_skip(avail, last_bytes, floor) {
        let available = avail.unwrap_or(0);
        tracing::warn!(
            "backup: skipped — low disk (available={available} B, required={required} B, \
             last_snapshot={last_bytes} B)"
        );
        state.core.eda_bus.publish(Event::new(
            "backup.skipped_low_disk",
            None,
            None,
            serde_json::json!({
                "available_bytes": available,
                "required_bytes": required,
                "last_snapshot_bytes": last_bytes,
                "backend": backend_name,
            }),
            Visibility::System,
        ));
        log_atividade(
            state.clone(),
            Atividade {
                acao: Acao::Criar,
                entidade: "backup_snapshot".to_string(),
                entidade_id: None,
                before: None,
                after: Some(serde_json::json!({
                    "skipped": "low_disk",
                    "available_bytes": available,
                    "required_bytes": required,
                    "backend": backend_name,
                })),
                tipo: Tipo::Erro,
                user_id: None,
                ip: None,
                user_agent: None,
            },
        );
        return Ok(());
    }

    tracing::info!("backup: starting snapshot (backend={backend_name})");

    let tmp_dir = std::env::temp_dir().join(format!("co-backup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;

    let result = (async {
        // CO-405: tar+gzip of /data is CPU/IO-heavy (minutes on a 1-vCPU
        // machine) — run on a blocking thread so the async runtime keeps
        // serving requests.
        let (dd, td) = (data_dir.clone(), tmp_dir.clone());
        let snapshot =
            tokio::task::spawn_blocking(move || super::build_snapshot(&dd, &td, backend_name))
                .await??;
        let id = backend.put(&snapshot).await?;
        tracing::info!("backup: snapshot stored as {id} ({} bytes)", snapshot.bytes);
        anyhow::Ok((id, snapshot.bytes))
    })
    .await;

    // Clean up temp dir regardless of outcome.
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match result {
        Ok((id, bytes)) => {
            log_atividade(
                state.clone(),
                Atividade {
                    acao: Acao::Criar,
                    entidade: "backup_snapshot".to_string(),
                    entidade_id: Some(id.0.clone()),
                    before: None,
                    after: Some(serde_json::json!({
                        "id": id.0,
                        "backend": backend_name,
                        "bytes": bytes,
                        "free_bytes_after": available_bytes(&data_dir).unwrap_or(0),
                        "created_at": Utc::now().to_rfc3339(),
                    })),
                    tipo: Tipo::Sucesso,
                    user_id: None,
                    ip: None,
                    user_agent: None,
                },
            );
            prune_snapshots(backend.as_ref(), state).await;
            Ok(())
        }
        Err(e) => {
            tracing::error!("backup: snapshot failed: {e:#}");
            log_atividade(
                state.clone(),
                Atividade {
                    acao: Acao::Criar,
                    entidade: "backup_snapshot".to_string(),
                    entidade_id: None,
                    before: None,
                    after: Some(serde_json::json!({
                        "error": e.to_string(),
                        "backend": backend_name,
                    })),
                    tipo: Tipo::Erro,
                    user_id: None,
                    ip: None,
                    user_agent: None,
                },
            );
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// CO-405 decision logic (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Debounce: true → skip, the newest snapshot is still fresh.
fn debounce_skip(newest_age: ChronoDuration, min_interval: ChronoDuration) -> bool {
    newest_age < min_interval
}

/// Free-space guard. Required headroom = max(2× last snapshot, floor).
/// Returns `Some(required)` when available space is insufficient (skip).
/// `avail = None` (statvfs unavailable/failed) disables the guard — missing
/// telemetry must not silently stop backups.
fn low_disk_skip(avail: Option<u64>, last_snapshot_bytes: u64, floor: u64) -> Option<u64> {
    let required = last_snapshot_bytes.saturating_mul(2).max(floor);
    match avail {
        Some(a) if a < required => Some(required),
        _ => None,
    }
}

/// Retention: which snapshots to delete. `metas` is newest-first (the
/// `BackupBackend::list` contract). The newest snapshot is always kept — a
/// restore point must exist no matter how old or large. Beyond it, a
/// snapshot is pruned when it exceeds `retain_count`, pushes the cumulative
/// size past `retain_max_bytes` (0 = unlimited), or is older than
/// `retention_days`.
pub(crate) fn select_prunable(
    metas: &[SnapshotMeta],
    retain_count: usize,
    retain_max_bytes: u64,
    retention_days: i64,
    now: DateTime<Utc>,
) -> Vec<SnapshotId> {
    let cutoff = now - ChronoDuration::days(retention_days);
    let mut cumulative: u64 = 0;
    let mut prunable = Vec::new();
    for (i, m) in metas.iter().enumerate() {
        cumulative = cumulative.saturating_add(m.bytes);
        if i == 0 {
            continue; // always keep the newest restore point
        }
        let over_count = i >= retain_count;
        let over_bytes = retain_max_bytes > 0 && cumulative > retain_max_bytes;
        let too_old = m.created_at < cutoff;
        if over_count || over_bytes || too_old {
            prunable.push(m.id.clone());
        }
    }
    prunable
}

/// Available bytes on the filesystem holding `path` (Linux: statvfs).
#[cfg(target_os = "linux")]
fn available_bytes(path: &std::path::Path) -> Option<u64> {
    match nix::sys::statvfs::statvfs(path) {
        Ok(stat) => Some(stat.blocks_available() * stat.fragment_size()),
        Err(e) => {
            tracing::warn!("backup: statvfs({path:?}) failed: {e}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn available_bytes(_path: &std::path::Path) -> Option<u64> {
    None // dev platforms: guard disabled
}

async fn prune_snapshots(backend: &dyn super::BackupBackend, state: &AppState) {
    let retain_count = env_u64("CO_BACKUP_RETAIN_COUNT", DEFAULT_RETAIN_COUNT) as usize;
    let retain_max_bytes = env_u64("CO_BACKUP_RETAIN_MAX_BYTES", 0);
    let retention_days = env_u64("CO_BACKUP_RETENTION_DAYS", DEFAULT_RETENTION_DAYS) as i64;

    match backend.list().await {
        Ok(metas) => {
            for id in select_prunable(
                &metas,
                retain_count,
                retain_max_bytes,
                retention_days,
                Utc::now(),
            ) {
                tracing::info!("backup: pruning snapshot {id} (retention)");
                if let Err(e) = backend.delete(&id).await {
                    tracing::warn!("backup: failed to prune {id}: {e}");
                    log_atividade(
                        state.clone(),
                        Atividade {
                            acao: Acao::Excluir,
                            entidade: "backup_snapshot".to_string(),
                            entidade_id: Some(id.0.clone()),
                            before: None,
                            after: Some(serde_json::json!({"error": e.to_string()})),
                            tipo: Tipo::Erro,
                            user_id: None,
                            ip: None,
                            user_agent: None,
                        },
                    );
                }
            }
        }
        Err(e) => tracing::warn!("backup: could not list for retention: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, age_hours: i64, bytes: u64) -> SnapshotMeta {
        SnapshotMeta {
            id: SnapshotId(id.to_string()),
            created_at: Utc::now() - ChronoDuration::hours(age_hours),
            bytes,
            sha256: String::new(),
        }
    }

    fn ids(v: &[SnapshotId]) -> Vec<&str> {
        v.iter().map(|s| s.0.as_str()).collect()
    }

    // ── debounce ──────────────────────────────────────────────────────────

    #[test]
    fn debounce_skips_fresh_snapshot() {
        assert!(debounce_skip(
            ChronoDuration::hours(1),
            ChronoDuration::hours(6)
        ));
    }

    #[test]
    fn debounce_allows_stale_snapshot() {
        assert!(!debounce_skip(
            ChronoDuration::hours(7),
            ChronoDuration::hours(6)
        ));
        // Exactly at the boundary → proceed.
        assert!(!debounce_skip(
            ChronoDuration::hours(6),
            ChronoDuration::hours(6)
        ));
    }

    // ── free-space guard ──────────────────────────────────────────────────

    #[test]
    fn low_disk_requires_twice_last_snapshot() {
        // last = 263 MB → required 526 MB; 400 MB free → skip.
        let last = 263 * 1024 * 1024;
        let floor = 256 * 1024 * 1024;
        let skip = low_disk_skip(Some(400 * 1024 * 1024), last, floor);
        assert_eq!(skip, Some(2 * last));
        // 1 GB free → proceed.
        assert_eq!(low_disk_skip(Some(1024 * 1024 * 1024), last, floor), None);
    }

    #[test]
    fn low_disk_floor_dominates_when_no_history() {
        // No previous snapshot: floor is the requirement.
        let floor = 256 * 1024 * 1024;
        assert_eq!(
            low_disk_skip(Some(100 * 1024 * 1024), 0, floor),
            Some(floor)
        );
        assert_eq!(low_disk_skip(Some(300 * 1024 * 1024), 0, floor), None);
    }

    #[test]
    fn low_disk_guard_disabled_without_telemetry() {
        // statvfs failed / non-Linux → never block backups on missing data.
        assert_eq!(low_disk_skip(None, u64::MAX / 4, u64::MAX / 4), None);
    }

    // ── retention ─────────────────────────────────────────────────────────

    #[test]
    fn retention_prunes_beyond_count() {
        let metas = vec![
            meta("s0", 1, 100),
            meta("s1", 2, 100),
            meta("s2", 3, 100),
            meta("s3", 4, 100),
            meta("s4", 5, 100),
        ];
        let prunable = select_prunable(&metas, 3, 0, 30, Utc::now());
        assert_eq!(ids(&prunable), vec!["s3", "s4"]);
    }

    #[test]
    fn retention_always_keeps_newest() {
        // Single ancient snapshot — never pruned: a restore point must exist.
        let metas = vec![meta("only", 24 * 100, 999_999_999)];
        assert!(select_prunable(&metas, 3, 1, 30, Utc::now()).is_empty());
    }

    #[test]
    fn retention_prunes_over_cumulative_bytes() {
        // Sizes 600+300 fit in 1000; the third (cum 1100) does not.
        let metas = vec![meta("a", 1, 600), meta("b", 2, 300), meta("c", 3, 200)];
        let prunable = select_prunable(&metas, 10, 1000, 30, Utc::now());
        assert_eq!(ids(&prunable), vec!["c"]);
    }

    #[test]
    fn retention_days_is_secondary_but_still_applies() {
        let metas = vec![
            meta("fresh0", 1, 10),
            meta("fresh1", 2, 10),
            meta("old", 24 * 40, 10),
        ];
        let prunable = select_prunable(&metas, 10, 0, 30, Utc::now());
        assert_eq!(ids(&prunable), vec!["old"]);
    }

    #[test]
    fn retention_restart_burst_scenario() {
        // 2026-06-11 outage: 4 snapshots in one day (264+266+280+299 MB)
        // filled a 3 GB volume. With retain_count=3 the oldest goes.
        let mb = 1024 * 1024;
        let metas = vec![
            meta("d", 1, 264 * mb),
            meta("c", 5, 266 * mb),
            meta("b", 9, 280 * mb),
            meta("a", 13, 299 * mb),
        ];
        let prunable = select_prunable(&metas, 3, 0, 30, Utc::now());
        assert_eq!(ids(&prunable), vec!["a"]);
    }

    #[test]
    fn retention_count_zero_keeps_only_newest() {
        let metas = vec![meta("s0", 1, 10), meta("s1", 2, 10)];
        let prunable = select_prunable(&metas, 0, 0, 30, Utc::now());
        assert_eq!(ids(&prunable), vec!["s1"]);
    }
}
