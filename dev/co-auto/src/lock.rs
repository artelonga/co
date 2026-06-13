//! CO-54 — per-task lockfile so two co-auto agents (e.g. two worktrees) never
//! execute the same task concurrently (Scenario 4).
//!
//! Each task gets a lockfile at `<data_dir>/.co-auto/locks/<TASK-KEY>.lock`.
//! Acquisition is atomic via `O_CREAT | O_EXCL` (`create_new(true)`): the OS
//! guarantees exactly one creator wins the race. A second agent that finds the
//! file already present **skips** that task and moves on to the next candidate.
//!
//! Crash recovery: the lockfile records the acquisition time. A lock older than
//! [`LOCK_TTL_SECS`] (30 minutes) is considered **stale** — its owner crashed or
//! hung — and is reclaimed by the next agent that tries to acquire it.
//!
//! The lock is released by dropping [`TaskLock`] (RAII), which removes the file.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Locks older than this (seconds) are stale and may be reclaimed — covers a
/// crashed/hung agent that never released its lock. 30 minutes.
pub const LOCK_TTL_SECS: u64 = 30 * 60;

/// An acquired per-task lock. Dropping it releases (deletes) the lockfile.
#[derive(Debug)]
pub struct TaskLock {
    path: PathBuf,
}

impl TaskLock {
    /// Try to acquire the lock for `task_key` using the current wall clock.
    ///
    /// * `Ok(Some(lock))` — acquired (freshly, or by reclaiming a stale lock).
    /// * `Ok(None)` — already held by a live agent; the caller should skip it.
    pub fn try_acquire(lock_dir: &Path, task_key: &str) -> io::Result<Option<TaskLock>> {
        Self::try_acquire_at(lock_dir, task_key, now_secs())
    }

    /// Clock-injected core of [`TaskLock::try_acquire`] — deterministic for
    /// tests (no real-time dependency).
    pub fn try_acquire_at(
        lock_dir: &Path,
        task_key: &str,
        now: u64,
    ) -> io::Result<Option<TaskLock>> {
        fs::create_dir_all(lock_dir)?;
        let path = lock_dir.join(format!("{task_key}.lock"));

        match Self::create_exclusive(&path, now) {
            Ok(()) => Ok(Some(TaskLock { path })),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Held — reclaim only if the existing lock is stale.
                if Self::is_stale(&path, now) {
                    // Best-effort reclaim: remove the stale file, then re-create.
                    // If another agent reclaims first, our create_new races and
                    // we correctly back off (skip).
                    let _ = fs::remove_file(&path);
                    match Self::create_exclusive(&path, now) {
                        Ok(()) => Ok(Some(TaskLock { path })),
                        Err(e2) if e2.kind() == io::ErrorKind::AlreadyExists => Ok(None),
                        Err(e2) => Err(e2),
                    }
                } else {
                    Ok(None)
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Atomically create the lockfile, failing if it already exists. Records the
    /// acquisition timestamp (line 1) and pid (line 2) for diagnostics.
    fn create_exclusive(path: &Path, now: u64) -> io::Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
        writeln!(f, "{now}")?;
        writeln!(f, "{}", std::process::id())?;
        Ok(())
    }

    /// A lock is stale when its recorded acquisition time is older than the TTL.
    /// An unreadable / unparseable lockfile is treated as stale (safe: it lets a
    /// healthy agent reclaim a corrupt lock rather than deadlocking forever).
    fn is_stale(path: &Path, now: u64) -> bool {
        match fs::read_to_string(path) {
            Ok(contents) => match contents
                .lines()
                .next()
                .and_then(|l| l.trim().parse::<u64>().ok())
            {
                Some(acquired) => now.saturating_sub(acquired) >= LOCK_TTL_SECS,
                None => true,
            },
            Err(_) => true,
        }
    }
}

impl Drop for TaskLock {
    fn drop(&mut self) {
        // Best-effort release — a failed delete just leaves a lock that the next
        // agent will reclaim once it ages past the TTL.
        let _ = fs::remove_file(&self.path);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_lock_dir(label: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("co-auto-lock-{label}-{ts}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_acquire_succeeds() {
        let dir = tmp_lock_dir("first");
        let lock = TaskLock::try_acquire_at(&dir, "CO-42", 1_000).unwrap();
        assert!(lock.is_some());
    }

    #[test]
    fn second_agent_skips_locked_task() {
        let dir = tmp_lock_dir("contend");
        let _held = TaskLock::try_acquire_at(&dir, "CO-42", 1_000)
            .unwrap()
            .unwrap();
        // A second agent at (almost) the same time must be turned away.
        let second = TaskLock::try_acquire_at(&dir, "CO-42", 1_050).unwrap();
        assert!(second.is_none(), "second agent must skip a live lock");
    }

    #[test]
    fn release_on_drop_allows_reacquire() {
        let dir = tmp_lock_dir("release");
        {
            let _held = TaskLock::try_acquire_at(&dir, "CO-42", 1_000)
                .unwrap()
                .unwrap();
            assert!(
                TaskLock::try_acquire_at(&dir, "CO-42", 1_000)
                    .unwrap()
                    .is_none()
            );
        } // _held dropped here → lock released
        let after = TaskLock::try_acquire_at(&dir, "CO-42", 1_000).unwrap();
        assert!(after.is_some(), "lock must be reacquirable after release");
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let dir = tmp_lock_dir("stale");
        // Acquire at t=1000 but never release (simulated crash) — leak the guard.
        std::mem::forget(
            TaskLock::try_acquire_at(&dir, "CO-42", 1_000)
                .unwrap()
                .unwrap(),
        );
        // Just before the TTL: still held.
        assert!(
            TaskLock::try_acquire_at(&dir, "CO-42", 1_000 + LOCK_TTL_SECS - 1)
                .unwrap()
                .is_none()
        );
        // Past the TTL: reclaimable.
        let reclaimed = TaskLock::try_acquire_at(&dir, "CO-42", 1_000 + LOCK_TTL_SECS).unwrap();
        assert!(
            reclaimed.is_some(),
            "stale lock must be reclaimable past TTL"
        );
    }

    #[test]
    fn distinct_tasks_lock_independently() {
        let dir = tmp_lock_dir("distinct");
        let a = TaskLock::try_acquire_at(&dir, "CO-1", 1_000).unwrap();
        let b = TaskLock::try_acquire_at(&dir, "CO-2", 1_000).unwrap();
        assert!(a.is_some() && b.is_some(), "different tasks don't contend");
    }
}
