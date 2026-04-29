/// Metadata for an RGB effect data transmission.
#[derive(Clone, Debug)]
pub struct RfDataMetadata {
    pub compressed_len: u32,
    pub total_frames: u16,
    pub led_num: u8,
    pub interval: f32,
    pub sub_interval: f32,
    pub total_sub_frame: u16,
    pub is_outer_match_max: bool,
}

/// Build the 240-byte metadata part (Part 0).
pub fn build_metadata_part(meta: &RfDataMetadata) -> [u8; 240] {
    let mut part = [0u8; 240];
    part[20..24].copy_from_slice(&meta.compressed_len.to_be_bytes());
    part[25..27].copy_from_slice(&meta.total_frames.to_be_bytes());
    part[27] = meta.led_num;
    let interval_int = meta.interval as u16;
    let interval_frac = ((meta.interval - interval_int as f32) * 100.0) as u8;
    part[32..34].copy_from_slice(&interval_int.to_be_bytes());
    part[34] = interval_frac;
    part[35..37].copy_from_slice(&(meta.sub_interval as u16).to_be_bytes());
    part[37] = if meta.is_outer_match_max { 1 } else { 0 };
    part[38..40].copy_from_slice(&meta.total_sub_frame.to_be_bytes());
    part
}

/// Build data parts from compressed data. Each 240 bytes: 20-byte header + 220-byte chunk.
///
/// Returns `Err` if the compressed data is too large (more than 255 parts).
pub fn build_data_parts(compressed: &[u8]) -> Result<Vec<[u8; 240]>, String> {
    let num_parts = compressed.len().div_ceil(220);
    if num_parts > 255 {
        return Err(format!("compressed data too large: {num_parts} parts exceeds u8 limit"));
    }
    Ok(compressed
        .chunks(220)
        .enumerate()
        .map(|(i, chunk)| {
            let mut part = [0u8; 240];
            part[18] = (i + 1) as u8;
            part[20..20 + chunk.len()].copy_from_slice(chunk);
            part
        })
        .collect())
}

/// Split a 240-byte RF data part into 64-byte USB packets.
/// Each: [0x10, seq, channel, group, 60_bytes_data]
pub fn rf_part_to_usb_packets(rf_part: &[u8; 240], group: u8, channel: u8) -> Vec<[u8; 64]> {
    rf_part
        .chunks(60)
        .enumerate()
        .map(|(seq, chunk)| {
            let mut pkt = [0u8; 64];
            pkt[0] = 0x10;
            pkt[1] = seq as u8;
            pkt[2] = channel;
            pkt[3] = group;
            pkt[4..4 + chunk.len()].copy_from_slice(chunk);
            pkt
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_part_size() {
        let meta = RfDataMetadata {
            compressed_len: 100,
            total_frames: 30,
            led_num: 24,
            interval: 20.0,
            sub_interval: 0.0,
            total_sub_frame: 0,
            is_outer_match_max: false,
        };
        let pkt = build_metadata_part(&meta);
        assert_eq!(pkt.len(), 240);
    }

    #[test]
    fn metadata_fields_encoded() {
        let meta = RfDataMetadata {
            compressed_len: 256,
            total_frames: 30,
            led_num: 24,
            interval: 20.5,
            sub_interval: 0.0,
            total_sub_frame: 0,
            is_outer_match_max: false,
        };
        let pkt = build_metadata_part(&meta);
        // compressed_len at bytes 20-23 big-endian
        assert_eq!(u32::from_be_bytes([pkt[20], pkt[21], pkt[22], pkt[23]]), 256);
        // total_frames at bytes 25-26 big-endian
        assert_eq!(u16::from_be_bytes([pkt[25], pkt[26]]), 30);
        // led_num at byte 27
        assert_eq!(pkt[27], 24);
        // interval int at bytes 32-33
        assert_eq!(u16::from_be_bytes([pkt[32], pkt[33]]), 20);
        // interval frac at byte 34
        assert_eq!(pkt[34], 50); // 0.5 * 100
    }

    #[test]
    fn data_parts_chunk_220() {
        let data = vec![0xAA; 500];
        let parts = build_data_parts(&data).unwrap();
        assert_eq!(parts.len(), 3); // ceil(500/220) = 3
        for part in &parts {
            assert_eq!(part.len(), 240);
        }
    }

    #[test]
    fn data_parts_numbering() {
        let data = vec![0xBB; 450];
        let parts = build_data_parts(&data).unwrap();
        assert_eq!(parts[0][18], 1); // part 1
        assert_eq!(parts[1][18], 2); // part 2
        assert_eq!(parts[2][18], 3); // part 3 (last partial)
    }

    #[test]
    fn data_parts_too_large_returns_err() {
        // 256 parts * 220 bytes = 56320 bytes
        let data = vec![0u8; 256 * 220];
        assert!(build_data_parts(&data).is_err());
    }

    #[test]
    fn usb_packets_from_rf() {
        let rf_part = [0u8; 240];
        let usb_pkts = rf_part_to_usb_packets(&rf_part, 1, 0x08);
        assert_eq!(usb_pkts.len(), 4); // 240/60 = 4
        for (i, pkt) in usb_pkts.iter().enumerate() {
            assert_eq!(pkt.len(), 64);
            assert_eq!(pkt[0], 0x10);
            assert_eq!(pkt[1], i as u8); // sequence
            assert_eq!(pkt[2], 0x08); // channel
            assert_eq!(pkt[3], 1); // group
        }
    }

    #[test]
    fn usb_packet_carries_60_bytes_payload() {
        let mut rf_part = [0u8; 240];
        rf_part[0] = 0x42; // first payload byte
        rf_part[59] = 0x43; // last byte of first USB packet payload
        rf_part[60] = 0x44; // first byte of second USB packet payload
        let usb_pkts = rf_part_to_usb_packets(&rf_part, 1, 0x08);
        assert_eq!(usb_pkts[0][4], 0x42); // first payload at offset 4
        assert_eq!(usb_pkts[0][63], 0x43); // last payload at offset 63
        assert_eq!(usb_pkts[1][4], 0x44); // second packet payload
    }
}
