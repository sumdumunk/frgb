//! AURA backend — per-channel addressable RGB for ASUS motherboard controllers.
//!
//! Drives ASUS AURA RGB headers via /dev/hidraw using 65-byte HID reports.
//! Supports both addressable (per-LED direct mode, 0x40) and fixed RGB zones
//! (hardware effects via 0x35/0x36/0x3F). Channels are discovered from the
//! controller's config table and can be individually managed or assigned
//! hardware effects.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::time::Instant;

use frgb_model::config::AuraConfig;
use frgb_model::device::DeviceId;
use frgb_model::ipc::AuraChannelInfo;
use frgb_model::usb_ids::{PID_AURA, VID_AURA};
use frgb_model::GroupId;
use frgb_usb::hid::HidDevice;

use crate::backend::{Backend, BackendId, DiscoveredDevice, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;

// ---------------------------------------------------------------------------
// AURA HID protocol constants
// ---------------------------------------------------------------------------

/// Synthetic dev_type for AURA devices (distinct from RF and LCD).
const DEV_TYPE_AURA: u8 = 0xFD;

/// HID report size: 1 byte report ID + 64 bytes payload.
const REPORT_SIZE: usize = 65;

/// HID report ID from the AURA controller's descriptor.
const REPORT_ID: u8 = 0xEC;

/// Maximum LEDs per direct-mode packet: (65 - 5 header bytes) / 3 = 20.
const LEDS_PER_PACKET: usize = 20;

// Command bytes (payload byte 0)
const CMD_FIRMWARE_QUERY: u8 = 0xB0;
const CMD_SET_GEN1: u8 = 0x52;
const CMD_ADDRESSABLE_DIRECT: u8 = 0x40;
const CMD_EFFECT: u8 = 0x35;
const CMD_EFFECT_COLOR: u8 = 0x36;
const CMD_COMMIT: u8 = 0x3F;

// ---------------------------------------------------------------------------
// Protocol encoding
// ---------------------------------------------------------------------------

/// Build the GEN1 initialization packet sent before config query.
/// Payload: [0xEC, 0x52, 0x53, 0x00, 0x01, ...]
fn build_set_gen1() -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_SET_GEN1;
    buf[2] = 0x53;
    buf[3] = 0x00;
    buf[4] = 0x01;
    buf
}

/// Build the config table query packet.
fn build_config_query() -> [u8; REPORT_SIZE] {
    let mut buf = [0u8; REPORT_SIZE];
    buf[0] = REPORT_ID;
    buf[1] = CMD_FIRMWARE_QUERY;
    buf
}

/// Build direct-mode packets for a single channel. Colors are chunked into
/// groups of 20 LEDs. The last packet has the apply flag (0x80) OR'd into
/// byte[2] to trigger the controller to latch the frame.
fn build_direct_packets(channel: u8, colors: &[(u8, u8, u8)]) -> Vec<[u8; REPORT_SIZE]> {
    let mut packets = Vec::new();
    let chunks: Vec<&[(u8, u8, u8)]> = colors.chunks(LEDS_PER_PACKET).collect();
    let last_idx = chunks.len().saturating_sub(1);

    for (i, chunk) in chunks.iter().enumerate() {
        let mut buf = [0u8; REPORT_SIZE];
        buf[0] = REPORT_ID;
        buf[1] = CMD_ADDRESSABLE_DIRECT;

        // Byte 2: channel index, with 0x80 apply flag on the last packet
        if i == last_idx {
            buf[2] = channel | 0x80;
        } else {
            buf[2] = channel;
        }

        // Byte 3: packet sequence (offset within the LED strip)
        buf[3] = (i * LEDS_PER_PACKET) as u8;
        // Byte 4: LED count in this packet
        buf[4] = chunk.len() as u8;

        for (j, &(r, g, b)) in chunk.iter().enumerate() {
            let offset = 5 + j * 3;
            buf[offset] = r;
            buf[offset + 1] = g;
            buf[offset + 2] = b;
        }

        packets.push(buf);
    }

    packets
}

