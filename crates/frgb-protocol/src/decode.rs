use frgb_model::device::DeviceId;

// ---------------------------------------------------------------------------
// TX Sync Response
// ---------------------------------------------------------------------------

/// TX sync response.
///
/// Response bytes: [0]=0x11, [1..7]=MasterMacAddr, [7..11]=clock (BE, ×0.625=ms),
/// [11..13]=firmware version (BE).
#[derive(Debug, Clone)]
pub struct TxSyncResponse {
    pub tx_device_id: DeviceId,
    pub firmware_version: u16,
    pub system_clock_ms: u32,
}

pub fn decode_tx_sync(data: &[u8; 64]) -> Result<TxSyncResponse, String> {
    if data[0] != 0x11 {
        return Err(format!("expected 0x11, got 0x{:02x}", data[0]));
    }
    let mut id = [0u8; 6];
    id.copy_from_slice(&data[1..7]);
    let clock_raw = u32::from_be_bytes([data[7], data[8], data[9], data[10]]);
    let system_clock_ms = ((clock_raw as f64) * 0.625) as u32;
    let firmware_version = u16::from_be_bytes([data[11], data[12]]);
    Ok(TxSyncResponse {
        tx_device_id: DeviceId::from(id),
        firmware_version,
        system_clock_ms,
    })
}

// ---------------------------------------------------------------------------
// L-Connect Device Query Response (42-byte records)
// ---------------------------------------------------------------------------

/// Record delimiter byte (0x1C = 28) at offset 41 of each 42-byte record.
const RECORD_DELIMITER: u8 = 0x1C;

/// Size of one device record in the L-Connect query response.
const DEVICE_RECORD_SIZE: usize = 42;

/// A single device record from the L-Connect device query response.
///
/// 42-byte record at offset `4 + n*42` in the response buffer.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    /// Fan device MAC address (6 bytes). Record offset 0.
    pub mac_addr: DeviceId,
    /// MAC of the master this device is bound to (zeros = unbound). Record offset 6.
    pub master_mac_addr: DeviceId,
    /// Wireless channel. Record offset 12.
    pub channel: u8,
    /// Group number (rx_type). Record offset 13.
    pub group: u8,
    /// System time (raw ticks, ×0.625 = ms). Record offset 14, 4 bytes BE.
    pub sys_time_raw: u32,
    /// Device type byte. 0xFF = master device. Record offset 18.
    /// Maps to DevTypes enum: 20=SLV3Fan, 28=TLV2Fan, 36=SLINF, 40=RL120,
    /// 41=CLV1, 65=LC217, 66=V150, 88=Led88, 90=GA2.
    pub dev_type: u8,
    /// Number of fans in daisy chain. Record offset 19.
    /// Values >= 10 mean (fan_num - 10) with SL Infinity right-attach flag set.
    pub fan_num: u8,
    /// Whether this is an SL Infinity right-attach configuration.
    pub is_inf_right_attach: bool,
    /// Current active effect index (4 bytes). Record offset 20.
    pub effect_index: [u8; 4],
    /// Fan subtype per slot (4 bytes). Record offset 24.
    pub fans_type: [u8; 4],
    /// Per-fan RPM (4 fans, u16 BE each). Record offset 28.
    pub fans_speed: [u16; 4],
    /// Per-fan PWM duty (4 bytes). Record offset 36.
    pub fans_pwm: [u8; 4],
    /// Command sequence counter. Record offset 40.
    pub cmd_seq: u8,
}

/// Response from the L-Connect device query.
#[derive(Debug, Clone)]
pub struct DeviceQueryResponse {
    /// Number of devices reported in the header.
    pub num_devices: u8,
    /// RX firmware version (if header byte 2 bit 7 == 1).
    pub rx_firmware: Option<u16>,
    /// Parsed device records.
    pub records: Vec<DeviceRecord>,
}

