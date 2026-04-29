//! LCD backend — USB transport for Lian Li LCD panels.
//!
//! Drives LCD displays on SL-LCD fans, HydroShift II AIO, TL V2, and Universal 8.8".
//! Protocol encoding is in frgb-lcd; this module handles USB device management,
//! init sequences, and frame push via bulk transfers.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use frgb_lcd::decode;
use frgb_lcd::encode;
use frgb_model::device::DeviceId;
use frgb_model::lcd::LcdRotation;
use frgb_model::usb_ids::VID_LCD;
use frgb_usb::device::UsbDevice;

use crate::backend::{Backend, BackendId, DiscoveredDevice, LcdExt, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;

/// LCD device variant — determines resolution and protocol details.
#[derive(Debug, Clone, Copy)]
pub enum LcdVariant {
    /// SL-LCD Wireless fan (400x400)
    SlLcd,
    /// TL V2 LCD fan (400x400)
    TlV2Lcd,
    /// HydroShift II Circle (480x480, pump max 2450 RPM)
    HydroShiftCircle,
    /// HydroShift II Square (480x480, pump max 3200 RPM)
    HydroShiftSquare,
    /// Universal 8.8" screen (480x1920)
    Universal88,
}

impl LcdVariant {
    pub fn from_pid(pid: u16) -> Option<Self> {
        use frgb_model::usb_ids::*;
        match pid {
            PID_SL_LCD => Some(Self::SlLcd),
            PID_TLV2_LCD => Some(Self::TlV2Lcd),
            PID_HYDROSHIFT_CIRCLE => Some(Self::HydroShiftCircle),
            PID_HYDROSHIFT_SQUARE => Some(Self::HydroShiftSquare),
            PID_UNIVERSAL_88 => Some(Self::Universal88),
            _ => None,
        }
    }

    /// Whether this variant uses the WinUSB packet format (500-byte plaintext + trailers)
    /// vs the standard LCD format (504-byte plaintext, PKCS7, no trailers).
    /// From reference project: HydroShift/Lancool/Universal use WinUSB, SLV3/TLV2 use standard.
    pub fn uses_winusb_format(&self) -> bool {
        matches!(
            self,
            Self::HydroShiftCircle | Self::HydroShiftSquare | Self::Universal88
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SlLcd => "SL-LCD Wireless",
            Self::TlV2Lcd => "TL V2 LCD",
            Self::HydroShiftCircle => "HydroShift II Circle",
            Self::HydroShiftSquare => "HydroShift II Square",
            Self::Universal88 => "Universal 8.8\"",
        }
    }
}

/// A single LCD USB device with its state.
struct LcdDevice {
    usb: RefCell<UsbDevice>,
    variant: LcdVariant,
    id: DeviceId,
    sequence: Cell<u16>,
    cooldown: Cell<Option<Instant>>,
    /// Firmware version as major*100 + minor (e.g. 120 = v1.2).
    /// Populated from CMD_GET_VER response during WinUSB init.
    ///
    /// HydroShift II firmware < 1.2 uses 1024-byte B-commands for LCD packets,
    /// firmware >= 1.2 uses 512-byte C-commands. This version is stored for
    /// future packet-size selection but does not yet affect encoding.
    firmware_version: Option<u16>,
}

impl LcdDevice {
    fn timestamp_ms() -> u32 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u32
    }

    /// Parse a firmware version string like "1.2" or "1.20" into major*100 + minor.
    /// Returns None if the string doesn't match the expected format.
    fn parse_firmware_version(ver_str: &str) -> Option<u16> {
        let mut parts = ver_str.split('.');
        let major: u16 = parts.next()?.parse().ok()?;
        let minor: u16 = parts.next()?.parse().ok()?;
        Some(major * 100 + minor)
    }

    /// Attempt to reconnect the USB device, with 5-second cooldown.
    fn try_reconnect(&self) {
        if let Some(last) = self.cooldown.get() {
            if last.elapsed() < std::time::Duration::from_secs(5) {
                return;
            }
        }
        self.cooldown.set(Some(Instant::now()));
        if let Err(e) = self.usb.borrow_mut().reopen() {
            tracing::warn!("LCD reconnect failed: {e}");
        }
    }

    /// Initialize the LCD device.
    ///
    /// Two init paths based on device variant (from reference project):
    /// - SLV3/TLV2: single 0x0D header (standard 504-byte PKCS7 encryption)
    /// - HydroShift/Universal: GetVer + StopPlay + StopClock + SetFrameRate
    ///   (WinUSB 500-byte encryption with trailers)
    fn init(&mut self) -> Result<()> {
        if self.variant.uses_winusb_format() {
            self.init_winusb()
        } else {
            self.init_standard()
        }
    }

    /// Standard init for SLV3/TLV2 wireless LCD fans.
    fn init_standard(&mut self) -> Result<()> {
        let ts = Self::timestamp_ms();
        let header = encode::build_init_packet(frgb_lcd::CMD_INIT_FINAL, ts, 0);
        self.usb.borrow().write(&header)?;
        self.read_response("init");
        Ok(())
    }

    /// WinUSB init for HydroShift II, Lancool 207, Universal Screen.
    /// Matches reference project's WinUsbLcdDevice::do_init().
    fn init_winusb(&mut self) -> Result<()> {
        let usb = self.usb.borrow();

        // Flush any stale data
        let _ = usb.read_lcd_timeout(std::time::Duration::from_millis(1));

        // GetVer — response bytes 8..40 contain a 32-byte UTF-8 version string
        // (e.g. "1.2\0..."). Parse into major*100 + minor for firmware_version.
        let ts = Self::timestamp_ms();
        let header = encode::build_winusb_packet(frgb_lcd::CMD_GET_VER, ts, 0);
        usb.write(&header)?;
        drop(usb);
        if let Some(resp) = self.read_response_raw("GetVer", std::time::Duration::from_millis(1000)) {
            if resp[0] == frgb_lcd::CMD_GET_VER {
                let ver_bytes = &resp[8..40];
                let ver_str = std::str::from_utf8(ver_bytes)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .trim();
                tracing::info!("LCD {} firmware: \"{}\"", self.variant.name(), ver_str);
                self.firmware_version = Self::parse_firmware_version(ver_str);
                if let Some(v) = self.firmware_version {
                    tracing::info!("LCD {} firmware version: {}.{}", self.variant.name(), v / 100, v % 100);
                }
            }
        }

        // StopPlay
        let ts = Self::timestamp_ms();
        let header = encode::build_winusb_packet(frgb_lcd::CMD_STOP_PLAY, ts, 0);
        self.usb.borrow().write(&header)?;
        self.read_response("StopPlay");

        // StopClock
        let ts = Self::timestamp_ms();
        let header = encode::build_winusb_packet(frgb_lcd::CMD_STOP_CLOCK, ts, 0);
        self.usb.borrow().write(&header)?;
        self.read_response("StopClock");

        // SetFrameRate(30)
        let ts = Self::timestamp_ms();
        let header = encode::build_winusb_packet(frgb_lcd::CMD_SET_FRAMERATE, ts, 30);
        self.usb.borrow().write(&header)?;
        self.read_response("SetFrameRate");

        Ok(())
    }

    /// Read a 512-byte response, logging result. Non-fatal on failure.
    fn read_response(&self, context: &str) {
        self.read_response_timeout(context, std::time::Duration::from_millis(1000))
    }

    /// Read response with short timeout — for commands where a slow/missing
    /// response should not block the daemon tick loop.
    fn read_response_fast(&self, context: &str) {
        self.read_response_timeout(context, std::time::Duration::from_millis(200))
    }

    fn read_response_timeout(&self, context: &str, timeout: std::time::Duration) {
        let _ = self.read_response_raw(context, timeout);
    }

    /// Read response and return the raw 512-byte buffer on success.
    fn read_response_raw(&self, context: &str, timeout: std::time::Duration) -> Option<[u8; 512]> {
        match self.usb.borrow().read_lcd_timeout(timeout) {
            Ok(resp) => {
                tracing::debug!("LCD {context} response: {:02x} {:02x}", resp[0], resp[1]);
                Some(resp)
            }
            Err(e) => {
                tracing::debug!("LCD {context}: no response: {e} (non-fatal)");
                None
            }
        }
    }

    /// Push a JPEG frame to the display, with reconnect-on-error.
    fn send_frame(&self, jpeg: &[u8]) -> Result<()> {
        match self.send_frame_inner(jpeg) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!("LCD send_frame failed: {e}, attempting reconnect");
                self.try_reconnect();
                self.send_frame_inner(jpeg)
            }
        }
    }

    /// Push a JPEG frame to the display.
    /// Two protocols:
    /// - Standard (SLV3/TLV2): fixed 102,400-byte packet + ack + control packet
    /// - WinUSB (HydroShift/Universal): variable-size (512+jpeg_len) + read response
    fn send_frame_inner(&self, jpeg: &[u8]) -> Result<()> {
        let ts = Self::timestamp_ms();
        let usb = self.usb.borrow();

        if self.variant.uses_winusb_format() {
            // WinUSB: header(512) + raw JPEG, no padding, no control packet
            let header = encode::build_image_header_winusb(ts, jpeg.len() as u32);
            let mut packet = Vec::with_capacity(512 + jpeg.len());
            packet.extend_from_slice(&header);
            packet.extend_from_slice(jpeg);
            usb.write(&packet)?;
            drop(usb);
            self.read_response("image");
        } else {
            // Standard: fixed 102,400-byte packet + ack + control packet
            let packet = encode::build_image_packet(ts, jpeg).map_err(CoreError::InvalidInput)?;
            usb.write(&packet)?;

            let resp = usb.read_lcd()?;
            let _seq =
                decode::validate_image_ack(&resp).map_err(|e| CoreError::Protocol(format!("LCD image ack: {e}")))?;

            let seq = self.sequence.get();
            self.sequence.set(seq.wrapping_add(1));
            let ctrl = encode::build_control_packet(seq, 0x00);
            usb.write(&ctrl)?;
        }

        Ok(())
    }

    fn set_brightness(&self, level: frgb_model::Brightness) -> Result<()> {
        let level = level.value();
        let hw_level = if level == 0 {
            0
        } else {
            ((level as u16 * 50 + 127) / 255).max(1) as u8
        };
        let ts = Self::timestamp_ms();
        let packet = if self.variant.uses_winusb_format() {
            encode::build_winusb_packet(frgb_lcd::CMD_INIT, ts, hw_level)
        } else {
            encode::build_brightness_packet(ts, hw_level).map_err(CoreError::InvalidInput)?
        };
        self.usb.borrow().write(&packet)?;
        self.read_response_fast("brightness");
        Ok(())
    }

    fn set_rotation(&self, rotation: LcdRotation) -> Result<()> {
        let rotation_byte = match rotation {
            LcdRotation::R0 => 0u8,
            LcdRotation::R90 => 1,
            LcdRotation::R180 => 2,
            LcdRotation::R270 => 3,
        };
        let ts = Self::timestamp_ms();
        let packet = if self.variant.uses_winusb_format() {
            encode::build_winusb_packet(frgb_lcd::CMD_INIT_FINAL, ts, rotation_byte)
        } else {
            encode::build_rotation_packet(ts, rotation)
        };
        self.usb.borrow().write(&packet)?;
        self.read_response_fast("rotation");
        Ok(())
    }

    fn set_clock(&self) -> Result<()> {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let packet = encode::build_set_clock_packet(
            Self::timestamp_ms(),
            now.year() as u16,
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
            now.weekday().num_days_from_sunday() as u8,
        );
        self.usb.borrow().write(&packet)?;
        self.read_response("SetClock");
        Ok(())
    }

    fn reboot(&self) -> Result<()> {
        let packet = encode::build_reboot_packet(Self::timestamp_ms());
        // Reboot is fire-and-forget: the firmware tears down the USB connection
        // immediately after accepting the packet, so post-send write errors are
        // expected. Do not wrap in with_recovery — retrying a reboot against a
        // firmware mid-reboot races the device re-enumerating.
        match self.usb.borrow().write(&packet) {
            Ok(()) => tracing::info!("LCD reboot sent to {}", self.id.to_hex()),
            Err(e @ (frgb_usb::error::UsbError::Permission | frgb_usb::error::UsbError::NotFound)) => {
                tracing::warn!("LCD reboot write failed before send: {e}");
            }
            Err(e) => tracing::debug!("LCD reboot write errored (likely firmware disconnect on ACK): {e}"),
        }
        Ok(())
    }

    /// Encrypt a raw 512-byte H.264 protocol packet using the correct format
    /// for this device variant (standard PKCS7 vs WinUSB NoPadding).
    fn encrypt_h264_packet(&self, raw: &[u8; frgb_lcd::PACKET_SIZE]) -> [u8; frgb_lcd::PACKET_SIZE] {
        if self.variant.uses_winusb_format() {
            let plaintext: [u8; 500] = raw[..500].try_into().unwrap();
            frgb_lcd::encrypt::encrypt_packet_winusb(&plaintext)
        } else {
            let plaintext: [u8; frgb_lcd::PLAINTEXT_SIZE] = raw[..frgb_lcd::PLAINTEXT_SIZE].try_into().unwrap();
            frgb_lcd::encrypt::encrypt_packet(&plaintext)
        }
    }

    /// Upload H.264 data and start on-device playback.
    fn upload_h264(&self, data: &[u8]) -> Result<()> {
        use frgb_lcd::h264::{build_start_play, build_upload_header, H264Upload};

        let upload = H264Upload::from_bytes(data.to_vec()).map_err(CoreError::InvalidInput)?;

        if upload.chunks > 255 {
            return Err(CoreError::InvalidInput(format!(
                "H.264 file too large: {} chunks (max 255, ~{} MB)",
                upload.chunks,
                upload.chunks * frgb_lcd::h264::H264_BLOCK_SIZE / (1024 * 1024)
            )));
        }

        let total_size = data.len() as u32;
        let ts = Self::timestamp_ms() as u64;

        for chunk_idx in 0..upload.chunks {
            let chunk = upload.chunk(chunk_idx).unwrap();
            let header_raw = build_upload_header(0, chunk_idx as u8, chunk.len() as u32, total_size, ts);
            let header_enc = self.encrypt_h264_packet(&header_raw);

            let mut packet = Vec::with_capacity(frgb_lcd::PACKET_SIZE + chunk.len());
            packet.extend_from_slice(&header_enc);
            packet.extend_from_slice(chunk);

            self.usb.borrow().write(&packet)?;
            self.read_response("h264-upload");

            tracing::info!(
                "LCD H.264 upload: chunk {}/{} ({} bytes)",
                chunk_idx + 1,
                upload.chunks,
                chunk.len()
            );
        }

        // Start playback on block 0
        let start_raw = build_start_play(0);
        let start_enc = self.encrypt_h264_packet(&start_raw);
        self.usb.borrow().write(&start_enc)?;
        self.read_response("h264-start-play");

        tracing::info!(
            "LCD H.264 playback started ({} bytes, {} chunks)",
            total_size,
            upload.chunks
        );
        Ok(())
    }

    /// Stop on-device H.264 playback.
    fn stop_h264(&self) -> Result<()> {
        let stop_raw = frgb_lcd::h264::build_stop_play();
        let stop_enc = self.encrypt_h264_packet(&stop_raw);
        self.usb.borrow().write(&stop_enc)?;
        self.read_response("h264-stop-play");
        tracing::info!("LCD H.264 playback stopped");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LcdBackend — Backend + LcdExt implementation
// ---------------------------------------------------------------------------

/// Backend for Lian Li LCD panels. Discovers and manages USB LCD devices.
pub struct LcdBackend {
    devices: Vec<LcdDevice>,
}

impl LcdBackend {
    fn new() -> Self {
        Self { devices: Vec::new() }
    }

    /// Number of LCD devices managed by this backend.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Open all LCD devices matching known PIDs.
    /// Uses open_all per PID to find multiple devices with the same PID
    /// (e.g., two SL-LCD Wireless fans).
    pub fn open_all() -> Result<Self> {
        let mut backend = Self::new();
        let known_pids = [
            frgb_model::usb_ids::PID_SL_LCD,
            frgb_model::usb_ids::PID_TLV2_LCD,
            frgb_model::usb_ids::PID_HYDROSHIFT_CIRCLE,
            frgb_model::usb_ids::PID_HYDROSHIFT_SQUARE,
            frgb_model::usb_ids::PID_UNIVERSAL_88,
        ];

        for pid in known_pids {
            let variant = match LcdVariant::from_pid(pid) {
                Some(v) => v,
                None => continue,
            };
            let devices = UsbDevice::open_all_with_reset(VID_LCD, pid);
            for (idx, usb) in devices.into_iter().enumerate() {
                // Unique ID per physical device: VID:PID + index
                let mut id = DeviceId::from_vid_pid(VID_LCD, pid);
                // Differentiate multiple devices of same PID by index in low byte
                id.set_index(idx as u8);
                backend.devices.push(LcdDevice {
                    usb: RefCell::new(usb),
                    variant,
                    id,
                    sequence: Cell::new(0),
                    cooldown: Cell::new(None),
                    firmware_version: None,
                });
            }
        }

        // Init all discovered LCDs — remove any that fail init
        backend.devices.retain_mut(|dev| match dev.init() {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("LCD init failed for {}: {e}", dev.variant.name());
                false
            }
        });

        Ok(backend)
    }

    fn find_device(&self, device_id: &DeviceId) -> Result<&LcdDevice> {
        self.devices
            .iter()
            .find(|d| d.id == *device_id)
            .ok_or_else(|| CoreError::NotFound(format!("LCD device {}", device_id.to_hex())))
    }
}

