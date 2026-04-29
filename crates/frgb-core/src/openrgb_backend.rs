//! OpenRGB backend — TCP client to an OpenRGB server for GPU, RAM, peripheral RGB.
//!
//! Connects to an OpenRGB server (default port 6742) and discovers devices.
//! Each OpenRGB device becomes a frgb device group that can receive RGB commands.
//! The server does the actual hardware I/O — this backend is a passthrough client.

use std::any::Any;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use frgb_model::device::DeviceId;
use frgb_model::GroupId;

use crate::backend::{Backend, BackendId, DiscoveredDevice, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;

// ---------------------------------------------------------------------------
// OpenRGB SDK protocol constants
// ---------------------------------------------------------------------------

/// Default OpenRGB server port.
const DEFAULT_PORT: u16 = 6742;

/// Protocol magic bytes: "ORGB"
const MAGIC: [u8; 4] = [b'O', b'R', b'G', b'B'];

/// Header size: magic(4) + device_id(4) + pkt_type(4) + pkt_size(4)
const HEADER_SIZE: usize = 16;

/// Connection timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Read timeout.
const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// Synthetic dev_type for OpenRGB devices.
const DEV_TYPE_OPENRGB: u8 = 0xFB;

// Packet types (OpenRGB SDK protocol v0)
const PKT_REQUEST_CONTROLLER_COUNT: u32 = 0;
const PKT_REQUEST_CONTROLLER_DATA: u32 = 1;
const PKT_SET_CLIENT_NAME: u32 = 50;
const PKT_RGBCONTROLLER_UPDATELEDS: u32 = 1050;

// ---------------------------------------------------------------------------
// Protocol encoding / decoding
// ---------------------------------------------------------------------------

fn build_header(device_id: u32, pkt_type: u32, pkt_size: u32) -> [u8; HEADER_SIZE] {
    let mut buf = [0u8; HEADER_SIZE];
    buf[0..4].copy_from_slice(&MAGIC);
    buf[4..8].copy_from_slice(&device_id.to_le_bytes());
    buf[8..12].copy_from_slice(&pkt_type.to_le_bytes());
    buf[12..16].copy_from_slice(&pkt_size.to_le_bytes());
    buf
}

fn parse_header(buf: &[u8; HEADER_SIZE]) -> Option<(u32, u32, u32)> {
    if buf[0..4] != MAGIC {
        return None;
    }
    let device_id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let pkt_type = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let pkt_size = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    Some((device_id, pkt_type, pkt_size))
}

/// Parse a u16-prefixed string from OpenRGB protocol data.
fn parse_string(data: &[u8], offset: &mut usize) -> String {
    if *offset + 2 > data.len() {
        return String::new();
    }
    let len = u16::from_le_bytes([data[*offset], data[*offset + 1]]) as usize;
    *offset += 2;
    if *offset + len > data.len() || len == 0 {
        return String::new();
    }
    // Strings are null-terminated
    let end = if data[*offset + len - 1] == 0 { len - 1 } else { len };
    let s = String::from_utf8_lossy(&data[*offset..*offset + end]).into();
    *offset += len;
    s
}

// ---------------------------------------------------------------------------
// OpenRGB device info (minimal — just what we need for discovery)
// ---------------------------------------------------------------------------

struct OrgbDevice {
    index: u32,
    name: String,
}

// ---------------------------------------------------------------------------
// OpenRgbBackend
// ---------------------------------------------------------------------------

/// Backend for OpenRGB server. Connects via TCP, discovers devices, sends LED updates.
pub struct OpenRgbBackend {
    stream: Option<TcpStream>,
    devices: Vec<OrgbDevice>,
}

impl OpenRgbBackend {
    /// Connect to an OpenRGB server.
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        let addr = format!("{}:{}", host, port);
        let socket_addr = addr
            .to_socket_addrs()
            .map_err(|e| CoreError::Protocol(format!("OpenRGB resolve: {e}")))?
            .next()
            .ok_or_else(|| CoreError::Protocol(format!("OpenRGB: cannot resolve {addr}")))?;

        let stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
            .map_err(|e| CoreError::Protocol(format!("OpenRGB connect: {e}")))?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|e| CoreError::Protocol(format!("OpenRGB timeout: {e}")))?;

        let mut backend = Self {
            stream: Some(stream),
            devices: Vec::new(),
        };

        backend.set_client_name("frgb")?;
        Ok(backend)
    }

    /// Try to connect to the default OpenRGB server. Non-fatal if server not running.
    pub fn open_default() -> Result<Self> {
        Self::connect("127.0.0.1", DEFAULT_PORT)
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    fn stream(&self) -> Result<&TcpStream> {
        self.stream
            .as_ref()
            .ok_or_else(|| CoreError::Protocol("OpenRGB: not connected".into()))
    }

    fn send_packet(&self, device_id: u32, pkt_type: u32, payload: &[u8]) -> Result<()> {
        let stream = self.stream()?;
        let header = build_header(device_id, pkt_type, payload.len() as u32);
        (&*stream)
            .write_all(&header)
            .map_err(|e| CoreError::Protocol(format!("OpenRGB send header: {e}")))?;
        if !payload.is_empty() {
            (&*stream)
                .write_all(payload)
                .map_err(|e| CoreError::Protocol(format!("OpenRGB send payload: {e}")))?;
        }
        Ok(())
    }

    fn recv_packet(&self) -> Result<(u32, u32, Vec<u8>)> {
        let stream = self.stream()?;
        let mut header_buf = [0u8; HEADER_SIZE];
        (&*stream)
            .read_exact(&mut header_buf)
            .map_err(|e| CoreError::Protocol(format!("OpenRGB recv header: {e}")))?;

        let (device_id, pkt_type, pkt_size) =
            parse_header(&header_buf).ok_or_else(|| CoreError::Protocol("OpenRGB: invalid header magic".into()))?;

        let mut payload = vec![0u8; pkt_size as usize];
        if pkt_size > 0 {
            (&*stream)
                .read_exact(&mut payload)
                .map_err(|e| CoreError::Protocol(format!("OpenRGB recv payload: {e}")))?;
        }

        Ok((device_id, pkt_type, payload))
    }

    fn set_client_name(&mut self, name: &str) -> Result<()> {
        let mut payload = name.as_bytes().to_vec();
        payload.push(0); // null-terminated
        self.send_packet(0, PKT_SET_CLIENT_NAME, &payload)
    }

    fn request_controller_count(&self) -> Result<u32> {
        self.send_packet(0, PKT_REQUEST_CONTROLLER_COUNT, &[])?;
        let (_dev, _pkt, payload) = self.recv_packet()?;
        if payload.len() < 4 {
            return Err(CoreError::Protocol("OpenRGB: controller count too short".into()));
        }
        Ok(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
    }

    fn request_controller_data(&self, index: u32) -> Result<OrgbDevice> {
        // Protocol version 0: send controller index, no version negotiation
        self.send_packet(index, PKT_REQUEST_CONTROLLER_DATA, &0u32.to_le_bytes())?;
        let (_dev, _pkt, data) = self.recv_packet()?;

        // Minimal parsing — extract name, zone count, LED count
        // Controller data format (protocol v0):
        // u32 data_size, u32 type, string name, string vendor, string description, ...
        if data.len() < 8 {
            return Err(CoreError::Protocol("OpenRGB: controller data too short".into()));
        }

        let mut offset = 4; // skip data_size (u32 at start, redundant with header)
        let _device_type = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        offset += 4;

        let name = parse_string(&data, &mut offset);
        let _vendor = parse_string(&data, &mut offset);
        let _description = parse_string(&data, &mut offset);
        let _version = parse_string(&data, &mut offset);
        let _serial = parse_string(&data, &mut offset);
        let _location = parse_string(&data, &mut offset);

        Ok(OrgbDevice { index, name })
    }

    /// Build LED update payload for a single device (all zones).
    fn build_led_update(colors: &[(u8, u8, u8)]) -> Vec<u8> {
        let count = colors.len() as u32;
        // Payload: u32 data_size, u32 led_count, then [u8 r, u8 g, u8 b, u8 pad] × count
        let data_size = 4 + count * 4;
        let mut payload = Vec::with_capacity(4 + data_size as usize);
        payload.extend_from_slice(&data_size.to_le_bytes());
        payload.extend_from_slice(&count.to_le_bytes());
        for &(r, g, b) in colors {
            payload.extend_from_slice(&[r, g, b, 0]); // RGB + padding
        }
        payload
    }
}

impl Backend for OpenRgbBackend {
    fn id(&self) -> BackendId {
        BackendId(4)
    }
    fn name(&self) -> &str {
        "openrgb"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        self.devices.clear();

        let count = match self.request_controller_count() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("OpenRGB discovery failed: {e}");
                return Ok(Vec::new());
            }
        };

        for i in 0..count {
            match self.request_controller_data(i) {
                Ok(dev) => {
                    tracing::debug!("OpenRGB device {}: '{}'", i, dev.name);
                    self.devices.push(dev);
                }
                Err(e) => tracing::warn!("OpenRGB device {} query failed: {e}", i),
            }
        }

        Ok(self
            .devices
            .iter()
            .map(|dev| {
                // Synthesize a DeviceId from the device index
                let id = DeviceId::from([
                    0xFF,
                    0xFE, // OpenRGB marker
                    (dev.index >> 8) as u8,
                    (dev.index & 0xFF) as u8,
                    0,
                    0,
                ]);
                DiscoveredDevice {
                    id,
                    fans_type: [0; 4],
                    dev_type: DEV_TYPE_OPENRGB,
                    group: GroupId::new(0),
                    fan_count: 0,
                    master: DeviceId::ZERO,
                    fans_rpm: [0; 4],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0,
                }
            })
            .collect())
    }

    fn set_speed(&self, _device: &Device, _cmd: &SpeedCommand) -> Result<()> {
        Ok(()) // OpenRGB doesn't control fan speed
    }

    fn send_rgb(&self, device: &Device, buffer: &frgb_rgb::generator::EffectResult) -> Result<()> {
        // Find the OrgbDevice by matching DeviceId
        let orgb_dev = self
            .devices
            .iter()
            .find(|d| {
                let expected = DeviceId::from([0xFF, 0xFE, (d.index >> 8) as u8, (d.index & 0xFF) as u8, 0, 0]);
                expected == device.id
            })
            .ok_or_else(|| CoreError::NotFound(format!("OpenRGB device {}", device.id.to_hex())))?;

        let led_count = buffer.buffer.led_count();
        let mut colors = Vec::with_capacity(led_count);
        for i in 0..led_count {
            let rgb = buffer.buffer.get_led(0, i);
            colors.push((rgb.r, rgb.g, rgb.b));
        }

        let payload = Self::build_led_update(&colors);
        self.send_packet(orgb_dev.index, PKT_RGBCONTROLLER_UPDATELEDS, &payload)
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
    fn header_roundtrip() {
        let header = build_header(42, PKT_REQUEST_CONTROLLER_COUNT, 100);
        let (dev, pkt, size) = parse_header(&header).unwrap();
        assert_eq!(dev, 42);
        assert_eq!(pkt, PKT_REQUEST_CONTROLLER_COUNT);
        assert_eq!(size, 100);
    }

    #[test]
    fn header_invalid_magic() {
        let mut header = build_header(0, 0, 0);
        header[0] = 0xFF;
        assert!(parse_header(&header).is_none());
    }

    #[test]
    fn parse_string_basic() {
        // u16 length (5) + "test\0"
        let data = [5, 0, b't', b'e', b's', b't', 0];
        let mut offset = 0;
        assert_eq!(parse_string(&data, &mut offset), "test");
        assert_eq!(offset, 7);
    }

    #[test]
    fn parse_string_empty() {
        let data = [0, 0];
        let mut offset = 0;
        assert_eq!(parse_string(&data, &mut offset), "");
        assert_eq!(offset, 2);
    }

    #[test]
    fn build_led_update_structure() {
        let payload = OpenRgbBackend::build_led_update(&[(255, 0, 128), (0, 255, 64)]);
        // u32 data_size + u32 count + 2 × 4 bytes
        assert_eq!(payload.len(), 4 + 4 + 8);
        // data_size = 4 + 2*4 = 12
        assert_eq!(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]), 12);
        // count = 2
        assert_eq!(u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]), 2);
        // First LED: R=255, G=0, B=128, pad=0
        assert_eq!(&payload[8..12], &[255, 0, 128, 0]);
        // Second LED: R=0, G=255, B=64, pad=0
        assert_eq!(&payload[12..16], &[0, 255, 64, 0]);
    }
}
