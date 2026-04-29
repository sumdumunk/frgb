use frgb_protocol::decode::{decode_device_query, decode_tx_sync, DeviceQueryResponse, TxSyncResponse};

use crate::error::{CoreError, Result};
use crate::sequencer;
use crate::transport::Transport;

/// Discover the TX device by sending a sync command.
///
/// Returns the full sync response including device ID, firmware version, and clock.
pub fn discover_tx(transport: &impl Transport, channel: u8) -> Result<TxSyncResponse> {
    let resp = sequencer::send_tx_sync(transport, channel)?;
    decode_tx_sync(&resp).map_err(|e| CoreError::Protocol(format!("TX sync decode failed: {e}")))
}

/// Query all devices via the L-Connect device query protocol.
///
/// Sends device query to RX, reads multi-packet response, parses 42-byte records.
/// Returns the full response including all device records.
pub fn discover_devices(transport: &impl Transport, page_count: u8) -> Result<DeviceQueryResponse> {
    let data = sequencer::query_devices(transport, page_count)?;
    Ok(decode_device_query(&data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock::MockTransport;

    #[test]
    fn discover_tx_returns_full_sync_info() {
        let mock = MockTransport::new();
        let mut resp = [0u8; 64];
        resp[0] = 0x11;
        resp[1..7].copy_from_slice(&[0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        // clock at bytes 7-10
        resp[7] = 0x00;
        resp[8] = 0x01;
        resp[9] = 0x00;
        resp[10] = 0x00;
        // firmware at bytes 11-12
        resp[11] = 0x01;
        resp[12] = 0x05;
        mock.queue_read(resp);

        let sync = discover_tx(&mock, 0x08).unwrap();
        assert_eq!(sync.tx_device_id.to_hex(), "297a84e566e4");
        assert_eq!(sync.firmware_version, 0x0105);
        assert_eq!(sync.system_clock_ms, 40960);
    }

    #[test]
    fn discover_tx_fails_on_wrong_header() {
        let mock = MockTransport::new();
        let mut resp = [0u8; 64];
        resp[0] = 0x10;
        mock.queue_read(resp);

        assert!(discover_tx(&mock, 0x08).is_err());
    }

    /// Build a mock multi-packet response containing one 42-byte device record.
    /// The response needs to span 7 packets (434 bytes for page_count=1).
    fn build_mock_device_response(
        mac: [u8; 6],
        master_mac: [u8; 6],
        channel: u8,
        group: u8,
        dev_type: u8,
        fan_num: u8,
        rpm0: u16,
    ) -> Vec<[u8; 64]> {
        // Build a flat buffer: header (4 bytes) + 1 record (42 bytes) + padding
        let mut buf = vec![0u8; 434];
        buf[0] = 0x10; // header byte 0
        buf[1] = 1; // num_devices
        buf[2] = 0; // no firmware version
        buf[3] = 0;

        let r = 4; // record start
        buf[r..r + 6].copy_from_slice(&mac);
        buf[r + 6..r + 12].copy_from_slice(&master_mac);
        buf[r + 12] = channel;
        buf[r + 13] = group;
        buf[r + 18] = dev_type;
        buf[r + 19] = fan_num;
        // RPM fan 0 at offset 28 (BE)
        buf[r + 28] = (rpm0 >> 8) as u8;
        buf[r + 29] = (rpm0 & 0xFF) as u8;
        buf[r + 41] = 0x1C; // delimiter

        // Split into 64-byte packets (ceil(434/64) = 7 packets)
        let mut packets = Vec::new();
        for chunk in buf.chunks(64) {
            let mut pkt = [0u8; 64];
            pkt[..chunk.len()].copy_from_slice(chunk);
            packets.push(pkt);
        }
        packets
    }

    #[test]
    fn discover_devices_parses_one_record() {
        let mock = MockTransport::new();
        let packets = build_mock_device_response(
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1], // mac
            [0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4], // master_mac
            0x08,                                 // channel
            2,                                    // group
            20,                                   // dev_type = SLV3Fan
            3,                                    // fan_num
            1200,                                 // rpm0
        );
        for pkt in packets {
            mock.queue_read(pkt);
        }

        let resp = discover_devices(&mock, 1).unwrap();
        assert_eq!(resp.num_devices, 1);
        assert_eq!(resp.records.len(), 1);

        let rec = &resp.records[0];
        assert_eq!(rec.mac_addr.to_hex(), "c8b4ef6232e1");
        assert_eq!(rec.master_mac_addr.to_hex(), "297a84e566e4");
        assert_eq!(rec.channel, 0x08);
        assert_eq!(rec.group, 2);
        assert_eq!(rec.dev_type, 20);
        assert_eq!(rec.fan_num, 3);
        assert_eq!(rec.fans_speed[0], 1200);
    }

    #[test]
    fn discover_devices_handles_no_response() {
        let mock = MockTransport::new();
        // No packets queued → timeout immediately
        let resp = discover_devices(&mock, 1).unwrap();
        assert_eq!(resp.num_devices, 0);
        assert!(resp.records.is_empty());
    }
}
