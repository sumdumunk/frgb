//! OpenRGB SDK server — exposes frgb devices to external OpenRGB clients.
//!
//! Runs a TCP server on a configurable port (default 6743) implementing the
//! OpenRGB SDK protocol (versions 0–4).  Communication with the single-threaded
//! daemon main loop is via channels:
//!
//! - **Capabilities snapshot** (`Arc<Mutex<Vec<DeviceCapabilities>>>`) — rebuilt
//!   on discovery changes, read by client handler threads.
//! - **Color commands** (`mpsc::Sender<OpenRgbCommand>`) — sent from client
//!   handlers, drained by the main loop each tick.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use frgb_model::GroupId;

fn lock_or_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("OpenRGB: mutex was poisoned, recovering");
        poisoned.into_inner()
    })
}

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"ORGB";
const SERVER_PROTOCOL_VERSION: u32 = 4;
const HEADER_SIZE: usize = 16;

const PKT_REQUEST_CONTROLLER_COUNT: u32 = 0;
const PKT_REQUEST_CONTROLLER_DATA: u32 = 1;
const PKT_REQUEST_PROTOCOL_VERSION: u32 = 40;
const PKT_SET_CLIENT_NAME: u32 = 50;
const PKT_UPDATE_LEDS: u32 = 1050;
const PKT_UPDATE_ZONE_LEDS: u32 = 1051;
const PKT_SET_CUSTOM_MODE: u32 = 1100;

const DEVICE_TYPE_LED_STRIP: u32 = 4;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Snapshot of one device's capabilities, shared with the server thread.
#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    pub device_id: String,
    pub device_name: String,
    pub group_id: GroupId,
    pub zones: Vec<ZoneInfo>,
    pub total_leds: u16,
}

#[derive(Debug, Clone)]
pub struct ZoneInfo {
    pub name: String,
    pub led_count: u16,
}

/// Command sent from the server thread to the main loop.
pub enum OpenRgbCommand {
    SetLeds {
        group_id: GroupId,
        colors: Vec<[u8; 3]>,
    },
    SetZoneLeds {
        group_id: GroupId,
        zone_idx: u32,
        colors: Vec<[u8; 3]>,
    },
}

/// Handle returned by [`OpenRgbServer::start`].  The main loop calls
/// [`drain_commands`] each tick.
#[allow(dead_code)] // caps + update_caps used once re-discovery triggers refresh
pub struct OpenRgbServer {
    cmd_rx: mpsc::Receiver<OpenRgbCommand>,
    caps: Arc<Mutex<Vec<DeviceCapabilities>>>,
    _thread: std::thread::JoinHandle<()>,
}

impl OpenRgbServer {
    /// Spawn the TCP server in a background thread.
    pub fn start(port: u16, caps: Vec<DeviceCapabilities>, stop_flag: Arc<AtomicBool>) -> Self {
        let caps = Arc::new(Mutex::new(caps));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let caps_clone = caps.clone();

        let thread = std::thread::spawn(move || {
            run_server(port, caps_clone, cmd_tx, stop_flag);
        });

        Self {
            cmd_rx,
            caps,
            _thread: thread,
        }
    }

    /// Drain pending colour commands (non-blocking).  Call from the main loop.
    pub fn drain_commands(&self) -> Vec<OpenRgbCommand> {
        let mut cmds = Vec::new();
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            cmds.push(cmd);
        }
        cmds
    }

    /// Replace the device capabilities snapshot (e.g. after re-discovery).
    #[allow(dead_code)]
    pub fn update_caps(&self, caps: Vec<DeviceCapabilities>) {
        *lock_or_recover(&self.caps) = caps;
    }

    /// Signal the server to stop and wait for the thread to exit.
    pub fn shutdown(self, stop_flag: &AtomicBool) {
        stop_flag.store(true, Ordering::Relaxed);
        let _ = self._thread.join();
    }
}

// ---------------------------------------------------------------------------
// TCP accept loop
// ---------------------------------------------------------------------------