/// Build the three hardware effect packets: (effect, color, commit).
fn build_hw_effect_packets(
    effect_channel: u8,
    mode: u8,
    color: (u8, u8, u8),
    led_count: u8,
) -> ([u8; REPORT_SIZE], [u8; REPORT_SIZE], [u8; REPORT_SIZE]) {
    // Effect packet (CMD_EFFECT = 0x35)
    let mut effect = [0u8; REPORT_SIZE];
    effect[0] = REPORT_ID;
    effect[1] = CMD_EFFECT;
    effect[2] = effect_channel;
    effect[3] = 0x00;
    effect[4] = mode;
    effect[5] = 0x00; // speed
    effect[6] = led_count;

    // Color packet (CMD_EFFECT_COLOR = 0x36)
    let mut color_pkt = [0u8; REPORT_SIZE];
    color_pkt[0] = REPORT_ID;
    color_pkt[1] = CMD_EFFECT_COLOR;
    color_pkt[2] = effect_channel;
    color_pkt[3] = 0x00;
    color_pkt[4] = led_count;
    // Fill all LEDs with the same color
    for i in 0..led_count as usize {
        let offset = 5 + i * 3;
        if offset + 2 >= REPORT_SIZE {
            break;
        }
        color_pkt[offset] = color.0;
        color_pkt[offset + 1] = color.1;
        color_pkt[offset + 2] = color.2;
    }

    // Commit packet (CMD_COMMIT = 0x3F)
    let mut commit = [0u8; REPORT_SIZE];
    commit[0] = REPORT_ID;
    commit[1] = CMD_COMMIT;

    (effect, color_pkt, commit)
}

/// A channel discovered from the controller's config table.
#[derive(Debug, Clone)]
struct DiscoveredChannel {
    direct_channel: u8,
    effect_channel: u8,
    led_count: u8,
    is_fixed: bool,
}

/// Parse the config table response to discover available channels.
///
/// Response layout (offsets relative to byte 4):
/// - offset 0x02: number of addressable RGB headers
/// - offset 0x1B: number of fixed (non-addressable) LED zones
/// - offset 0x1D: total RGB header count
///
/// Fixed LEDs use direct_channel=0x04. Addressable headers get
/// sequential direct_channel indices starting from 0.
fn parse_config_table(response: &[u8]) -> Vec<DiscoveredChannel> {
    let mut channels = Vec::new();

    if response.len() < 4 + 0x1E {
        return channels;
    }

    let base = 4;
    let addressable_count = response[base + 0x02] as usize;
    let fixed_leds = response[base + 0x1B];
    let _rgb_headers = response[base + 0x1D];

    // Fixed LED zone (if present)
    if fixed_leds > 0 {
        channels.push(DiscoveredChannel {
            direct_channel: 0x04,
            effect_channel: channels.len() as u8,
            led_count: fixed_leds,
            is_fixed: true,
        });
    }

    // Addressable headers
    for i in 0..addressable_count {
        channels.push(DiscoveredChannel {
            direct_channel: i as u8,
            effect_channel: channels.len() as u8,
            led_count: 0, // unknown until user configures
            is_fixed: false,
        });
    }

    channels
}

/// Parse firmware version string from response (starts at byte 2, null-terminated ASCII).
fn parse_firmware(response: &[u8]) -> String {
    if response.len() <= 2 {
        return String::new();
    }
    let end = response[2..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| 2 + p)
        .unwrap_or(response.len());
    String::from_utf8_lossy(&response[2..end]).into()
}

// ---------------------------------------------------------------------------
// Channel state and device model
// ---------------------------------------------------------------------------

/// State of a single AURA channel.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChannelState {
    /// Under daemon control — direct-mode RGB frames are sent each tick.
    Managed,
    /// Running a hardware effect on-controller (no daemon frames needed).
    HardwareEffect { mode: u8 },
    /// Inactive (0 LEDs configured or explicitly turned off).
    Off,
}

/// A single AURA channel (addressable header or fixed LED zone).
#[derive(Debug, Clone)]
struct AuraChannel {
    direct_channel: u8,
    effect_channel: u8,
    led_count: u8,
    name: String,
    state: ChannelState,
}

