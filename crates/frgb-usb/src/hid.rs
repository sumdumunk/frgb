//! HID device abstraction — Linux hidraw access for USB HID controllers.
//!
//! Provides a thin wrapper around /dev/hidraw* for sending/receiving HID reports.
//! Used by backends that communicate via USB HID (e.g., ASUS AURA motherboard RGB).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Result, UsbError};
use crate::recovery::Reopenable;

/// Default read timeout for HID devices.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(500);

/// A Linux hidraw device handle.
pub struct HidDevice {
    file: File,
    vid: u16,
    pid: u16,
}

impl HidDevice {
    /// Open a hidraw device by VID/PID. Scans /sys/class/hidraw/ to find
    /// the correct /dev/hidrawN node.
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        Self::open_from_sysfs(vid, pid, Path::new("/sys/class/hidraw"))
    }

    /// Open with explicit sysfs path (for testing).
    pub fn open_from_sysfs(vid: u16, pid: u16, sysfs_dir: &Path) -> Result<Self> {
        let dev_path = find_hidraw_device(vid, pid, sysfs_dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&dev_path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => UsbError::Permission,
                std::io::ErrorKind::NotFound => UsbError::NotFound,
                _ => UsbError::Io(format!("{}: {e}", dev_path.display())),
            })?;
        Ok(Self { file, vid, pid })
    }

    /// Re-open this HID device by scanning sysfs for the same VID/PID.
    ///
    /// Increments `counters::reopen_attempts/successes/failures` so every
    /// HID reopen path (AURA `try_reconnect`, etc.) is observable through
    /// `recovery_counters()`.
    pub fn reopen(&mut self) -> Result<()> {
        crate::counters::record_reopen_attempt();
        tracing::info!("HID reconnect: {:04x}:{:04x}", self.vid, self.pid);
        match Self::open(self.vid, self.pid) {
            Ok(new) => {
                self.file = new.file;
                crate::counters::record_reopen_success();
                Ok(())
            }
            Err(e) => {
                crate::counters::record_reopen_failure();
                Err(e)
            }
        }
    }

    /// Send an output report (first byte = report ID).
    /// Uses `&self` — writes via the OS file descriptor are safe without &mut.
    pub fn write_report(&self, data: &[u8]) -> Result<()> {
        (&self.file)
            .write_all(data)
            .map_err(|e| UsbError::Io(format!("hidraw write: {e}")))?;
        Ok(())
    }

    /// Read an input report with timeout. Returns the number of bytes read.
    pub fn read_report(&self, buf: &mut [u8]) -> Result<usize> {
        self.read_report_timeout(buf, DEFAULT_READ_TIMEOUT)
    }

    /// Read an input report with explicit timeout. Uses poll() to avoid
    /// blocking the daemon indefinitely on unresponsive hardware.
    pub fn read_report_timeout(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret < 0 {
            return Err(UsbError::Io(format!(
                "hidraw poll: {}",
                std::io::Error::last_os_error()
            )));
        }
        if ret == 0 {
            return Err(UsbError::Timeout);
        }
        let n = (&self.file)
            .read(buf)
            .map_err(|e| UsbError::Io(format!("hidraw read: {e}")))?;
        Ok(n)
    }

    /// Send a HID feature report via ioctl (HIDIOCSFEATURE).
    /// Used by protocols that communicate via feature reports (e.g., ENE wired hub).
    pub fn set_feature_report(&self, data: &[u8]) -> Result<()> {
        let request = hidiocsfeature(data.len());
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), request, data.as_ptr()) };
        if ret < 0 {
            return Err(UsbError::Io(format!(
                "HIDIOCSFEATURE failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }

    /// Get a HID feature report via ioctl (HIDIOCGFEATURE).
    /// `buf[0]` must be set to the report ID before calling.
    pub fn get_feature_report(&self, buf: &mut [u8]) -> Result<usize> {
        let request = hidiocgfeature(buf.len());
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), request, buf.as_mut_ptr()) };
        if ret < 0 {
            return Err(UsbError::Io(format!(
                "HIDIOCGFEATURE failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(ret as usize)
    }
}

impl Reopenable for HidDevice {
    fn reopen(&mut self) -> Result<()> {
        HidDevice::reopen(self)
    }
    fn label(&self) -> String {
        format!("HID {:04x}:{:04x}", self.vid, self.pid)
    }
}

/// Abstraction over HID device operations, so backends can be tested
/// against fault-injecting mocks without a real hidraw node. Scope
/// matches what backends actually call — extend when new call sites
/// appear, not speculatively.
pub trait HidHandle: Reopenable {
    fn write_report(&self, data: &[u8]) -> Result<()>;
    fn read_report(&self, buf: &mut [u8]) -> Result<usize>;
}

impl HidHandle for HidDevice {
    fn write_report(&self, data: &[u8]) -> Result<()> {
        HidDevice::write_report(self, data)
    }
    fn read_report(&self, buf: &mut [u8]) -> Result<usize> {
        HidDevice::read_report(self, buf)
    }
}

// ---------------------------------------------------------------------------
// hidraw ioctl numbers
// ---------------------------------------------------------------------------

const IOC_WRITE: libc::c_ulong = 1;
const IOC_READ: libc::c_ulong = 2;

fn ioc(dir: libc::c_ulong, ty: libc::c_ulong, nr: libc::c_ulong, size: usize) -> libc::c_ulong {
    (dir << 30) | ((size as libc::c_ulong) << 16) | (ty << 8) | nr
}

fn hidiocsfeature(len: usize) -> libc::c_ulong {
    ioc(IOC_WRITE | IOC_READ, b'H' as libc::c_ulong, 0x06, len)
}

fn hidiocgfeature(len: usize) -> libc::c_ulong {
    ioc(IOC_WRITE | IOC_READ, b'H' as libc::c_ulong, 0x07, len)
}

// ---------------------------------------------------------------------------
// Device discovery via sysfs
// ---------------------------------------------------------------------------

/// Find the /dev/hidrawN path for a device matching the given VID/PID.
fn find_hidraw_device(vid: u16, pid: u16, sysfs_dir: &Path) -> Result<PathBuf> {
    let entries = fs::read_dir(sysfs_dir).map_err(|_| UsbError::NotFound)?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("hidraw") {
            continue;
        }

        // Read the uevent file to extract HID_ID (bus:VID:PID)
        let uevent_path = entry.path().join("device").join("uevent");
        if let Ok(uevent) = fs::read_to_string(&uevent_path) {
            if matches_vid_pid(&uevent, vid, pid) {
                return Ok(PathBuf::from(format!("/dev/{name_str}")));
            }
        }
    }

    Err(UsbError::NotFound)
}

/// Parse uevent content for HID_ID=BBBB:VVVVVVVV:PPPPPPPP and check VID/PID.
fn matches_vid_pid(uevent: &str, vid: u16, pid: u16) -> bool {
    for line in uevent.lines() {
        if let Some(value) = line.strip_prefix("HID_ID=") {
            // Format: BBBB:VVVVVVVV:PPPPPPPP (bus:vendor:product, all hex)
            let parts: Vec<&str> = value.split(':').collect();
            if parts.len() == 3 {
                let parsed_vid = u32::from_str_radix(parts[1], 16).unwrap_or(0);
                let parsed_pid = u32::from_str_radix(parts[2], 16).unwrap_or(0);
                return parsed_vid == vid as u32 && parsed_pid == pid as u32;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hid_id_matches() {
        let uevent = "DRIVER=hid-generic\nHID_ID=0003:00000B05:00001AA6\nHID_NAME=ASUS AURA\n";
        assert!(matches_vid_pid(uevent, 0x0B05, 0x1AA6));
    }

    #[test]
    fn parse_hid_id_no_match() {
        let uevent = "DRIVER=hid-generic\nHID_ID=0003:00000B05:00001AA6\nHID_NAME=ASUS AURA\n";
        assert!(!matches_vid_pid(uevent, 0x0B05, 0xFFFF));
    }

    #[test]
    fn parse_hid_id_missing() {
        let uevent = "DRIVER=hid-generic\nHID_NAME=ASUS AURA\n";
        assert!(!matches_vid_pid(uevent, 0x0B05, 0x1AA6));
    }

    #[test]
    fn find_device_in_fake_sysfs() {
        let dir = std::env::temp_dir().join(format!("frgb_hid_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        // Create fake sysfs structure
        let hidraw_dir = dir.join("hidraw0").join("device");
        fs::create_dir_all(&hidraw_dir).unwrap();
        fs::write(
            hidraw_dir.join("uevent"),
            "DRIVER=hid-generic\nHID_ID=0003:00000B05:00001AA6\nHID_NAME=ASUS AURA\n",
        )
        .unwrap();

        let result = find_hidraw_device(0x0B05, 0x1AA6, &dir);
        assert_eq!(result.unwrap(), PathBuf::from("/dev/hidraw0"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_device_not_present() {
        let dir = std::env::temp_dir().join(format!("frgb_hid_empty_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let result = find_hidraw_device(0x0B05, 0x1AA6, &dir);
        assert!(result.is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