/// Parse an L-Connect device query response.
///
/// Input is the raw concatenated bytes from reading `434 * page_count` bytes
/// from the RX device after sending `encode_device_query(page_count)`.
///
/// Header: byte 0 = 0x10, byte 1 = num_devices, bytes 2-3 = version/PWM sync.
/// Records: N × 42-byte records starting at offset 4.
pub fn decode_device_query(data: &[u8]) -> DeviceQueryResponse {
    let empty = DeviceQueryResponse {
        num_devices: 0,
        rx_firmware: None,
        records: Vec::new(),
    };

    if data.len() < 4 || data[0] != 0x10 {
        return empty;
    }

    let num_devices = data[1];
    if num_devices == 0 {
        return DeviceQueryResponse {
            num_devices: 0,
            rx_firmware: None,
            records: Vec::new(),
        };
    }

    // Parse firmware version from header bytes 2-3.
    // Header byte 2, bit 7 set indicates a firmware version is present.
    let rx_firmware = if (data[2] >> 7) == 1 {
        Some((((data[2] & 0x7F) as u16) << 8) | data[3] as u16)
    } else {
        None
    };

    let mut records = Vec::new();
    let mut offset = 4;

    for _ in 0..num_devices {
        // Each record is 42 bytes. Check we have enough data.
        if offset + DEVICE_RECORD_SIZE > data.len() {
            break;
        }

        // Validate delimiter at offset 41
        if data[offset + 41] != RECORD_DELIMITER {
            offset += DEVICE_RECORD_SIZE;
            continue;
        }

        let mut mac = [0u8; 6];
        mac.copy_from_slice(&data[offset..offset + 6]);

        let mut master_mac = [0u8; 6];
        master_mac.copy_from_slice(&data[offset + 6..offset + 12]);

        let channel = data[offset + 12];
        let group = data[offset + 13];

        let sys_time_raw = u32::from_be_bytes([
            data[offset + 14],
            data[offset + 15],
            data[offset + 16],
            data[offset + 17],
        ]);

        let dev_type = data[offset + 18];

        let raw_fan_num = data[offset + 19];
        let (fan_num, is_inf_right_attach) = if raw_fan_num >= 10 {
            (raw_fan_num - 10, true)
        } else {
            (raw_fan_num, false)
        };

        let mut effect_index = [0u8; 4];
        effect_index.copy_from_slice(&data[offset + 20..offset + 24]);

        let mut fans_type = [0u8; 4];
        fans_type.copy_from_slice(&data[offset + 24..offset + 28]);

        let fans_speed = [
            u16::from_be_bytes([data[offset + 28], data[offset + 29]]),
            u16::from_be_bytes([data[offset + 30], data[offset + 31]]),
            u16::from_be_bytes([data[offset + 32], data[offset + 33]]),
            u16::from_be_bytes([data[offset + 34], data[offset + 35]]),
        ];

        let mut fans_pwm = [0u8; 4];
        fans_pwm.copy_from_slice(&data[offset + 36..offset + 40]);

        let cmd_seq = data[offset + 40];

        records.push(DeviceRecord {
            mac_addr: DeviceId::from(mac),
            master_mac_addr: DeviceId::from(master_mac),
            channel,
            group,
            sys_time_raw,
            dev_type,
            fan_num,
            is_inf_right_attach,
            effect_index,
            fans_type,
            fans_speed,
            fans_pwm,
            cmd_seq,
        });

        offset += DEVICE_RECORD_SIZE;
    }

    DeviceQueryResponse {
        num_devices,
        rx_firmware,
        records,
    }
}

// ---------------------------------------------------------------------------
// Legacy types for decode_basic_status (single-packet polling)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StatusGroupRecord {
    pub device_id: DeviceId,
    pub tx_ref: DeviceId,
    pub group: u8,
    pub rpm: u16,
    pub mode: u16,
    pub session: u16,
    pub channel: u8,
}

#[derive(Debug, Clone)]
pub struct StatusResponse {
    pub groups: Vec<StatusGroupRecord>,
    pub channel: u8,
}

/// Maximum valid fan group number.
/// L-Connect 3 supports up to 10 fan groups per controller.
const MAX_GROUP: u8 = 10;

