use crate::error::{Result, UsbError};
use crate::recovery::Reopenable;
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;

const TIMEOUT_FIRST: Duration = Duration::from_millis(1000);
const TIMEOUT_READ: Duration = Duration::from_millis(200);
const PACKET_SIZE: usize = 64;
const LCD_PACKET_SIZE: usize = 512;
const LCD_WRITE_TIMEOUT: Duration = Duration::from_millis(10_000);

const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

pub struct UsbDevice {
    handle: DeviceHandle<GlobalContext>,
    vid: u16,
    pid: u16,
    interface: u8,
    /// True if EP_IN uses interrupt transfers, false for bulk.
    ep_in_interrupt: bool,
    /// True if EP_OUT uses interrupt transfers, false for bulk.
    ep_out_interrupt: bool,
    /// True if the handle was created via `open_with_reset`. `reopen` uses
    /// the same path so device firmware comes up in the expected state.
    opened_with_reset: bool,
}

/// Detect endpoint transfer types from USB descriptors.
fn detect_endpoint_types(device: &Device<GlobalContext>) -> (bool, bool) {
    let config = match device.active_config_descriptor() {
        Ok(c) => c,
        Err(_) => return (false, false),
    };
    let mut in_interrupt = false;
    let mut out_interrupt = false;
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            for ep in desc.endpoint_descriptors() {
                if ep.address() == EP_IN && ep.transfer_type() == rusb::TransferType::Interrupt {
                    in_interrupt = true;
                }
                if ep.address() == EP_OUT && ep.transfer_type() == rusb::TransferType::Interrupt {
                    out_interrupt = true;
                }
            }
        }
    }
    tracing::debug!(
        "Endpoint types: IN=0x{:02x} {}, OUT=0x{:02x} {}",
        EP_IN,
        if in_interrupt { "interrupt" } else { "bulk" },
        EP_OUT,
        if out_interrupt { "interrupt" } else { "bulk" },
    );
    (in_interrupt, out_interrupt)
}

