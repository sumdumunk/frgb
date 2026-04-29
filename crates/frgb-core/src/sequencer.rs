use std::time::Duration;

use crate::error::Result;
use crate::transport::{Transport, PACKET_SIZE};
use frgb_protocol::encode;

/// Inter-command delay (15ms) — between command writes and follow-ups.
pub const DELAY_COMMAND: Duration = Duration::from_millis(15);

/// Delay after TX sync or RX status queries (50ms).
pub const DELAY_SYNC: Duration = Duration::from_millis(50);

/// Delay between setup commands — ring/zone select (20ms).
pub const DELAY_SETUP: Duration = Duration::from_millis(20);

/// Delay between RF frame retransmissions (20ms).
pub const DELAY_RF_REPEAT: Duration = Duration::from_millis(20);

/// Send a command packet followed by 3 follow-up packets with delays.
pub fn send_with_followups(transport: &impl Transport, cmd: &[u8], group: u8, channel: u8) -> Result<()> {
    transport.write(cmd)?;
    transport.sleep(DELAY_COMMAND);

    for seq in 1..=3u8 {
        let followup = encode::encode_followup(group, seq, channel);
        transport.write(&followup)?;
        transport.sleep(DELAY_COMMAND);
    }

    Ok(())
}

/// Send a command packet repeated `count` times, each with 3 follow-ups.
pub fn send_repeated_with_followups(
    transport: &impl Transport,
    cmd: &[u8],
    count: u8,
    group: u8,
    channel: u8,
) -> Result<()> {
    for _ in 0..count {
        send_with_followups(transport, cmd, group, channel)?;
    }
    Ok(())
}

/// Send TX sync and read the response.
///
/// Sends `[0x11, channel, 0, ...]` to TX, reads 64-byte response.
pub fn send_tx_sync(transport: &impl Transport, channel: u8) -> Result<[u8; PACKET_SIZE]> {
    let pkt = encode::encode_tx_sync(channel);
    transport.write(&pkt)?;
    transport.sleep(DELAY_SYNC);
    transport.read(Duration::from_millis(500))
}

/// Send L-Connect device query to RX and read multi-packet response.
///
/// Sends GetDev(16, page_cnt), reads 434 * page_count bytes = ceil(434*page_count/64) USB packets.
/// Each page holds up to 10 device records.
///
/// Returns the raw concatenated response bytes for parsing by decode_device_query.
pub fn query_devices(transport: &impl Transport, page_count: u8) -> Result<Vec<u8>> {
    let pkt = encode::encode_device_query(page_count);
    transport.write(&pkt)?;

    // The L-Connect protocol reads exactly 434 * page_count bytes (ceil(434/64) = 7 packets per page).
    // The wireless receiver needs time to poll all fans before it can respond.
    // The first packet may take longer (up to 500ms) while subsequent packets
    // arrive quickly once the receiver starts transmitting.
    let total_bytes = 434 * page_count as usize;
    let num_packets = total_bytes.div_ceil(PACKET_SIZE);

    let mut buf = Vec::with_capacity(total_bytes);
    for i in 0..num_packets {
        // First packet: long timeout (receiver needs to poll all fans).
        // Subsequent packets: short timeout (data is already buffered).
        let timeout = if i == 0 {
            Duration::from_millis(2000)
        } else {
            Duration::from_millis(500)
        };
        match transport.read(timeout) {
            Ok(pkt) => buf.extend_from_slice(&pkt),
            Err(_) => break,
        }
    }

    Ok(buf)
}