/// Maximum plausible RPM value.
const MAX_RPM: u16 = 5000;

/// Minimum spacing between valid 0x08 markers to avoid false positives.
const MIN_MARKER_SPACING: usize = 20;

// ---------------------------------------------------------------------------
// Basic Status (single-packet, legacy — used for quick polling, not discovery)
// ---------------------------------------------------------------------------

/// Parse a single 64-byte basic status response.
///
/// This is the old heuristic marker-scanning decoder for the basic `[0x10, 0x00, ...]`
/// query. It only finds ~1-2 groups per packet. For full discovery, use
/// `decode_device_query` with the L-Connect multi-packet response instead.
pub fn decode_basic_status(data: &[u8; 64]) -> StatusResponse {
    let mut groups = Vec::new();
    let channel = data[2];

    if data[0] != 0x10 {
        return StatusResponse { groups, channel };
    }

    // Scan for "08 XX" markers where XX is a valid group number (1..=MAX_GROUP).
    // Record format (from Python reference, verified against captures):
    //   <fan_dev_id:6> <tx_ref:6> 08 <group> <rpm:2> <counter:4> <mode:2> <session:2>
    // The marker byte 0x08 appears at offset i, with the 12 preceding bytes being
    // the fan device ID (6 bytes) and TX ref (6 bytes).
    let mut last_marker: usize = 0;

    // Loop range 12..len-8 ensures i+3 is always valid for RPM reads (data[i+2..i+4]).
    // Mode (i+8..i+10) and session (i+10..i+12) have explicit bounds checks below.
    for i in 12..data.len().saturating_sub(8) {
        if data[i] != 0x08 || data[i + 1] == 0 || data[i + 1] > MAX_GROUP {
            continue;
        }
        // Skip markers that are too close to the previous one (false positives)
        if last_marker > 0 && i - last_marker < MIN_MARKER_SPACING {
            continue;
        }

        // Validate the preceding 12 bytes contain a non-zero device ID
        let fan_id_start = i - 12;
        let fan_id_slice = &data[fan_id_start..fan_id_start + 6];
        if fan_id_slice.iter().all(|&b| b == 0) {
            continue;
        }

        let mut dev_id = [0u8; 6];
        dev_id.copy_from_slice(fan_id_slice);
        let mut tx_ref = [0u8; 6];
        tx_ref.copy_from_slice(&data[fan_id_start + 6..fan_id_start + 12]);

        let group = data[i + 1];

        // RPM at marker+2..marker+4 (big-endian), validate range
        let rpm_raw = u16::from_be_bytes([data[i + 2], data[i + 3]]);
        let rpm = if rpm_raw <= MAX_RPM { rpm_raw } else { 0 };

        // Mode at marker+8..marker+10 (big-endian)
        let mode = if i + 10 <= data.len() {
            u16::from_be_bytes([data[i + 8], data[i + 9]])
        } else {
            0
        };

        // Session at marker+10..marker+12 (big-endian)
        let session = if i + 12 <= data.len() {
            u16::from_be_bytes([data[i + 10], data[i + 11]])
        } else {
            0
        };

        groups.push(StatusGroupRecord {
            device_id: DeviceId::from(dev_id),
            tx_ref: DeviceId::from(tx_ref),
            group,
            rpm,
            mode,
            session,
            channel,
        });

        last_marker = i;
    }

    StatusResponse { groups, channel }
}

// ---------------------------------------------------------------------------
// RF Query Responses
// ---------------------------------------------------------------------------
//
// Responses to RF query commands (GetGroupNum, GetRPM, GetErr) are read
// from the RX device after sending the query via send_rf_data.
// These decoders are based on the expected payload structure.

/// Response to GetGroupNum query. Returns the fan's current group number.
#[derive(Debug, Clone)]
pub struct RfGroupNumResponse {
    pub fan_mac: DeviceId,
    pub group: u8,
}

/// Response to GetRPM query. Returns per-fan RPM values.
#[derive(Debug, Clone)]
pub struct RfRpmResponse {
    pub fan_mac: DeviceId,
    pub rpms: [u16; 4],
}