impl UsbDevice {
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let device = rusb::devices()?
            .iter()
            .find(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
            .ok_or(UsbError::NotFound)?;

        let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
        let handle = device.open()?;
        let interface = 0;

        if handle.kernel_driver_active(interface).unwrap_or(false) {
            handle.detach_kernel_driver(interface)?;
        }

        match handle.set_active_configuration(1) {
            Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
            Err(rusb::Error::Io) => {
                // I/O error on configuration — try USB reset + retry
                // (matches reference project detach_and_configure)
                tracing::warn!("USB config I/O error, attempting reset");
                handle.reset()?;
                std::thread::sleep(Duration::from_millis(500));
                match handle.set_active_configuration(1) {
                    Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) => return Err(e.into()),
        }

        match handle.claim_interface(interface) {
            Ok(()) => {
                let _ = handle.set_alternate_setting(interface, 0);
            }
            Err(rusb::Error::Busy) => {
                tracing::warn!("USB interface busy, attempting reset");
                handle.reset()?;
                std::thread::sleep(Duration::from_millis(500));
                handle.claim_interface(interface)?;
                let _ = handle.set_alternate_setting(interface, 0);
            }
            Err(e) => return Err(e.into()),
        }

        Ok(Self {
            handle,
            vid,
            pid,
            interface,
            ep_in_interrupt,
            ep_out_interrupt,
            opened_with_reset: false,
        })
    }

    /// Open with USB reset before configuration — matches the Python LCD init
    /// sequence: reset → detach interfaces 0+1 → set_configuration → claim 0.
    pub fn open_with_reset(vid: u16, pid: u16) -> Result<Self> {
        let device = rusb::devices()?
            .iter()
            .find(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
            .ok_or(UsbError::NotFound)?;

        let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
        let handle = device.open()?;

        // Reset first — puts firmware in a known state
        let _ = handle.reset();
        std::thread::sleep(Duration::from_millis(100));

        // Detach kernel drivers from interfaces 0 and 1
        for iface in 0..2u8 {
            if handle.kernel_driver_active(iface).unwrap_or(false) {
                let _ = handle.detach_kernel_driver(iface);
            }
        }

        // Configure and claim
        let _ = handle.set_active_configuration(1);
        handle.claim_interface(0)?;
        let _ = handle.set_alternate_setting(0, 0);

        Ok(Self {
            handle,
            vid,
            pid,
            interface: 0,
            ep_in_interrupt,
            ep_out_interrupt,
            opened_with_reset: true,
        })
    }

    /// Open all devices with reset — for LCD devices that need a USB reset
    /// before init to clear firmware state.
    pub fn open_all_with_reset(vid: u16, pid: u16) -> Vec<Self> {
        let Ok(devices) = rusb::devices() else {
            return Vec::new();
        };
        devices
            .iter()
            .filter(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
            .filter_map(|device| {
                let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
                let handle = device.open().ok()?;

                let _ = handle.reset();
                std::thread::sleep(Duration::from_millis(100));

                for iface in 0..2u8 {
                    if handle.kernel_driver_active(iface).unwrap_or(false) {
                        let _ = handle.detach_kernel_driver(iface);
                    }
                }

                let _ = handle.set_active_configuration(1);
                handle.claim_interface(0).ok()?;
                let _ = handle.set_alternate_setting(0, 0);

                Some(Self {
                    handle,
                    vid,
                    pid,
                    interface: 0,
                    ep_in_interrupt,
                    ep_out_interrupt,
                    opened_with_reset: true,
                })
            })
            .collect()
    }

    /// Write data using the correct transfer type (bulk or interrupt).
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let timeout = if data.len() > PACKET_SIZE {
            LCD_WRITE_TIMEOUT
        } else {
            TIMEOUT_FIRST
        };
        if self.ep_out_interrupt {
            self.handle.write_interrupt(EP_OUT, data, timeout)?;
        } else {
            self.handle.write_bulk(EP_OUT, data, timeout)?;
        }
        Ok(())
    }

    /// Read using the correct transfer type, 64-byte RF packet.
    pub fn read(&self) -> Result<[u8; PACKET_SIZE]> {
        let mut buf = [0u8; PACKET_SIZE];
        if self.ep_in_interrupt {
            self.handle.read_interrupt(EP_IN, &mut buf, TIMEOUT_READ)?;
        } else {
            self.handle.read_bulk(EP_IN, &mut buf, TIMEOUT_READ)?;
        }
        Ok(buf)
    }

    pub fn read_timeout(&self, timeout: Duration) -> Result<[u8; PACKET_SIZE]> {
        let mut buf = [0u8; PACKET_SIZE];
        if self.ep_in_interrupt {
            self.handle.read_interrupt(EP_IN, &mut buf, timeout)?;
        } else {
            self.handle.read_bulk(EP_IN, &mut buf, timeout)?;
        }
        Ok(buf)
    }

    /// Read a 512-byte LCD response packet.
    pub fn read_lcd(&self) -> Result<[u8; LCD_PACKET_SIZE]> {
        let mut buf = [0u8; LCD_PACKET_SIZE];
        if self.ep_in_interrupt {
            self.handle.read_interrupt(EP_IN, &mut buf, TIMEOUT_FIRST)?;
        } else {
            self.handle.read_bulk(EP_IN, &mut buf, TIMEOUT_FIRST)?;
        }
        Ok(buf)
    }

    /// Read a 512-byte LCD response with custom timeout.
    pub fn read_lcd_timeout(&self, timeout: Duration) -> Result<[u8; LCD_PACKET_SIZE]> {
        let mut buf = [0u8; LCD_PACKET_SIZE];
        if self.ep_in_interrupt {
            self.handle.read_interrupt(EP_IN, &mut buf, timeout)?;
        } else {
            self.handle.read_bulk(EP_IN, &mut buf, timeout)?;
        }
        Ok(buf)
    }

    /// Detach kernel driver from a specific interface (no-op if not active).
    pub fn detach_kernel_driver(&self, interface: u8) {
        if self.handle.kernel_driver_active(interface).unwrap_or(false) {
            let _ = self.handle.detach_kernel_driver(interface);
        }
    }

    /// Re-do set_configuration + claim_interface after a USB reset.
    pub fn reconfigure_and_claim(&self) -> Result<()> {
        match self.handle.set_active_configuration(1) {
            Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        self.handle.claim_interface(self.interface)?;
        let _ = self.handle.set_alternate_setting(self.interface, 0);
        Ok(())
    }

    pub fn reset(&self) -> Result<()> {
        self.handle.reset()?;
        Ok(())
    }

    /// Clear a HALT/stall condition on the output endpoint.
    /// Lighter recovery than a full device reset — resumes a stalled
    /// bulk/interrupt pipe without re-enumerating.
    pub fn clear_halt_out(&self) -> Result<()> {
        self.handle.clear_halt(EP_OUT)?;
        Ok(())
    }

    pub fn vid(&self) -> u16 {
        self.vid
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }

    pub fn open_all(vid: u16, pid: u16) -> Vec<Self> {
        let Ok(devices) = rusb::devices() else {
            return Vec::new();
        };
        devices
            .iter()
            .filter(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
            .filter_map(|device| {
                let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
                let handle = device.open().ok()?;
                let interface = 0;
                if handle.kernel_driver_active(interface).unwrap_or(false) {
                    handle.detach_kernel_driver(interface).ok()?;
                }
                match handle.set_active_configuration(1) {
                    Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
                    Err(_) => return None,
                }
                handle.claim_interface(interface).ok()?;
                let _ = handle.set_alternate_setting(interface, 0);
                Some(Self {
                    handle,
                    vid,
                    pid,
                    interface,
                    ep_in_interrupt,
                    ep_out_interrupt,
                    opened_with_reset: false,
                })
            })
            .collect()
    }

    /// Re-acquire the same USB device by VID/PID, preserving the init path
    /// (plain open vs reset-then-open) that was used originally.
    ///
    /// Releases the old handle's interface and reattaches the kernel driver
    /// *before* opening a fresh handle, so the new `claim_interface` does not
    /// race the old handle for interface ownership.
    ///
    /// Increments `counters::reopen_attempts/successes/failures` so every
    /// reopen path (RF transport, stale-read auto-reopen, LCD reconnect,
    /// ENE `with_recovery`) is observable through `recovery_counters()`.
    pub fn reopen(&mut self) -> Result<()> {
        crate::counters::record_reopen_attempt();
        tracing::info!("USB reconnect: {:04x}:{:04x}", self.vid, self.pid);
        // Release interface + reattach kernel driver on the old handle first.
        // Both calls are best-effort — the kernel may already have done this
        // for us on a disconnect event.
        let _ = self.handle.release_interface(self.interface);
        let _ = self.handle.attach_kernel_driver(self.interface);
        let result = if self.opened_with_reset {
            Self::open_with_reset(self.vid, self.pid)
        } else {
            Self::open(self.vid, self.pid)
        };
        match result {
            Ok(new) => {
                *self = new;
                crate::counters::record_reopen_success();
                Ok(())
            }
            Err(e) => {
                crate::counters::record_reopen_failure();
                Err(e)
            }
        }
    }
}

impl Reopenable for UsbDevice {
    fn reopen(&mut self) -> Result<()> {
        UsbDevice::reopen(self)
    }
    fn label(&self) -> String {
        format!("USB {:04x}:{:04x}", self.vid, self.pid)
    }
    /// Try clearing a stalled OUT endpoint before falling back to a full
    /// device reset — typical recovery for EPIPE-style write failures.
    fn try_soft_recover(&self) -> Result<bool> {
        UsbDevice::clear_halt_out(self).map(|_| true)
    }
}

impl Drop for UsbDevice {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.interface);
        let _ = self.handle.attach_kernel_driver(self.interface);
    }
}

pub struct DevicePair {
    pub tx: UsbDevice,
    pub rx: UsbDevice,
}