/// Send a 240-byte RF payload to the TX device using 4-packet framing.
///
/// Splits 240 bytes into 4 × 60-byte chunks, each prepended with
/// `[0x10, seq, channel, rx_type]` to form a 64-byte USB packet.
/// Packets are sent sequentially with no delay between them.
pub fn send_rf_data(transport: &impl Transport, channel: u8, rx_type: u8, payload: &[u8; 240]) -> Result<()> {
    let mut pkt = [0u8; PACKET_SIZE];
    pkt[0] = 0x10;
    pkt[2] = channel;
    pkt[3] = rx_type;

    for seq in 0..4u8 {
        pkt[1] = seq;
        let offset = seq as usize * 60;
        pkt[4..64].copy_from_slice(&payload[offset..offset + 60]);
        transport.write(&pkt)?;
        transport.sleep(Duration::from_millis(1));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock::MockTransport;

    #[test]
    fn send_with_followups_sends_4_packets() {
        let mock = MockTransport::new();
        let cmd = [0x10u8; 64];
        send_with_followups(&mock, &cmd, 1, 0x08).unwrap();
        let packets = mock.written_packets();
        assert_eq!(packets.len(), 4);
    }

    #[test]
    fn send_with_followups_correct_sequence() {
        let mock = MockTransport::new();
        let cmd = [0x10u8; 64];
        send_with_followups(&mock, &cmd, 2, 0x08).unwrap();
        let packets = mock.written_packets();
        assert_eq!(packets[0], cmd.to_vec());
        assert_eq!(packets[1][0], 0x10);
        assert_eq!(packets[1][1], 1); // seq=1
        assert_eq!(packets[1][2], 0x08); // channel
        assert_eq!(packets[1][3], 2); // group
        assert_eq!(packets[2][1], 2); // seq=2
        assert_eq!(packets[3][1], 3); // seq=3
    }

    #[test]
    fn send_with_followups_has_correct_delays() {
        let mock = MockTransport::new();
        let cmd = [0x10u8; 64];
        send_with_followups(&mock, &cmd, 1, 0x08).unwrap();
        let sleeps = mock.sleep_durations();
        assert_eq!(sleeps.len(), 4);
        for s in &sleeps {
            assert_eq!(*s, DELAY_COMMAND);
        }
    }

    #[test]
    fn send_repeated_with_followups_correct_count() {
        let mock = MockTransport::new();
        let cmd = [0x10u8; 64];
        send_repeated_with_followups(&mock, &cmd, 4, 1, 0x08).unwrap();
        let packets = mock.written_packets();
        assert_eq!(packets.len(), 16);
    }

    #[test]
    fn send_tx_sync_writes_and_reads() {
        let mock = MockTransport::new();
        let mut resp = [0u8; 64];
        resp[0] = 0x11;
        resp[1..7].copy_from_slice(&[0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        mock.queue_read(resp);

        let result = send_tx_sync(&mock, 0x08).unwrap();
        assert_eq!(result[0], 0x11);

        let packets = mock.written_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][0], 0x11);
        assert_eq!(packets[0][1], 0x08);

        let sleeps = mock.sleep_durations();
        assert_eq!(sleeps[0], DELAY_SYNC);
    }

    #[test]
    fn send_tx_sync_uses_channel() {
        let mock = MockTransport::new();
        let mut resp = [0u8; 64];
        resp[0] = 0x11;
        mock.queue_read(resp);

        send_tx_sync(&mock, 0x0B).unwrap();
        let packets = mock.written_packets();
        assert_eq!(packets[0][1], 0x0B);
    }

    #[test]
    fn query_devices_sends_query_and_reads_packets() {
        let mock = MockTransport::new();
        // page_count=1 → 434 bytes → ceil(434/64) = 7 packets
        for _ in 0..7 {
            mock.queue_read([0u8; 64]);
        }

        let buf = query_devices(&mock, 1).unwrap();
        // Should have read 7 × 64 = 448 bytes
        assert_eq!(buf.len(), 448);

        // Should have written 1 query packet
        let packets = mock.written_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][0], 0x10); // CMD_QUERY
        assert_eq!(packets[0][1], 1); // page_count
    }

    #[test]
    fn query_devices_handles_partial_read() {
        let mock = MockTransport::new();
        // Only 3 packets available, then timeout
        for _ in 0..3 {
            mock.queue_read([0x10; 64]);
        }

        let buf = query_devices(&mock, 1).unwrap();
        assert_eq!(buf.len(), 192); // 3 × 64
    }

    #[test]
    fn send_rf_data_sends_4_packets() {
        let mock = MockTransport::new();
        let payload = [0xAA; 240];
        send_rf_data(&mock, 0x08, 2, &payload).unwrap();

        let packets = mock.written_packets();
        assert_eq!(packets.len(), 4);
    }

    #[test]
    fn send_rf_data_correct_headers() {
        let mock = MockTransport::new();
        let payload = [0u8; 240];
        send_rf_data(&mock, 0x08, 3, &payload).unwrap();

        let packets = mock.written_packets();
        for (seq, pkt) in packets.iter().enumerate() {
            assert_eq!(pkt[0], 0x10); // command byte
            assert_eq!(pkt[1], seq as u8); // sequence 0,1,2,3
            assert_eq!(pkt[2], 0x08); // channel
            assert_eq!(pkt[3], 3); // rx_type (group)
        }
    }

    #[test]
    fn send_rf_data_splits_payload_correctly() {
        let mock = MockTransport::new();
        // Payload with distinct bytes to verify splitting
        let mut payload = [0u8; 240];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte = i as u8;
        }
        send_rf_data(&mock, 0x08, 1, &payload).unwrap();

        let packets = mock.written_packets();
        // Packet 0: payload[0..60] at pkt[4..64]
        assert_eq!(packets[0][4], 0); // payload[0]
        assert_eq!(packets[0][63], 59); // payload[59]
                                        // Packet 1: payload[60..120] at pkt[4..64]
        assert_eq!(packets[1][4], 60); // payload[60]
        assert_eq!(packets[1][63], 119);
        // Packet 2: payload[120..180]
        assert_eq!(packets[2][4], 120);
        // Packet 3: payload[180..240]
        assert_eq!(packets[3][4], 180);
        assert_eq!(packets[3][63], 239); // payload[239]
    }

    #[test]
    fn send_rf_data_inter_packet_delay() {
        // 1ms delay between each of the 4 packets (matches reference project)
        let mock = MockTransport::new();
        let payload = [0u8; 240];
        send_rf_data(&mock, 0x08, 1, &payload).unwrap();

        let sleeps = mock.sleep_durations();
        assert_eq!(sleeps.len(), 4);
        assert!(sleeps.iter().all(|d| d.as_millis() == 1));
    }
}