/// Response to GetErr query. Returns error flags.
#[derive(Debug, Clone)]
pub struct RfErrorResponse {
    pub fan_mac: DeviceId,
    pub error_code: u8,
    pub stalled_fans: u8,
}

/// Decode an RF query response from a 64-byte RX read.
///
/// RF query responses echo the command type at byte 1 of the response,
/// with the queried fan MAC at bytes 2-7 and response data following.
pub fn decode_rf_group_num(data: &[u8; 64]) -> Option<RfGroupNumResponse> {
    if data[1] != crate::constants::RF_CMD_GET_GROUP_NUM {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[2..8]);
    Some(RfGroupNumResponse {
        fan_mac: DeviceId::from(mac),
        group: data[14],
    })
}

pub fn decode_rf_rpm(data: &[u8; 64]) -> Option<RfRpmResponse> {
    if data[1] != crate::constants::RF_CMD_GET_RPM {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[2..8]);
    Some(RfRpmResponse {
        fan_mac: DeviceId::from(mac),
        rpms: [
            u16::from_be_bytes([data[14], data[15]]),
            u16::from_be_bytes([data[16], data[17]]),
            u16::from_be_bytes([data[18], data[19]]),
            u16::from_be_bytes([data[20], data[21]]),
        ],
    })
}

