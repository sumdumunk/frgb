use frgb_model::device::DeviceId;

use crate::constants::{CMD_QUERY, CMD_TX_SYNC, CMD_TYPE_BIND, CMD_TYPE_MASTER_CLOCK, PACKET_SIZE};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn zero_packet() -> [u8; PACKET_SIZE] {
    [0u8; PACKET_SIZE]
}

// ---------------------------------------------------------------------------
// Public encoding functions
// ---------------------------------------------------------------------------

/// L-Connect device query. Sent to RX device.
///
/// Response is multi-packet: `434 * page_count` bytes containing a header
/// and N × 42-byte device records.
///
/// Layout: `[0x10, page_count, 0x00, 0x00, ...zeros]`
///
/// Bytes 2-3 are speed sync data (only non-zero when FgSync mode is on;
/// we always send 0x00 since frgb handles speed separately).
pub fn encode_device_query(page_count: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_QUERY;
    pkt[1] = page_count;
    pkt
}

/// TX sync packet. Sent to TX device.
///
/// Layout: `[0x11, channel, 0x00, ...zeros]`
pub fn encode_tx_sync(channel: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_TX_SYNC;
    pkt[1] = channel;
    pkt
}

/// TX init packet — establishes wireless session before binding.
///
/// From USB capture: L-Connect sends `[0x11, 0x08, 0xe7, 0xff, 0xff, 0x00]`
/// 3× during initialization. The 0xe7 0xff 0xff payload may set session params.
/// We use dynamic channel (consistent with encode_tx_sync).
pub fn encode_tx_init(channel: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_TX_SYNC;
    pkt[1] = channel;
    pkt[2] = 0xe7;
    pkt[3] = 0xff;
    pkt[4] = 0xff;
    pkt
}

/// Speed control packet.
///
/// Layout (verified against protocol captures and Python reference):
/// - `[0]`     = 0x10
/// - `[1]`     = 0x00
/// - `[2]`     = channel
/// - `[3]`     = group
/// - `[4..6]`  = 0x12, 0x10 (speed command type)
/// - `[6..12]` = fan_id (6 bytes)
/// - `[12..18]`= tx_ref (6 bytes)
/// - `[18]`    = group (repeated)
/// - `[19]`    = channel (repeated)
/// - `[20]`    = sequence position (controls effect flow order across groups)
/// - `[21..25]`= speed_byte × 4 (one per daisy-chain slot)
///
/// Simple follow-up / acknowledgement packet.
///
/// Layout: `[0x10, seq, channel, group, ...]`
pub fn encode_followup(group: u8, seq: u8, channel: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_QUERY;
    pkt[1] = seq;
    pkt[2] = channel;
    pkt[3] = group;
    pkt
}

/// Master clock sync packet — command 0x1214.
///
/// Sent as a broadcast (rx_type=0xFF) after SyncPwm. Also used during bind sequences and unlock.
///
/// Layout:
/// - `[0..4]`  = `[0x10, 0x00, channel, 0xFF]`
/// - `[4..6]`  = 0x12, 0x14
/// - `[6..12]` = 0x00 padding
/// - `[12..18]`= tx_dev_id (master MAC)
pub fn encode_master_clock_sync(tx_dev_id: &DeviceId, channel: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_QUERY;
    pkt[1] = 0x00;
    pkt[2] = channel;
    pkt[3] = 0xFF;
    pkt[4..6].copy_from_slice(&CMD_TYPE_MASTER_CLOCK);
    // bytes 6-11 = zeros (already zero from zero_packet)
    pkt[12..18].copy_from_slice(tx_dev_id.as_bytes());
    pkt
}

/// Unlock packet — command 0x1214 broadcast, same as master clock sync.
pub fn encode_unlock(tx_ref: &DeviceId, channel: u8) -> [u8; PACKET_SIZE] {
    encode_master_clock_sync(tx_ref, channel)
}