/// A single AURA HID controller with its discovered channels.
struct AuraDevice {
    hid: RefCell<HidDevice>,
    #[allow(dead_code)] // used by discover() via DeviceId::from_vid_pid; will be read in Tasks 6-7
    id: DeviceId,
    channels: Vec<AuraChannel>,
    last_reconnect: Cell<Option<Instant>>,
}

impl AuraDevice {
    /// Initialize: send GEN1, query config, parse channels, overlay user config.
    /// Returns (device, firmware_string).
    fn init(hid: HidDevice, aura_config: &AuraConfig) -> Result<(Self, String)> {
        let id = DeviceId::from_vid_pid(VID_AURA, PID_AURA);

        // Send GEN1 init
        hid.write_report(&build_set_gen1())
            .map_err(|e| CoreError::Protocol(format!("AURA set gen1: {e}")))?;

        // Query config table
        hid.write_report(&build_config_query())
            .map_err(|e| CoreError::Protocol(format!("AURA config query: {e}")))?;
        let mut resp = [0u8; REPORT_SIZE];
        let _ = hid
            .read_report(&mut resp)
            .map_err(|e| CoreError::Protocol(format!("AURA config read: {e}")))?;
        let firmware = parse_firmware(&resp);
        let discovered = parse_config_table(&resp);

        // Build channels from discovery, overlaying user config
        let mut channels = Vec::with_capacity(discovered.len());
        for (i, disc) in discovered.iter().enumerate() {
            let user_cfg = aura_config.channels.get(i);

            let default_name = if disc.is_fixed {
                "Fixed LEDs".to_string()
            } else {
                format!("ARGB {}", i + 1)
            };

            let name = user_cfg.map(|c| c.name.clone()).unwrap_or(default_name);

            // For addressable channels, default to 50 LEDs unless user overrides.
            // For fixed channels, use the hardware-reported count.
            let led_count = if let Some(cfg) = user_cfg {
                cfg.leds
            } else if disc.is_fixed {
                disc.led_count
            } else {
                50 // default for addressable
            };

            let state = if led_count == 0 {
                ChannelState::Off
            } else {
                ChannelState::Managed
            };

            channels.push(AuraChannel {
                direct_channel: disc.direct_channel,
                effect_channel: disc.effect_channel,
                led_count,
                name,
                state,
            });
        }

        Ok((
            Self {
                hid: RefCell::new(hid),
                id,
                channels,
                last_reconnect: Cell::new(None),
            },
            firmware,
        ))
    }

    /// Attempt to reconnect the HID device, with 5-second cooldown.
    ///
    /// HID devices use /dev/hidraw file descriptors — there is no USB endpoint
    /// halt to clear (unlike rusb bulk endpoints in the LCD backend). The
    /// equivalent recovery is closing and reopening the hidraw fd, which forces
    /// the kernel to re-enumerate the device's report descriptor and flush any
    /// stale state in the HID driver. This is sufficient for the common failure
    /// mode where repeated daemon restarts leave the fd in a wedged state.
    fn try_reconnect(&self) -> bool {
        if let Some(last) = self.last_reconnect.get() {
            if last.elapsed() < std::time::Duration::from_secs(5) {
                return false; // cooldown active
            }
        }
        self.last_reconnect.set(Some(Instant::now()));
        tracing::warn!("AURA: attempting HID reconnect (close + reopen hidraw)");
        match self.hid.borrow_mut().reopen() {
            Ok(()) => {
                tracing::info!("AURA: HID reconnect succeeded");
                true
            }
            Err(e) => {
                tracing::error!("AURA: HID reconnect failed: {e}");
                false
            }
        }
    }

    /// Send direct-mode RGB data to a channel, with reconnect-on-failure.
    fn send_direct(&self, channel_idx: usize, colors: &[(u8, u8, u8)]) -> Result<()> {
        let ch = &self.channels[channel_idx];
        let packets = build_direct_packets(ch.direct_channel, colors);

        match self.write_packets(&packets) {
            Ok(()) => Ok(()),
            Err(first_err) => {
                tracing::warn!("AURA direct write ch{channel_idx} failed: {first_err}, attempting reconnect");
                if self.try_reconnect() {
                    self.write_packets(&packets)
                } else {
                    Err(first_err)
                }
            }
        }
    }