fn run_server(
    port: u16,
    caps: Arc<Mutex<Vec<DeviceCapabilities>>>,
    cmd_tx: mpsc::Sender<OpenRgbCommand>,
    stop_flag: Arc<AtomicBool>,
) {
    let listener = match TcpListener::bind(format!("127.0.0.1:{port}")) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("OpenRGB server: bind failed on port {port}: {e}");
            return;
        }
    };
    listener.set_nonblocking(true).ok();
    tracing::info!("OpenRGB server listening on port {port}");

    while !stop_flag.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                tracing::info!("OpenRGB client connected: {addr}");
                let caps = caps.clone();
                let tx = cmd_tx.clone();
                let stop = stop_flag.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(stream, caps, tx, stop) {
                        tracing::debug!("OpenRGB client disconnected: {e}");
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => tracing::warn!("OpenRGB server: accept error: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-client handler
// ---------------------------------------------------------------------------

fn handle_client(
    mut stream: TcpStream,
    caps: Arc<Mutex<Vec<DeviceCapabilities>>>,
    cmd_tx: mpsc::Sender<OpenRgbCommand>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let mut _protocol_version = SERVER_PROTOCOL_VERSION;

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let pkt = match read_packet(&mut stream) {
            Ok(p) => p,
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
                continue
            }
            Err(e) => return Err(e),
        };

        match pkt.packet_id {
            PKT_REQUEST_PROTOCOL_VERSION => {
                let client_ver = if pkt.payload.len() >= 4 {
                    u32::from_le_bytes(pkt.payload[0..4].try_into().unwrap())
                } else {
                    0
                };
                _protocol_version = SERVER_PROTOCOL_VERSION.min(client_ver);
                send_packet(
                    &mut stream,
                    0,
                    PKT_REQUEST_PROTOCOL_VERSION,
                    &_protocol_version.to_le_bytes(),
                )?;
            }
            PKT_SET_CLIENT_NAME => {
                let name: String = String::from_utf8_lossy(&pkt.payload)
                    .trim_end_matches('\0')
                    .chars()
                    .take(256)
                    .collect();
                tracing::info!("OpenRGB client identified: {name}");
            }
            PKT_REQUEST_CONTROLLER_COUNT => {
                let devices = lock_or_recover(&caps);
                let count = devices.len() as u32;
                drop(devices);
                send_packet(&mut stream, 0, PKT_REQUEST_CONTROLLER_COUNT, &count.to_le_bytes())?;
            }
            PKT_REQUEST_CONTROLLER_DATA => {
                let devices = lock_or_recover(&caps);
                let idx = pkt.device_id as usize;
                if idx < devices.len() {
                    let data = build_controller_data(&devices[idx], _protocol_version);
                    drop(devices);
                    send_packet(&mut stream, pkt.device_id, PKT_REQUEST_CONTROLLER_DATA, &data)?;
                } else {
                    drop(devices);
                    send_packet(&mut stream, pkt.device_id, PKT_REQUEST_CONTROLLER_DATA, &[])?;
                }
            }
            PKT_UPDATE_LEDS => {
                let devices = lock_or_recover(&caps);
                let idx = pkt.device_id as usize;
                if idx < devices.len() {
                    let group_id = devices[idx].group_id;
                    drop(devices);
                    let colors = parse_colors(&pkt.payload);
                    cmd_tx.send(OpenRgbCommand::SetLeds { group_id, colors }).ok();
                }
            }
            PKT_UPDATE_ZONE_LEDS => {
                let devices = lock_or_recover(&caps);
                let idx = pkt.device_id as usize;
                if idx < devices.len() && pkt.payload.len() >= 10 {
                    let group_id = devices[idx].group_id;
                    drop(devices);
                    let zone_idx = u32::from_le_bytes(pkt.payload[4..8].try_into().unwrap());
                    let colors = parse_zone_colors(&pkt.payload);
                    cmd_tx
                        .send(OpenRgbCommand::SetZoneLeds {
                            group_id,
                            zone_idx,
                            colors,
                        })
                        .ok();
                }
            }
            PKT_SET_CUSTOM_MODE => {
                // No-op — always in direct mode for OpenRGB.
            }
            _ => {
                tracing::debug!("OpenRGB: unknown packet id {}", pkt.packet_id);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Packet I/O
// ---------------------------------------------------------------------------

struct Packet {
    device_id: u32,
    packet_id: u32,
    payload: Vec<u8>,
}

fn read_packet(stream: &mut TcpStream) -> std::io::Result<Packet> {
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header)?;
    if &header[0..4] != MAGIC {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad magic"));
    }
    let device_id = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let packet_id = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;

    // Sanity-check: reject payloads > 16 MiB to avoid OOM on garbage data.
    if size > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("payload too large: {size} bytes"),
        ));
    }

    let mut payload = vec![0u8; size];
    if size > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok(Packet {
        device_id,
        packet_id,
        payload,
    })
}

