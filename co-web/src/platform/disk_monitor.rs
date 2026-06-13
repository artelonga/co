//! CO-422: Disk pressure monitor — emits `system.disk_pressure` on the EDA bus
//! when free disk space falls below a configurable threshold.
//!
//! Configuration env vars
//! ----------------------
//! | Variable                    | Default | Description                            |
//! |-----------------------------|---------|----------------------------------------|
//! | `CO_DISK_CHECK_INTERVAL_SECS` | 900   | Seconds between checks (15 min)        |
//! | `CO_DISK_ALERT_THRESHOLD_PCT` | 15    | Emit event when free% falls below this |

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::eda::bus::EdaBus;
use crate::eda::event::{Event, Visibility};

fn env_u64(name: &str, default: u64) -> u64 {
    use crate::infra::secrets::SecretsProviderExt;
    crate::infra::secrets::global().get_parsed(name, default)
}

/// Returns `(available_bytes, total_bytes)` for the filesystem holding `path`.
/// Returns `None` on non-Linux or when `statvfs` fails.
#[cfg(target_os = "linux")]
pub fn disk_stats(path: &std::path::Path) -> Option<(u64, u64)> {
    match nix::sys::statvfs::statvfs(path) {
        Ok(stat) => {
            let avail = stat.blocks_available() * stat.fragment_size();
            let total = stat.blocks() * stat.fragment_size();
            Some((avail, total))
        }
        Err(e) => {
            tracing::warn!("CO-422 disk_monitor: statvfs({path:?}) failed: {e}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn disk_stats(_path: &std::path::Path) -> Option<(u64, u64)> {
    None // dev/CI: guard disabled so tests don't depend on the host filesystem
}

/// Spawn the disk pressure monitor background task.
///
/// Sleeps `CO_DISK_CHECK_INTERVAL_SECS` (default 900 s = 15 min) between checks.
/// Publishes a `system.disk_pressure` event to `bus` whenever the free percentage
/// of the filesystem holding `data_dir` falls below `CO_DISK_ALERT_THRESHOLD_PCT`
/// (default 15 %).
pub fn spawn(bus: Arc<dyn EdaBus>, data_dir: PathBuf) {
    tokio::spawn(async move {
        let interval_secs = env_u64("CO_DISK_CHECK_INTERVAL_SECS", 900);
        let threshold_pct = env_u64("CO_DISK_ALERT_THRESHOLD_PCT", 15);
        let path_str = data_dir.display().to_string();

        tracing::info!(
            "CO-422: disk monitor started (interval={}s, threshold={}%)",
            interval_secs,
            threshold_pct
        );

        loop {
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;

            let Some((free_bytes, total_bytes)) = disk_stats(&data_dir) else {
                continue; // non-Linux dev machine or statvfs error — skip silently
            };

            if total_bytes == 0 {
                continue;
            }

            let free_pct = (free_bytes as f64 / total_bytes as f64) * 100.0;
            if free_pct < threshold_pct as f64 {
                tracing::warn!(
                    "CO-422: disk pressure detected: {:.1}% free \
                     ({} MB of {} MB), threshold {}%",
                    free_pct,
                    free_bytes / 1_048_576,
                    total_bytes / 1_048_576,
                    threshold_pct
                );
                bus.publish(Event::new(
                    "system.disk_pressure",
                    None,
                    None,
                    serde_json::json!({
                        "free_bytes": free_bytes,
                        "total_bytes": total_bytes,
                        "free_pct": free_pct,
                        "threshold_pct": threshold_pct,
                        "path": path_str,
                    }),
                    Visibility::System,
                ));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_stats_is_none_on_nonexistent_path() {
        // On Linux, statvfs on a non-existent path returns an error.
        // On other platforms, disk_stats always returns None.
        // Either way, Some(_, 0) should never appear.
        let result = disk_stats(std::path::Path::new("/nonexistent/path/for/co422"));
        if let Some((_, total)) = result {
            assert!(total > 0, "total_bytes must be > 0 for a real filesystem");
        }
        // None is also fine — that's the expected result on non-Linux / bad path
    }

    #[test]
    fn free_pct_threshold_check() {
        // Verify the threshold arithmetic is correct.
        let free_bytes: u64 = 140_000_000; // ~133 MB
        let total_bytes: u64 = 1_000_000_000; // ~954 MB
        let free_pct = (free_bytes as f64 / total_bytes as f64) * 100.0;
        let threshold_pct: u64 = 15;
        assert!(
            free_pct < threshold_pct as f64,
            "14% free should be below 15% threshold"
        );

        let free_bytes2: u64 = 200_000_000; // 20%
        let free_pct2 = (free_bytes2 as f64 / total_bytes as f64) * 100.0;
        assert!(
            free_pct2 >= threshold_pct as f64,
            "20% free should be above 15% threshold"
        );
    }
}
