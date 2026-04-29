//! Global counters for hwmon chip-rescan recovery observability.
//!
//! Process-wide, lock-free. Monotonic — read `snapshot()` periodically
//! to compute deltas.

use std::sync::atomic::{AtomicU64, Ordering};

static RESCAN_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static RESCAN_SUCCESSES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_rescan_attempt() {
    RESCAN_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_rescan_success() {
    RESCAN_SUCCESSES.fetch_add(1, Ordering::Relaxed);
}

/// Point-in-time snapshot of hwmon recovery counters.
///
/// **Not atomic across fields** — the snapshot performs two independent
/// `Relaxed` loads, so a concurrent rescan happening mid-snapshot can produce
/// `attempts < successes` for an instant. Use the snapshot for trend analysis
/// (deltas across periodic samples), not for invariant assertions on a single
/// sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HwmonRecoveryCounters {
    /// Number of times with_recovery triggered a chip rescan.
    pub rescan_attempts: u64,
    /// Number of times a rescan + retry-op round-trip both succeeded.
    /// Subtract from `rescan_attempts` to get the count of unsuccessful
    /// recoveries (chip not found, name mismatched, or retry op failed).
    pub rescan_successes: u64,
}

pub fn snapshot() -> HwmonRecoveryCounters {
    HwmonRecoveryCounters {
        rescan_attempts: RESCAN_ATTEMPTS.load(Ordering::Relaxed),
        rescan_successes: RESCAN_SUCCESSES.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_monotonic() {
        let before = snapshot();
        record_rescan_attempt();
        let after = snapshot();
        assert!(after.rescan_attempts > before.rescan_attempts);
    }
}