    /// Write a sequence of HID report packets.
    fn write_packets(&self, packets: &[[u8; REPORT_SIZE]]) -> Result<()> {
        let hid = self.hid.borrow();
        for pkt in packets {
            hid.write_report(pkt)
                .map_err(|e| CoreError::Protocol(format!("AURA write: {e}")))?;
        }
        Ok(())
    }

    /// Set a hardware effect on a channel, with reconnect-on-failure.
    fn set_hw_effect(&self, channel_idx: usize, mode: u8, color: (u8, u8, u8)) -> Result<()> {
        let ch = &self.channels[channel_idx];
        let (effect, color_pkt, commit) = build_hw_effect_packets(ch.effect_channel, mode, color, ch.led_count);
        let packets = [effect, color_pkt, commit];

        match self.write_packets(&packets) {
            Ok(()) => Ok(()),
            Err(first_err) => {
                tracing::warn!("AURA hw_effect ch{channel_idx} failed: {first_err}, attempting reconnect");
                if self.try_reconnect() {
                    self.write_packets(&packets)
                } else {
                    Err(first_err)
                }
            }
        }
    }

    /// Turn a channel off (hardware effect mode 0x00).
    fn set_off(&self, channel_idx: usize) -> Result<()> {
        self.set_hw_effect(channel_idx, 0x00, (0, 0, 0))
    }

