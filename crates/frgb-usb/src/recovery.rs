//! Retry-with-reopen wrapper for transient USB handle staleness.
//!
//! USB handles go stale on suspend/resume, hub reset, and transient unplug.
//! `with_recovery` runs an operation; on failure it reopens the handle (subject
//! to a cooldown) and retries once. The cooldown prevents reconnect storms
//! when the device is genuinely absent.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::counters;
use crate::error::{Result, UsbError};

const COOLDOWN: Duration = Duration::from_secs(5);

/// A USB/HID handle that can be reopened in place after going stale.
pub trait Reopenable {
    fn reopen(&mut self) -> Result<()>;
    fn label(&self) -> String;

    /// Attempt a lightweight recovery (e.g. USB endpoint clear-halt) before
    /// resorting to a full reopen. Returns Ok(true) on success, Ok(false) if
    /// soft recovery isn't applicable for this handle type, Err on failure.
    /// Default: no-op (not applicable).
    fn try_soft_recover(&self) -> Result<bool> {
        Ok(false)
    }
}

/// Run `op`; on failure, reopen the handle (subject to cooldown) and retry once.
///
/// Timeouts are returned without retry — they typically indicate a slow device,
/// not a stale handle, and reopening would mask real protocol issues.
///
/// Reopen counters (`reopen_attempts/successes/failures`) live inside
/// `Reopenable::reopen` itself, so any caller of `reopen` — not just this
/// helper — is captured. `soft_recovery_successes` is incremented here only
/// when the soft path *and* the retry op both succeed (round-trip semantic).
pub fn with_recovery<H, T, F>(
    handle: &RefCell<H>,
    cooldown: &Cell<Option<Instant>>,
    op: F,
) -> Result<T>
where
    H: Reopenable,
    F: Fn(&H) -> Result<T>,
{
    let first = op(&handle.borrow());
    match first {
        Ok(v) => Ok(v),
        Err(UsbError::Timeout) => Err(UsbError::Timeout),
        Err(e) => {
            if let Some(last) = cooldown.get() {
                if last.elapsed() < COOLDOWN {
                    return Err(e);
                }
            }
            cooldown.set(Some(Instant::now()));
            let label = handle.borrow().label();

            // Try soft recovery first (e.g. USB endpoint clear-halt).
            match handle.borrow().try_soft_recover() {
                Ok(true) => {
                    tracing::info!("{label}: soft recovery succeeded, retrying");
                    let retry_result = op(&handle.borrow());
                    if retry_result.is_ok() {
                        counters::record_soft_recovery_success();
                    }
                    return retry_result;
                }
                Ok(false) => { /* fall through to full reopen */ }
                Err(se) => {
                    tracing::debug!("{label}: soft recovery errored ({se}); falling back to reopen");
                }
            }

            tracing::warn!("{label}: op failed ({e}); reopening");
            if let Err(reopen_err) = handle.borrow_mut().reopen() {
                tracing::warn!("{label}: reopen failed ({reopen_err}); returning original error");
                return Err(e);
            }
            tracing::info!("{label}: reopened, retrying");
            op(&handle.borrow())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock {
        reopen_calls: Cell<u32>,
    }

    impl Mock {
        fn new() -> Self {
            Self { reopen_calls: Cell::new(0) }
        }
    }

    impl Reopenable for Mock {
        fn reopen(&mut self) -> Result<()> {
            self.reopen_calls.set(self.reopen_calls.get() + 1);
            Ok(())
        }
        fn label(&self) -> String {
            "mock".into()
        }
    }

    #[test]
    fn success_does_not_reopen() {
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(None);
        let r: Result<u32> = with_recovery(&h, &cd, |_| Ok(42));
        assert_eq!(r.unwrap(), 42);
        assert_eq!(h.borrow().reopen_calls.get(), 0);
        assert!(cd.get().is_none());
    }

    #[test]
    fn failure_then_success_after_reopen() {
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let r: Result<&'static str> = with_recovery(&h, &cd, |_| {
            let n = attempts.get();
            attempts.set(n + 1);
            if n == 0 {
                Err(UsbError::Io("transient".into()))
            } else {
                Ok("ok")
            }
        });
        assert_eq!(r.unwrap(), "ok");
        assert_eq!(h.borrow().reopen_calls.get(), 1);
        assert!(cd.get().is_some());
    }

    #[test]
    fn timeout_is_not_retried() {
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let r: Result<()> = with_recovery(&h, &cd, |_| {
            attempts.set(attempts.get() + 1);
            Err(UsbError::Timeout)
        });
        assert!(matches!(r, Err(UsbError::Timeout)));
        assert_eq!(attempts.get(), 1);
        assert_eq!(h.borrow().reopen_calls.get(), 0);
    }

    #[test]
    fn cooldown_blocks_second_reopen() {
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(Some(Instant::now()));
        let r: Result<()> = with_recovery(&h, &cd, |_| Err(UsbError::Io("x".into())));
        assert!(r.is_err());
        assert_eq!(h.borrow().reopen_calls.get(), 0);
    }

    #[test]
    fn reopen_failure_returns_original_error() {
        struct BadMock;
        impl Reopenable for BadMock {
            fn reopen(&mut self) -> Result<()> {
                Err(UsbError::NotFound)
            }
            fn label(&self) -> String {
                "bad".into()
            }
        }
        let h = RefCell::new(BadMock);
        let cd = Cell::new(None);
        let r: Result<()> = with_recovery(&h, &cd, |_| Err(UsbError::Io("x".into())));
        match r {
            Err(UsbError::Io(msg)) => assert_eq!(msg, "x"),
            other => panic!("expected Io('x'), got {other:?}"),
        }
    }

    #[test]
    fn op_is_called_twice_on_failure_path() {
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let _: Result<()> = with_recovery(&h, &cd, |_| {
            attempts.set(attempts.get() + 1);
            Err(UsbError::Io("stale".into()))
        });
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn original_error_preserved_when_reopen_fails() {
        struct BadMock {
            reopen_attempted: Cell<bool>,
        }
        impl Reopenable for BadMock {
            fn reopen(&mut self) -> Result<()> {
                self.reopen_attempted.set(true);
                Err(UsbError::NotFound)
            }
            fn label(&self) -> String {
                "bad".into()
            }
        }
        let h = RefCell::new(BadMock { reopen_attempted: Cell::new(false) });
        let cd = Cell::new(None);
        let r: Result<()> = with_recovery(&h, &cd, |_| Err(UsbError::Io("original-failure".into())));
        // Expect the original op error, not the NotFound from reopen
        match r {
            Err(UsbError::Io(msg)) => assert_eq!(msg, "original-failure"),
            other => panic!("expected Io('original-failure'), got {other:?}"),
        }
        assert!(h.borrow().reopen_attempted.get(), "reopen should have been attempted");
    }

    #[test]
    fn soft_recovery_preempts_full_reopen() {
        struct SoftMock {
            soft_tried: Cell<bool>,
            reopen_calls: Cell<u32>,
        }
        impl Reopenable for SoftMock {
            fn reopen(&mut self) -> Result<()> {
                self.reopen_calls.set(self.reopen_calls.get() + 1);
                Ok(())
            }
            fn label(&self) -> String {
                "soft".into()
            }
            fn try_soft_recover(&self) -> Result<bool> {
                self.soft_tried.set(true);
                Ok(true) // soft recovery succeeds
            }
        }
        let h = RefCell::new(SoftMock {
            soft_tried: Cell::new(false),
            reopen_calls: Cell::new(0),
        });
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let r: Result<&'static str> = with_recovery(&h, &cd, |_| {
            let n = attempts.get();
            attempts.set(n + 1);
            if n == 0 {
                Err(UsbError::Io("x".into()))
            } else {
                Ok("ok")
            }
        });
        assert_eq!(r.unwrap(), "ok");
        assert!(
            h.borrow().soft_tried.get(),
            "soft recovery should be attempted"
        );
        assert_eq!(
            h.borrow().reopen_calls.get(),
            0,
            "full reopen should not run if soft recovery succeeded"
        );
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn falls_back_to_full_reopen_when_soft_unavailable() {
        // Mock returns Ok(false) from try_soft_recover — should fall through to reopen.
        let h = RefCell::new(Mock::new());
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let r: Result<&'static str> = with_recovery(&h, &cd, |_| {
            let n = attempts.get();
            attempts.set(n + 1);
            if n == 0 {
                Err(UsbError::Io("x".into()))
            } else {
                Ok("ok")
            }
        });
        assert_eq!(r.unwrap(), "ok");
        assert_eq!(
            h.borrow().reopen_calls.get(),
            1,
            "fell back to full reopen"
        );
    }

    #[test]
    fn soft_recovery_error_falls_through_to_reopen() {
        struct ErroringSoftMock {
            reopen_calls: Cell<u32>,
        }
        impl Reopenable for ErroringSoftMock {
            fn reopen(&mut self) -> Result<()> {
                self.reopen_calls.set(self.reopen_calls.get() + 1);
                Ok(())
            }
            fn label(&self) -> String { "err-soft".into() }
            fn try_soft_recover(&self) -> Result<bool> {
                Err(UsbError::Io("soft-failed".into()))
            }
        }
        let h = RefCell::new(ErroringSoftMock { reopen_calls: Cell::new(0) });
        let cd = Cell::new(None);
        let attempts = Cell::new(0u32);
        let r: Result<&'static str> = with_recovery(&h, &cd, |_| {
            let n = attempts.get();
            attempts.set(n + 1);
            if n == 0 { Err(UsbError::Io("x".into())) } else { Ok("ok") }
        });
        assert_eq!(r.unwrap(), "ok", "op should succeed after fall-through to reopen");
        assert_eq!(h.borrow().reopen_calls.get(), 1, "fell through to full reopen");
        assert_eq!(attempts.get(), 2);
    }
}
