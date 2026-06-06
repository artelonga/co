//! CO-365: Backup worker tick — build snapshot, put to backend, prune old copies.

use chrono::Utc;

use crate::atividade::{Acao, Atividade, Tipo, log_atividade};
use crate::server::AppState;

/// Executes one backup cycle:
/// 1. Build snapshot tarball.
/// 2. Upload via the configured backend.
/// 3. Prune snapshots older than `CO_BACKUP_RETENTION_DAYS`.
/// 4. Log success/failure to atividades.
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
    tracing::info!("backup: starting snapshot (backend={backend_name})");

    let tmp_dir = std::env::temp_dir().join(format!("co-backup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;

    let result = (async {
        let snapshot = super::build_snapshot(&data_dir, &tmp_dir, backend_name)?;
        let id = backend.put(&snapshot).await?;
        tracing::info!("backup: snapshot stored as {id} ({} bytes)", snapshot.bytes);
        anyhow::Ok(id)
    })
    .await;

    // Clean up temp dir regardless of outcome.
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match result {
        Ok(id) => {
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
                        "created_at": Utc::now().to_rfc3339(),
                    })),
                    tipo: Tipo::Sucesso,
                    user_id: None,
                    ip: None,
                    user_agent: None,
                },
            );
            prune_old_snapshots(backend.as_ref(), state).await;
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

async fn prune_old_snapshots(backend: &dyn super::BackupBackend, state: &AppState) {
    let retention_days: i64 = std::env::var("CO_BACKUP_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let cutoff = Utc::now() - chrono::Duration::days(retention_days);

    match backend.list().await {
        Ok(metas) => {
            for meta in metas {
                if meta.created_at < cutoff {
                    tracing::info!(
                        "backup: pruning snapshot {} (older than {retention_days}d)",
                        meta.id
                    );
                    if let Err(e) = backend.delete(&meta.id).await {
                        tracing::warn!("backup: failed to prune {}: {e}", meta.id);
                        log_atividade(
                            state.clone(),
                            Atividade {
                                acao: Acao::Excluir,
                                entidade: "backup_snapshot".to_string(),
                                entidade_id: Some(meta.id.0.clone()),
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
        }
        Err(e) => tracing::warn!("backup: could not list for retention: {e}"),
    }
}