impl Backend for LcdBackend {
    fn id(&self) -> BackendId {
        BackendId(1)
    }
    fn name(&self) -> &str {
        "lcd"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        // LCD USB devices are capabilities of fan groups, not standalone groups.
        // The fan groups already know they have LCDs (via spec has_lcd).
        // LCD backend is accessed directly via as_lcd_ext() for frame operations.
        Ok(Vec::new())
    }

    fn set_speed(&self, _device: &Device, _cmd: &SpeedCommand) -> Result<()> {
        Ok(()) // LCD doesn't control fan speed
    }

    fn send_rgb(&self, _device: &Device, _buffer: &frgb_rgb::generator::EffectResult) -> Result<()> {
        Ok(()) // LCD doesn't do RGB strips
    }

    fn reset_device(&self, device: &Device) -> Result<()> {
        let dev = self.find_device(&device.id)?;
        dev.reboot()
    }

    fn as_lcd_ext(&self) -> Option<&dyn LcdExt> {
        Some(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl LcdExt for LcdBackend {
    fn lcd_device_ids(&self) -> Vec<DeviceId> {
        self.devices.iter().map(|d| d.id).collect()
    }

    fn lcd_device_info(&self) -> Vec<frgb_model::lcd::LcdDeviceInfo> {
        self.devices
            .iter()
            .enumerate()
            .map(|(i, dev)| {
                let (w, h) = match dev.variant {
                    LcdVariant::SlLcd | LcdVariant::TlV2Lcd => (400, 400),
                    LcdVariant::HydroShiftCircle | LcdVariant::HydroShiftSquare => (480, 480),
                    LcdVariant::Universal88 => (480, 1920),
                };
                frgb_model::lcd::LcdDeviceInfo {
                    index: i as u8,
                    name: format!("{} {}", dev.variant.name(), i + 1),
                    width: w,
                    height: h,
                }
            })
            .collect()
    }

    fn send_frame(&self, device_id: &DeviceId, jpeg: &[u8]) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.send_frame(jpeg)
    }

    fn set_brightness(&self, device_id: &DeviceId, level: frgb_model::Brightness) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.set_brightness(level)
    }

    fn set_rotation(&self, device_id: &DeviceId, rotation: LcdRotation) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.set_rotation(rotation)
    }