pub fn decode_rf_error(data: &[u8; 64]) -> Option<RfErrorResponse> {
    if data[1] != crate::constants::RF_CMD_GET_ERROR {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[2..8]);
    Some(RfErrorResponse {
        fan_mac: DeviceId::from(mac),
        error_code: data[14],
        stalled_fans: data[15],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_tx_sync_response() {
        let mut data = [0u8; 64];
        data[0] = 0x11;
        data[1..7].copy_from_slice(&[0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let resp = decode_tx_sync(&data).unwrap();
        assert_eq!(resp.tx_device_id.to_hex(), "297a84e566e4");
    }

    #[test]
    fn decode_tx_sync_rejects_wrong_header() {
        let mut data = [0u8; 64];
        data[0] = 0x10; // wrong header
        assert!(decode_tx_sync(&data).is_err());
    }

    #[test]
    fn decode_status_empty_packet() {
        let data = [0u8; 64];
        let resp = decode_basic_status(&data);
        assert!(resp.groups.is_empty());
    }

    #[test]
    fn decode_status_with_group_record() {
        // Construct a packet matching the real hardware record format:
        // <fan_dev_id:6> <tx_ref:6> 08 <group> <rpm:2> <counter:4> <mode:2> <session:2>
        let mut data = [0u8; 64];
        data[0] = 0x10;
        data[1] = 0x07; // non-zero — real hardware uses various status bytes here
        data[2] = 0x32; // channel/flags
        data[3] = 0x00;
        // Fan device ID at bytes 4-9
        data[4..10].copy_from_slice(&[0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        // TX ref at bytes 10-15
        data[10..16].copy_from_slice(&[0x43, 0x42, 0x86, 0xe5, 0x66, 0xe4]);
        // 0x08 marker + group number
        data[16] = 0x08;
        data[17] = 0x02; // group 2
                         // RPM big-endian (1200 = 0x04B0)
        data[18] = 0x04;
        data[19] = 0xB0;
        // Counter (4 bytes)
        data[20..24].copy_from_slice(&[0x5b, 0x81, 0x00, 0x01]);
        // Mode big-endian (0x03A4)
        data[24] = 0x03;
        data[25] = 0xA4;
        // Session big-endian
        data[26] = 0x5C;
        data[27] = 0xD0;

        let resp = decode_basic_status(&data);
        assert_eq!(resp.groups.len(), 1);
        assert_eq!(resp.groups[0].group, 2);
        assert_eq!(resp.groups[0].rpm, 1200);
        assert_eq!(resp.groups[0].mode, 0x03A4);
        assert_eq!(resp.groups[0].device_id.to_hex(), "c8b4ef6232e1");
        assert_eq!(resp.groups[0].tx_ref.to_hex(), "434286e566e4");
    }

    #[test]
    fn decode_status_rejects_non_query_response() {
        let mut data = [0u8; 64];
        data[0] = 0x11; // TX sync response, not status
        let resp = decode_basic_status(&data);
        assert!(resp.groups.is_empty());
    }

    #[test]
    fn decode_status_filters_impossible_rpm() {
        let mut data = [0u8; 64];
        data[0] = 0x10;
        data[1] = 0x07;
        data[4..10].copy_from_slice(&[0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        data[10..16].copy_from_slice(&[0x43, 0x42, 0x86, 0xe5, 0x66, 0xe4]);
        data[16] = 0x08;
        data[17] = 0x01;
        // RPM = 0x6295 = 25237 → above MAX_RPM, should be filtered to 0
        data[18] = 0x62;
        data[19] = 0x95;

        let resp = decode_basic_status(&data);
        assert_eq!(resp.groups.len(), 1);
        assert_eq!(resp.groups[0].rpm, 0); // filtered
    }

    // --- decode_tx_sync extended fields ---

    #[test]
    fn decode_tx_sync_parses_firmware_and_clock() {
        let mut data = [0u8; 64];
        data[0] = 0x11;
        data[1..7].copy_from_slice(&[0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        // System clock at bytes 7-10 (BE): 0x00010000 = 65536 ticks × 0.625 = 40960 ms
        data[7] = 0x00;
        data[8] = 0x01;
        data[9] = 0x00;
        data[10] = 0x00;
        // Firmware version at bytes 11-12 (BE): 0x0102 = 258
        data[11] = 0x01;
        data[12] = 0x02;

        let resp = decode_tx_sync(&data).unwrap();
        assert_eq!(resp.tx_device_id.to_hex(), "297a84e566e4");
        assert_eq!(resp.system_clock_ms, 40960);
        assert_eq!(resp.firmware_version, 0x0102);
    }

    // --- decode_device_query ---

    #[test]
    fn decode_device_query_empty_response() {
        let resp = decode_device_query(&[]);
        assert_eq!(resp.num_devices, 0);
        assert!(resp.records.is_empty());
    }

    #[test]
    fn decode_device_query_wrong_header() {
        let mut data = [0u8; 64];
        data[0] = 0x11; // wrong header
        let resp = decode_device_query(&data);
        assert_eq!(resp.num_devices, 0);
    }

    #[test]
    fn decode_device_query_zero_devices() {
        let mut data = [0u8; 64];
        data[0] = 0x10;
        data[1] = 0; // num_devices = 0
        let resp = decode_device_query(&data);
        assert_eq!(resp.num_devices, 0);
        assert!(resp.records.is_empty());
    }

    #[test]
    fn decode_device_query_one_record() {
        // Build a response with 1 device record starting at offset 4
        let mut data = vec![0u8; 4 + 42]; // header + 1 record
        data[0] = 0x10;
        data[1] = 1; // num_devices
        data[2] = 0x80 | 0x01; // bit 7 = 1 → firmware version
        data[3] = 0x23; // firmware low byte → version = 0x0123

        let rec_start = 4;
        // mac_addr at offset 0
        data[rec_start..rec_start + 6].copy_from_slice(&[0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        // master_mac_addr at offset 6
        data[rec_start + 6..rec_start + 12].copy_from_slice(&[0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        // channel at offset 12
        data[rec_start + 12] = 0x08;
        // group (rx_type) at offset 13
        data[rec_start + 13] = 2;
        // sys_time at offset 14 (BE: 0x00002710 = 10000 ticks)
        data[rec_start + 14] = 0x00;
        data[rec_start + 15] = 0x00;
        data[rec_start + 16] = 0x27;
        data[rec_start + 17] = 0x10;
        // dev_type at offset 18
        data[rec_start + 18] = 20; // SLV3Fan
                                   // fan_num at offset 19
        data[rec_start + 19] = 3;
        // effect_index at offset 20 (4 bytes)
        data[rec_start + 20] = 0x01;
        // fans_type at offset 24 (4 bytes)
        data[rec_start + 24] = 20; // SLV3Fan type
                                   // fans_speed at offset 28 (4 × u16 BE)
                                   // Fan 0: 1200 RPM = 0x04B0
        data[rec_start + 28] = 0x04;
        data[rec_start + 29] = 0xB0;
        // Fan 1: 1150 RPM = 0x047E
        data[rec_start + 30] = 0x04;
        data[rec_start + 31] = 0x7E;
        // Fan 2: 1100 RPM = 0x044C
        data[rec_start + 32] = 0x04;
        data[rec_start + 33] = 0x4C;
        // fans_pwm at offset 36 (4 bytes)
        data[rec_start + 36] = 50;
        data[rec_start + 37] = 50;
        data[rec_start + 38] = 50;
        // cmd_seq at offset 40
        data[rec_start + 40] = 7;
        // delimiter at offset 41 MUST be 0x1C
        data[rec_start + 41] = 0x1C;

        let resp = decode_device_query(&data);
        assert_eq!(resp.num_devices, 1);
        assert_eq!(resp.rx_firmware, Some(0x0123));
        assert_eq!(resp.records.len(), 1);

        let rec = &resp.records[0];
        assert_eq!(rec.mac_addr.to_hex(), "c8b4ef6232e1");
        assert_eq!(rec.master_mac_addr.to_hex(), "297a84e566e4");
        assert_eq!(rec.channel, 0x08);
        assert_eq!(rec.group, 2);
        assert_eq!(rec.dev_type, 20); // SLV3Fan
        assert_eq!(rec.fan_num, 3);
        assert!(!rec.is_inf_right_attach);
        assert_eq!(rec.fans_speed[0], 1200);
        assert_eq!(rec.fans_speed[1], 1150);
        assert_eq!(rec.fans_speed[2], 1100);
        assert_eq!(rec.fans_speed[3], 0);
        assert_eq!(rec.fans_pwm[0], 50);
        assert_eq!(rec.cmd_seq, 7);
    }

    #[test]
    fn decode_device_query_skips_bad_delimiter() {
        let mut data = vec![0u8; 4 + 42];
        data[0] = 0x10;
        data[1] = 1;
        // Record with wrong delimiter
        data[4] = 0xAA; // some mac byte
        data[4 + 41] = 0xFF; // wrong delimiter (should be 0x1C)

        let resp = decode_device_query(&data);
        assert_eq!(resp.num_devices, 1);
        assert_eq!(resp.records.len(), 0); // skipped
    }

    #[test]
    fn decode_device_query_inf_right_attach() {
        let mut data = vec![0u8; 4 + 42];
        data[0] = 0x10;
        data[1] = 1;
        data[4 + 18] = 36; // SLINF dev_type
        data[4 + 19] = 13; // fan_num >= 10 → right attach, actual count = 3
        data[4 + 41] = 0x1C;

        let resp = decode_device_query(&data);
        assert_eq!(resp.records.len(), 1);
        assert_eq!(resp.records[0].fan_num, 3);
        assert!(resp.records[0].is_inf_right_attach);
    }

    #[test]
    fn decode_device_query_master_record() {
        let mut data = vec![0u8; 4 + 42];
        data[0] = 0x10;
        data[1] = 1;
        data[4 + 18] = 0xFF; // master device
        data[4 + 41] = 0x1C;

        let resp = decode_device_query(&data);
        assert_eq!(resp.records.len(), 1);
        assert_eq!(resp.records[0].dev_type, 0xFF);
    }

    #[test]
    fn decode_device_query_firmware_version_absent() {
        let mut data = vec![0u8; 4 + 42];
        data[0] = 0x10;
        data[1] = 1;
        data[2] = 0x30; // bit 7 = 0 → no firmware version
        data[3] = 0x40;
        data[4 + 41] = 0x1C;

        let resp = decode_device_query(&data);
        assert!(resp.rx_firmware.is_none());
    }
}
