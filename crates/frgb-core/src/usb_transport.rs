use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::error::{CoreError, Result};
use crate::transport::{Transport, PACKET_SIZE};
use frgb_usb::device::UsbDevice;
use frgb_usb::error::UsbError;

/// After this many consecutive read timeouts without an intervening success,
/// the next read forces a reopen before attempting I/O. Reads on a stale
/// handle return timeouts indefinitely otherwise — writes catch stale-handle
/// conditions via their own recovery paths, but read-only code paths can't.
const STALE_READ_THRESHOLD: u32 = 3;

/// Reads with shorter timeouts are drain-loop idioms (polling a buffer that's
/// normally empty) and should NOT count toward the stale-handle signal.
/// Response-awaited reads in this codebase use timeouts >= 200ms.
const STALE_COUNT_MIN_TIMEOUT: Duration = Duration::from_millis(100);

/// Real USB transport wrapping frgb_usb::UsbDevice.
///
/// Writes are pass-through — recovery happens at the call-site layer
/// (rf_backend's with_tx_recovery / with_rx_recovery) to avoid mid-sequence
/// reopens that would invalidate state established by prior writes.
///
/// Reads track consecutive response-timeouts (reads with timeout >=
/// STALE_COUNT_MIN_TIMEOUT); after STALE_READ_THRESHOLD in a row, the next
/// read forces a reopen first. Drain-loop idioms that use very short
/// timeouts (1-5ms) to poll empty buffers are explicitly excluded from the
/// counter, since timeouts there are expected control flow.
pub struct UsbTransport {
    device: RefCell<UsbDevice>,
    consecutive_read_timeouts: Cell<u32>,
}

impl UsbTransport {
    /// Open a USB device by VID/PID and wrap it as a Transport.
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let device = UsbDevice::open(vid, pid)?;
        Ok(Self {
            device: RefCell::new(device),
            consecutive_read_timeouts: Cell::new(0),
        })
    }

    /// Wrap an already-opened UsbDevice.
    pub fn from_device(device: UsbDevice) -> Self {
        Self {
            device: RefCell::new(device),
            consecutive_read_timeouts: Cell::new(0),
        }
    }

    /// Re-open the underlying USB device, preserving its init path.
    pub fn reconnect(&mut self) -> Result<()> {
        self.device.borrow_mut().reopen().map_err(CoreError::from)?;
        self.consecutive_read_timeouts.set(0);
        Ok(())
    }
}

impl Transport for UsbTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        self.device.borrow().write(data).map_err(CoreError::from)
    }

    fn read(&self, timeout: Duration) -> Result<[u8; PACKET_SIZE]> {
        // If we've seen too many consecutive long-timeout reads time out,
        // the handle is likely stale. Try a reopen before the next read.
        if self.consecutive_read_timeouts.get() >= STALE_READ_THRESHOLD {
            tracing::warn!(
                "USB: {} consecutive read timeouts, attempting reopen",
                self.consecutive_read_timeouts.get()
            );
            if let Err(e) = self.device.borrow_mut().reopen() {
                tracing::warn!("USB: stale-read reopen failed: {e}");
            } else {
                tracing::info!("USB: reopened after stale reads");
            }
            self.consecutive_read_timeouts.set(0);
        }

        match self.device.borrow().read_timeout(timeout) {
            Ok(buf) => {
                self.consecutive_read_timeouts.set(0);
                Ok(buf)
            }
            Err(UsbError::Timeout) => {
                // Only count as stale-handle signal for response-awaited reads.
                // Short-timeout reads are drain-loop idioms (polling empty buffers).
                if timeout >= STALE_COUNT_MIN_TIMEOUT {
                    self.consecutive_read_timeouts
                        .set(self.consecutive_read_timeouts.get() + 1);
                }
                Err(UsbError::Timeout.into())
            }
            Err(e) => {
                self.consecutive_read_timeouts.set(0);
                Err(e.into())
            }
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn reconnect(&mut self) -> Result<()> {
        UsbTransport::reconnect(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_count_min_timeout_excludes_drain_reads() {
        // Drain-loop timeouts in rf_backend: 1ms, 5ms.
        assert!(Duration::from_millis(1) < STALE_COUNT_MIN_TIMEOUT);
        assert!(Duration::from_millis(5) < STALE_COUNT_MIN_TIMEOUT);
        // Response-awaited reads: 200ms+ (bind_device uses 500ms).
        assert!(Duration::from_millis(200) >= STALE_COUNT_MIN_TIMEOUT);
        assert!(Duration::from_millis(500) >= STALE_COUNT_MIN_TIMEOUT);
    }
}
