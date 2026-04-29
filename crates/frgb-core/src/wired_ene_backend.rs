//! Wired ENE backend — HID protocol for UNI HUB / SLV3H wired fan controllers.
//!
//! Drives wired Lian Li fans (SL V2, AL, Strimer Plus V2) via USB HID reports.
//! The SLV3H (1A86:2107) descriptor defines only Input/Output reports (no Feature reports),
//! so all communication uses write_report() / read_report() through the interrupt endpoints.
//! Protocol uses 0xE0 as a command prefix with 64-byte payloads. Color order is R,B,G.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::time::Instant;

use frgb_model::device::DeviceId;
use frgb_model::usb_ids::{
    PID_ENE_AL, PID_ENE_AL_V2, PID_ENE_SL, PID_ENE_SL_INF, PID_ENE_SL_INF2, PID_ENE_SL_V2, PID_ENE_UNKNOWN, VID_ENE,
};
use frgb_model::GroupId;
use frgb_usb::error::UsbError;
use frgb_usb::hid::{HidDevice, HidHandle};
use frgb_usb::recovery::with_recovery;

use crate::backend::{Backend, BackendId, DiscoveredDevice, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;

// ---------------------------------------------------------------------------
// ENE HID protocol constants
// ---------------------------------------------------------------------------

/// ENE command prefix byte — first byte of every 64-byte packet.
const REPORT_ID: u8 = 0xE0;

/// Synthetic dev_type for ENE wired devices.
const DEV_TYPE_ENE: u8 = 0xFC;

/// Report buffer size (64 bytes, matching the HID descriptor).
const REPORT_SIZE: usize = 64;

// Command masks (OR'd with port number in byte[1])
const CMD_CONTROL: u8 = 0x10;
const CMD_FAN_SPEED: u8 = 0x20;
const CMD_COLOR: u8 = 0x30;
const CMD_QUERY: u8 = 0x50;

// Query sub-commands (byte[2])
const QUERY_FAN_RPM: u8 = 0x00;

// Control sub-commands (byte[2])
const SUBCMD_FAN_MB_SYNC: u8 = 0x31;
const SUBCMD_RESET: u8 = 0x8E;

// ---------------------------------------------------------------------------
// Protocol encoding
// ---------------------------------------------------------------------------

/// Build a fan speed command. `group` is 0-based, `duty` is 0-100.
fn build_fan_speed(group: u8, duty: u8) -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_FAN_SPEED | group;
    buf[2] = 0x00;
    buf[3] = duty.min(100);
    buf
}

/// Build a color command for a port. Colors are in R,B,G order per the ENE protocol.
fn build_colors(port: u8, colors: &[(u8, u8, u8)]) -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_COLOR | port;
    // Pack R,B,G triplets starting at byte 2
    for (i, &(r, g, b)) in colors.iter().enumerate() {
        let offset = 2 + i * 3;
        if offset + 2 >= REPORT_SIZE {
            break;
        }
        buf[offset] = r;
        buf[offset + 1] = b; // ENE wire order: R, B, G
        buf[offset + 2] = g;
    }
    buf
}

/// Build a motherboard fan sync command. `bitmask` has bit N set for group N.
fn build_fan_mb_sync(bitmask: u8) -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_CONTROL;
    buf[2] = SUBCMD_FAN_MB_SYNC;
    buf[3] = bitmask;
    buf
}

/// Build a device reset command (0x8E).
fn build_reset() -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = SUBCMD_RESET;
    buf
}

/// Build the SetMergeOrder packet: { 0xE0, 0x10, 0x63, idx0-3, 8 }. `order` is 4 group indices.
fn build_merge_order(order: &[u8; 4]) -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_CONTROL;
    buf[2] = 0x63; // SetMergeOrder sub-command
    buf[3] = order[0];
    buf[4] = order[1];
    buf[5] = order[2];
    buf[6] = order[3];
    buf[7] = 8; // length/terminator
    buf
}

/// Build a fan RPM query.
fn build_fan_rpm_query() -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_QUERY;
    buf[2] = QUERY_FAN_RPM;
    buf
}