/// Bind/lock broadcast packet — command 0x1215, broadcasts to all devices.
///
/// Layout (verified against Python build_bind):
/// - `[0..4]`  = `[0x10, 0x00, channel, 0xFF]`
/// - `[4..6]`  = 0x12, 0x15
/// - `[6..12]` = BROADCAST device ID (FF FF FF FF FF FF)
/// - `[12..18]`= tx_ref
/// - `[18]`    = 0xFF (lock flag)
pub fn encode_bind(tx_ref: &DeviceId, channel: u8) -> [u8; PACKET_SIZE] {
    let mut pkt = zero_packet();
    pkt[0] = CMD_QUERY;
    pkt[1] = 0x00;
    pkt[2] = channel;
    pkt[3] = 0xFF;
    pkt[4..6].copy_from_slice(&CMD_TYPE_BIND);
    pkt[6..12].copy_from_slice(DeviceId::BROADCAST.as_bytes());
    pkt[12..18].copy_from_slice(tx_ref.as_bytes());
    pkt[18] = 0xFF; // lock flag
    pkt
}

// ---------------------------------------------------------------------------
// Control RF Payloads (240-byte RF data for SendRfData)
// ---------------------------------------------------------------------------

/// Build a 240-byte MB sync toggle payload.
///
/// Toggles motherboard PWM sync on/off for a specific device.
/// Uses cmd_seq for firmware change detection — fan ignores if unchanged.
///
/// Layout:
/// - `[0]`     = 0x12 (RF command prefix)
/// - `[1]`     = 0x24 (MB sync sub-command)
/// - `[2..8]`  = fan device MAC
/// - `[8..14]` = master MAC
/// - `[14]`    = group
/// - `[15]`    = channel
/// - `[16]`    = slave_index
/// - `[17]`    = new cmd_seq (must differ from current for firmware to accept)
/// - `[20]`    = 0 to disable MB sync, 1 to enable
pub fn encode_mb_sync_payload(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    group: u8,
    channel: u8,
    slave_index: u8,
    new_cmd_seq: u8,
    enable: bool,
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = 0x24;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14] = group;
    payload[15] = channel;
    payload[16] = slave_index;
    payload[17] = new_cmd_seq;
    payload[20] = if enable { 1 } else { 0 };
    payload
}

/// Build a 240-byte AIO info RF payload (command 0x12 0x21).
///
/// Streams AIO parameters (pump PWM, LCD brightness, colors, etc.) to HydroShift II
/// pump devices (dev_type 10/11). The receiver latches onto the `aio_param` buffer
/// copied into bytes 18..50; see [`crate::pump`] for the buffer layout and
/// RPM→PWM scaling.
///
/// Layout:
/// - `[0]`      = 0x12 (RF command prefix)
/// - `[1]`      = 0x21 (AIO info sub-command)
/// - `[2..8]`   = device MAC
/// - `[8..14]`  = master MAC
/// - `[14]`     = target_rx_type (typically the group number)
/// - `[15]`     = target_channel
/// - `[16..18]` = reserved (zero)
/// - `[18..50]` = aio_param (32 bytes)
/// - `[50..]`   = zero padding
pub fn encode_aio_info_payload(
    device_mac: &DeviceId,
    master_mac: &DeviceId,
    target_rx_type: u8,
    target_channel: u8,
    aio_param: &[u8; 32],
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = 0x21;
    payload[2..8].copy_from_slice(device_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14] = target_rx_type;
    payload[15] = target_channel;
    payload[18..50].copy_from_slice(aio_param);
    payload
}

/// Build a 240-byte bind/reassign RF payload (command 0x12 0x10).
///
/// Tells a fan to adopt a new master MAC and group.
/// Sent via `send_rf_data(channel, device_rx_type, payload)`.
///
/// Layout:
/// - `[0]`     = 0x12 (RF command prefix)
/// - `[1]`     = 0x10 (speed/bind sub-command)
/// - `[2..8]`  = fan device MAC
/// - `[8..14]` = master MAC (TX ref — fan adopts this as owner)
/// - `[14]`    = target_rx_type (new group number)
/// - `[15]`    = target_channel
/// - `[16]`    = slave_index (device ordering, 1-based)
/// - `[17..21]`= target_fans_pwm (per-slot speed bytes; unoccupied slots must be 0)
pub fn encode_bind_rf_payload(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    target_group: u8,
    channel: u8,
    slave_index: u8,
    fans_pwm: &[u8; 4],
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = 0x10;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14] = target_group;
    payload[15] = channel;
    payload[16] = slave_index;
    payload[17..21].copy_from_slice(fans_pwm);
    payload
}

// ---------------------------------------------------------------------------
// RF Query Command Payloads (240-byte RF data for SendRfData)
// ---------------------------------------------------------------------------
//
// All follow the standard 240-byte RF payload layout:
//   [0]     = 0x12 (RF command prefix)
//   [1]     = RFCmdType
//   [2..8]  = fan device MAC
//   [8..14] = master MAC