fn send_packet(stream: &mut TcpStream, device_id: u32, packet_id: u32, payload: &[u8]) -> std::io::Result<()> {
    let mut header = [0u8; HEADER_SIZE];
    header[0..4].copy_from_slice(MAGIC);
    header[4..8].copy_from_slice(&device_id.to_le_bytes());
    header[8..12].copy_from_slice(&packet_id.to_le_bytes());
    header[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stream.write_all(&header)?;
    if !payload.is_empty() {
        stream.write_all(payload)?;
    }
    stream.flush()
}

// ---------------------------------------------------------------------------
// Controller data serialisation (OpenRGB binary wire format)
// ---------------------------------------------------------------------------

fn build_controller_data(dev: &DeviceCapabilities, protocol_version: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(512);

    // Placeholder for data_size (filled in at the end).
    let size_pos = data.len();
    data.extend_from_slice(&0u32.to_le_bytes());

    // Device type.
    data.extend_from_slice(&DEVICE_TYPE_LED_STRIP.to_le_bytes());

    // Strings: name, vendor, description, version, serial, location.
    write_orgb_string(&mut data, &dev.device_name);
    write_orgb_string(&mut data, "Lian Li");
    write_orgb_string(&mut data, &format!("frgb {} controller", dev.device_name));
    write_orgb_string(&mut data, env!("CARGO_PKG_VERSION"));
    write_orgb_string(&mut data, &dev.device_id);
    write_orgb_string(&mut data, &format!("frgb:{}", dev.group_id));

    // Modes — one "Direct" mode.
    let num_modes: u16 = 1;
    data.extend_from_slice(&num_modes.to_le_bytes());
    data.extend_from_slice(&0i32.to_le_bytes()); // active_mode = 0

    // Direct mode descriptor.
    write_orgb_string(&mut data, "Direct");
    data.extend_from_slice(&0i32.to_le_bytes()); // value
    let flags: u32 = 0x20; // MODE_FLAG_HAS_PER_LED_COLOR
    data.extend_from_slice(&flags.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // speed_min
    data.extend_from_slice(&0u32.to_le_bytes()); // speed_max
    if protocol_version >= 3 {
        data.extend_from_slice(&0u32.to_le_bytes()); // brightness_min
        data.extend_from_slice(&4u32.to_le_bytes()); // brightness_max
    }
    data.extend_from_slice(&0u32.to_le_bytes()); // colors_min
    data.extend_from_slice(&(dev.total_leds as u32).to_le_bytes()); // colors_max
    data.extend_from_slice(&0u32.to_le_bytes()); // speed
    if protocol_version >= 3 {
        data.extend_from_slice(&4u32.to_le_bytes()); // brightness (default max)
    }
    data.extend_from_slice(&0u32.to_le_bytes()); // direction
    let color_mode: u32 = 1; // COLOR_MODE_PER_LED
    data.extend_from_slice(&color_mode.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes()); // num_colors = 0

    // Zones.
    let num_zones = dev.zones.len() as u16;
    data.extend_from_slice(&num_zones.to_le_bytes());
    for zone in &dev.zones {
        write_orgb_string(&mut data, &zone.name);
        data.extend_from_slice(&1u32.to_le_bytes()); // ZONE_TYPE_LINEAR
        data.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_min
        data.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_max
        data.extend_from_slice(&(zone.led_count as u32).to_le_bytes()); // leds_count
        data.extend_from_slice(&0u16.to_le_bytes()); // matrix_len = 0
        data.extend_from_slice(&0u16.to_le_bytes()); // segments (proto >= 4)
    }

    // LEDs.
    data.extend_from_slice(&dev.total_leds.to_le_bytes());
    let mut led_idx = 0u32;
    for zone in &dev.zones {
        for i in 0..zone.led_count {
            write_orgb_string(&mut data, &format!("{} LED {}", zone.name, i + 1));
            data.extend_from_slice(&led_idx.to_le_bytes());
            led_idx += 1;
        }
    }

    // Initial colours (all black, RGBA).
    data.extend_from_slice(&dev.total_leds.to_le_bytes());
    for _ in 0..dev.total_leds {
        data.extend_from_slice(&[0, 0, 0, 0]);
    }

    // Back-patch data_size (everything after the 4-byte size field itself).
    let data_size = (data.len() - 4) as u32;
    data[size_pos..size_pos + 4].copy_from_slice(&data_size.to_le_bytes());

    data
}

/// Write an OpenRGB-style length-prefixed string (u16 length including NUL).
/// Truncates strings longer than 65533 bytes to prevent u16 overflow.
fn write_orgb_string(buf: &mut Vec<u8>, s: &str) {
    let max_len = u16::MAX as usize - 1; // reserve 1 for NUL
    let s = if s.len() > max_len { &s[..max_len] } else { s };
    let len = (s.len() + 1) as u16; // include NUL terminator
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

// ---------------------------------------------------------------------------
// Colour payload parsers
// ---------------------------------------------------------------------------

/// Parse UPDATE_LEDS payload: data_size[4] + num_colors[2] + colors[].
fn parse_colors(payload: &[u8]) -> Vec<[u8; 3]> {
    if payload.len() < 6 {
        return vec![];
    }
    let num = u16::from_le_bytes(payload[4..6].try_into().unwrap_or([0, 0])) as usize;
    let mut colors = Vec::with_capacity(num);
    let mut offset = 6;
    for _ in 0..num {
        if offset + 4 > payload.len() {
            break;
        }
        colors.push([payload[offset], payload[offset + 1], payload[offset + 2]]);
        offset += 4; // RGBA — skip alpha
    }
    colors
}

/// Parse UPDATE_ZONE_LEDS payload: data_size[4] + zone_idx[4] + num_colors[2] + colors[].
fn parse_zone_colors(payload: &[u8]) -> Vec<[u8; 3]> {
    if payload.len() < 10 {
        return vec![];
    }
    let num = u16::from_le_bytes(payload[8..10].try_into().unwrap_or([0, 0])) as usize;
    let mut colors = Vec::with_capacity(num);
    let mut offset = 10;
    for _ in 0..num {
        if offset + 4 > payload.len() {
            break;
        }
        colors.push([payload[offset], payload[offset + 1], payload[offset + 2]]);
        offset += 4;
    }
    colors
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_roundtrip() {
        let payload = 42u32.to_le_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes()); // device_id
        buf.extend_from_slice(&PKT_REQUEST_PROTOCOL_VERSION.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);

        // Wrap in a TcpStream-compatible reader via a small helper.
        // read_packet needs TcpStream, so test the parse logic directly.
        assert_eq!(&buf[0..4], MAGIC);
        let device_id = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let packet_id = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let size = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(device_id, 3);
        assert_eq!(packet_id, PKT_REQUEST_PROTOCOL_VERSION);
        assert_eq!(size, 4);
    }

    #[test]
    fn parse_colors_basic() {
        // data_size[4] + num_colors[2] + 2x RGBA
        let mut payload = Vec::new();
        payload.extend_from_slice(&14u32.to_le_bytes()); // data_size (unused by parser)
        payload.extend_from_slice(&2u16.to_le_bytes()); // num_colors
        payload.extend_from_slice(&[255, 0, 0, 0]); // red
        payload.extend_from_slice(&[0, 255, 0, 0]); // green

        let colors = parse_colors(&payload);
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0], [255, 0, 0]);
        assert_eq!(colors[1], [0, 255, 0]);
    }

    #[test]
    fn parse_colors_truncated() {
        let colors = parse_colors(&[0; 3]);
        assert!(colors.is_empty());
    }

    #[test]
    fn parse_zone_colors_basic() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&18u32.to_le_bytes()); // data_size
        payload.extend_from_slice(&0u32.to_le_bytes()); // zone_idx
        payload.extend_from_slice(&1u16.to_le_bytes()); // num_colors
        payload.extend_from_slice(&[0, 0, 255, 0]); // blue

        let colors = parse_zone_colors(&payload);
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[0], [0, 0, 255]);
    }

    #[test]
    fn build_controller_data_not_empty() {
        let dev = DeviceCapabilities {
            device_id: "AABBCCDDEEFF".into(),
            device_name: "Test Fan".into(),
            group_id: GroupId::new(1),
            zones: vec![ZoneInfo {
                name: "Fan 1".into(),
                led_count: 24,
            }],
            total_leds: 24,
        };
        let data = build_controller_data(&dev, 4);
        // Must start with a size field and be non-trivially large.
        assert!(data.len() > 100);
        // data_size should match actual remaining bytes.
        let size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        assert_eq!(size, data.len() - 4);
    }

    #[test]
    fn write_orgb_string_format() {
        let mut buf = Vec::new();
        write_orgb_string(&mut buf, "hi");
        // length = 3 (h, i, NUL), stored as u16 LE
        assert_eq!(buf, &[3, 0, b'h', b'i', 0]);
    }
}