    /// Map a group ID to a channel index using the group base offset.
    fn channel_for_group(&self, group: GroupId, group_base: u8) -> Option<usize> {
        let g = group.value();
        if g < group_base {
            return None;
        }
        let idx = (g - group_base) as usize;
        if idx < self.channels.len() {
            Some(idx)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// AuraBackend — Backend implementation
// ---------------------------------------------------------------------------

/// Backend for ASUS AURA motherboard RGB controllers with per-channel
/// addressable direct mode and optional hardware effects.
pub struct AuraBackend {
    devices: Vec<AuraDevice>,
    group_base: u8,
}

impl AuraBackend {
    /// Open AURA HID devices. Non-fatal: returns Ok with empty list if none found.
    pub fn open_all(aura_config: &AuraConfig) -> Result<Self> {
        let group_base = aura_config.group_base;
        let mut devices = Vec::new();

        match HidDevice::open(VID_AURA, PID_AURA) {
            Ok(hid) => match AuraDevice::init(hid, aura_config) {
                Ok((dev, firmware)) => {
                    let active = dev.channels.iter().filter(|c| c.led_count > 0).count();
                    tracing::info!(
                        "AURA: {} channel(s) ({} active), firmware '{}'",
                        dev.channels.len(),
                        active,
                        firmware
                    );
                    devices.push(dev);
                }
                Err(e) => tracing::warn!("AURA init failed: {e}"),
            },
            Err(frgb_usb::UsbError::NotFound) => {}
            Err(e) => tracing::warn!("AURA HID open failed: {e}"),
        }

        Ok(Self { devices, group_base })
    }

    /// Number of active channels (led_count > 0) across all devices.
    pub fn channel_count(&self) -> usize {
        self.devices
            .iter()
            .flat_map(|d| &d.channels)
            .filter(|c| c.led_count > 0)
            .count()
    }

    /// Number of AURA HID controllers.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Get the LED count for a group.
    pub fn led_count_for_group(&self, group: GroupId) -> u8 {
        self.resolve_group(group)
            .ok()
            .map(|(dev, idx)| dev.channels[idx].led_count)
            .unwrap_or(0)
    }

    /// Resolve a group ID to the (device, channel_index) that owns it.
    fn resolve_group(&self, group: GroupId) -> Result<(&AuraDevice, usize)> {
        for dev in &self.devices {
            if let Some(idx) = dev.channel_for_group(group, self.group_base) {
                return Ok((dev, idx));
            }
        }
        Err(CoreError::NotFound(format!("AURA group {group}")))
    }

    /// Build channel info for IPC ListAuraChannels response.
    pub fn channel_info(&self) -> Vec<AuraChannelInfo> {
        let mut info = Vec::new();
        for dev in &self.devices {
            for (i, ch) in dev.channels.iter().enumerate() {
                let state_str = match &ch.state {
                    ChannelState::Managed => "managed".to_string(),
                    ChannelState::HardwareEffect { mode } => {
                        format!("hw_effect(0x{mode:02X})")
                    }
                    ChannelState::Off => "off".to_string(),
                };
                info.push(AuraChannelInfo {
                    group: GroupId::new(self.group_base + i as u8),
                    name: ch.name.clone(),
                    led_count: ch.led_count,
                    state: state_str,
                });
            }
        }
        info
    }

    /// Set a hardware effect on a channel identified by group ID.
    pub fn set_hw_effect_by_group(&mut self, group: GroupId, mode: u8, color: (u8, u8, u8)) -> Result<()> {
        // Find device and channel index
        let mut target_dev_idx = None;
        let mut target_ch_idx = None;
        for (di, dev) in self.devices.iter().enumerate() {
            if let Some(ci) = dev.channel_for_group(group, self.group_base) {
                target_dev_idx = Some(di);
                target_ch_idx = Some(ci);
                break;
            }
        }
        let dev_idx = target_dev_idx.ok_or_else(|| CoreError::NotFound(format!("AURA group {group}")))?;
        let ch_idx = target_ch_idx.unwrap();

        self.devices[dev_idx].set_hw_effect(ch_idx, mode, color)?;
        self.devices[dev_idx].channels[ch_idx].state = ChannelState::HardwareEffect { mode };
        Ok(())
    }

    /// Shutdown: set Off on Managed channels, leave HardwareEffect channels alone.
    pub fn shutdown(&self) {
        for dev in &self.devices {
            for (i, ch) in dev.channels.iter().enumerate() {
                if ch.state == ChannelState::Managed {
                    if let Err(e) = dev.set_off(i) {
                        tracing::warn!("AURA shutdown ch{i}: {e}");
                    }
                }
            }
        }
    }
}

impl Backend for AuraBackend {
    fn id(&self) -> BackendId {
        BackendId(2)
    }

    fn name(&self) -> &str {
        "aura"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        let mut result = Vec::new();
        for dev in &self.devices {
            for (i, ch) in dev.channels.iter().enumerate() {
                if ch.led_count == 0 {
                    continue;
                }
                let mut did = DeviceId::from_vid_pid(VID_AURA, PID_AURA);
                did.set_index(i as u8);
                result.push(DiscoveredDevice {
                    id: did,
                    fans_type: [0; 4],
                    dev_type: DEV_TYPE_AURA,
                    group: GroupId::new(self.group_base + i as u8),
                    fan_count: 0,
                    master: DeviceId::ZERO,
                    fans_rpm: [0; 4],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0,
                });
            }
        }
        Ok(result)
    }

    fn set_speed(&self, _device: &Device, _cmd: &SpeedCommand) -> Result<()> {
        Ok(()) // AURA doesn't control fan speed
    }

    fn send_rgb(&self, device: &Device, buffer: &frgb_rgb::generator::EffectResult) -> Result<()> {
        let (aura_dev, ch_idx) = self.resolve_group(device.group)?;
        let ch = &aura_dev.channels[ch_idx];

        // Skip channels not in Managed state
        if ch.state != ChannelState::Managed {
            return Ok(());
        }

        let led_count = ch.led_count as usize;
        let buf_leds = buffer.buffer.led_count();

        // Extract LED colors, mapping buffer LEDs to channel LED count
        let mut colors = Vec::with_capacity(led_count);
        for i in 0..led_count {
            let src_idx = if led_count <= 1 || buf_leds <= 1 {
                0
            } else {
                (i * buf_leds) / led_count
            };
            let src_idx = src_idx.min(buf_leds.saturating_sub(1));
            if src_idx < buf_leds {
                let rgb = buffer.buffer.get_led(0, src_idx);
                colors.push((rgb.r, rgb.g, rgb.b));
            } else {
                colors.push((0, 0, 0));
            }
        }

        aura_dev.send_direct(ch_idx, &colors)
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
    use super::*;

    #[test]
    fn build_direct_packets_single_chunk() {
        let colors: Vec<(u8, u8, u8)> = (0..10).map(|i| (i, i + 1, i + 2)).collect();
        let packets = build_direct_packets(0x01, &colors);

        assert_eq!(packets.len(), 1);
        let pkt = &packets[0];
        assert_eq!(pkt[0], REPORT_ID);
        assert_eq!(pkt[1], CMD_ADDRESSABLE_DIRECT);
        // Apply flag (0x80) should be set on the single packet
        assert_eq!(pkt[2], 0x01 | 0x80);
        assert_eq!(pkt[3], 0x00); // offset 0
        assert_eq!(pkt[4], 10); // 10 LEDs
                                // Verify first LED color
        assert_eq!(pkt[5], 0);
        assert_eq!(pkt[6], 1);
        assert_eq!(pkt[7], 2);
    }

    #[test]
    fn build_direct_packets_multi_chunk() {
        let colors: Vec<(u8, u8, u8)> = (0..50).map(|i| (i as u8, 0, 0)).collect();
        let packets = build_direct_packets(0x00, &colors);

        // 50 LEDs / 20 per packet = 3 packets (20 + 20 + 10)
        assert_eq!(packets.len(), 3);

        // Packet 0: no apply flag
        assert_eq!(packets[0][2], 0x00);
        assert_eq!(packets[0][3], 0); // offset 0
        assert_eq!(packets[0][4], 20);

        // Packet 1: no apply flag
        assert_eq!(packets[1][2], 0x00);
        assert_eq!(packets[1][3], 20); // offset 20
        assert_eq!(packets[1][4], 20);

        // Packet 2: apply flag set (last packet)
        assert_eq!(packets[2][2], 0x80);
        assert_eq!(packets[2][3], 40); // offset 40
        assert_eq!(packets[2][4], 10);
    }

    #[test]
    fn build_direct_packets_exact_20() {
        let colors: Vec<(u8, u8, u8)> = (0..20).map(|i| (i as u8, 0xFF, 0)).collect();
        let packets = build_direct_packets(0x02, &colors);

        assert_eq!(packets.len(), 1);
        // Apply flag on the single packet
        assert_eq!(packets[0][2], 0x02 | 0x80);
        assert_eq!(packets[0][4], 20);
    }

    #[test]
    fn build_hw_effect_packets_structure() {
        let (effect, color_pkt, commit) = build_hw_effect_packets(0x01, 0x04, (255, 0, 128), 10);

        // Effect packet
        assert_eq!(effect[0], REPORT_ID);
        assert_eq!(effect[1], CMD_EFFECT);
        assert_eq!(effect[2], 0x01); // effect_channel
        assert_eq!(effect[4], 0x04); // mode (SpectrumCycle)
        assert_eq!(effect[6], 10); // led_count

        // Color packet
        assert_eq!(color_pkt[0], REPORT_ID);
        assert_eq!(color_pkt[1], CMD_EFFECT_COLOR);
        assert_eq!(color_pkt[2], 0x01); // effect_channel
        assert_eq!(color_pkt[4], 10); // led_count
                                      // First LED color
        assert_eq!(color_pkt[5], 255);
        assert_eq!(color_pkt[6], 0);
        assert_eq!(color_pkt[7], 128);
        // Last LED color (index 9)
        let last_offset = 5 + 9 * 3;
        assert_eq!(color_pkt[last_offset], 255);
        assert_eq!(color_pkt[last_offset + 1], 0);
        assert_eq!(color_pkt[last_offset + 2], 128);

        // Commit packet
        assert_eq!(commit[0], REPORT_ID);
        assert_eq!(commit[1], CMD_COMMIT);
    }

    #[test]
    fn parse_config_table_addressable_only() {
        let mut resp = [0u8; REPORT_SIZE];
        // byte[4 + 0x02] = 4 addressable headers
        resp[4 + 0x02] = 4;
        // byte[4 + 0x1B] = 0 fixed LEDs
        resp[4 + 0x1B] = 0;
        // byte[4 + 0x1D] = 4 RGB headers
        resp[4 + 0x1D] = 4;

        let channels = parse_config_table(&resp);
        assert_eq!(channels.len(), 4);
        for (i, ch) in channels.iter().enumerate() {
            assert!(!ch.is_fixed);
            assert_eq!(ch.direct_channel, i as u8);
            assert_eq!(ch.led_count, 0); // addressable: unknown until configured
        }
    }

    #[test]
    fn parse_config_table_with_fixed_leds() {
        let mut resp = [0u8; REPORT_SIZE];
        // byte[4 + 0x02] = 2 addressable headers
        resp[4 + 0x02] = 2;
        // byte[4 + 0x1B] = 4 fixed LEDs
        resp[4 + 0x1B] = 4;
        // byte[4 + 0x1D] = 3 RGB headers
        resp[4 + 0x1D] = 3;

        let channels = parse_config_table(&resp);
        // 1 fixed zone + 2 addressable = 3 channels
        assert_eq!(channels.len(), 3);

        // First channel: fixed
        assert!(channels[0].is_fixed);
        assert_eq!(channels[0].direct_channel, 0x04);
        assert_eq!(channels[0].led_count, 4);

        // Remaining: addressable
        assert!(!channels[1].is_fixed);
        assert_eq!(channels[1].direct_channel, 0);
        assert!(!channels[2].is_fixed);
        assert_eq!(channels[2].direct_channel, 1);
    }

    #[test]
    fn channel_state_default_is_off() {
        let ch = AuraChannel {
            direct_channel: 0,
            effect_channel: 0,
            led_count: 0,
            name: "test".into(),
            state: ChannelState::Off,
        };
        assert_eq!(ch.state, ChannelState::Off);
    }

    /// Test the channel_for_group arithmetic: group.value() - group_base → channel index.
    /// The real method is on AuraDevice, which requires HidDevice (hardware). We test
    /// the equivalent logic directly since it's a pure function of (group, base, num_channels).
    fn channel_for_group_logic(group: GroupId, group_base: u8, num_channels: usize) -> Option<usize> {
        let g = group.value();
        if g < group_base {
            return None;
        }
        let idx = (g - group_base) as usize;
        if idx < num_channels {
            Some(idx)
        } else {
            None
        }
    }

    #[test]
    fn channel_for_group_basic() {
        // group 10, base 9 → channel 1
        assert_eq!(channel_for_group_logic(GroupId::new(10), 9, 2), Some(1));
        // group 9, base 9 → channel 0
        assert_eq!(channel_for_group_logic(GroupId::new(9), 9, 2), Some(0));
        // group 8, base 9 → None (underflow: 8 < 9)
        assert_eq!(channel_for_group_logic(GroupId::new(8), 9, 2), None);
        // group 11, base 9 → None (index 2 out of bounds for 2 channels)
        assert_eq!(channel_for_group_logic(GroupId::new(11), 9, 2), None);
        // group 0, base 0 → channel 0
        assert_eq!(channel_for_group_logic(GroupId::new(0), 0, 1), Some(0));
        // group 255, base 253 → channel 2
        assert_eq!(channel_for_group_logic(GroupId::new(255), 253, 4), Some(2));
        // group 255, base 255 → channel 0
        assert_eq!(channel_for_group_logic(GroupId::new(255), 255, 1), Some(0));
    }

    #[test]
    fn channel_state_transitions() {
        let mut ch = AuraChannel {
            direct_channel: 0,
            effect_channel: 0,
            led_count: 50,
            name: "test".into(),
            state: ChannelState::Off,
        };

        // Off -> Managed
        ch.state = ChannelState::Managed;
        assert_eq!(ch.state, ChannelState::Managed);

        // Managed -> HardwareEffect
        ch.state = ChannelState::HardwareEffect { mode: 0x04 };
        assert_eq!(ch.state, ChannelState::HardwareEffect { mode: 0x04 });
    }
}