/// Query which group a fan belongs to. RFCmdType::GetGroupNum (1).
///
/// Response expected on RX read: group number in response data.
pub fn encode_rf_get_group_num(fan_mac: &DeviceId, master_mac: &DeviceId) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_GET_GROUP_NUM;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload
}

/// Query current RPM of a fan. RFCmdType::GetRPM (2).
///
/// Response expected on RX read: RPM value in response data.
pub fn encode_rf_get_rpm(fan_mac: &DeviceId, master_mac: &DeviceId) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_GET_RPM;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload
}

/// Query error state of a fan. RFCmdType::GetErr (3).
///
/// Response expected on RX read: error flags in response data.
pub fn encode_rf_get_error(fan_mac: &DeviceId, master_mac: &DeviceId) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_GET_ERROR;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload
}

/// Reassign a fan to a different group. RFCmdType::SetFG (18).
///
/// Sets the fan's group membership without a full bind sequence.
/// `target_group` is the new group number (1-8).
pub fn encode_rf_set_fan_group(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    target_group: u8,
    channel: u8,
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_SET_FAN_GROUP;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14] = target_group;
    payload[15] = channel;
    payload
}

/// Reset LCD on a fan device. RFCmdType::SetLcdReset (21).
///
/// Triggers a hardware LCD reset on devices with built-in displays.
pub fn encode_rf_lcd_reset(fan_mac: &DeviceId, master_mac: &DeviceId) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_LCD_RESET;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload
}

/// Set hardware group merge order. RFCmdType::SetOrder (0x19).
/// `order` is the group indices in desired playback order.
pub fn encode_rf_set_order(fan_mac: &DeviceId, master_mac: &DeviceId, order: &[u8]) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_SET_ORDER;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    // Group order indices packed starting at byte 14
    for (i, &idx) in order.iter().take(4).enumerate() {
        payload[14 + i] = idx;
    }
    payload[18] = order.len().min(4) as u8; // count
    payload
}

/// Set LED effect direction per group. RFCmdType::SetLEDirection (0x16). CW=0, CCW=1.
pub fn encode_rf_set_direction(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    direction: u8, // 0=CW, 1=CCW
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = crate::constants::RF_CMD_SET_LE_DIRECTION;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14] = direction;
    payload
}

// ---------------------------------------------------------------------------
// RGB RF Payloads (240-byte RF data for SendRfData)
// ---------------------------------------------------------------------------

/// Size of an RF data payload sent via SendRfData.
pub const RF_PAYLOAD_SIZE: usize = 240;

/// Size of compressed data chunk per RF payload (Parts 1+).
pub const RF_DATA_CHUNK_SIZE: usize = 220;

/// Build the shared RF payload header (bytes 0-19) for RGB commands.
///
/// - `[0..2]`  = 0x12, 0x20 (RGB command)
/// - `[2..8]`  = fan MAC
/// - `[8..14]` = master MAC
/// - `[14..18]`= effect_index (4 bytes, change ID — must differ per send)
/// - `[18]`    = part_index
/// - `[19]`    = total_parts
///
/// The effect_index is a change detection field. The firmware compares it
/// against the current value and ignores the transmission if unchanged.
/// We use random bytes (both a timestamp or random value work; firmware needs a different value each send).
fn build_rgb_rf_header(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    effect_index: &[u8; 4],
    part_index: u8,
    total_parts: u8,
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = [0u8; RF_PAYLOAD_SIZE];
    payload[0] = 0x12;
    payload[1] = 0x20;
    payload[2..8].copy_from_slice(fan_mac.as_bytes());
    payload[8..14].copy_from_slice(master_mac.as_bytes());
    payload[14..18].copy_from_slice(effect_index);
    payload[18] = part_index;
    payload[19] = total_parts;
    payload
}

/// RGB effect metadata for Part 0 payload.
pub struct RgbMetadata {
    pub total_parts: u8,
    pub compressed_data_len: u32,
    pub total_frame: u16,
    pub led_num: u8,
    pub interval: f64,
    pub sub_interval: f64,
    pub is_outer_match_max: u8,
    pub total_sub_frame: u16,
}