    fn set_clock(&self, device_id: &DeviceId) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.set_clock()
    }

    fn upload_h264(&self, device_id: &DeviceId, data: &[u8]) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.upload_h264(data)
    }

    fn stop_h264(&self, device_id: &DeviceId) -> Result<()> {
        let dev = self.find_device(device_id)?;
        dev.stop_h264()
    }
}

// AIO pump control lives on LianLiRfBackend, not the LCD backend. Pump speed/enable
// is streamed over RF via command 0x12 0x21. The LCD backend handles only the
// display on HydroShift II, not the pump.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use frgb_model::Brightness;

    /// Replicate the brightness → hardware level formula from LcdDevice::set_brightness.
    /// level 0 → 0, otherwise (level * 50 + 127) / 255, clamped to min 1.
    fn brightness_to_hw(level: Brightness) -> u8 {
        let v = level.value();
        if v == 0 {
            0
        } else {
            ((v as u16 * 50 + 127) / 255).max(1) as u8
        }
    }

    #[test]
    fn brightness_hardware_mapping() {
        // 0 → hw level 0 (display off)
        assert_eq!(brightness_to_hw(Brightness::new(0)), 0);
        // 1 → (1*50+127)/255 = 177/255 = 0, but clamped to min 1
        assert_eq!(brightness_to_hw(Brightness::new(1)), 1);
        // 128 → (128*50+127)/255 = 6527/255 = 25
        assert_eq!(brightness_to_hw(Brightness::new(128)), 25);
        // 255 → (255*50+127)/255 = 12877/255 = 50
        assert_eq!(brightness_to_hw(Brightness::new(255)), 50);
        // 5 → (5*50+127)/255 = 377/255 = 1
        assert_eq!(brightness_to_hw(Brightness::new(5)), 1);
        // 51 → (51*50+127)/255 = 2677/255 = 10
        assert_eq!(brightness_to_hw(Brightness::new(51)), 10);
    }

    #[test]
    fn brightness_monotonic() {
        // Hardware level should be non-decreasing as brightness increases
        let mut prev = 0u8;
        for i in 0..=255u8 {
            let hw = brightness_to_hw(Brightness::new(i));
            assert!(hw >= prev, "brightness {i}: hw {hw} < prev {prev}");
            prev = hw;
        }
    }

    #[test]
    fn parse_firmware_version_valid() {
        use super::LcdDevice;
        assert_eq!(LcdDevice::parse_firmware_version("1.2"), Some(102));
        assert_eq!(LcdDevice::parse_firmware_version("1.20"), Some(120));
        assert_eq!(LcdDevice::parse_firmware_version("2.0"), Some(200));
    }

    #[test]
    fn parse_firmware_version_invalid() {
        use super::LcdDevice;
        assert_eq!(LcdDevice::parse_firmware_version(""), None);
        assert_eq!(LcdDevice::parse_firmware_version("abc"), None);
        assert_eq!(LcdDevice::parse_firmware_version("1"), None);
    }
}