/// Parse 4 RPM values from a fan speed query response.
fn parse_fan_rpms(resp: &[u8]) -> [u16; 4] {
    let mut rpms = [0u16; 4];
    // Response: bytes 2-9 contain 4× u16 LE RPM values
    for (i, rpm) in rpms.iter_mut().enumerate() {
        let offset = 2 + i * 2;
        if offset + 1 < resp.len() {
            *rpm = u16::from_le_bytes([resp[offset], resp[offset + 1]]);
        }
    }
    rpms
}

// ---------------------------------------------------------------------------
// EneDevice — single wired HID controller
// ---------------------------------------------------------------------------

struct EneDevice<H: HidHandle = HidDevice> {
    hid: RefCell<H>,
    id: DeviceId,
    fan_rpms: [u16; 4],
    cooldown: Cell<Option<Instant>>,
}

impl<H: HidHandle> EneDevice<H> {
    #[allow(dead_code)] // used when real ENE 6K77 hardware (0x0CF2) is connected
    fn init(hid: H, vid: u16, pid: u16) -> Result<Self> {
        let id = DeviceId::from_vid_pid(vid, pid);

        // Query fan RPMs to confirm device is responsive.
        // SLV3H only supports Output/Input reports (no Feature reports), so
        // we use write_report + read_report through the interrupt endpoints.
        hid.write_report(&build_fan_rpm_query())
            .map_err(|e| CoreError::Protocol(format!("ENE RPM query: {e}")))?;
        let mut resp = [0u8; REPORT_SIZE];
        let _ = hid
            .read_report(&mut resp)
            .map_err(|e| CoreError::Protocol(format!("ENE RPM read: {e}")))?;
        let fan_rpms = parse_fan_rpms(&resp);

        Ok(Self {
            hid: RefCell::new(hid),
            id,
            fan_rpms,
            cooldown: Cell::new(None),
        })
    }

    fn set_fan_speed(&self, group: u8, duty: u8) -> Result<()> {
        with_recovery(&self.hid, &self.cooldown, |h| {
            h.write_report(&build_fan_speed(group, duty))
        })
        .map_err(|e| CoreError::Protocol(format!("ENE set speed: {e}")))
    }

    fn set_fan_mb_sync(&self, group: u8) -> Result<()> {
        let bitmask = 1u8 << group;
        with_recovery(&self.hid, &self.cooldown, |h| {
            h.write_report(&build_fan_mb_sync(bitmask))
        })
        .map_err(|e| CoreError::Protocol(format!("ENE MB sync: {e}")))
    }

    fn set_colors(&self, port: u8, colors: &[(u8, u8, u8)]) -> Result<()> {
        with_recovery(&self.hid, &self.cooldown, |h| {
            h.write_report(&build_colors(port, colors))
        })
        .map_err(|e| CoreError::Protocol(format!("ENE set colors: {e}")))
    }

    fn reset(&self) -> Result<()> {
        with_recovery(&self.hid, &self.cooldown, |h| h.write_report(&build_reset()))
            .map_err(|e| CoreError::Protocol(format!("ENE reset: {e}")))
    }

    fn set_merge_order(&self, order: &[u8; 4]) -> Result<()> {
        with_recovery(&self.hid, &self.cooldown, |h| {
            h.write_report(&build_merge_order(order))
        })
        .map_err(|e| CoreError::Protocol(format!("ENE merge order: {e}")))
    }
}

// ---------------------------------------------------------------------------
// WiredEneBackend — Backend implementation
// ---------------------------------------------------------------------------

/// Backend for Lian Li wired fan controllers (UNI HUB, SL V2 hub).
pub struct WiredEneBackend {
    devices: Vec<EneDevice<HidDevice>>,
}

