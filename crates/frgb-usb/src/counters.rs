//! Global counters for USB/HID stale-handle recovery observability.
//!
//! Process-wide, lock-free. Each counter is monotonic — read `snapshot()`
//! periodically to compute deltas.
//!
//! Reopen counters are incremented in `UsbDevice::reopen` and `HidDevice::reopen`
//! themselves, so every reopen call site (RF transport, USB stale-read auto-reopen,
//! LCD/AURA `try_reconnect`, ENE `with_recovery`) is captured uniformly.

use std::sync::atomic::{AtomicU64, Ordering};

static REOPEN_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static REOPEN_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static REOPEN_FAILURES: AtomicU64 = AtomicU64::new(0);
static SOFT_RECOVERY_SUCCESSES: AtomicU64 = AtomicU64::new(0);

pub fn record_reopen_attempt() {
    REOPEN_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_reopen_success() {
    REOPEN_SUCCESSES.fetch_add(1, Ordering::Relaxed);
}

pub fn record_reopen_failure() {
    REOPEN_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_soft_recovery_success() {
    SOFT_RECOVERY_SUCCESSES.fetch_add(1, Ordering::Relaxed);
}

/// Point-in-time snapshot of all USB recovery counters.
///
/// **Not atomic across fields** — the snapshot performs three independent
/// `Relaxed` loads, so a concurrent reopen happening mid-snapshot can produce
/// `attempts < successes + failures` for an instant. Use the snapshot for
/// trend analysis (deltas across periodic samples), not for invariant
/// assertions on a single sample.
#[derive(Debug, Clone, Copy)]
pub struct RecoveryCounters {
    /// Number of times `UsbDevice::reopen` or `HidDevice::reopen` was called.
    /// Captures every reopen path: RF transport, stale-read auto-reopen,
    /// LCD/AURA reconnect, and ENE `with_recovery`.
    pub reopen_attempts: u64,
    /// Number of times the reopen syscall returned Ok. Reflects kernel/USB-stack
    /// recovery success — does *not* imply that a subsequent retry op succeeded.
    pub reopen_successes: u64,
    /// Number of times the reopen syscall returned Err. Reflects kernel/USB-stack
    /// failure (device truly absent, permission lost, etc.). Modulo the
    /// snapshot-atomicity caveat: `attempts == successes + failures`.
    pub reopen_failures: u64,
    /// Number of times soft recovery (e.g. clear_halt_out) preempted the need
    /// for a full reopen *and* the subsequent retry op also succeeded —
    /// i.e., a complete soft-recovery round-trip.
    pub soft_recovery_successes: u64,
}

pub fn snapshot() -> RecoveryCounters {
    RecoveryCounters {
        reopen_attempts: REOPEN_ATTEMPTS.load(Ordering::Relaxed),
        reopen_successes: REOPEN_SUCCESSES.load(Ordering::Relaxed),
        reopen_failures: REOPEN_FAILURES.load(Ordering::Relaxed),
        soft_recovery_successes: SOFT_RECOVERY_SUCCESSES.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_monotonic() {
        let before = snapshot();
        record_reopen_attempt();
        let after = snapshot();
        assert!(after.reopen_attempts > before.reopen_attempts);
    }
}