/// Build Part 0 (metadata) RF payload for RGB effect transmission.
///
/// Contains effect metadata: compressed data size, frame count, LED count,
/// interval timing, sub-frame info.
pub fn encode_rgb_metadata_payload(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    effect_index: &[u8; 4],
    meta: &RgbMetadata,
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = build_rgb_rf_header(fan_mac, master_mac, effect_index, 0, meta.total_parts);

    // Compressed data length (BE) at bytes 20-23
    payload[20] = (meta.compressed_data_len >> 24) as u8;
    payload[21] = ((meta.compressed_data_len >> 16) & 0xFF) as u8;
    payload[22] = ((meta.compressed_data_len >> 8) & 0xFF) as u8;
    payload[23] = (meta.compressed_data_len & 0xFF) as u8;

    // Reserved byte at 24
    payload[24] = 0;

    // Total frame count (BE) at bytes 25-26
    payload[25] = (meta.total_frame >> 8) as u8;
    payload[26] = (meta.total_frame & 0xFF) as u8;

    // LED count at byte 27
    payload[27] = meta.led_num;

    // Bytes 28-31: Strimer sync (unused for fans, left as 0)

    // Interval (BE integer part) at bytes 32-33
    let interval_int = meta.interval as u16;
    payload[32] = (interval_int >> 8) as u8;
    payload[33] = (interval_int & 0xFF) as u8;

    // Interval fractional (×100) at byte 34
    payload[34] = ((meta.interval * 100.0) % 100.0) as u8;

    // Sub-interval (BE) at bytes 35-36
    let sub_int = meta.sub_interval as u16;
    payload[35] = (sub_int >> 8) as u8;
    payload[36] = (sub_int & 0xFF) as u8;

    // isOuterMatchMax at byte 37
    payload[37] = meta.is_outer_match_max;

    // Total sub-frame count (BE) at bytes 38-39
    payload[38] = (meta.total_sub_frame >> 8) as u8;
    payload[39] = (meta.total_sub_frame & 0xFF) as u8;

    payload
}