impl WiredEneBackend {
    fn new() -> Self {
        Self { devices: Vec::new() }
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Open wired ENE HID devices. Non-fatal: returns Ok with empty list if none found.
    ///
    /// Note: the SLV3H (1A86:2107) and UNI HUB chips (1A86:8091) are NOT ENE fan
    /// controllers. The SLV3H is a hub management chip that doesn't respond to ENE
    /// commands. Real ENE wired controllers use VID 0x0CF2, PIDs 0xA100-0xA106.
    pub fn open_all() -> Result<Self> {
        let mut backend = Self::new();

        // ENE 6K77 model variants ordered by PID. V2 variants (A103/A104/A106)
        // support 6 fan groups; V1 variants (A100/A101/A102/A105) support 4.
        const ENE_PIDS: &[(u16, bool, &str)] = &[
            (PID_ENE_SL, false, "SL"),
            (PID_ENE_AL, false, "AL"),
            (PID_ENE_SL_INF, false, "SL-INF"),
            (PID_ENE_AL_V2, true, "AL-V2"),
            (PID_ENE_SL_V2, true, "SL-V2"),
            (PID_ENE_UNKNOWN, false, "unknown"),
            (PID_ENE_SL_INF2, true, "SL-INF2"),
        ];

        for &(pid, _is_v2, model_name) in ENE_PIDS {
            match HidDevice::open(VID_ENE, pid) {
                Ok(hid) => {
                    tracing::info!("ENE 6K77 found: {:04x}:{:04x} ({})", VID_ENE, pid, model_name);
                    match EneDevice::init(hid, VID_ENE, pid) {
                        Ok(dev) => {
                            backend.devices.push(dev);
                        }
                        Err(e) => {
                            tracing::warn!("ENE {:04x}:{:04x} ({}): init failed: {}", VID_ENE, pid, model_name, e);
                        }
                    }
                }
                Err(UsbError::NotFound) => {
                    // Not present — expected when this model variant isn't connected.
                }
                Err(UsbError::Permission) => {
                    tracing::warn!(
                        "ENE {:04x}:{:04x} ({}): permission denied — check udev rules",
                        VID_ENE,
                        pid,
                        model_name
                    );
                }
                Err(e) => {
                    tracing::warn!("ENE {:04x}:{:04x} ({}): open failed: {}", VID_ENE, pid, model_name, e);
                }
            }
        }

        tracing::debug!("ENE backend: {} device(s) found", backend.devices.len());
        Ok(backend)
    }

    fn find_device(&self, device_id: &DeviceId) -> Result<&EneDevice<HidDevice>> {
        self.devices
            .iter()
            .find(|d| d.id == *device_id)
            .ok_or_else(|| CoreError::NotFound(format!("ENE device {}", device_id.to_hex())))
    }
}

impl Backend for WiredEneBackend {
    fn id(&self) -> BackendId {
        BackendId(3)
    }
    fn name(&self) -> &str {
        "wired-ene"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        // Re-read RPMs on each discovery cycle
        for dev in &mut self.devices {
            let hid = dev.hid.borrow();
            if hid.write_report(&build_fan_rpm_query()).is_ok() {
                let mut resp = [0u8; REPORT_SIZE];
                if hid.read_report(&mut resp).is_ok() {
                    dev.fan_rpms = parse_fan_rpms(&resp);
                }
            }
        }

        Ok(self
            .devices
            .iter()
            .map(|dev| {
                DiscoveredDevice {
                    id: dev.id,
                    fans_type: [0; 4],
                    dev_type: DEV_TYPE_ENE,
                    group: GroupId::new(0),
                    fan_count: 0, // populated by registry from SetQuantity or config
                    master: DeviceId::ZERO,
                    fans_rpm: dev.fan_rpms,
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0,
                }
            })
            .collect())
    }

    fn set_speed(&self, device: &Device, cmd: &SpeedCommand) -> Result<()> {
        let ene_dev = self.find_device(&device.id)?;
        let group_raw = device.group.value();
        match cmd {
            SpeedCommand::Manual(percent) => ene_dev.set_fan_speed(group_raw, percent.value()),
            SpeedCommand::Pwm => ene_dev.set_fan_mb_sync(group_raw),
        }
    }

    fn reset_device(&self, device: &Device) -> Result<()> {
        let ene_dev = self.find_device(&device.id)?;
        ene_dev.reset()
    }

