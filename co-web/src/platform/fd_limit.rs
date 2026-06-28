//! CO-496 (tier-3 scalability): file-descriptor budget for the per-universe
//! connection pool.
//!
//! The [`UniversePool`](crate::universe_pool::UniversePool) holds one open SQLite
//! connection per hot universe; each WAL connection also opens `-wal`/`-shm`
//! sidecars, so the pool's fd appetite scales with its capacity. On a box with a
//! low `RLIMIT_NOFILE` (the common Linux default is 1024) a 1000-slot pool would
//! exhaust the descriptor table and fail unrelated `accept()`/`open()` calls.
//!
//! At boot we (a) raise the soft limit toward the hard limit, then (b) size the
//! pool from whatever soft limit we end up with — capped by an explicit
//! `CO_UNIVERSE_POOL_CAPACITY` override when set.

/// A conservative cap when raising the soft limit toward an unbounded hard limit
/// (macOS reports `RLIM_INFINITY` as the hard limit but rejects setting the soft
/// limit above `kern.maxfilesperproc`; 65 536 stays well under it everywhere).
const NOFILE_TARGET_CAP: u64 = 65_536;

/// Raise the soft `RLIMIT_NOFILE` toward the hard limit (bounded by
/// [`NOFILE_TARGET_CAP`]) and return the resulting `(soft, hard)`.
///
/// Best-effort: a failure to read or set the limit returns `None` and is logged,
/// never fatal — the pool then falls back to its default capacity.
#[cfg(unix)]
pub fn raise_nofile() -> Option<(u64, u64)> {
    use nix::sys::resource::{Resource, getrlimit, setrlimit};

    let (soft, hard) = getrlimit(Resource::RLIMIT_NOFILE)
        .map_err(|e| tracing::warn!(error = %e, "CO-496: getrlimit(RLIMIT_NOFILE) failed"))
        .ok()?;

    let target = hard.min(NOFILE_TARGET_CAP);
    if soft < target {
        if let Err(e) = setrlimit(Resource::RLIMIT_NOFILE, target, hard) {
            tracing::warn!(error = %e, soft, hard, target, "CO-496: setrlimit(RLIMIT_NOFILE) failed");
        } else {
            tracing::info!(
                from = soft,
                to = target,
                hard,
                "CO-496: raised RLIMIT_NOFILE soft limit"
            );
        }
    }

    getrlimit(Resource::RLIMIT_NOFILE).ok()
}

/// Non-unix fallback: no descriptor limit to manage.
#[cfg(not(unix))]
pub fn raise_nofile() -> Option<(u64, u64)> {
    None
}

/// Decide the universe-pool capacity.
///
/// Precedence: an explicit `CO_UNIVERSE_POOL_CAPACITY` (`env_override`) always
/// wins. Otherwise the capacity is bounded by the fd budget: each connection can
/// hold ~3 descriptors (db + `-wal` + `-shm`), so we let the pool use up to a
/// third of the soft limit, never below a 64-slot floor and never above
/// `default_cap`. With no limit info we fall back to `default_cap`. Pure →
/// unit-testable.
pub fn pool_capacity(
    env_override: Option<usize>,
    soft_nofile: Option<u64>,
    default_cap: usize,
) -> usize {
    if let Some(n) = env_override.filter(|&n| n > 0) {
        return n;
    }
    match soft_nofile {
        Some(soft) => {
            let budget = (soft / 3) as usize;
            default_cap.min(budget.max(64))
        }
        None => default_cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_over_fd_budget() {
        // Explicit override is honored regardless of fd limit / default.
        assert_eq!(pool_capacity(Some(2000), Some(1024), 1000), 2000);
        // Zero override is ignored (falls through to fd budget).
        assert_eq!(pool_capacity(Some(0), Some(1024), 1000), 341);
    }

    #[test]
    fn low_fd_limit_shrinks_pool() {
        // soft=1024 → budget 341 → min(1000, 341) = 341 (avoids fd exhaustion).
        assert_eq!(pool_capacity(None, Some(1024), 1000), 341);
        // A very low limit still keeps the 64-slot floor.
        assert_eq!(pool_capacity(None, Some(90), 1000), 64);
    }

    #[test]
    fn high_fd_limit_keeps_default_cap() {
        // soft=65536 → budget 21845 → capped to default 1000.
        assert_eq!(pool_capacity(None, Some(65_536), 1000), 1000);
    }

    #[test]
    fn no_limit_info_falls_back_to_default() {
        assert_eq!(pool_capacity(None, None, 1000), 1000);
    }
}