/// Build a data part (Parts 1+) RF payload carrying compressed RGB data.
///
/// Each part carries up to 220 bytes of compressed data at payload offset 20.
pub fn encode_rgb_data_payload(
    fan_mac: &DeviceId,
    master_mac: &DeviceId,
    effect_index: &[u8; 4],
    part_index: u8,
    total_parts: u8,
    data_chunk: &[u8],
) -> [u8; RF_PAYLOAD_SIZE] {
    let mut payload = build_rgb_rf_header(fan_mac, master_mac, effect_index, part_index, total_parts);

    let len = data_chunk.len().min(RF_DATA_CHUNK_SIZE);
    payload[20..20 + len].copy_from_slice(&data_chunk[..len]);

    payload
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceId;
    #[test]
    fn encode_device_query_format() {
        let pkt = encode_device_query(1);
        assert_eq!(pkt.len(), 64);
        assert_eq!(pkt[0], 0x10);
        assert_eq!(pkt[1], 1); // page_count
        assert_eq!(pkt[2], 0); // speed sync hi (unused)
        assert_eq!(pkt[3], 0); // speed sync lo (unused)
        assert!(pkt[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn encode_device_query_page_count() {
        let pkt = encode_device_query(3);
        assert_eq!(pkt[1], 3);
    }

    #[test]
    fn encode_tx_sync_format() {
        let pkt = encode_tx_sync(0x08);
        assert_eq!(pkt[0], 0x11);
        assert_eq!(pkt[1], 0x08);
    }

    #[test]
    fn encode_tx_sync_custom_channel() {
        let pkt = encode_tx_sync(0x0B);
        assert_eq!(pkt[0], 0x11);
        assert_eq!(pkt[1], 0x0B);
    }

    #[test]
    fn encode_followup_format() {
        // Use distinct values to verify byte ordering: group=3, seq=1, channel=0x08
        let pkt = encode_followup(3, 1, 0x08);
        assert_eq!(pkt[0], 0x10);
        assert_eq!(pkt[1], 0x01); // seq at byte 1
        assert_eq!(pkt[2], 0x08); // channel at byte 2
        assert_eq!(pkt[3], 0x03); // group at byte 3
    }

    #[test]
    fn encode_pwm_sync_uses_tx_device_id() {
        let tx_dev = DeviceId::from([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        let pkt = encode_master_clock_sync(&tx_dev, 0x08);
        assert_eq!(pkt[0..4], [0x10, 0x00, 0x08, 0xFF]);
        assert_eq!(pkt[4..6], [0x12, 0x14]);
        assert_eq!(&pkt[6..12], &[0x00; 6]); // zeros, NOT broadcast
        assert_eq!(&pkt[12..18], tx_dev.as_bytes());
    }

    #[test]
    fn encode_unlock_uses_tx_ref_id() {
        let tx_ref = DeviceId::from([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let pkt = encode_unlock(&tx_ref, 0x08);
        assert_eq!(pkt[0..4], [0x10, 0x00, 0x08, 0xFF]);
        assert_eq!(pkt[4..6], [0x12, 0x14]);
        assert_eq!(&pkt[12..18], tx_ref.as_bytes());
    }

    #[test]
    fn all_packets_are_64_bytes() {
        let fan_id = DeviceId::from([1, 2, 3, 4, 5, 6]);
        let tx_ref = DeviceId::from([7, 8, 9, 10, 11, 12]);
        assert_eq!(encode_device_query(1).len(), 64);
        assert_eq!(encode_tx_sync(0x08).len(), 64);
        assert_eq!(encode_followup(1, 1, 0x08).len(), 64);
        assert_eq!(encode_master_clock_sync(&fan_id, 0x08).len(), 64);
        assert_eq!(encode_unlock(&tx_ref, 0x08).len(), 64);
    }

    #[test]
    fn encode_bind_broadcasts() {
        let tx_ref = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let pkt = encode_bind(&tx_ref, 0x08);
        assert_eq!(pkt[0..4], [0x10, 0x00, 0x08, 0xFF]); // header with broadcast group
        assert_eq!(pkt[4..6], [0x12, 0x15]); // bind command
        assert_eq!(&pkt[6..12], DeviceId::BROADCAST.as_bytes()); // broadcast device
        assert_eq!(&pkt[12..18], tx_ref.as_bytes());
        assert_eq!(pkt[18], 0xFF); // lock flag
    }

    // --- RGB RF Payload tests ---

    #[test]
    fn encode_rgb_metadata_payload_structure() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let effect_idx = [0x01, 0x02, 0x03, 0x04];

        let payload = encode_rgb_metadata_payload(
            &fan,
            &master,
            &effect_idx,
            &RgbMetadata {
                total_parts: 5,
                compressed_data_len: 1000,
                total_frame: 30,
                led_num: 40,
                interval: 5.0,
                sub_interval: 0.0,
                is_outer_match_max: 0,
                total_sub_frame: 0,
            },
        );

        assert_eq!(payload.len(), 240);
        // RF header
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x20);
        assert_eq!(&payload[2..8], fan.as_bytes());
        assert_eq!(&payload[8..14], master.as_bytes());
        assert_eq!(&payload[14..18], &effect_idx);
        assert_eq!(payload[18], 0); // part_index = 0
        assert_eq!(payload[19], 5); // total_parts

        // Compressed data length (1000 = 0x000003E8 BE)
        assert_eq!(payload[20], 0x00);
        assert_eq!(payload[21], 0x00);
        assert_eq!(payload[22], 0x03);
        assert_eq!(payload[23], 0xE8);

        // Total frame (30 = 0x001E BE)
        assert_eq!(payload[25], 0x00);
        assert_eq!(payload[26], 30);

        // LED num
        assert_eq!(payload[27], 40);

        // Interval = 5.0 → int part = 5, frac = 0
        assert_eq!(payload[32], 0x00);
        assert_eq!(payload[33], 5);
        assert_eq!(payload[34], 0);
    }

    #[test]
    fn encode_rgb_data_payload_structure() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let effect_idx = [0x01, 0x02, 0x03, 0x04];

        let chunk = vec![0xAB; 220];
        let payload = encode_rgb_data_payload(&fan, &master, &effect_idx, 1, 5, &chunk);

        assert_eq!(payload.len(), 240);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x20);
        assert_eq!(payload[18], 1); // part_index
        assert_eq!(payload[19], 5); // total_parts
                                    // Data chunk at offset 20
        assert_eq!(payload[20], 0xAB);
        assert_eq!(payload[239], 0xAB);
    }

    #[test]
    fn encode_tx_init_format() {
        let pkt = encode_tx_init(0x08);
        assert_eq!(pkt[0], 0x11);
        assert_eq!(pkt[1], 0x08);
        assert_eq!(pkt[2], 0xe7);
        assert_eq!(pkt[3], 0xff);
        assert_eq!(pkt[4], 0xff);
        assert!(pkt[5..].iter().all(|&b| b == 0));
    }

    #[test]
    fn encode_rgb_data_payload_short_chunk() {
        let fan = DeviceId::from([1, 2, 3, 4, 5, 6]);
        let master = DeviceId::from([7, 8, 9, 10, 11, 12]);
        let effect_idx = [0xAA, 0xBB, 0xCC, 0xDD];

        // Last chunk may be shorter than 220
        let chunk = vec![0xCC; 50];
        let payload = encode_rgb_data_payload(&fan, &master, &effect_idx, 3, 4, &chunk);

        assert_eq!(payload[20], 0xCC);
        assert_eq!(payload[69], 0xCC); // byte 20+49
        assert_eq!(payload[70], 0x00); // padding after short chunk
    }

    // --- RF Query Command tests ---

    #[test]
    fn encode_rf_get_group_num_format() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let payload = encode_rf_get_group_num(&fan, &master);
        assert_eq!(payload.len(), 240);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x01); // GetGroupNum
        assert_eq!(&payload[2..8], fan.as_bytes());
        assert_eq!(&payload[8..14], master.as_bytes());
    }

    #[test]
    fn encode_rf_get_rpm_format() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let payload = encode_rf_get_rpm(&fan, &master);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x02); // GetRPM
    }

    #[test]
    fn encode_rf_get_error_format() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let payload = encode_rf_get_error(&fan, &master);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x03); // GetErr
    }

    #[test]
    fn encode_rf_set_fan_group_format() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let payload = encode_rf_set_fan_group(&fan, &master, 5, 0x08);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x12); // SetFG
        assert_eq!(payload[14], 5); // target group
        assert_eq!(payload[15], 0x08); // channel
    }

    #[test]
    fn encode_rf_lcd_reset_format() {
        let fan = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let payload = encode_rf_lcd_reset(&fan, &master);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x15); // SetLcdReset
    }

    #[test]
    fn all_rf_query_payloads_are_240_bytes() {
        let fan = DeviceId::from([1, 2, 3, 4, 5, 6]);
        let master = DeviceId::from([7, 8, 9, 10, 11, 12]);
        assert_eq!(encode_rf_get_group_num(&fan, &master).len(), 240);
        assert_eq!(encode_rf_get_rpm(&fan, &master).len(), 240);
        assert_eq!(encode_rf_get_error(&fan, &master).len(), 240);
        assert_eq!(encode_rf_set_fan_group(&fan, &master, 1, 0x08).len(), 240);
        assert_eq!(encode_rf_lcd_reset(&fan, &master).len(), 240);
    }

    // --- AIO info RF payload tests ---

    #[test]
    fn encode_aio_info_payload_structure() {
        let device = DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        let master = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let mut aio = [0u8; 32];
        aio[7] = 1;
        aio[25] = 80;
        aio[26] = 1;
        aio[28] = 0x04;
        aio[29] = 0xD8;

        let payload = encode_aio_info_payload(&device, &master, 7, 0x08, &aio);

        assert_eq!(payload.len(), 240);
        assert_eq!(payload[0], 0x12);
        assert_eq!(payload[1], 0x21); // AIO info sub-command
        assert_eq!(&payload[2..8], device.as_bytes());
        assert_eq!(&payload[8..14], master.as_bytes());
        assert_eq!(payload[14], 7); // target_rx_type
        assert_eq!(payload[15], 0x08); // target_channel
        assert_eq!(payload[16], 0); // reserved
        assert_eq!(payload[17], 0); // reserved
        assert_eq!(&payload[18..50], &aio);
        // Rest must be zero padding
        assert!(payload[50..].iter().all(|&b| b == 0), "bytes 50.. must be zero");
    }

    #[test]
    fn encode_aio_info_payload_uses_command_constant() {
        use crate::constants::CMD_TYPE_AIO_INFO;
        let device = DeviceId::from([1, 2, 3, 4, 5, 6]);
        let master = DeviceId::from([7, 8, 9, 10, 11, 12]);
        let payload = encode_aio_info_payload(&device, &master, 1, 0x08, &[0u8; 32]);
        assert_eq!(&payload[0..2], &CMD_TYPE_AIO_INFO);
    }
}