    fn set_merge_order(&self, order: &[u8]) -> Result<()> {
        let mut order_arr = [0u8; 4];
        for (i, &idx) in order.iter().take(4).enumerate() {
            order_arr[i] = idx;
        }
        // Send to first device (hub receives on behalf of all groups)
        if let Some(dev) = self.devices.first() {
            dev.set_merge_order(&order_arr)
        } else {
            Err(CoreError::NotFound("no wired ENE device".into()))
        }
    }

    fn send_rgb(&self, device: &Device, buffer: &frgb_rgb::generator::EffectResult) -> Result<()> {
        let ene_dev = self.find_device(&device.id)?;

        // Sample LEDs from the buffer and convert RGB → RBG for ENE wire order
        let led_count = buffer.buffer.led_count();
        let max_leds = (REPORT_SIZE - 2) / 3; // max LEDs per feature report
        let count = led_count.min(max_leds);
        let mut colors = [(0u8, 0u8, 0u8); 20]; // max 20 LEDs per packet

        for (i, color) in colors.iter_mut().enumerate().take(count) {
            let rgb = buffer.buffer.get_led(0, i);
            *color = (rgb.r, rgb.g, rgb.b); // build_colors() handles the RBG swap
        }

        let group_raw = device.group.value();
        ene_dev.set_colors(group_raw, &colors[..count])
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use super::*;
    use frgb_usb::error::UsbError;
    use frgb_usb::hid::HidHandle;
    use frgb_usb::recovery::Reopenable;

    // -----------------------------------------------------------------------
    // Fault-injecting HID mock
    // -----------------------------------------------------------------------

    /// Mock HidHandle that fails writes N times, then succeeds.
    /// On reopen, clears the failure counter so the retry succeeds.
    struct FaultInjectingHid {
        writes_to_fail: Cell<u32>,
        write_calls: Cell<u32>,
        reopen_calls: Cell<u32>,
        rpm_response: [u8; REPORT_SIZE],
    }

    impl FaultInjectingHid {
        fn new(writes_to_fail: u32) -> Self {
            // Baseline RPM response: four fans at 1000 RPM (0x03E8) each.
            // parse_fan_rpms reads u16 LE pairs starting at byte 2.
            // 0x03E8 LE = [0xE8, 0x03].
            let mut rpm = [0u8; REPORT_SIZE];
            for i in 0..4 {
                rpm[2 + i * 2] = 0xE8;
                rpm[2 + i * 2 + 1] = 0x03;
            }
            Self {
                writes_to_fail: Cell::new(writes_to_fail),
                write_calls: Cell::new(0),
                reopen_calls: Cell::new(0),
                rpm_response: rpm,
            }
        }
    }

    impl Reopenable for FaultInjectingHid {
        fn reopen(&mut self) -> frgb_usb::error::Result<()> {
            self.reopen_calls.set(self.reopen_calls.get() + 1);
            // Clear the failure counter so the retry succeeds.
            self.writes_to_fail.set(0);
            Ok(())
        }
        fn label(&self) -> String {
            "fault-hid".into()
        }
    }

    impl HidHandle for FaultInjectingHid {
        fn write_report(&self, _data: &[u8]) -> frgb_usb::error::Result<()> {
            self.write_calls.set(self.write_calls.get() + 1);
            let remaining = self.writes_to_fail.get();
            if remaining > 0 {
                self.writes_to_fail.set(remaining - 1);
                Err(UsbError::Io("injected write failure".into()))
            } else {
                Ok(())
            }
        }
        fn read_report(&self, buf: &mut [u8]) -> frgb_usb::error::Result<usize> {
            let n = buf.len().min(REPORT_SIZE);
            buf[..n].copy_from_slice(&self.rpm_response[..n]);
            Ok(n)
        }
    }

    // -----------------------------------------------------------------------
    // Recovery test
    // -----------------------------------------------------------------------

    #[test]
    fn set_fan_speed_retries_on_write_failure() {
        // Phase 1: init with no failures so EneDevice is ready.
        let hid = FaultInjectingHid::new(0);
        let dev = EneDevice::<FaultInjectingHid>::init(hid, 0x0CF2, 0xA100)
            .expect("init should succeed with no injected failures");

        // Snapshot write count after init (should be 1: the RPM query write).
        let writes_after_init = dev.hid.borrow().write_calls.get();
        assert_eq!(writes_after_init, 1, "init should perform exactly one write");

        // Phase 2: inject one write failure so set_fan_speed's first attempt fails.
        dev.hid.borrow().writes_to_fail.set(1);

        let res = dev.set_fan_speed(0, 50);
        assert!(res.is_ok(), "expected Ok after recovery retry, got {res:?}");

        // Should have made 2 writes: 1 failure + 1 success after reopen.
        let total_writes = dev.hid.borrow().write_calls.get();
        assert_eq!(
            total_writes - writes_after_init,
            2,
            "set_fan_speed should attempt two writes: one fail, one retry"
        );

        // Reopen must have fired exactly once.
        assert_eq!(
            dev.hid.borrow().reopen_calls.get(),
            1,
            "reopen should fire exactly once on transient write failure"
        );
    }

    #[test]
    fn build_fan_speed_packet() {
        let pkt = build_fan_speed(1, 75);
        assert_eq!(pkt[0], REPORT_ID);
        assert_eq!(pkt[1], CMD_FAN_SPEED | 1);
        assert_eq!(pkt[3], 75);
    }

    #[test]
    fn build_fan_speed_clamps() {
        let pkt = build_fan_speed(0, 200);
        assert_eq!(pkt[3], 100);
    }

    #[test]
    fn build_colors_rbg_order() {
        let pkt = build_colors(0, &[(255, 128, 64)]);
        assert_eq!(pkt[0], REPORT_ID);
        assert_eq!(pkt[1], CMD_COLOR);
        assert_eq!(pkt[2], 255); // R
        assert_eq!(pkt[3], 64); // B (swapped)
        assert_eq!(pkt[4], 128); // G (swapped)
    }

    #[test]
    fn build_colors_multiple() {
        let pkt = build_colors(2, &[(10, 20, 30), (40, 50, 60)]);
        assert_eq!(pkt[1], CMD_COLOR | 2);
        // First LED: R=10, B=30, G=20
        assert_eq!(pkt[2], 10);
        assert_eq!(pkt[3], 30);
        assert_eq!(pkt[4], 20);
        // Second LED: R=40, B=60, G=50
        assert_eq!(pkt[5], 40);
        assert_eq!(pkt[6], 60);
        assert_eq!(pkt[7], 50);
    }

    #[test]
    fn build_fan_speed_different_groups() {
        // Verify the group byte (OR'd into byte[1]) is correct for various groups.
        for group in [0u8, 1, 2, 3] {
            let pkt = build_fan_speed(group, 50);
            assert_eq!(pkt[0], REPORT_ID);
            assert_eq!(
                pkt[1],
                CMD_FAN_SPEED | group,
                "group {group}: byte[1] should be CMD_FAN_SPEED | {group}"
            );
            assert_eq!(pkt[3], 50);
        }
    }

    #[test]
    fn build_colors_different_groups() {
        // Verify the port byte (OR'd into byte[1]) is correct for various ports.
        for port in [0u8, 1, 2, 3] {
            let pkt = build_colors(port, &[(100, 200, 150)]);
            assert_eq!(pkt[0], REPORT_ID);
            assert_eq!(
                pkt[1],
                CMD_COLOR | port,
                "port {port}: byte[1] should be CMD_COLOR | {port}"
            );
        }
    }

    #[test]
    fn parse_fan_rpms_from_response() {
        let mut resp = [0u8; REPORT_SIZE];
        resp[0] = REPORT_ID;
        resp[2] = 0x78;
        resp[3] = 0x05; // 1400 RPM
        resp[4] = 0x00;
        resp[5] = 0x00; // 0 RPM
        resp[6] = 0xE8;
        resp[7] = 0x03; // 1000 RPM
        resp[8] = 0xDC;
        resp[9] = 0x05; // 1500 RPM
        let rpms = parse_fan_rpms(&resp);
        assert_eq!(rpms, [1400, 0, 1000, 1500]);
    }
}
